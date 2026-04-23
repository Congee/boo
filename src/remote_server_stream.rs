//! Stream publish helpers for remote server state updates.

use crate::remote_batcher::OutboundMessage;
use crate::remote_state::State;
use crate::remote_wire::{MessageType, RemoteFullState, encode_delta, encode_full_state, encode_message};
use std::sync::{Arc, Mutex};

pub(crate) fn send_state_to_client(
    state: &Arc<Mutex<State>>,
    client_id: u64,
    _visible_tab_id: u32,
    next_state: Arc<RemoteFullState>,
) {
    let _scope = crate::profiling::scope("server.stream.encode_state", crate::profiling::Kind::Cpu);
    let (outbound, previous_state, latest_input_seq, is_local) = {
        let guard = state.lock().expect("remote server state poisoned");
        let Some(client) = guard.clients.get(&client_id) else {
            return;
        };
        if client.runtime_view.viewing_tab_id.is_none() {
            return;
        }
        (
            client.outbound.clone(),
            client.runtime_view.last_state.clone(),
            client.runtime_view.latest_input_seq,
            client.is_local,
        )
    };
    let (ty, payload) = match previous_state.as_ref().and_then(|previous| {
        encode_delta(
            previous.as_ref(),
            next_state.as_ref(),
            latest_input_seq,
            is_local,
        )
    }) {
        Some(delta) => (MessageType::Delta, delta),
        None => (
            MessageType::FullState,
            encode_full_state(next_state.as_ref(), latest_input_seq, is_local),
        ),
    };
    let should_send = {
        let mut guard = state.lock().expect("remote server state poisoned");
        let Some(client) = guard.clients.get_mut(&client_id) else {
            return;
        };
        if client.runtime_view.viewing_tab_id.is_none() {
            false
        } else {
            client.runtime_view.last_state = Some(Arc::clone(&next_state));
            true
        }
    };
    if !should_send {
        return;
    }
    crate::profiling::record_units(
        match (ty, is_local) {
            (MessageType::Delta, true) => "server.stream.publish_delta.local",
            (MessageType::Delta, false) => "server.stream.publish_delta.remote",
            (MessageType::FullState, true) => "server.stream.publish_full.local",
            (MessageType::FullState, false) => "server.stream.publish_full.remote",
            _ => "server.stream.publish_other",
        },
        crate::profiling::Kind::Cpu,
        1,
    );
    let frame = encode_message(ty, &payload);
    let _ = outbound.send(OutboundMessage::ScreenUpdate(frame));
}

pub(crate) fn send_pane_state_to_client(
    state: &Arc<Mutex<State>>,
    client_id: u64,
    _visible_tab_id: u32,
    pane_id: u64,
    next_state: Arc<RemoteFullState>,
) {
    let (outbound, previous_state) = {
        let guard = state.lock().expect("remote server state poisoned");
        let Some(client) = guard.clients.get(&client_id) else {
            return;
        };
        if client.runtime_view.viewing_tab_id.is_none() {
            return;
        }
        (client.outbound.clone(), client.runtime_view.pane_states.get(&pane_id).cloned())
    };
    let (ty, payload) =
        match previous_state
            .as_ref()
            .and_then(|previous| encode_delta(previous.as_ref(), next_state.as_ref(), None, true))
        {
            Some(delta) => (MessageType::UiPaneDelta, delta),
            None => (
                MessageType::UiPaneFullState,
                encode_full_state(next_state.as_ref(), None, true),
            ),
        };
    let should_send = {
        let mut guard = state.lock().expect("remote server state poisoned");
        let Some(client) = guard.clients.get_mut(&client_id) else {
            return;
        };
        if client.runtime_view.viewing_tab_id.is_none() {
            false
        } else {
            client.runtime_view.pane_states.insert(pane_id, Arc::clone(&next_state));
            true
        }
    };
    if !should_send {
        return;
    }
    let mut prefixed = Vec::with_capacity(8 + payload.len());
    prefixed.extend_from_slice(&pane_id.to_le_bytes());
    prefixed.extend_from_slice(&payload);
    let frame = encode_message(ty, &prefixed);
    let _ = outbound.send(OutboundMessage::ScreenUpdate(frame));
}
