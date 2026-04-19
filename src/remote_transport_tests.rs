#[cfg(test)]
mod tests {
    use crate::remote::{DirectTransportKind, DirectTransportSession, RemoteCmd};
    use crate::remote_identity::{
        DaemonIdentityMaterial, build_remote_server_tls_config, derive_identity_id,
        load_or_create_daemon_identity_material_at,
    };
    use crate::remote_listener::{handle_quic_incoming, serve_incoming_tcp_client};
    use crate::remote_quic::{QuicListener, bind_quic_listener, build_quic_server_config, shared_quic_runtime};
    use crate::remote_state::{
        AUTH_CHALLENGE_WINDOW, ClientState, DIRECT_CLIENT_HEARTBEAT_WINDOW, REMOTE_READ_TIMEOUT,
        State, should_disconnect_idle_client,
    };
    use crate::remote_transport::{QuicClientStream, TlsClientStream};
    use crate::remote_wire::{
        MessageType, REMOTE_CAPABILITY_QUIC_DIRECT_TRANSPORT, REMOTE_CAPABILITY_TCP_TLS_TRANSPORT,
        decode_auth_ok_payload, encode_message, read_message, validate_auth_ok_payload,
    };
    use std::collections::HashMap;
    use std::io::Write;
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::path::PathBuf;
    use std::sync::{Arc, LazyLock, Mutex, MutexGuard, mpsc};
    use std::time::{Duration, Instant};

    static TLS_TEST_MUTEX: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    fn lock_tls_test() -> MutexGuard<'static, ()> {
        TLS_TEST_MUTEX.lock().expect("tls test mutex poisoned")
    }

    #[derive(Debug)]
    struct TrustAllServerCertVerifier {
        provider: Arc<rustls::crypto::CryptoProvider>,
    }

    impl rustls::client::danger::ServerCertVerifier for TrustAllServerCertVerifier {
        fn verify_server_cert(
            &self,
            _end_entity: &rustls::pki_types::CertificateDer<'_>,
            _intermediates: &[rustls::pki_types::CertificateDer<'_>],
            _server_name: &rustls::pki_types::ServerName<'_>,
            _ocsp_response: &[u8],
            _now: rustls::pki_types::UnixTime,
        ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            message: &[u8],
            cert: &rustls::pki_types::CertificateDer<'_>,
            dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
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
            cert: &rustls::pki_types::CertificateDer<'_>,
            dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            rustls::crypto::verify_tls13_signature(
                message,
                cert,
                dss,
                &self.provider.signature_verification_algorithms,
            )
        }

        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            self.provider.signature_verification_algorithms.supported_schemes()
        }
    }

    fn unique_identity_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "boo-remote-daemon-identity-{label}-{}-{nanos}",
            std::process::id()
        ))
    }

    fn build_test_trust_all_client_config() -> rustls::ClientConfig {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        rustls::ClientConfig::builder_with_provider(Arc::clone(&provider))
            .with_safe_default_protocol_versions()
            .expect("client tls protocol versions")
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(TrustAllServerCertVerifier { provider }))
            .with_no_client_auth()
    }

    fn start_tls_test_server(
        material: &DaemonIdentityMaterial,
        tls_config: Arc<rustls::ServerConfig>,
    ) -> (SocketAddr, std::thread::JoinHandle<()>) {
        start_tls_test_server_with_auth(material, tls_config, None)
    }

    fn start_tls_test_server_with_auth(
        material: &DaemonIdentityMaterial,
        tls_config: Arc<rustls::ServerConfig>,
        auth_key: Option<&[u8]>,
    ) -> (SocketAddr, std::thread::JoinHandle<()>) {
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::new(),
            revivable_attachments: HashMap::new(),
            auth_key: auth_key.map(<[u8]>::to_vec),
            server_identity_id: material.identity_id.clone(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let _ = stream.set_read_timeout(Some(REMOTE_READ_TIMEOUT));
            serve_incoming_tcp_client(stream, tls_config, state, cmd_tx);
        });
        std::mem::forget(cmd_rx);
        (addr, handle)
    }

    fn start_quic_test_server(
        material: &DaemonIdentityMaterial,
        auth_key: Option<&[u8]>,
    ) -> (SocketAddr, QuicListener, mpsc::Receiver<RemoteCmd>) {
        let tls_config = build_remote_server_tls_config(material).expect("build server tls config");
        let server_config = build_quic_server_config(tls_config).expect("quic config");
        let bind_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let listener = bind_quic_listener(bind_addr, server_config).expect("bind quic");
        let addr = listener.local_addr();

        let state = Arc::new(Mutex::new(State {
            clients: HashMap::new(),
            revivable_attachments: HashMap::new(),
            auth_key: auth_key.map(<[u8]>::to_vec),
            server_identity_id: material.identity_id.clone(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let (cmd_tx, cmd_rx) = mpsc::channel();

        let endpoint = listener.endpoint();
        let runtime = shared_quic_runtime().expect("runtime");
        runtime.spawn(async move {
            while let Some(incoming) = endpoint.accept().await {
                let state = Arc::clone(&state);
                let cmd_tx = cmd_tx.clone();
                tokio::spawn(async move {
                    handle_quic_incoming(incoming, state, cmd_tx).await;
                });
            }
        });

        (addr, listener, cmd_rx)
    }

    #[test]
    fn direct_tls_client_connects_with_matching_pin() {
        let _guard = lock_tls_test();
        let dir = unique_identity_dir("tls-pin-ok");
        let _ = std::fs::remove_dir_all(&dir);
        let material = load_or_create_daemon_identity_material_at(&dir);
        let tls_config = build_remote_server_tls_config(&material).expect("build server tls config");
        let (addr, server_handle) = start_tls_test_server(&material, tls_config);

        let session = DirectTransportSession::<TlsClientStream>::connect_tls(
            &addr.ip().to_string(),
            addr.port(),
            None,
            &material.identity_id,
        )
        .expect("tls connect");

        assert!(!session.auth_required);
        assert_eq!(session.server_identity_id.as_deref(), Some(material.identity_id.as_str()));
        assert_ne!(session.capabilities & REMOTE_CAPABILITY_TCP_TLS_TRANSPORT, 0);

        drop(session);
        let _ = server_handle.join();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn direct_quic_client_connects_with_matching_pin() {
        let dir = unique_identity_dir("quic-pin-ok");
        let _ = std::fs::remove_dir_all(&dir);
        let material = load_or_create_daemon_identity_material_at(&dir);
        let (addr, _listener, _cmd_rx) = start_quic_test_server(&material, None);

        let session = DirectTransportSession::<QuicClientStream>::connect_quic(
            &addr.ip().to_string(),
            addr.port(),
            None,
            &material.identity_id,
        )
        .expect("quic connect");

        assert!(!session.auth_required);
        assert_eq!(session.server_identity_id.as_deref(), Some(material.identity_id.as_str()));
        assert_ne!(session.capabilities & REMOTE_CAPABILITY_QUIC_DIRECT_TRANSPORT, 0);

        drop(session);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn probe_selected_direct_transport_tls_dispatches_quic_over_quinn() {
        let dir = unique_identity_dir("probe-quic");
        let _ = std::fs::remove_dir_all(&dir);
        let material = load_or_create_daemon_identity_material_at(&dir);
        let (addr, _listener, _cmd_rx) = start_quic_test_server(&material, None);

        let summary = crate::remote::probe_selected_direct_transport_tls(
            DirectTransportKind::QuicDirect,
            &addr.ip().to_string(),
            addr.port(),
            None,
            &material.identity_id,
        )
        .expect("upgrade probe over QUIC");

        assert_eq!(summary.selected_transport, DirectTransportKind::QuicDirect);
        assert_eq!(
            summary.probe.server_identity_id.as_deref(),
            Some(material.identity_id.as_str())
        );
        assert_ne!(summary.probe.capabilities & REMOTE_CAPABILITY_QUIC_DIRECT_TRANSPORT, 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn direct_quic_client_rejects_mismatched_pin() {
        let dir = unique_identity_dir("quic-pin-mismatch");
        let _ = std::fs::remove_dir_all(&dir);
        let material = load_or_create_daemon_identity_material_at(&dir);
        let (addr, _listener, _cmd_rx) = start_quic_test_server(&material, None);

        let bogus_pin = derive_identity_id([0xCCu8; 32].as_slice());
        assert_ne!(bogus_pin, material.identity_id);

        let err = match DirectTransportSession::<QuicClientStream>::connect_quic(
            &addr.ip().to_string(),
            addr.port(),
            None,
            &bogus_pin,
        ) {
            Ok(_) => panic!("quic connect must fail on mismatched pin"),
            Err(err) => err,
        };
        assert!(
            err.to_lowercase().contains("quic")
                || err.to_lowercase().contains("handshake")
                || err.to_lowercase().contains("cert")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn direct_tls_client_succeeds_with_auth_key() {
        let _guard = lock_tls_test();
        let dir = unique_identity_dir("tls-auth-ok");
        let _ = std::fs::remove_dir_all(&dir);
        let material = load_or_create_daemon_identity_material_at(&dir);
        let tls_config = build_remote_server_tls_config(&material).expect("build server tls config");
        let (addr, server_handle) =
            start_tls_test_server_with_auth(&material, tls_config, Some(b"test-secret"));

        let session = DirectTransportSession::<TlsClientStream>::connect_tls(
            &addr.ip().to_string(),
            addr.port(),
            Some("test-secret"),
            &material.identity_id,
        )
        .expect("tls+auth connect");
        assert!(session.auth_required);
        assert_eq!(session.server_identity_id.as_deref(), Some(material.identity_id.as_str()));

        drop(session);
        let _ = server_handle.join();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn direct_tls_client_rejects_wrong_auth_key() {
        let _guard = lock_tls_test();
        let dir = unique_identity_dir("tls-auth-bad");
        let _ = std::fs::remove_dir_all(&dir);
        let material = load_or_create_daemon_identity_material_at(&dir);
        let tls_config = build_remote_server_tls_config(&material).expect("build server tls config");
        let (addr, server_handle) =
            start_tls_test_server_with_auth(&material, tls_config, Some(b"correct-key"));

        let err = match DirectTransportSession::<TlsClientStream>::connect_tls(
            &addr.ip().to_string(),
            addr.port(),
            Some("wrong-key"),
            &material.identity_id,
        ) {
            Ok(_) => panic!("wrong auth key must fail inside tls tunnel"),
            Err(err) => err,
        };
        assert!(err.to_lowercase().contains("auth"));

        let _ = server_handle.join();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn direct_tls_client_rejects_mismatched_pin() {
        let _guard = lock_tls_test();
        let dir = unique_identity_dir("tls-pin-mismatch");
        let _ = std::fs::remove_dir_all(&dir);
        let material = load_or_create_daemon_identity_material_at(&dir);
        let tls_config = build_remote_server_tls_config(&material).expect("build server tls config");
        let (addr, server_handle) = start_tls_test_server(&material, tls_config);

        let bogus_pin = derive_identity_id([0xAAu8; 32].as_slice());
        assert_ne!(bogus_pin, material.identity_id);

        let err = match DirectTransportSession::<TlsClientStream>::connect_tls(
            &addr.ip().to_string(),
            addr.port(),
            None,
            &bogus_pin,
        ) {
            Ok(_) => panic!("tls connect must fail on mismatched pin"),
            Err(error) => error,
        };
        assert!(err.contains("tls handshake") || err.contains("failed"));

        let _ = server_handle.join();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn should_disconnect_idle_client_enforces_absolute_auth_deadline() {
        let (outbound_tx, _outbound_rx) = mpsc::channel();
        let now = Instant::now();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                1,
                ClientState {
                    outbound: outbound_tx,
                    authenticated: false,
                    challenge: Some(crate::remote_state::AuthChallengeState {
                        bytes: [0u8; 32],
                        expires_at: now + AUTH_CHALLENGE_WINDOW,
                    }),
                    connected_at: now - AUTH_CHALLENGE_WINDOW - Duration::from_secs(1),
                    authenticated_at: None,
                    last_heartbeat_at: None,
                    attached_session: None,
                    attachment_id: None,
                    resume_token: None,
                    last_session_list_payload: None,
                    last_ui_runtime_state_payload: None,
                    last_ui_appearance_payload: None,
                    last_state: None,
                    pane_states: HashMap::new(),
                    latest_input_seq: None,
                    is_local: false,
                },
            )]),
            revivable_attachments: HashMap::new(),
            auth_key: Some(b"test-key".to_vec()),
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));

        assert_eq!(
            should_disconnect_idle_client(
                &state,
                1,
                AUTH_CHALLENGE_WINDOW,
                DIRECT_CLIENT_HEARTBEAT_WINDOW,
            ),
            Some("auth-timeout"),
        );
    }

    #[test]
    fn serve_incoming_tcp_client_completes_tls_handshake_and_auth() {
        let _guard = lock_tls_test();
        let dir = unique_identity_dir("tls-serve");
        let _ = std::fs::remove_dir_all(&dir);
        let material = load_or_create_daemon_identity_material_at(&dir);
        let expected_identity = material.identity_id.clone();
        let tls_config = build_remote_server_tls_config(&material).expect("build server tls config");

        let state = Arc::new(Mutex::new(State {
            clients: HashMap::new(),
            revivable_attachments: HashMap::new(),
            auth_key: None,
            server_identity_id: expected_identity.clone(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let (cmd_tx, _cmd_rx) = mpsc::channel();

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let server_handle = std::thread::spawn({
            let tls_config = Arc::clone(&tls_config);
            let state = Arc::clone(&state);
            move || {
                let (stream, _) = listener.accept().expect("accept");
                let _ = stream.set_read_timeout(Some(REMOTE_READ_TIMEOUT));
                serve_incoming_tcp_client(stream, tls_config, state, cmd_tx);
            }
        });

        let tcp = TcpStream::connect(addr).expect("tcp connect");
        tcp.set_read_timeout(Some(Duration::from_secs(30)))
            .expect("set read timeout");
        let client_config = build_test_trust_all_client_config();
        let server_name = rustls::pki_types::ServerName::try_from("boo-remote-daemon")
            .expect("parse server name");
        let client_conn =
            rustls::ClientConnection::new(Arc::new(client_config), server_name)
                .expect("client connection");
        let mut tls_stream = rustls::StreamOwned::new(client_conn, tcp);

        tls_stream
            .write_all(&encode_message(MessageType::Auth, &[]))
            .expect("send auth");
        let (ty, payload) = read_message(&mut tls_stream).expect("read auth reply");
        assert!(matches!(ty, MessageType::AuthOk), "got {ty:?}");

        validate_auth_ok_payload(&payload, false).expect("auth ok payload valid");
        let (_, capabilities, _, _, server_identity_id) =
            decode_auth_ok_payload(&payload).expect("decode auth ok");
        assert_eq!(server_identity_id.as_deref(), Some(expected_identity.as_str()));
        assert_ne!(capabilities & REMOTE_CAPABILITY_TCP_TLS_TRANSPORT, 0);

        {
            let state_guard = state.lock().expect("state lock");
            assert_eq!(state_guard.tls_clients.len(), 1);
        }

        drop(tls_stream);
        let _ = server_handle.join();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
