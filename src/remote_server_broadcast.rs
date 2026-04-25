//! Broadcast-oriented remote server tests and future extraction landing zone.

#[cfg(test)]
mod tests {
    use crate::control;
    use crate::remote::{RemoteServer, RemoteTabInfo};
    use crate::remote_batcher::OutboundMessage;
    use crate::remote_state::{ClientRuntimeView, ClientState, State};
    use crate::remote_wire::{MAGIC, MessageType, RemoteCell, RemoteFullState};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::Instant;

    fn empty_state() -> State {
        State::test_empty()
    }

    fn test_client(
        outbound: mpsc::Sender<OutboundMessage>,
        subscribed_to_runtime: bool,
        is_local: bool,
    ) -> ClientState {
        ClientState {
            outbound,
            authenticated: true,
            connected_at: Instant::now(),
            authenticated_at: Some(Instant::now()),
            last_heartbeat_at: None,
            runtime_view: ClientRuntimeView {
                subscribed_to_runtime,
                ..ClientRuntimeView::idle()
            },
            is_local,
        }
    }

    fn sample_ui_state() -> control::UiRuntimeState {
        control::UiRuntimeState {
            active_tab: 0,
            focused_pane: 7,
            tabs: Vec::new(),
            visible_panes: Vec::new(),
            mouse_selection: control::UiMouseSelectionSnapshot::default(),
            status_bar: crate::status_components::UiStatusBarSnapshot::default(),
            pwd: "/tmp".to_string(),
            runtime_revision: 1,
            view_revision: 1,
            view_id: 1,
            viewed_tab_id: Some(1),
            viewport_cols: None,
            viewport_rows: None,
            visible_pane_ids: Vec::new(),
            acked_client_action_id: None,
        }
    }

    #[test]
    fn send_ui_runtime_state_to_local_viewers_targets_subscribed_clients() {
        let (viewer_tx, viewer_rx) = mpsc::channel();
        let (idle_tx, idle_rx) = mpsc::channel();
        let mut state = empty_state();
        state.clients.insert(1, test_client(viewer_tx, true, true));
        state.clients.insert(2, test_client(idle_tx, false, true));
        let server = RemoteServer::for_test(Arc::new(Mutex::new(state)));

        server.send_ui_runtime_state_to_local_viewers(&sample_ui_state());

        match viewer_rx.recv().expect("viewer frame") {
            OutboundMessage::Frame(frame) => {
                assert_eq!(&frame[..2], &MAGIC);
                assert_eq!(frame[2], MessageType::UiRuntimeState as u8);
            }
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
        }
        assert!(idle_rx.try_recv().is_err());
    }

    #[test]
    fn send_ui_runtime_state_skips_unchanged_payloads() {
        let (tx, rx) = mpsc::channel();
        let mut state = empty_state();
        state.clients.insert(1, test_client(tx, true, true));
        let server = RemoteServer::for_test(Arc::new(Mutex::new(state)));
        let ui_state = sample_ui_state();

        server.send_ui_runtime_state(1, &ui_state);
        server.send_ui_runtime_state(1, &ui_state);

        match rx.recv().expect("runtime state frame") {
            OutboundMessage::Frame(frame) => {
                assert_eq!(&frame[..2], &MAGIC);
                assert_eq!(frame[2], MessageType::UiRuntimeState as u8);
            }
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
        }
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn send_tab_list_skips_unchanged_payloads() {
        let (tx, rx) = mpsc::channel();
        let mut state = empty_state();
        state.clients.insert(1, test_client(tx, true, true));
        let server = RemoteServer::for_test(Arc::new(Mutex::new(state)));
        let tabs = vec![RemoteTabInfo {
            id: 11,
            name: "Tab 1".to_string(),
            title: "boo".to_string(),
            pwd: "/tmp".to_string(),
            active: true,
            child_exited: false,
        }];

        server.send_tab_list(1, &tabs);
        server.send_tab_list(1, &tabs);

        match rx.recv().expect("tab list frame") {
            OutboundMessage::Frame(frame) => {
                assert_eq!(&frame[..2], &MAGIC);
                assert_eq!(frame[2], crate::remote_wire::MESSAGE_TYPE_TAB_LIST as u8);
            }
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
        }
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn send_tab_list_to_local_clients_reaches_every_local_client_only() {
        let (local_a_tx, local_a_rx) = mpsc::channel();
        let (local_b_tx, local_b_rx) = mpsc::channel();
        let (remote_tx, remote_rx) = mpsc::channel();
        let mut state = empty_state();
        state.clients.insert(1, test_client(local_a_tx, true, true));
        state
            .clients
            .insert(2, test_client(local_b_tx, false, true));
        state.clients.insert(3, test_client(remote_tx, true, false));
        let server = RemoteServer::for_test(Arc::new(Mutex::new(state)));
        let tabs = vec![RemoteTabInfo {
            id: 11,
            name: "Tab 1".to_string(),
            title: "boo".to_string(),
            pwd: "/tmp".to_string(),
            active: true,
            child_exited: false,
        }];

        server.send_tab_list_to_local_clients(&tabs);

        for rx in [local_a_rx, local_b_rx] {
            match rx.recv().expect("local tab list frame") {
                OutboundMessage::Frame(frame) => {
                    assert_eq!(&frame[..2], &MAGIC);
                    assert_eq!(frame[2], crate::remote_wire::MESSAGE_TYPE_TAB_LIST as u8);
                }
                OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
            }
        }
        assert!(remote_rx.try_recv().is_err());
    }

    #[test]
    fn reply_tab_list_does_not_skip_unchanged_payloads() {
        let (tx, rx) = mpsc::channel();
        let mut state = empty_state();
        state.clients.insert(1, test_client(tx, true, true));
        let server = RemoteServer::for_test(Arc::new(Mutex::new(state)));
        let tabs = vec![RemoteTabInfo {
            id: 11,
            name: "Tab 1".to_string(),
            title: "boo".to_string(),
            pwd: "/tmp".to_string(),
            active: true,
            child_exited: false,
        }];

        server.reply_tab_list(1, &tabs);
        server.reply_tab_list(1, &tabs);

        for _ in 0..2 {
            match rx.recv().expect("tab list frame") {
                OutboundMessage::Frame(frame) => {
                    assert_eq!(&frame[..2], &MAGIC);
                    assert_eq!(frame[2], crate::remote_wire::MESSAGE_TYPE_TAB_LIST as u8);
                }
                OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
            }
        }
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn send_tab_created_acknowledges_explicit_create_request() {
        let (tx, rx) = mpsc::channel();
        let mut state = empty_state();
        state.clients.insert(1, test_client(tx, true, false));
        let server = RemoteServer::for_test(Arc::new(Mutex::new(state)));

        server.send_tab_created(1, 42);

        match rx.recv().expect("tab-created frame") {
            OutboundMessage::Frame(frame) => {
                assert_eq!(&frame[..2], &MAGIC);
                assert_eq!(frame[2], MessageType::TabCreated as u8);
                assert_eq!(u32::from_le_bytes(frame[7..11].try_into().unwrap()), 42);
            }
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
        }
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn subscribe_client_to_runtime_is_boolean_not_tab_targeted() {
        let (viewer_tx, viewer_rx) = mpsc::channel();
        let mut state = empty_state();
        state.clients.insert(1, test_client(viewer_tx, false, true));
        let state = Arc::new(Mutex::new(state));
        let server = RemoteServer::for_test(Arc::clone(&state));

        server.subscribe_client_to_runtime(1);

        let guard = state.lock().expect("remote server state poisoned");
        assert!(
            guard
                .clients
                .get(&1)
                .is_some_and(|c| c.runtime_view.subscribed_to_runtime)
        );
        assert!(viewer_rx.try_recv().is_err());
    }

    #[test]
    fn retain_local_viewer_pane_states_prunes_invisible_panes() {
        let (tx, _rx) = mpsc::channel();
        let mut state = empty_state();
        let mut client = test_client(tx, true, true);
        client.runtime_view.pane_states = HashMap::from([
            (
                10,
                Arc::new(RemoteFullState {
                    rows: 1,
                    cols: 1,
                    cursor_x: 0,
                    cursor_y: 0,
                    cursor_visible: true,
                    cursor_blinking: false,
                    cursor_style: 1,
                    cells: vec![RemoteCell {
                        codepoint: u32::from('a'),
                        fg: [1, 2, 3],
                        bg: [0, 0, 0],
                        style_flags: 0,
                        wide: false,
                    }],
                }),
            ),
            (
                20,
                Arc::new(RemoteFullState {
                    rows: 1,
                    cols: 1,
                    cursor_x: 0,
                    cursor_y: 0,
                    cursor_visible: true,
                    cursor_blinking: false,
                    cursor_style: 1,
                    cells: vec![RemoteCell {
                        codepoint: u32::from('b'),
                        fg: [1, 2, 3],
                        bg: [0, 0, 0],
                        style_flags: 0,
                        wide: false,
                    }],
                }),
            ),
        ]);
        state.clients.insert(1, client);
        let state = Arc::new(Mutex::new(state));
        let server = RemoteServer::for_test(Arc::clone(&state));

        server.retain_local_viewer_pane_states(&[20]);

        let guard = state.lock().expect("remote server state poisoned");
        let client = guard.clients.get(&1).expect("client state");
        assert!(!client.runtime_view.pane_states.contains_key(&10));
        assert!(client.runtime_view.pane_states.contains_key(&20));
    }

    #[test]
    fn initialize_client_view_tracks_view_scope() {
        let (tx, _rx) = mpsc::channel();
        let mut state = empty_state();
        state.clients.insert(1, test_client(tx, true, false));
        let server = RemoteServer::for_test(Arc::new(Mutex::new(state)));

        server.initialize_client_view(1, Some(44), Some(55), &[55, 66]);

        let snapshot = server.client_runtime_view(1).expect("client view snapshot");
        assert_eq!(snapshot.view_id, 1);
        assert_eq!(snapshot.viewed_tab_id, Some(44));
        assert_eq!(snapshot.focused_pane_id, Some(55));
        assert_eq!(snapshot.visible_pane_ids, vec![55, 66]);
    }
}
