//! Cached control-payload helpers for local remote clients.

use crate::remote::RemoteTabInfo;
use crate::remote_batcher::OutboundMessage;
use crate::remote_server_targets::{local_attached_client_ids_for_tab, local_client_ids};
use crate::remote_state::{ClientState, State};
use crate::remote_wire::{MESSAGE_TYPE_TAB_LIST, MessageType, encode_message, encode_tab_list};
use std::sync::{Arc, Mutex};

pub(crate) enum CachedControlPayload {
    TabList,
    UiRuntimeState,
    UiAppearance,
}

pub(crate) fn send_tab_list(
    state: &Arc<Mutex<State>>,
    client_id: u64,
    tabs: &[RemoteTabInfo],
) {
    send_cached_control_payload_bytes(
        state,
        client_id,
        MESSAGE_TYPE_TAB_LIST,
        &encode_tab_list(tabs),
        CachedControlPayload::TabList,
    );
}

pub(crate) fn reply_tab_list(
    state: &Arc<Mutex<State>>,
    client_id: u64,
    tabs: &[RemoteTabInfo],
) {
    let frame = encode_message(MESSAGE_TYPE_TAB_LIST, &encode_tab_list(tabs));
    let guard = state.lock().expect("remote server state poisoned");
    if let Some(client) = guard.clients.get(&client_id) {
        let _ = client.outbound.send(OutboundMessage::Frame(frame));
    }
}

pub(crate) fn send_tab_list_to_local_clients(
    state: &Arc<Mutex<State>>,
    tabs: &[RemoteTabInfo],
) {
    let payload = encode_tab_list(tabs);
    let client_ids = {
        let guard = state.lock().expect("remote server state poisoned");
        local_client_ids(&guard)
    };
    for client_id in client_ids {
        send_cached_control_payload_bytes(
            state,
            client_id,
            MESSAGE_TYPE_TAB_LIST,
            &payload,
            CachedControlPayload::TabList,
        );
    }
}

pub(crate) fn send_ui_runtime_state(
    state: &Arc<Mutex<State>>,
    client_id: u64,
    runtime_state: &crate::control::UiRuntimeState,
) {
    let is_local = {
        let guard = state.lock().expect("remote server state poisoned");
        guard
            .clients
            .get(&client_id)
            .is_some_and(|client| client.is_local)
    };
    if !is_local {
        return;
    }
    let Ok(payload) = serde_json::to_vec(runtime_state) else {
        return;
    };
    send_cached_control_payload_bytes(
        state,
        client_id,
        MessageType::UiRuntimeState,
        &payload,
        CachedControlPayload::UiRuntimeState,
    );
}

pub(crate) fn send_ui_runtime_state_to_local_attached(
    state: &Arc<Mutex<State>>,
    tab_id: u32,
    runtime_state: &crate::control::UiRuntimeState,
) {
    let Ok(payload) = serde_json::to_vec(runtime_state) else {
        return;
    };
    let client_ids = {
        let guard = state.lock().expect("remote server state poisoned");
        local_attached_client_ids_for_tab(&guard, tab_id)
    };
    for client_id in client_ids {
        send_cached_control_payload_bytes(
            state,
            client_id,
            MessageType::UiRuntimeState,
            &payload,
            CachedControlPayload::UiRuntimeState,
        );
    }
}

pub(crate) fn send_ui_appearance(
    state: &Arc<Mutex<State>>,
    client_id: u64,
    appearance: &crate::control::UiAppearanceSnapshot,
) {
    let is_local = {
        let guard = state.lock().expect("remote server state poisoned");
        guard
            .clients
            .get(&client_id)
            .is_some_and(|client| client.is_local)
    };
    if !is_local {
        return;
    }
    let Ok(payload) = serde_json::to_vec(appearance) else {
        return;
    };
    send_cached_control_payload_bytes(
        state,
        client_id,
        MessageType::UiAppearance,
        &payload,
        CachedControlPayload::UiAppearance,
    );
}

pub(crate) fn send_ui_appearance_to_local_clients(
    state: &Arc<Mutex<State>>,
    appearance: &crate::control::UiAppearanceSnapshot,
) {
    let Ok(payload) = serde_json::to_vec(appearance) else {
        return;
    };
    let client_ids = {
        let guard = state.lock().expect("remote server state poisoned");
        local_client_ids(&guard)
    };
    for client_id in client_ids {
        send_cached_control_payload_bytes(
            state,
            client_id,
            MessageType::UiAppearance,
            &payload,
            CachedControlPayload::UiAppearance,
        );
    }
}

fn send_cached_control_payload_bytes(
    state: &Arc<Mutex<State>>,
    client_id: u64,
    ty: MessageType,
    payload: &[u8],
    cache_slot: CachedControlPayload,
) {
    let outbound = {
        let mut guard = state.lock().expect("remote server state poisoned");
        let Some(client) = guard.clients.get_mut(&client_id) else {
            return;
        };
        let cached_payload = cache_slot_mut(client, cache_slot);
        if cached_payload.as_deref() == Some(payload) {
            return;
        }
        *cached_payload = Some(payload.to_vec());
        client.outbound.clone()
    };
    let frame = encode_message(ty, payload);
    let _ = outbound.send(OutboundMessage::Frame(frame));
}

fn cache_slot_mut(
    client: &mut ClientState,
    cache_slot: CachedControlPayload,
) -> &mut Option<Vec<u8>> {
    match cache_slot {
        CachedControlPayload::TabList => &mut client.runtime_subscription.last_tab_list_payload,
        CachedControlPayload::UiRuntimeState => &mut client.runtime_subscription.last_ui_runtime_state_payload,
        CachedControlPayload::UiAppearance => &mut client.runtime_subscription.last_ui_appearance_payload,
    }
}
