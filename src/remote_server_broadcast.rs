//! Broadcast-oriented remote server tests and future extraction landing zone.

#[cfg(test)]
mod tests {
    use crate::control;
    use crate::remote::{RemoteServer, RemoteTabInfo};
    use crate::remote_batcher::OutboundMessage;
    use crate::remote_state::{ClientRuntimeSubscription, ClientState, State};
    use crate::remote_wire::{MAGIC, MessageType, RemoteCell, RemoteFullState};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::Instant;

    fn empty_state() -> State {
        State::test_empty()
    }

    fn test_client(
        outbound: mpsc::Sender<OutboundMessage>,
        attached_tab: Option<u32>,
        is_local: bool,
    ) -> ClientState {
        ClientState {
            outbound,
            authenticated: true,
            connected_at: Instant::now(),
            authenticated_at: Some(Instant::now()),
            last_heartbeat_at: None,
            runtime_subscription: ClientRuntimeSubscription {
                tab_id: attached_tab,
                ..ClientRuntimeSubscription::detached()
            },
            attachment_lease: None,
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
        }
    }

    #[test]
    fn send_ui_runtime_state_to_local_attached_only_targets_matching_tab() {
        let (attached_tx, attached_rx) = mpsc::channel();
        let (unattached_tx, unattached_rx) = mpsc::channel();
        let mut state = empty_state();
        state.clients.insert(1, test_client(attached_tx, Some(11), true));
        state.clients.insert(2, test_client(unattached_tx, None, true));
        let server = RemoteServer::for_test(Arc::new(Mutex::new(state)));

        server.send_ui_runtime_state_to_local_attached(11, &sample_ui_state());

        match attached_rx.recv().expect("attached frame") {
            OutboundMessage::Frame(frame) => {
                assert_eq!(&frame[..2], &MAGIC);
                assert_eq!(frame[2], MessageType::UiRuntimeState as u8);
            }
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
        }
        assert!(unattached_rx.try_recv().is_err());
    }

    #[test]
    fn send_ui_runtime_state_skips_unchanged_payloads() {
        let (tx, rx) = mpsc::channel();
        let mut state = empty_state();
        state.clients.insert(1, test_client(tx, Some(11), true));
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
        state.clients.insert(1, test_client(tx, Some(11), true));
        let server = RemoteServer::for_test(Arc::new(Mutex::new(state)));
        let tabs = vec![RemoteTabInfo {
            id: 11,
            name: "Tab 1".to_string(),
            title: "boo".to_string(),
            pwd: "/tmp".to_string(),
            attached: true,
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
        state.clients.insert(1, test_client(local_a_tx, Some(11), true));
        state.clients.insert(2, test_client(local_b_tx, None, true));
        state.clients.insert(3, test_client(remote_tx, Some(11), false));
        let server = RemoteServer::for_test(Arc::new(Mutex::new(state)));
        let tabs = vec![RemoteTabInfo {
            id: 11,
            name: "Tab 1".to_string(),
            title: "boo".to_string(),
            pwd: "/tmp".to_string(),
            attached: true,
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
        state.clients.insert(1, test_client(tx, Some(11), true));
        let server = RemoteServer::for_test(Arc::new(Mutex::new(state)));
        let tabs = vec![RemoteTabInfo {
            id: 11,
            name: "Tab 1".to_string(),
            title: "boo".to_string(),
            pwd: "/tmp".to_string(),
            attached: true,
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
    fn retarget_local_attached_to_tab_skips_same_tab_unattached_and_remote_clients() {
        let (local_attached_tx, local_attached_rx) = mpsc::channel();
        let (local_attached_two_tx, local_attached_two_rx) = mpsc::channel();
        let (local_unattached_tx, local_unattached_rx) = mpsc::channel();
        let (local_same_tab_tx, local_same_tab_rx) = mpsc::channel();
        let (remote_attached_tx, remote_attached_rx) = mpsc::channel();
        let mut state = empty_state();
        state
            .clients
            .insert(1, test_client(local_attached_tx, Some(11), true));
        state
            .clients
            .insert(5, test_client(local_attached_two_tx, Some(11), true));
        state.clients.insert(2, test_client(local_unattached_tx, None, true));
        state
            .clients
            .insert(3, test_client(local_same_tab_tx, Some(22), true));
        state
            .clients
            .insert(4, test_client(remote_attached_tx, Some(11), false));
        let server = RemoteServer::for_test(Arc::new(Mutex::new(state)));

        server.retarget_local_attached_to_tab(22);

        match local_attached_rx.recv().expect("local attached frame") {
            OutboundMessage::Frame(frame) => {
                assert_eq!(&frame[..2], &MAGIC);
                assert_eq!(frame[2], MessageType::Attached as u8);
            }
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
        }
        match local_attached_two_rx.recv().expect("second local attached frame") {
            OutboundMessage::Frame(frame) => {
                assert_eq!(&frame[..2], &MAGIC);
                assert_eq!(frame[2], MessageType::Attached as u8);
            }
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
        }
        assert!(local_unattached_rx.try_recv().is_err());
        assert!(local_same_tab_rx.try_recv().is_err());
        assert!(remote_attached_rx.try_recv().is_err());
    }

    #[test]
    fn retain_local_attached_pane_states_prunes_invisible_panes() {
        let (tx, _rx) = mpsc::channel();
        let mut state = empty_state();
        let mut client = test_client(tx, Some(11), true);
        client.runtime_subscription.pane_states = HashMap::from([
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

        server.retain_local_attached_pane_states(11, &[20]);

        let guard = state.lock().expect("remote server state poisoned");
        let client = guard.clients.get(&1).expect("client state");
        assert!(!client.runtime_subscription.pane_states.contains_key(&10));
        assert!(client.runtime_subscription.pane_states.contains_key(&20));
    }
}
