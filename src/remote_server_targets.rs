//! Target-selection helpers for remote server fan-out operations.

use crate::remote_state::State;
use std::collections::HashSet;

pub(crate) fn local_attached_client_ids_for_tab(state: &State, tab_id: u32) -> Vec<u64> {
    state
        .clients
        .iter()
        .filter_map(|(client_id, client)| {
            (client.is_local && client.attached_session == Some(tab_id)).then_some(*client_id)
        })
        .collect()
}

pub(crate) fn retarget_local_attached_client_ids_for_tab(state: &State, tab_id: u32) -> Vec<u64> {
    state
        .clients
        .iter()
        .filter_map(|(client_id, client)| {
            (client.is_local
                && client.attached_session.is_some()
                && client.attached_session != Some(tab_id))
            .then_some(*client_id)
        })
        .collect()
}

pub(crate) fn local_client_ids(state: &State) -> Vec<u64> {
    state
        .clients
        .iter()
        .filter_map(|(client_id, client)| client.is_local.then_some(*client_id))
        .collect()
}

pub(crate) fn client_ids_for_tab(state: &State, tab_id: u32) -> Vec<u64> {
    state
        .clients
        .iter()
        .filter_map(|(client_id, client)| {
            (client.attached_session == Some(tab_id)).then_some(*client_id)
        })
        .collect()
}

pub(crate) fn retain_local_attached_pane_states(
    state: &mut State,
    tab_id: u32,
    visible_pane_ids: &[u64],
) {
    let visible = visible_pane_ids.iter().copied().collect::<HashSet<_>>();
    for client in state.clients.values_mut() {
        if client.is_local && client.attached_session == Some(tab_id) {
            client.pane_states.retain(|pane_id, _| visible.contains(pane_id));
        }
    }
}

#[allow(dead_code)]
pub(crate) fn local_attached_client_ids(state: &State, session_id: u32) -> Vec<u64> {
    local_attached_client_ids_for_tab(state, session_id)
}

#[allow(dead_code)]
pub(crate) fn retarget_local_attached_client_ids(state: &State, session_id: u32) -> Vec<u64> {
    retarget_local_attached_client_ids_for_tab(state, session_id)
}

#[allow(dead_code)]
pub(crate) fn client_ids_for_session(state: &State, session_id: u32) -> Vec<u64> {
    client_ids_for_tab(state, session_id)
}
