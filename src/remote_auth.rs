//! Inbound message loop for the remote daemon: auth acknowledgement + dispatch.
//!
//! Two interacting pieces:
//! * [`handle_auth_message`] — accepts the protocol `Auth` message, flips
//!   `ClientState::authenticated`, and emits `AuthOk` with handshake metadata.
//! * [`read_loop`] — runs on the per-client reader thread. Pulls frames off the
//!   socket, routes `Auth` / `Heartbeat` inline, and translates every other
//!   message into a `RemoteCmd`.
//!
//! Policy lives in `remote_state` (the window constants); wire encoding lives
//! in `remote_wire`. This module is the glue between them.

use std::io::{self, Read};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Instant;

use crate::remote::RemoteCmd;
use crate::remote_batcher::OutboundMessage;
use crate::remote_state::{
    AUTH_CHALLENGE_WINDOW, DIRECT_CLIENT_HEARTBEAT_WINDOW, State, send_direct_error,
    send_direct_frame, should_disconnect_idle_client,
};
use crate::remote_wire::{
    MessageType, RemoteErrorCode, encode_auth_ok_payload, encode_message, parse_input_payload,
    parse_key_payload, parse_pane_id, parse_resize, parse_tab_id, read_message_retrying,
};

pub(crate) enum AuthHandling {
    Authenticated,
    Disconnect,
}

pub(crate) fn read_loop(
    mut stream: impl Read,
    client_id: u64,
    state: Arc<Mutex<State>>,
    cmd_tx: mpsc::Sender<RemoteCmd>,
) {
    loop {
        let mut scope =
            crate::profiling::scope("server.stream.read_message", crate::profiling::Kind::Io);
        let (ty, payload) = match read_message_retrying(&mut stream, 10) {
            Ok(message) => message,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                if let Some(reason) = should_disconnect_idle_client(
                    &state,
                    client_id,
                    AUTH_CHALLENGE_WINDOW,
                    DIRECT_CLIENT_HEARTBEAT_WINDOW,
                ) {
                    if reason == "heartbeat-timeout" {
                        log::warn!(
                            "remote direct client disconnected: client_id={client_id} reason={reason}"
                        );
                        send_direct_error(
                            &state,
                            client_id,
                            RemoteErrorCode::HeartbeatTimeout,
                            RemoteErrorCode::HeartbeatTimeout.default_message(),
                        );
                    } else {
                        log::warn!("remote auth failed: client_id={client_id} reason={reason}");
                        send_direct_frame(&state, client_id, MessageType::AuthFail, Vec::new());
                    }
                    break;
                }
                continue;
            }
            Err(_) => break,
        };
        scope.add_bytes(payload.len() as u64);
        let (authenticated, is_local) = {
            let state = state.lock().expect("remote server state poisoned");
            state
                .clients
                .get(&client_id)
                .map(|client| (client.authenticated, client.is_local))
                .unwrap_or((false, false))
        };

        if matches!(ty, MessageType::Auth) {
            match handle_auth_message(client_id, &payload, &state) {
                AuthHandling::Authenticated => {
                    let _ = cmd_tx.send(RemoteCmd::Connected { client_id });
                    crate::notify_headless_wakeup();
                }
                AuthHandling::Disconnect => break,
            }
            continue;
        }

        if !authenticated {
            send_direct_error(
                &state,
                client_id,
                RemoteErrorCode::AuthenticationFailed,
                "authentication required",
            );
            continue;
        }

        if matches!(ty, MessageType::Heartbeat) {
            let mut state_guard = state.lock().expect("remote server state poisoned");
            if let Some(client) = state_guard.clients.get_mut(&client_id) {
                client.last_heartbeat_at = Some(Instant::now());
            }
            drop(state_guard);
            send_direct_frame(&state, client_id, MessageType::HeartbeatAck, payload);
            continue;
        }

        let command = match ty {
            crate::remote_wire::MESSAGE_TYPE_LIST_TABS => Some(RemoteCmd::ListTabs { client_id }),
            MessageType::Create => parse_resize(&payload).map(|(cols, rows)| RemoteCmd::Create {
                client_id,
                cols,
                rows,
            }),
            MessageType::Input => {
                parse_input_payload(&payload, is_local).map(|(input_seq, bytes)| RemoteCmd::Input {
                    client_id,
                    bytes,
                    input_seq,
                })
            }
            MessageType::Key => {
                parse_key_payload(&payload, is_local).and_then(|(input_seq, payload)| {
                    String::from_utf8(payload)
                        .ok()
                        .map(|keyspec| RemoteCmd::Key {
                            client_id,
                            keyspec,
                            input_seq,
                        })
                })
            }
            MessageType::Resize => parse_resize(&payload).map(|(cols, rows)| RemoteCmd::Resize {
                client_id,
                cols,
                rows,
            }),
            MessageType::ExecuteCommand => String::from_utf8(payload)
                .ok()
                .map(|input| RemoteCmd::ExecuteCommand { client_id, input }),
            MessageType::AppKeyEvent => serde_json::from_slice::<crate::AppKeyEvent>(&payload)
                .ok()
                .map(|event| RemoteCmd::AppKeyEvent { client_id, event }),
            MessageType::AppMouseEvent => serde_json::from_slice::<crate::AppMouseEvent>(&payload)
                .ok()
                .map(|event| RemoteCmd::AppMouseEvent { client_id, event }),
            MessageType::AppAction => serde_json::from_slice::<crate::bindings::Action>(&payload)
                .ok()
                .map(|action| RemoteCmd::AppAction { client_id, action }),
            MessageType::FocusPane => {
                parse_pane_id(&payload).map(|pane_id| RemoteCmd::FocusPane { client_id, pane_id })
            }
            MessageType::Destroy => Some(RemoteCmd::Destroy {
                client_id,
                tab_id: parse_tab_id(&payload),
            }),
            MessageType::RuntimeAction => {
                crate::remote::decode_runtime_action_payload(&payload)
                    .ok()
                    .map(|(client_action_id, action)| RemoteCmd::RuntimeAction {
                        client_id,
                        client_action_id,
                        action,
                    })
            }
            MessageType::RenderAck => parse_render_ack(&payload).map(
                |(view_id, tab_id, pane_id, pane_revision, runtime_revision)| {
                    RemoteCmd::RenderAck {
                        client_id,
                        view_id,
                        tab_id,
                        pane_id,
                        pane_revision,
                        runtime_revision,
                    }
                },
            ),
            _ => None,
        };

        if let Some(command) = command {
            if cmd_tx.send(command).is_err() {
                break;
            }
            crate::notify_headless_wakeup();
        } else {
            send_direct_error(
                &state,
                client_id,
                RemoteErrorCode::Unknown,
                "invalid payload",
            );
        }
    }

    let mut state = state.lock().expect("remote server state poisoned");
    if let Some(client) = state.clients.remove(&client_id) {
        let _ = client;
        log::info!("remote client disconnected: client_id={client_id}");
    }
}

fn parse_render_ack(payload: &[u8]) -> Option<(u64, u32, u64, u64, u64)> {
    if payload.len() < 36 {
        return None;
    }
    Some((
        u64::from_le_bytes(payload[0..8].try_into().ok()?),
        u32::from_le_bytes(payload[8..12].try_into().ok()?),
        u64::from_le_bytes(payload[12..20].try_into().ok()?),
        u64::from_le_bytes(payload[20..28].try_into().ok()?),
        u64::from_le_bytes(payload[28..36].try_into().ok()?),
    ))
}

pub(crate) fn handle_auth_message(
    client_id: u64,
    _payload: &[u8],
    state: &Arc<Mutex<State>>,
) -> AuthHandling {
    let mut state = state.lock().expect("remote server state poisoned");
    let server_instance_id = state.server_instance_id.clone();
    let Some(client) = state.clients.get_mut(&client_id) else {
        return AuthHandling::Disconnect;
    };
    client.authenticated = true;
    client.authenticated_at = Some(Instant::now());
    log::info!("remote client authenticated: client_id={client_id} mode=tailnet-trust");
    let _ = client.outbound.send(OutboundMessage::Frame(encode_message(
        MessageType::AuthOk,
        &encode_auth_ok_payload(&server_instance_id),
    )));
    AuthHandling::Authenticated
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote_wire::{encode_message, read_message};
    use std::collections::{HashMap, VecDeque};

    struct TimeoutScriptedReader {
        chunks: VecDeque<Result<Vec<u8>, io::ErrorKind>>,
    }

    impl TimeoutScriptedReader {
        fn new(chunks: impl IntoIterator<Item = Result<Vec<u8>, io::ErrorKind>>) -> Self {
            Self {
                chunks: chunks.into_iter().collect(),
            }
        }
    }

    impl Read for TimeoutScriptedReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.chunks.pop_front() {
                Some(Ok(chunk)) => {
                    let len = chunk.len().min(buf.len());
                    buf[..len].copy_from_slice(&chunk[..len]);
                    if len < chunk.len() {
                        self.chunks.push_front(Ok(chunk[len..].to_vec()));
                    }
                    Ok(len)
                }
                Some(Err(kind)) => Err(io::Error::new(kind, "scripted timeout")),
                None => Ok(0),
            }
        }
    }

    use crate::remote_batcher::OutboundMessage;
    use crate::remote_state::ClientState;
    use std::sync::Mutex;
    use std::time::Duration;

    fn remote_client(
        outbound: mpsc::Sender<crate::remote_batcher::OutboundMessage>,
        authenticated: bool,
    ) -> ClientState {
        ClientState::test_client(outbound, authenticated, false)
    }

    #[test]
    fn read_loop_emits_list_tabs_for_authenticated_client() {
        let (outbound_tx, _outbound_rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(1, remote_client(outbound_tx, true))]),
            ..State::test_empty()
        }));
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let mut frames = Vec::new();
        frames.extend_from_slice(&encode_message(
            crate::remote_wire::MESSAGE_TYPE_LIST_TABS,
            &[],
        ));

        read_loop(std::io::Cursor::new(frames), 1, state, cmd_tx);

        match cmd_rx.recv().expect("remote command") {
            RemoteCmd::ListTabs { client_id } => assert_eq!(client_id, 1),
            other => panic!("unexpected remote command: {other:?}"),
        }
    }

    #[test]
    fn read_loop_decodes_runtime_action() {
        let (outbound_tx, _outbound_rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(1, remote_client(outbound_tx, true))]),
            ..State::test_empty()
        }));
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let payload = serde_json::to_vec(&crate::remote::RuntimeAction::FocusPane {
            view_id: 7,
            tab_id: 9,
            pane_id: 11,
        })
        .expect("encode runtime action");
        let frame = encode_message(MessageType::RuntimeAction, &payload);

        read_loop(std::io::Cursor::new(frame), 1, state, cmd_tx);

        match cmd_rx.recv().expect("runtime action command") {
            RemoteCmd::RuntimeAction {
                client_id,
                client_action_id,
                action,
            } => {
                assert_eq!(client_id, 1);
                assert_eq!(client_action_id, None);
                assert_eq!(
                    action,
                    crate::remote::RuntimeAction::FocusPane {
                        view_id: 7,
                        tab_id: 9,
                        pane_id: 11,
                    }
                );
            }
            other => panic!("unexpected remote command: {other:?}"),
        }
    }

    #[test]
    fn read_loop_decodes_runtime_action_envelope() {
        let (outbound_tx, _outbound_rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(1, remote_client(outbound_tx, true))]),
            ..State::test_empty()
        }));
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let payload = serde_json::to_vec(&crate::remote::RuntimeActionEnvelope {
            client_action_id: 42,
            action: crate::remote::RuntimeAction::Noop { view_id: 7 },
        })
        .expect("encode runtime action envelope");
        let frame = encode_message(MessageType::RuntimeAction, &payload);

        read_loop(std::io::Cursor::new(frame), 1, state, cmd_tx);

        match cmd_rx.recv().expect("runtime action command") {
            RemoteCmd::RuntimeAction {
                client_id,
                client_action_id,
                action,
            } => {
                assert_eq!(client_id, 1);
                assert_eq!(client_action_id, Some(42));
                assert_eq!(action, crate::remote::RuntimeAction::Noop { view_id: 7 });
            }
            other => panic!("unexpected remote command: {other:?}"),
        }
    }

    #[test]
    fn read_loop_decodes_render_ack() {
        let (outbound_tx, _outbound_rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(1, remote_client(outbound_tx, true))]),
            ..State::test_empty()
        }));
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let mut payload = Vec::new();
        payload.extend_from_slice(&7_u64.to_le_bytes());
        payload.extend_from_slice(&9_u32.to_le_bytes());
        payload.extend_from_slice(&11_u64.to_le_bytes());
        payload.extend_from_slice(&13_u64.to_le_bytes());
        payload.extend_from_slice(&17_u64.to_le_bytes());
        let frame = encode_message(MessageType::RenderAck, &payload);

        read_loop(std::io::Cursor::new(frame), 1, state, cmd_tx);

        match cmd_rx.recv().expect("render ack command") {
            RemoteCmd::RenderAck {
                client_id,
                view_id,
                tab_id,
                pane_id,
                pane_revision,
                runtime_revision,
            } => {
                assert_eq!(client_id, 1);
                assert_eq!(view_id, 7);
                assert_eq!(tab_id, 9);
                assert_eq!(pane_id, 11);
                assert_eq!(pane_revision, 13);
                assert_eq!(runtime_revision, 17);
            }
            other => panic!("unexpected remote command: {other:?}"),
        }
    }

    #[test]
    fn read_loop_replies_to_heartbeat_without_runtime_command() {
        let (outbound_tx, outbound_rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(1, remote_client(outbound_tx, true))]),
            ..State::test_empty()
        }));
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let frame = encode_message(MessageType::Heartbeat, b"ping");

        read_loop(std::io::Cursor::new(frame), 1, state, cmd_tx);

        match outbound_rx.recv().expect("heartbeat ack frame") {
            OutboundMessage::Frame(frame) => {
                let mut cursor = std::io::Cursor::new(frame);
                let (ty, payload) = read_message(&mut cursor).expect("decoded heartbeat ack");
                assert_eq!(ty, MessageType::HeartbeatAck);
                assert_eq!(payload, b"ping");
            }
            OutboundMessage::ScreenUpdate(_) | OutboundMessage::PaneUpdate { .. } => panic!("unexpected screen update"),
        }
        assert!(cmd_rx.try_recv().is_err());
    }

    #[test]
    fn read_loop_closes_idle_unauthenticated_connection_after_timeout() {
        let (outbound_tx, outbound_rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                1,
                ClientState {
                    connected_at: Instant::now() - AUTH_CHALLENGE_WINDOW - Duration::from_secs(1),
                    authenticated_at: None,
                    ..remote_client(outbound_tx, false)
                },
            )]),
            ..State::test_empty()
        }));
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let reader = TimeoutScriptedReader::new([Err(io::ErrorKind::TimedOut)]);

        read_loop(reader, 1, Arc::clone(&state), cmd_tx);

        match outbound_rx.recv().expect("auth fail frame") {
            OutboundMessage::Frame(frame) => {
                let mut cursor = std::io::Cursor::new(frame);
                let (ty, payload) = read_message(&mut cursor).expect("decoded auth fail");
                assert_eq!(ty, MessageType::AuthFail);
                assert!(payload.is_empty());
            }
            OutboundMessage::ScreenUpdate(_) | OutboundMessage::PaneUpdate { .. } => panic!("unexpected screen update"),
        }
        assert!(cmd_rx.try_recv().is_err());
        let guard = state.lock().expect("remote server state poisoned");
        assert!(!guard.clients.contains_key(&1));
    }

    #[test]
    fn read_loop_closes_authenticated_remote_client_after_heartbeat_timeout() {
        let (outbound_tx, outbound_rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                1,
                ClientState {
                    connected_at: Instant::now()
                        - DIRECT_CLIENT_HEARTBEAT_WINDOW
                        - Duration::from_secs(2),
                    authenticated_at: Some(
                        Instant::now() - DIRECT_CLIENT_HEARTBEAT_WINDOW - Duration::from_secs(2),
                    ),
                    ..remote_client(outbound_tx, true)
                },
            )]),
            ..State::test_empty()
        }));
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let reader = TimeoutScriptedReader::new([Err(io::ErrorKind::TimedOut)]);

        read_loop(reader, 1, Arc::clone(&state), cmd_tx);

        match outbound_rx.recv().expect("heartbeat timeout error frame") {
            OutboundMessage::Frame(frame) => {
                let mut cursor = std::io::Cursor::new(frame);
                let (ty, payload) = read_message(&mut cursor).expect("decoded error frame");
                assert_eq!(ty, MessageType::ErrorMsg);
                let (code, message) = crate::remote_wire::decode_error_payload(&payload)
                    .expect("decode error payload");
                assert_eq!(code, RemoteErrorCode::HeartbeatTimeout);
                assert_eq!(message, "heartbeat timeout");
            }
            OutboundMessage::ScreenUpdate(_) | OutboundMessage::PaneUpdate { .. } => panic!("unexpected screen update"),
        }
        assert!(cmd_rx.try_recv().is_err());
        let guard = state.lock().expect("remote server state poisoned");
        assert!(!guard.clients.contains_key(&1));
    }
}
