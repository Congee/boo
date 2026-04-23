#[cfg(test)]
mod tests {
    use crate::remote::{DirectTransportSession, RemoteTabInfo};
    use crate::remote_auth::read_loop;
    use crate::remote_batcher::OutboundMessage;
    use crate::remote_state::{
        ClientRuntimeSubscription, ClientState, DIRECT_CLIENT_HEARTBEAT_WINDOW, State,
    };
    use crate::remote_wire::{
        MESSAGE_TYPE_LIST_TABS, MESSAGE_TYPE_TAB_LIST, MessageType, RemoteCell, RemoteErrorCode,
        RemoteFullState, decode_error_payload, encode_auth_ok_payload, encode_message,
        encode_tab_list, read_message,
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

    fn local_client(outbound: mpsc::Sender<crate::remote_batcher::OutboundMessage>) -> ClientState {
        ClientState::test_client(outbound, true, true)
    }

    fn remote_client(outbound: mpsc::Sender<crate::remote_batcher::OutboundMessage>) -> ClientState {
        ClientState::test_client(outbound, true, false)
    }

    #[test]
    fn direct_transport_lists_tabs_over_unix_stream() {
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

            let (ty, payload) = read_message(&mut server_stream).expect("read list tabs");
            assert_eq!(ty, MESSAGE_TYPE_LIST_TABS);
            assert!(payload.is_empty());
            server_stream
                .write_all(&encode_message(
                    MESSAGE_TYPE_TAB_LIST,
                    &encode_tab_list(&[RemoteTabInfo {
                        id: 21,
                        name: "unix".to_string(),
                        title: "shell".to_string(),
                        pwd: "/tmp".to_string(),
                        attached: false,
                        child_exited: false,
                    }]),
                ))
                .expect("write tab list");
        });

        let mut client = DirectTransportSession::connect_over_stream(
            client_stream,
            "unix-test".to_string(),
            0,
            Some("unix-daemon"),
        )
        .expect("connect over unix stream");
        let heartbeat_rtt_ms = client
            .heartbeat_round_trip(b"unix-heartbeat")
            .expect("heartbeat round trip");
        assert!(heartbeat_rtt_ms <= 5_000);
        let tabs = client.list_tabs().expect("list tabs over unix stream");
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0].id, 21);
        assert_eq!(tabs[0].name, "unix");
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
                    connected_at: Instant::now()
                        - DIRECT_CLIENT_HEARTBEAT_WINDOW
                        - Duration::from_secs(2),
                    authenticated_at: Some(
                        Instant::now()
                            - DIRECT_CLIENT_HEARTBEAT_WINDOW
                            - Duration::from_secs(2),
                    ),
                    ..local_client(outbound_tx)
                },
            )]),
            ..State::test_empty()
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
                    connected_at: Instant::now()
                        - DIRECT_CLIENT_HEARTBEAT_WINDOW
                        - Duration::from_secs(2),
                    authenticated_at: Some(
                        Instant::now()
                            - DIRECT_CLIENT_HEARTBEAT_WINDOW
                            - Duration::from_secs(2),
                    ),
                    runtime_subscription: ClientRuntimeSubscription {
                        tab_id: Some(11),
                        last_state: Some(Arc::clone(&cached_state)),
                        latest_input_seq: Some(9),
                        ..ClientRuntimeSubscription::detached()
                    },
                    ..remote_client(outbound_tx)
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
                let (code, message) = decode_error_payload(&payload).expect("decode error payload");
                assert_eq!(code, RemoteErrorCode::HeartbeatTimeout);
                assert_eq!(message, "heartbeat timeout");
            }
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
        }
        assert!(cmd_rx.try_recv().is_err());
        let guard = state.lock().expect("remote server state poisoned");
        assert!(!guard.clients.contains_key(&1));
    }
}
