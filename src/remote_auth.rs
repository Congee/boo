//! Inbound message loop for the remote daemon: authentication + dispatch.
//!
//! Two interacting pieces:
//! * [`handle_auth_message`] — implements the HMAC challenge/response handshake.
//!   On first `Auth(empty)` it mints a challenge; on the matching response it
//!   verifies the HMAC-SHA256 MAC and flips `ClientState::authenticated`. Short-
//!   circuits to a happy path when `auth_key.is_none()`.
//! * [`read_loop`] — runs on the per-client reader thread. Pulls frames off the
//!   socket, routes `Auth` / `Heartbeat` inline, translates every other message
//!   into a `RemoteCmd`, and on exit parks the client's pane state into
//!   `State::revivable_attachments` so a reconnect can resume.
//!
//! Policy lives in `remote_state` (the window constants); wire encoding lives
//! in `remote_wire`. This module is the glue between them.

use std::io::{self, Read};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Instant;

use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::remote::RemoteCmd;
use crate::remote_batcher::OutboundMessage;
use crate::remote_state::{
    AUTH_CHALLENGE_WINDOW, AuthChallengeState, DIRECT_CLIENT_HEARTBEAT_WINDOW,
    REVIVABLE_ATTACHMENT_WINDOW, RevivableAttachment, State, prune_revivable_attachments,
    send_direct_error, send_direct_frame, should_disconnect_idle_client,
};
use crate::remote_wire::{
    MessageType, encode_auth_ok_payload, encode_message, parse_attach_request,
    parse_input_payload, parse_key_payload, parse_pane_id, parse_resize, parse_session_id,
    random_challenge, read_message,
};

type HmacSha256 = Hmac<Sha256>;

pub(crate) enum AuthHandling {
    Authenticated,
    Pending,
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
        let (ty, payload) = match read_message(&mut stream) {
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
                        send_direct_error(&state, client_id, "heartbeat timeout");
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
                AuthHandling::Pending => {}
                AuthHandling::Disconnect => break,
            }
            continue;
        }

        if !authenticated {
            send_direct_error(&state, client_id, "authentication required");
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
            MessageType::ListSessions => Some(RemoteCmd::ListSessions { client_id }),
            MessageType::Attach => {
                parse_attach_request(&payload).map(|(session_id, attachment_id, resume_token)| RemoteCmd::Attach {
                    client_id,
                    session_id,
                    attachment_id,
                    resume_token,
                })
            }
            MessageType::Detach => Some(RemoteCmd::Detach { client_id }),
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
                session_id: parse_session_id(&payload),
            }),
            _ => None,
        };

        if let Some(command) = command {
            if cmd_tx.send(command).is_err() {
                break;
            }
            crate::notify_headless_wakeup();
        } else {
            send_direct_error(&state, client_id, "invalid payload");
        }
    }

    let mut state = state.lock().expect("remote server state poisoned");
    if let Some(client) = state.clients.remove(&client_id) {
        prune_revivable_attachments(&mut state);
        if !client.is_local
            && let (Some(session_id), Some(attachment_id), Some(resume_token)) =
                (client.attached_session, client.attachment_id, client.resume_token)
        {
            state.revivable_attachments.insert(
                attachment_id,
                RevivableAttachment {
                    session_id,
                    resume_token,
                    last_state: client.last_state,
                    pane_states: client.pane_states,
                    latest_input_seq: client.latest_input_seq,
                    expires_at: Instant::now() + REVIVABLE_ATTACHMENT_WINDOW,
                },
            );
            log::info!(
                "remote client disconnected with revivable attachment: client_id={client_id} session_id={session_id} attachment_id={attachment_id}"
            );
        } else {
            log::info!("remote client disconnected: client_id={client_id}");
        }
    }
}

pub(crate) fn handle_auth_message(
    client_id: u64,
    payload: &[u8],
    state: &Arc<Mutex<State>>,
) -> AuthHandling {
    let mut state = state.lock().expect("remote server state poisoned");
    let auth_key = state.auth_key.clone();
    let server_identity_id = state.server_identity_id.clone();
    let server_instance_id = state.server_instance_id.clone();
    let Some(client) = state.clients.get_mut(&client_id) else {
        return AuthHandling::Disconnect;
    };

    if auth_key.is_none() {
        client.authenticated = true;
        client.authenticated_at = Some(Instant::now());
        log::info!("remote auth bypassed: client_id={client_id} mode=authless");
        let _ = client.outbound.send(OutboundMessage::Frame(encode_message(
            MessageType::AuthOk,
            &encode_auth_ok_payload(&server_identity_id, &server_instance_id),
        )));
        return AuthHandling::Authenticated;
    }

    if payload.is_empty() {
        let challenge = random_challenge();
        client.challenge = Some(AuthChallengeState {
            bytes: challenge,
            expires_at: Instant::now() + AUTH_CHALLENGE_WINDOW,
        });
        let _ = client.outbound.send(OutboundMessage::Frame(encode_message(
            MessageType::AuthChallenge,
            &challenge,
        )));
        log::info!("remote auth challenge issued: client_id={client_id}");
        return AuthHandling::Pending;
    }

    let Some(challenge) = client.challenge.take() else {
        log::warn!("remote auth failed: client_id={client_id} reason=missing-challenge");
        let _ = client.outbound.send(OutboundMessage::Frame(encode_message(
            MessageType::AuthFail,
            &[],
        )));
        return AuthHandling::Disconnect;
    };
    if Instant::now() > challenge.expires_at {
        log::warn!("remote auth failed: client_id={client_id} reason=expired-challenge");
        let _ = client.outbound.send(OutboundMessage::Frame(encode_message(
            MessageType::AuthFail,
            &[],
        )));
        return AuthHandling::Disconnect;
    }
    let Some(key) = auth_key else {
        log::warn!("remote auth failed: client_id={client_id} reason=missing-auth-key");
        let _ = client.outbound.send(OutboundMessage::Frame(encode_message(
            MessageType::AuthFail,
            &[],
        )));
        return AuthHandling::Disconnect;
    };

    let mut mac = HmacSha256::new_from_slice(&key).expect("valid HMAC key");
    mac.update(&challenge.bytes);
    match mac.verify_slice(payload) {
        Ok(()) => {
            client.authenticated = true;
            client.authenticated_at = Some(Instant::now());
            log::info!("remote auth succeeded: client_id={client_id}");
            let _ = client.outbound.send(OutboundMessage::Frame(encode_message(
                MessageType::AuthOk,
                &encode_auth_ok_payload(&server_identity_id, &server_instance_id),
            )));
            AuthHandling::Authenticated
        }
        Err(_) => {
            log::warn!("remote auth failed: client_id={client_id} reason=invalid-hmac");
            let _ = client.outbound.send(OutboundMessage::Frame(encode_message(
                MessageType::AuthFail,
                &[],
            )));
            AuthHandling::Disconnect
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote_wire::validate_auth_ok_payload;
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

    #[test]
    fn handle_auth_message_accepts_valid_challenge_response_within_window() {
        let (outbound_tx, outbound_rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                1,
                ClientState {
                    outbound: outbound_tx,
                    authenticated: false,
                    challenge: None,
                    connected_at: Instant::now(),
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

        assert!(matches!(
            handle_auth_message(1, &[], &state),
            AuthHandling::Pending
        ));
        let challenge = match outbound_rx.recv().expect("challenge frame") {
            OutboundMessage::Frame(frame) => {
                let mut cursor = std::io::Cursor::new(frame);
                let (ty, payload) = read_message(&mut cursor).expect("decoded challenge");
                assert_eq!(ty, MessageType::AuthChallenge);
                payload
            }
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
        };
        let mut mac =
            HmacSha256::new_from_slice(b"test-key").expect("valid HMAC key for test");
        mac.update(&challenge);
        let response = mac.finalize().into_bytes().to_vec();

        assert!(matches!(
            handle_auth_message(1, &response, &state),
            AuthHandling::Authenticated
        ));
        match outbound_rx.recv().expect("auth ok frame") {
            OutboundMessage::Frame(frame) => {
                let mut cursor = std::io::Cursor::new(frame);
                let (ty, payload) = read_message(&mut cursor).expect("decoded auth ok");
                assert_eq!(ty, MessageType::AuthOk);
                assert!(validate_auth_ok_payload(&payload, true).is_ok());
            }
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
        }
        assert!(
            state
                .lock()
                .expect("remote server state poisoned")
                .clients
                .get(&1)
                .expect("client state")
                .authenticated
        );
    }

    #[test]
    fn handle_auth_message_rejects_expired_challenge_response() {
        let (outbound_tx, outbound_rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                1,
                ClientState {
                    outbound: outbound_tx,
                    authenticated: false,
                    challenge: None,
                    connected_at: Instant::now(),
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

        assert!(matches!(
            handle_auth_message(1, &[], &state),
            AuthHandling::Pending
        ));
        let challenge = match outbound_rx.recv().expect("challenge frame") {
            OutboundMessage::Frame(frame) => {
                let mut cursor = std::io::Cursor::new(frame);
                let (ty, payload) = read_message(&mut cursor).expect("decoded challenge");
                assert_eq!(ty, MessageType::AuthChallenge);
                payload
            }
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
        };
        {
            let mut guard = state.lock().expect("remote server state poisoned");
            let client = guard.clients.get_mut(&1).expect("client state");
            let auth_challenge = client.challenge.as_mut().expect("stored challenge");
            auth_challenge.expires_at = Instant::now() - Duration::from_millis(1);
        }
        let mut mac =
            HmacSha256::new_from_slice(b"test-key").expect("valid HMAC key for test");
        mac.update(&challenge);
        let response = mac.finalize().into_bytes().to_vec();

        assert!(matches!(
            handle_auth_message(1, &response, &state),
            AuthHandling::Disconnect
        ));
        match outbound_rx.recv().expect("auth fail frame") {
            OutboundMessage::Frame(frame) => {
                let mut cursor = std::io::Cursor::new(frame);
                let (ty, payload) = read_message(&mut cursor).expect("decoded auth fail");
                assert_eq!(ty, MessageType::AuthFail);
                assert!(payload.is_empty());
            }
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
        }
        let guard = state.lock().expect("remote server state poisoned");
        let client = guard.clients.get(&1).expect("client state");
        assert!(!client.authenticated);
        assert!(client.challenge.is_none());
    }

    #[test]
    fn read_loop_emits_list_sessions_for_authenticated_client() {
        let (outbound_tx, _outbound_rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                1,
                ClientState {
                    outbound: outbound_tx,
                    authenticated: true,
                    challenge: None,
                    connected_at: Instant::now(),
                    authenticated_at: Some(Instant::now()),
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
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let mut frames = Vec::new();
        frames.extend_from_slice(&encode_message(MessageType::ListSessions, &[]));

        read_loop(std::io::Cursor::new(frames), 1, state, cmd_tx);

        match cmd_rx.recv().expect("remote command") {
            RemoteCmd::ListSessions { client_id } => assert_eq!(client_id, 1),
            other => panic!("unexpected remote command: {other:?}"),
        }
    }

    #[test]
    fn read_loop_replies_to_heartbeat_without_runtime_command() {
        let (outbound_tx, outbound_rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                1,
                ClientState {
                    outbound: outbound_tx,
                    authenticated: true,
                    challenge: None,
                    connected_at: Instant::now(),
                    authenticated_at: Some(Instant::now()),
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
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
        }
        assert!(cmd_rx.try_recv().is_err());
    }

    #[test]
    fn read_loop_closes_connection_after_auth_failure() {
        let (outbound_tx, outbound_rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                1,
                ClientState {
                    outbound: outbound_tx,
                    authenticated: false,
                    challenge: None,
                    connected_at: Instant::now(),
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
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let mut frames = Vec::new();
        frames.extend_from_slice(&encode_message(MessageType::Auth, &[]));
        frames.extend_from_slice(&encode_message(MessageType::Auth, b"definitely-wrong-hmac"));
        frames.extend_from_slice(&encode_message(MessageType::ListSessions, &[]));

        read_loop(std::io::Cursor::new(frames), 1, Arc::clone(&state), cmd_tx);

        let mut saw_challenge = false;
        let mut saw_fail = false;
        while let Ok(message) = outbound_rx.try_recv() {
            match message {
                OutboundMessage::Frame(frame) => {
                    let mut cursor = std::io::Cursor::new(frame);
                    let (ty, _) = read_message(&mut cursor).expect("decoded outbound frame");
                    if ty == MessageType::AuthChallenge {
                        saw_challenge = true;
                    } else if ty == MessageType::AuthFail {
                        saw_fail = true;
                    }
                }
                OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
            }
        }
        assert!(saw_challenge);
        assert!(saw_fail);
        assert!(cmd_rx.try_recv().is_err());
        let guard = state.lock().expect("remote server state poisoned");
        assert!(!guard.clients.contains_key(&1));
    }

    #[test]
    fn read_loop_closes_idle_unauthenticated_connection_after_timeout() {
        let (outbound_tx, outbound_rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                1,
                ClientState {
                    outbound: outbound_tx,
                    authenticated: false,
                    challenge: None,
                    connected_at: Instant::now() - AUTH_CHALLENGE_WINDOW - Duration::from_secs(1),
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
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
        }
        assert!(cmd_rx.try_recv().is_err());
        let guard = state.lock().expect("remote server state poisoned");
        assert!(!guard.clients.contains_key(&1));
    }

    #[test]
    fn read_loop_closes_expired_auth_challenge_after_timeout() {
        let (outbound_tx, outbound_rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                1,
                ClientState {
                    outbound: outbound_tx,
                    authenticated: false,
                    challenge: Some(AuthChallengeState {
                        bytes: [5; 32],
                        expires_at: Instant::now() - Duration::from_secs(1),
                    }),
                    connected_at: Instant::now(),
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
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
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
                    outbound: outbound_tx,
                    authenticated: true,
                    challenge: None,
                    connected_at: Instant::now()
                        - DIRECT_CLIENT_HEARTBEAT_WINDOW
                        - Duration::from_secs(2),
                    authenticated_at: Some(
                        Instant::now()
                            - DIRECT_CLIENT_HEARTBEAT_WINDOW
                            - Duration::from_secs(2),
                    ),
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
            auth_key: None,
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let reader = TimeoutScriptedReader::new([Err(io::ErrorKind::TimedOut)]);

        read_loop(reader, 1, Arc::clone(&state), cmd_tx);

        match outbound_rx.recv().expect("heartbeat timeout error frame") {
            OutboundMessage::Frame(frame) => {
                let mut cursor = std::io::Cursor::new(frame);
                let (ty, payload) = read_message(&mut cursor).expect("decoded error frame");
                assert_eq!(ty, MessageType::ErrorMsg);
                assert_eq!(String::from_utf8(payload).expect("utf8"), "heartbeat timeout");
            }
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
        }
        assert!(cmd_rx.try_recv().is_err());
        let guard = state.lock().expect("remote server state poisoned");
        assert!(!guard.clients.contains_key(&1));
    }}
