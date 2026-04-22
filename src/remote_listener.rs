//! Accept-loop and per-connection socket lifecycle for the remote daemon.
//!
//! Each accepted connection ends up in [`run_remote_client_connection`],
//! which registers a `ClientState`, spawns the writer thread, and hands
//! the reader off to the blocking `read_loop` in `remote_auth`.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Instant;

use crate::remote::RemoteCmd;
use crate::remote_auth::read_loop;
use crate::remote_batcher::writer_loop;
use crate::remote_state::{ClientRuntimeSubscription, ClientState, State};

/// Monotonic client-id allocator. Shared across transports so diagnostics can
/// cross-reference a client_id in the clients snapshot with the reader/writer
/// threads that serve it.
pub(crate) static NEXT_CLIENT_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) fn run_remote_client_connection<R, W>(
    reader: R,
    writer: W,
    state: Arc<Mutex<State>>,
    cmd_tx: mpsc::Sender<RemoteCmd>,
    transport_label: &'static str,
) where
    R: Read + Send + 'static,
    W: Write + Send + 'static,
{
    let (client_id, outbound_rx) = {
        let mut state = state.lock().expect("remote server state poisoned");
        let client_id = NEXT_CLIENT_ID.fetch_add(1, Ordering::Relaxed);
        let (outbound_tx, outbound_rx) = mpsc::channel();
        state.clients.insert(
            client_id,
            ClientState {
                outbound: outbound_tx,
                authenticated: true,
                connected_at: Instant::now(),
                authenticated_at: Some(Instant::now()),
                last_heartbeat_at: None,
                runtime_subscription: ClientRuntimeSubscription::detached(),
                attachment_lease: None,
                is_local: false,
            },
        );
        (client_id, outbound_rx)
    };
    log::info!(
        "remote client connected: client_id={client_id} transport={transport_label}"
    );

    std::thread::spawn(move || writer_loop(writer, outbound_rx, true, true));

    {
        let _ = cmd_tx.send(RemoteCmd::Connected { client_id });
        crate::notify_headless_wakeup();
    }
    read_loop(reader, client_id, Arc::clone(&state), cmd_tx);
}
