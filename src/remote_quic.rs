//! QUIC direct transport for the remote daemon and direct-client RPCs.
//!
//! This carries the existing framed remote protocol over a single QUIC
//! bidirectional stream. The wire format stays unchanged; only the transport
//! substrate differs from the local Unix-socket lane.

use std::collections::BTreeSet;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::{Arc, Mutex, mpsc};

use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use quinn::{ClientConfig, Connection, Endpoint, EndpointConfig, ServerConfig, TokioRuntime};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, RootCertStore, SignatureScheme};
use tokio::runtime::{Builder, Runtime};

use crate::remote::RemoteCmd;
use crate::remote_direct_transport::DirectTransportClient;
use crate::remote_listener::run_remote_client_connection;
use crate::remote_state::State;

pub(crate) const TRANSPORT_LABEL_QUIC: &str = "quic";
const QUIC_ALPN: &[u8] = b"boo-remote";

#[derive(Clone)]
pub(crate) struct QuicServerHandle {
    _runtime: Arc<Runtime>,
}

pub(crate) fn start_quic_listener(
    bind_address: &str,
    port: u16,
    state: Arc<Mutex<State>>,
    cmd_tx: mpsc::Sender<RemoteCmd>,
) -> io::Result<QuicServerHandle> {
    let runtime = Arc::new(
        Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(io::Error::other)?,
    );
    let server_config = make_server_config().map_err(io::Error::other)?;
    let bind_addrs = resolve_socket_addrs(bind_address, port)?;
    for bind_addr in bind_addrs {
        let udp_socket = UdpSocket::bind(bind_addr)?;
        udp_socket.set_nonblocking(true)?;
        let endpoint = {
            let _guard = runtime.enter();
            Endpoint::new(
                EndpointConfig::default(),
                Some(server_config.clone()),
                udp_socket,
                Arc::new(TokioRuntime),
            )
            .map_err(io::Error::other)?
        };
        let runtime_for_task = Arc::clone(&runtime);
        let state = Arc::clone(&state);
        let cmd_tx = cmd_tx.clone();
        runtime.spawn(async move {
            while let Some(connecting) = endpoint.accept().await {
                let state = Arc::clone(&state);
                let cmd_tx = cmd_tx.clone();
                let runtime = Arc::clone(&runtime_for_task);
                tokio::spawn(async move {
                    let Ok(connection) = connecting.await else {
                        return;
                    };
                    let Ok((send, recv)) = connection.accept_bi().await else {
                        return;
                    };
                    let reader = QuicRecvReader {
                        recv,
                        runtime: Arc::clone(&runtime),
                        _connection: connection.clone(),
                    };
                    let writer = QuicSendWriter {
                        send,
                        runtime,
                        _connection: connection,
                    };
                    std::thread::spawn(move || {
                        run_remote_client_connection(
                            reader,
                            writer,
                            state,
                            cmd_tx,
                            TRANSPORT_LABEL_QUIC,
                        );
                    });
                });
            }
        });
    }
    Ok(QuicServerHandle { _runtime: runtime })
}

pub(crate) fn connect_direct(
    host: &str,
    port: u16,
) -> Result<DirectTransportClient<QuicDirectStream>, String> {
    let addr = (host, port)
        .to_socket_addrs()
        .map_err(|error| format!("failed to resolve {host}:{port}: {error}"))?
        .next()
        .ok_or_else(|| format!("no address records for {host}:{port}"))?;
    let runtime = Arc::new(
        Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| format!("failed to start quic runtime: {error}"))?,
    );
    let bind_addr = if addr.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let local_bind_addr: SocketAddr = bind_addr
        .parse()
        .map_err(|error| format!("invalid quic bind address {bind_addr}: {error}"))?;
    let udp_socket = UdpSocket::bind(local_bind_addr)
        .map_err(|error| format!("failed to bind quic socket: {error}"))?;
    udp_socket
        .set_nonblocking(true)
        .map_err(|error| format!("failed to configure quic socket: {error}"))?;
    let mut endpoint = {
        let _guard = runtime.enter();
        Endpoint::new(
            EndpointConfig::default(),
            None,
            udp_socket,
            Arc::new(TokioRuntime),
        )
        .map_err(|error| format!("failed to create quic client endpoint: {error}"))?
    };
    endpoint.set_default_client_config(make_client_config()?);
    let connection = runtime.block_on(async {
        endpoint
            .connect(addr, "boo-remote")
            .map_err(|error| format!("failed to start quic connection to {host}:{port}: {error}"))?
            .await
            .map_err(|error| {
                format!("failed to establish quic connection to {host}:{port}: {error}")
            })
    })?;
    let (send, recv) = runtime
        .block_on(connection.open_bi())
        .map_err(|error| format!("failed to open quic stream to {host}:{port}: {error}"))?;
    DirectTransportClient::connect_over_stream(
        QuicDirectStream {
            send: Mutex::new(send),
            recv: Mutex::new(recv),
            runtime,
            _endpoint: endpoint,
            _connection: connection,
        },
        host.to_string(),
        port,
    )
}

fn resolve_socket_addrs(bind_address: &str, port: u16) -> io::Result<Vec<SocketAddr>> {
    let mut addrs = BTreeSet::new();
    for addr in (bind_address, port).to_socket_addrs()? {
        addrs.insert(addr);
    }

    if bind_address == "0.0.0.0" {
        addrs.insert(SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port));
    } else if bind_address == "::" || bind_address == "[::]" {
        addrs.insert(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port));
    }

    if addrs.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            "no bind address records",
        ));
    }

    Ok(addrs.into_iter().collect())
}

fn make_server_config() -> Result<ServerConfig, String> {
    let cert = rcgen::generate_simple_self_signed(vec!["boo-remote".to_string()])
        .map_err(|error| format!("failed to generate quic certificate: {error}"))?;
    let cert_der = cert.cert.der().clone();
    let key_der = PrivateKeyDer::from(PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der()));
    let mut server_crypto = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .map_err(|error| format!("failed to configure quic server certificate: {error}"))?;
    server_crypto.alpn_protocols = vec![QUIC_ALPN.to_vec()];
    let mut server_config = ServerConfig::with_crypto(Arc::new(
        QuicServerConfig::try_from(server_crypto)
            .map_err(|error| format!("failed to build quic server crypto: {error}"))?,
    ));
    let transport = Arc::get_mut(&mut server_config.transport)
        .ok_or_else(|| "quic transport config unexpectedly shared".to_string())?;
    transport.max_concurrent_bidi_streams(16_u32.into());
    Ok(server_config)
}

fn make_client_config() -> Result<ClientConfig, String> {
    let provider = rustls::crypto::ring::default_provider();
    let mut client_crypto = rustls::ClientConfig::builder_with_provider(provider.into())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|error| format!("failed to configure quic tls version: {error}"))?
        .with_root_certificates(RootCertStore::empty())
        .with_no_client_auth();
    client_crypto.alpn_protocols = vec![QUIC_ALPN.to_vec()];
    client_crypto
        .dangerous()
        .set_certificate_verifier(Arc::new(SkipServerVerification::new()));
    let quic_crypto = QuicClientConfig::try_from(client_crypto)
        .map_err(|error| format!("failed to build quic client crypto: {error}"))?;
    Ok(ClientConfig::new(Arc::new(quic_crypto)))
}

#[derive(Debug)]
struct SkipServerVerification {
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl SkipServerVerification {
    fn new() -> Self {
        Self {
            provider: rustls::crypto::ring::default_provider().into(),
        }
    }
}

impl ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

pub(crate) struct QuicDirectStream {
    send: Mutex<quinn::SendStream>,
    recv: Mutex<quinn::RecvStream>,
    runtime: Arc<Runtime>,
    _endpoint: Endpoint,
    _connection: Connection,
}

impl Read for QuicDirectStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut recv = self.recv.lock().expect("quic recv mutex poisoned");
        match self.runtime.block_on(recv.read(buf)) {
            Ok(Some(read)) => Ok(read),
            Ok(None) => Ok(0),
            Err(error) => Err(io::Error::other(error)),
        }
    }
}

impl Write for QuicDirectStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut send = self.send.lock().expect("quic send mutex poisoned");
        self.runtime
            .block_on(send.write_all(buf))
            .map_err(io::Error::other)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct QuicRecvReader {
    recv: quinn::RecvStream,
    runtime: Arc<Runtime>,
    _connection: Connection,
}

impl Read for QuicRecvReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.runtime.block_on(self.recv.read(buf)) {
            Ok(Some(read)) => Ok(read),
            Ok(None) => Ok(0),
            Err(error) => Err(io::Error::other(error)),
        }
    }
}

struct QuicSendWriter {
    send: quinn::SendStream,
    runtime: Arc<Runtime>,
    _connection: Connection,
}

impl Write for QuicSendWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.runtime
            .block_on(self.send.write_all(buf))
            .map_err(io::Error::other)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
