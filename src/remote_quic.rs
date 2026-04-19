//! QUIC direct transport for the native remote protocol.
//!
//! Design notes
//!
//! The existing remote subsystem is sync: `std::net::TcpListener`, blocking mpsc
//! channels, and per-connection OS threads running `read_loop` / `writer_loop`.
//! quinn is fundamentally async and tokio-driven, so we keep a *single* shared
//! tokio runtime on a dedicated thread and bridge to/from the sync world via two
//! channels per connection:
//!
//! - Inbound (quinn → sync reader): `std::sync::mpsc` carrying byte chunks.
//!   The async recv task pushes, the sync reader blocks on `recv()`.
//! - Outbound (sync writer → quinn): `tokio::sync::mpsc` bounded.
//!   The sync writer uses `blocking_send` from its OS thread; the async task
//!   drains into `quinn::SendStream::write_all`.
//!
//! The bridge exposes a `Read + Write + Send` handle (`QuicBridgeStream`) so the
//! existing `run_remote_client_session` / `DirectTransportSession::connect_over_stream`
//! machinery handles the protocol unchanged. That keeps the QUIC integration a
//! pure transport swap — the framing, auth, and session logic stay in `remote.rs`.
//!
//! Trust model reuses `PinnedSpkiServerCertVerifier` and the Phase 1 ed25519
//! cert directly — quinn accepts `rustls::ServerConfig`/`ClientConfig`, so QUIC
//! inherits the same SPKI pinning semantics as the TCP+TLS path.

use std::io::{self, Read, Write};
use std::sync::OnceLock;
use std::sync::mpsc as std_mpsc;

use tokio::runtime::Runtime;

/// Name prefix for tokio worker threads spawned for QUIC work. Makes runtime
/// tasks visible in profiler/debugger stacks without colliding with other
/// threads Boo spawns.
const QUIC_RUNTIME_THREAD_NAME: &str = "boo-quic";

/// Size of the outbound bounded channel carrying write-request buffers from the
/// sync writer into the async send loop. 32 hands out enough slack to let a
/// burst of terminal output queue without the sync writer blocking, while
/// keeping memory bounded under pathological backpressure.
const QUIC_OUTBOUND_CHANNEL_CAPACITY: usize = 32;

/// Process-global tokio runtime used for all QUIC endpoints in this process.
/// Lazily created so there is no tokio overhead in a build/run that never uses
/// QUIC. A single shared runtime keeps thread counts predictable; every QUIC
/// server and client task lives on it.
fn shared_quic_runtime() -> io::Result<&'static Runtime> {
    static RUNTIME: OnceLock<io::Result<Runtime>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name(QUIC_RUNTIME_THREAD_NAME)
                .enable_all()
                .build()
        })
        .as_ref()
        .map_err(|error| {
            io::Error::other(format!(
                "failed to initialize shared QUIC tokio runtime: {error}"
            ))
        })
}

/// Sync-facing bridge over a single QUIC bidirectional stream.
///
/// Reads come from `inbound` (fed by an async recv loop), writes go into
/// `outbound` (drained by an async send loop). The internal `pending_read`
/// holds any bytes left over from a recv chunk that was larger than the
/// caller's buffer.
pub(crate) struct QuicBridgeStream {
    inbound: std_mpsc::Receiver<io::Result<Vec<u8>>>,
    outbound: tokio::sync::mpsc::Sender<Vec<u8>>,
    pending_read: Vec<u8>,
}

impl QuicBridgeStream {
    pub(crate) fn new(
        inbound: std_mpsc::Receiver<io::Result<Vec<u8>>>,
        outbound: tokio::sync::mpsc::Sender<Vec<u8>>,
    ) -> Self {
        Self {
            inbound,
            outbound,
            pending_read: Vec::new(),
        }
    }
}

impl Read for QuicBridgeStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if !self.pending_read.is_empty() {
            let take = buf.len().min(self.pending_read.len());
            buf[..take].copy_from_slice(&self.pending_read[..take]);
            self.pending_read.drain(..take);
            return Ok(take);
        }
        match self.inbound.recv() {
            Ok(Ok(chunk)) => {
                if chunk.is_empty() {
                    return Ok(0);
                }
                let take = buf.len().min(chunk.len());
                buf[..take].copy_from_slice(&chunk[..take]);
                if take < chunk.len() {
                    self.pending_read.extend_from_slice(&chunk[take..]);
                }
                Ok(take)
            }
            Ok(Err(error)) => Err(error),
            Err(_) => Ok(0),
        }
    }
}

impl Write for QuicBridgeStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        self.outbound
            .blocking_send(buf.to_vec())
            .map_err(|_| io::Error::other("quic outbound channel closed"))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quic_bridge_stream_read_splits_large_chunks() {
        let (tx, rx) = std_mpsc::channel::<io::Result<Vec<u8>>>();
        let (out_tx, _out_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(1);
        let mut stream = QuicBridgeStream::new(rx, out_tx);

        tx.send(Ok(vec![1, 2, 3, 4, 5])).expect("seed chunk");
        drop(tx);

        let mut buf = [0u8; 2];
        assert_eq!(stream.read(&mut buf).expect("read 1"), 2);
        assert_eq!(&buf, &[1, 2]);
        assert_eq!(stream.read(&mut buf).expect("read 2"), 2);
        assert_eq!(&buf, &[3, 4]);
        assert_eq!(stream.read(&mut buf).expect("read 3"), 1);
        assert_eq!(buf[0], 5);
        assert_eq!(stream.read(&mut buf).expect("eof"), 0);
    }

    #[test]
    fn quic_bridge_stream_surfaces_inbound_errors() {
        let (tx, rx) = std_mpsc::channel::<io::Result<Vec<u8>>>();
        let (out_tx, _out_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(1);
        let mut stream = QuicBridgeStream::new(rx, out_tx);
        tx.send(Err(io::Error::from(io::ErrorKind::ConnectionReset)))
            .expect("seed err");

        let mut buf = [0u8; 8];
        let err = stream.read(&mut buf).expect_err("must surface error");
        assert_eq!(err.kind(), io::ErrorKind::ConnectionReset);
    }

    #[test]
    fn shared_quic_runtime_returns_same_instance() {
        let a = shared_quic_runtime().expect("runtime a") as *const _;
        let b = shared_quic_runtime().expect("runtime b") as *const _;
        assert_eq!(a, b, "runtime must be a single OnceLock-initialised instance");
    }
}
