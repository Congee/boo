#[cfg(test)]
mod tests {
    use crate::remote::{DirectTransportSession, RemoteSessionInfo};
    use crate::remote_auth::read_loop;
    use crate::remote_batcher::OutboundMessage;
    use crate::remote_state::{ClientState, DIRECT_CLIENT_HEARTBEAT_WINDOW, State};
    use crate::remote_wire::{
        MessageType, RemoteCell, RemoteFullState, encode_auth_ok_payload, encode_message,
        encode_session_list, read_message,
    };
    use std::collections::{HashMap, VecDeque};
    use std::io::{self, Read, Write};
    use std::os::unix::net::UnixStream;
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{Duration, Instant};

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

    #[test]
    fn direct_transport_session_lists_sessions_over_unix_stream() {
        let (client_stream, mut server_stream) =
            UnixStream::pair().expect("create unix stream pair");
        let server = std::thread::spawn(move || {
            let (ty, payload) = read_message(&mut server_stream).expect("read auth request");
            assert_eq!(ty, MessageType::Auth);
            assert!(payload.is_empty());

            server_stream
                .write_all(&encode_message(
                    MessageType::AuthOk,
                    &encode_auth_ok_payload("unix-daemon", "unix-instance"),
                ))
                .expect("write auth ok");

            let (ty, payload) = read_message(&mut server_stream).expect("read heartbeat");
            assert_eq!(ty, MessageType::Heartbeat);
            server_stream
                .write_all(&encode_message(MessageType::HeartbeatAck, &payload))
                .expect("write heartbeat ack");

            let (ty, payload) = read_message(&mut server_stream).expect("read list sessions");
            assert_eq!(ty, MessageType::ListSessions);
            assert!(payload.is_empty());
            server_stream
                .write_all(&encode_message(
                    MessageType::SessionList,
                    &encode_session_list(&[RemoteSessionInfo {
                        id: 21,
                        name: "unix".to_string(),
                        title: "shell".to_string(),
                        pwd: "/tmp".to_string(),
                        attached: false,
                        child_exited: false,
                    }]),
                ))
                .expect("write session list");
        });

        let mut client = DirectTransportSession::connect_over_stream(
            client_stream,
            "unix-test".to_string(),
            0,
            None,
            Some("unix-daemon"),
        )
        .expect("connect over unix stream");
        let heartbeat_rtt_ms = client
            .heartbeat_round_trip(b"unix-heartbeat")
            .expect("heartbeat round trip");
        assert!(heartbeat_rtt_ms <= 5_000);
        let sessions = client.list_sessions().expect("list sessions over unix stream");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, 21);
        assert_eq!(sessions[0].name, "unix");
        assert_eq!(client.server_identity_id.as_deref(), Some("unix-daemon"));
        assert_eq!(client.server_instance_id.as_deref(), Some("unix-instance"));

        server.join().expect("unix server thread");
    }

    #[test]
    fn local_authenticated_clients_do_not_timeout_without_heartbeat() {
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
                    is_local: true,
                },
            )]),
            revivable_attachments: HashMap::new(),
            auth_key: None,
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let reader = TimeoutScriptedReader::new([
            Err(io::ErrorKind::TimedOut),
            Err(io::ErrorKind::BrokenPipe),
        ]);

        read_loop(reader, 1, Arc::clone(&state), cmd_tx);

        assert!(outbound_rx.try_recv().is_err());
        assert!(cmd_rx.try_recv().is_err());
        let guard = state.lock().expect("remote server state poisoned");
        assert!(!guard.clients.contains_key(&1));
    }

    #[test]
    fn heartbeat_timeout_preserves_revivable_attachment_for_remote_client() {
        let (outbound_tx, outbound_rx) = mpsc::channel();
        let cached_state = Arc::new(RemoteFullState {
            rows: 1,
            cols: 1,
            cursor_x: 0,
            cursor_y: 0,
            cursor_visible: true,
            cursor_blinking: false,
            cursor_style: 1,
            cells: vec![RemoteCell {
                codepoint: u32::from('x'),
                fg: [1, 2, 3],
                bg: [0, 0, 0],
                style_flags: 0,
                wide: false,
            }],
        });
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
                    attached_session: Some(11),
                    attachment_id: Some(0xabc),
                    resume_token: Some(0xdef),
                    last_session_list_payload: None,
                    last_ui_runtime_state_payload: None,
                    last_ui_appearance_payload: None,
                    last_state: Some(Arc::clone(&cached_state)),
                    pane_states: HashMap::new(),
                    latest_input_seq: Some(9),
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
        let attachment = guard
            .revivable_attachments
            .get(&0xabc)
            .expect("revivable attachment preserved");
        assert_eq!(attachment.session_id, 11);
        assert_eq!(attachment.resume_token, 0xdef);
        assert_eq!(attachment.latest_input_seq, Some(9));
        assert_eq!(attachment.last_state.as_ref(), Some(&cached_state));
    }
}
