//! Stream publish helpers for remote server state updates.

use crate::remote_batcher::OutboundMessage;
use crate::remote_state::State;
use crate::remote_wire::{
    MessageType, RemoteFullState, encode_delta, encode_full_state, encode_message,
    encode_ui_pane_update_payload,
};
use std::sync::{Arc, Mutex};

pub(crate) fn send_state_to_client(
    state: &Arc<Mutex<State>>,
    client_id: u64,
    _tab_id: u32,
    next_state: Arc<RemoteFullState>,
) {
    let _scope = crate::profiling::scope("server.stream.encode_state", crate::profiling::Kind::Cpu);
    let (outbound, previous_state, latest_input_seq, is_local) = {
        let guard = state.lock().expect("remote server state poisoned");
        let Some(client) = guard.clients.get(&client_id) else {
            return;
        };
        if !client.runtime_view.subscribed_to_runtime {
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
        if !client.runtime_view.subscribed_to_runtime {
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
    tab_id: u32,
    pane_id: u64,
    pane_revision: u64,
    runtime_revision: u64,
    next_state: Arc<RemoteFullState>,
) {
    let (outbound, previous_state, view_id, view_revision, focused) = {
        let guard = state.lock().expect("remote server state poisoned");
        let Some(client) = guard.clients.get(&client_id) else {
            return;
        };
        if !client.runtime_view.subscribed_to_runtime {
            return;
        }
        (
            client.outbound.clone(),
            client.runtime_view.pane_states.get(&pane_id).cloned(),
            client.runtime_view.view_id,
            client.runtime_view.view_revision,
            client.runtime_view.focused_pane_id == Some(pane_id),
        )
    };
    let (ty, payload) = match previous_state
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
        if !client.runtime_view.subscribed_to_runtime {
            false
        } else {
            client
                .runtime_view
                .pane_states
                .insert(pane_id, Arc::clone(&next_state));
            true
        }
    };
    if !should_send {
        return;
    }
    tracing::info!(
        target: "boo::latency",
        interaction_id = 0_u64,
        view_id = view_id,
        tab_id = tab_id,
        pane_id = pane_id,
        action = "pane_update",
        route = "remote",
        runtime_revision = runtime_revision,
        view_revision = view_revision,
        pane_revision = pane_revision,
        elapsed_ms = 0.0_f64,
        "{}",
        crate::trace_schema::events::REMOTE_PANE_UPDATE
    );
    let prefixed = encode_ui_pane_update_payload(
        tab_id,
        pane_id,
        pane_revision,
        runtime_revision,
        &payload,
    );
    let frame = encode_message(ty, &prefixed);
    let _ = outbound.send(OutboundMessage::PaneUpdate {
        pane_id,
        focused,
        frame,
    });
}
