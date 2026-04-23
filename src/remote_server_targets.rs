//! Runtime-subscription helpers for remote server fan-out operations.

use crate::remote_state::State;
use std::collections::HashSet;

pub(crate) fn local_viewer_client_ids(state: &State) -> Vec<u64> {
    state
        .clients
        .iter()
        .filter_map(|(client_id, client)| {
            (client.is_local && client.runtime_view.subscribed_to_runtime).then_some(*client_id)
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

pub(crate) fn viewer_client_ids(state: &State) -> Vec<u64> {
    state
        .clients
        .iter()
        .filter_map(|(client_id, client)| {
            client
                .runtime_view
                .subscribed_to_runtime
                .then_some(*client_id)
        })
        .collect()
}

pub(crate) fn retain_local_viewer_pane_states(state: &mut State, visible_pane_ids: &[u64]) {
    let visible = visible_pane_ids.iter().copied().collect::<HashSet<_>>();
    for client in state.clients.values_mut() {
        if client.is_local && client.runtime_view.subscribed_to_runtime {
            client
                .runtime_view
                .pane_states
                .retain(|pane_id, _| visible.contains(pane_id));
        }
    }
}
