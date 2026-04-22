//! Attachment preparation helpers for the remote daemon.
//!
//! This module owns the revive-or-attach state transition that runs before the
//! server sends `Attached` frames to remote clients.

use crate::remote_state::{State, prune_revivable_attachments};
use crate::remote_wire::RemoteErrorCode;

pub(crate) fn prepare_attachment(
    state: &mut State,
    client_id: u64,
    tab_id: u32,
    attachment_id: Option<u64>,
    resume_token: Option<u64>,
) -> Result<(), RemoteErrorCode> {
    prune_revivable_attachments(state);
    let Some(client) = state.clients.get(&client_id) else {
        return Err(RemoteErrorCode::Unknown);
    };
    if client.is_local || attachment_id.is_none() {
        return Ok(());
    }
    let attachment_id = attachment_id.expect("checked above");
    if state.clients.iter().any(|(other_client_id, other_client)| {
        *other_client_id != client_id
            && !other_client.is_local
            && other_client.attachment_id == Some(attachment_id)
            && other_client.attached_tab.is_some()
    }) {
        log::warn!(
            "remote revive rejected: client_id={client_id} attachment_id={attachment_id} reason=already-active"
        );
        return Err(RemoteErrorCode::AttachmentAlreadyActive);
    }
    let revive = state.revivable_attachments.get(&attachment_id).cloned();
    if let Some(revive) = revive {
        if revive.tab_id != tab_id {
            log::warn!(
                "remote revive rejected: client_id={client_id} attachment_id={attachment_id} reason=session-mismatch expected={} actual={tab_id}",
                revive.tab_id
            );
            return Err(RemoteErrorCode::AttachmentBelongsToDifferentSession);
        }
        if resume_token != Some(revive.resume_token) {
            log::warn!(
                "remote revive rejected: client_id={client_id} attachment_id={attachment_id} reason=resume-token-mismatch"
            );
            return Err(RemoteErrorCode::AttachmentResumeTokenMismatch);
        }
        let _ = state.revivable_attachments.remove(&attachment_id);
        let Some(client) = state.clients.get_mut(&client_id) else {
            return Err(RemoteErrorCode::Unknown);
        };
        client.attached_tab = Some(tab_id);
        client.attachment_id = Some(attachment_id);
        client.resume_token = Some(revive.resume_token);
        client.last_state = revive.last_state;
        client.pane_states = revive.pane_states;
        client.latest_input_seq = revive.latest_input_seq;
        log::info!(
            "remote revive restored: client_id={client_id} tab_id={tab_id} attachment_id={attachment_id}"
        );
    } else {
        if resume_token.is_some() {
            log::warn!(
                "remote revive rejected: client_id={client_id} attachment_id={attachment_id} reason=revive-window-expired"
            );
            return Err(RemoteErrorCode::AttachmentResumeWindowExpired);
        }
        let Some(client) = state.clients.get_mut(&client_id) else {
            return Err(RemoteErrorCode::Unknown);
        };
        client.resume_token = None;
        log::info!(
            "remote attach prepared without revive: client_id={client_id} tab_id={tab_id} attachment_id={attachment_id}"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::prepare_attachment;
    use crate::remote::RemoteServer;
    use crate::remote_batcher::OutboundMessage;
    use crate::remote_state::{ClientState, REVIVABLE_ATTACHMENT_WINDOW, RevivableAttachment, State};
    use crate::remote_wire::{MAGIC, MessageType, RemoteCell, RemoteErrorCode, RemoteFullState, read_message};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::Instant;

    fn empty_state() -> State {
        State {
            clients: HashMap::new(),
            revivable_attachments: HashMap::new(),
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
        }
    }

    fn remote_client(outbound: mpsc::Sender<crate::remote_batcher::OutboundMessage>) -> ClientState {
        ClientState {
            outbound,
            authenticated: true,
            connected_at: Instant::now(),
            authenticated_at: Some(Instant::now()),
            last_heartbeat_at: None,
            attached_tab: None,
            attachment_id: None,
            resume_token: None,
            last_tab_list_payload: None,
            last_ui_runtime_state_payload: None,
            last_ui_appearance_payload: None,
            last_state: None,
            pane_states: HashMap::new(),
            latest_input_seq: None,
            is_local: false,
        }
    }

    fn local_client(outbound: mpsc::Sender<crate::remote_batcher::OutboundMessage>) -> ClientState {
        ClientState {
            is_local: true,
            ..remote_client(outbound)
        }
    }

    #[test]
    fn prepare_attachment_restores_revived_state_for_matching_identity() {
        let (tx, _rx) = mpsc::channel();
        let restored_state = Arc::new(RemoteFullState {
            rows: 1,
            cols: 1,
            cursor_x: 0,
            cursor_y: 0,
            cursor_visible: true,
            cursor_blinking: false,
            cursor_style: 1,
            cells: vec![RemoteCell {
                codepoint: u32::from('R'),
                fg: [1, 2, 3],
                bg: [0, 0, 0],
                style_flags: 0,
                wide: false,
            }],
        });
        let mut state = empty_state();
        state.clients.insert(1, remote_client(tx));
        state.revivable_attachments.insert(
            0xabc,
            RevivableAttachment {
                tab_id: 11,
                resume_token: 0xdef,
                last_state: Some(Arc::clone(&restored_state)),
                pane_states: HashMap::new(),
                latest_input_seq: Some(9),
                expires_at: Instant::now() + REVIVABLE_ATTACHMENT_WINDOW,
            },
        );

        prepare_attachment(&mut state, 1, 11, Some(0xabc), Some(0xdef))
            .expect("prepare attachment");

        let client = state.clients.get(&1).expect("client state");
        assert_eq!(client.attached_tab, Some(11));
        assert_eq!(client.attachment_id, Some(0xabc));
        assert_eq!(client.resume_token, Some(0xdef));
        assert_eq!(client.latest_input_seq, Some(9));
        assert_eq!(client.last_state.as_deref(), Some(restored_state.as_ref()));
        assert!(!state.revivable_attachments.contains_key(&0xabc));
    }

    #[test]
    fn prepare_attachment_rejects_duplicate_active_attachment() {
        let (active_tx, _active_rx) = mpsc::channel();
        let (new_tx, _new_rx) = mpsc::channel();
        let mut state = empty_state();
        let mut active = remote_client(active_tx);
        active.attached_tab = Some(11);
        active.attachment_id = Some(0xabc);
        state.clients.insert(1, active);
        state.clients.insert(2, remote_client(new_tx));

        let error = prepare_attachment(&mut state, 2, 11, Some(0xabc), Some(0xdef))
            .expect_err("duplicate active attachment should fail");
        assert_eq!(error, RemoteErrorCode::AttachmentAlreadyActive);
    }

    #[test]
    fn prepare_attachment_rejects_wrong_resume_token() {
        let (tx, _rx) = mpsc::channel();
        let mut state = empty_state();
        state.clients.insert(1, remote_client(tx));
        state.revivable_attachments.insert(
            0xabc,
            RevivableAttachment {
                tab_id: 11,
                resume_token: 0xdef,
                last_state: None,
                pane_states: HashMap::new(),
                latest_input_seq: None,
                expires_at: Instant::now() + REVIVABLE_ATTACHMENT_WINDOW,
            },
        );

        let error = prepare_attachment(&mut state, 1, 11, Some(0xabc), Some(0x123))
            .expect_err("wrong resume token should fail");
        assert_eq!(error, RemoteErrorCode::AttachmentResumeTokenMismatch);
        assert!(state.revivable_attachments.contains_key(&0xabc));
    }

    #[test]
    fn prepare_attachment_rejects_expired_resume_attempt() {
        let (tx, _rx) = mpsc::channel();
        let mut state = empty_state();
        state.clients.insert(1, remote_client(tx));

        let error = prepare_attachment(&mut state, 1, 11, Some(0xabc), Some(0xdef))
            .expect_err("expired resume attempt should fail");
        assert_eq!(error, RemoteErrorCode::AttachmentResumeWindowExpired);

        let client = state.clients.get(&1).expect("client state");
        assert!(client.attached_tab.is_none());
        assert!(client.attachment_id.is_none());
        assert!(client.resume_token.is_none());
    }

    #[test]
    fn prepare_attachment_allows_new_attach_without_resume_token() {
        let (tx, _rx) = mpsc::channel();
        let mut state = empty_state();
        state.clients.insert(1, remote_client(tx));

        prepare_attachment(&mut state, 1, 11, Some(0xabc), None)
            .expect("attach without resume token should succeed");
    }

    #[test]
    fn send_attached_to_same_session_preserves_stream_state() {
        let (outbound, outbound_rx) = mpsc::channel();
        let mut state = empty_state();
        let mut client = local_client(outbound);
        client.attached_tab = Some(11);
        client.last_state = Some(Arc::new(RemoteFullState {
            rows: 1,
            cols: 1,
            cursor_x: 0,
            cursor_y: 0,
            cursor_visible: true,
            cursor_blinking: false,
            cursor_style: 1,
            cells: vec![RemoteCell {
                codepoint: u32::from('A'),
                fg: [1, 2, 3],
                bg: [0, 0, 0],
                style_flags: 0,
                wide: false,
            }],
        }));
        client.latest_input_seq = Some(42);
        state.clients.insert(7, client);
        let state = Arc::new(Mutex::new(state));
        let server = RemoteServer::for_test(Arc::clone(&state));

        server.send_attached(7, 11, None);

        let guard = state.lock().expect("remote server state poisoned");
        let client = guard.clients.get(&7).expect("client state");
        assert_eq!(client.attached_tab, Some(11));
        assert_eq!(client.latest_input_seq, Some(42));
        assert!(client.last_state.is_some());
        drop(guard);

        match outbound_rx.recv().expect("attached frame") {
            OutboundMessage::Frame(frame) => {
                assert_eq!(&frame[..2], &MAGIC);
                assert_eq!(frame[2], MessageType::Attached as u8);
            }
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
        }
    }

    #[test]
    fn send_attached_for_remote_client_includes_resume_token() {
        let (outbound, outbound_rx) = mpsc::channel();
        let mut state = empty_state();
        state.clients.insert(7, remote_client(outbound));
        let state = Arc::new(Mutex::new(state));
        let server = RemoteServer::for_test(Arc::clone(&state));

        server.send_attached(7, 11, Some(0xabc));

        let guard = state.lock().expect("remote server state poisoned");
        let client = guard.clients.get(&7).expect("client state");
        let resume_token = client.resume_token.expect("resume token");
        assert_ne!(resume_token, 0);
        drop(guard);

        match outbound_rx.recv().expect("attached frame") {
            OutboundMessage::Frame(frame) => {
                let mut cursor = std::io::Cursor::new(frame);
                let (ty, payload) = read_message(&mut cursor).expect("attached frame decode");
                assert_eq!(ty, MessageType::Attached);
                assert_eq!(payload.len(), 20);
                assert_eq!(u32::from_le_bytes(payload[0..4].try_into().unwrap()), 11);
                assert_eq!(u64::from_le_bytes(payload[4..12].try_into().unwrap()), 0xabc);
                assert_eq!(u64::from_le_bytes(payload[12..20].try_into().unwrap()), resume_token);
            }
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
        }
    }

    #[test]
    fn has_client_is_true_before_attach() {
        let (tx, _rx) = mpsc::channel();
        let mut state = empty_state();
        state.clients.insert(1, remote_client(tx));
        let server = RemoteServer::for_test(Arc::new(Mutex::new(state)));

        assert!(server.has_client(1));
        assert_eq!(server.client_session(1), None);
    }
}
