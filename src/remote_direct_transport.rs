//! Sync direct-client runtime transport for the remote daemon.

use std::io::{Read, Write};

use crate::remote::RuntimeAction;
use crate::remote_types::RemoteDirectTabInfo;
use crate::remote_wire::{
    MESSAGE_TYPE_LIST_TABS, MESSAGE_TYPE_TAB_CREATED, MESSAGE_TYPE_TAB_LIST, MessageType,
    decode_auth_ok_payload, decode_tab_list_payload, encode_message, parse_created_tab_id,
    read_message, read_probe_auth_reply, read_probe_reply, validate_auth_ok_payload,
};

pub(crate) trait DirectReadWrite: Read + Write {}
impl<T: Read + Write> DirectReadWrite for T {}

pub(crate) struct DirectTransportClient<S: DirectReadWrite> {
    pub(crate) stream: S,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) protocol_version: u16,
    pub(crate) capabilities: u32,
    pub(crate) build_id: Option<String>,
    pub(crate) server_instance_id: Option<String>,
    pub(crate) server_identity_id: Option<String>,
}

impl<S: DirectReadWrite> DirectTransportClient<S> {
    pub(crate) fn connect_over_stream(
        mut stream: S,
        host: String,
        port: u16,
        expected_server_identity: Option<&str>,
    ) -> Result<Self, String> {
        stream
            .write_all(&encode_message(MessageType::Auth, &[]))
            .map_err(|error| format!("failed to send auth request to {host}:{port}: {error}"))?;
        let (ty, auth_ok_payload) = read_probe_auth_reply(&mut stream, &host, port)?;
        match ty {
            MessageType::AuthOk => {}
            MessageType::AuthFail => {
                return Err(format!(
                    "authentication failed for remote endpoint {host}:{port}"
                ));
            }
            other => {
                return Err(format!(
                    "unexpected auth reply from {host}:{port}: {other:?}"
                ));
            }
        }

        validate_auth_ok_payload(&auth_ok_payload)?;
        let (protocol_version, capabilities, build_id, server_instance_id, server_identity_id) =
            decode_auth_ok_payload(&auth_ok_payload).ok_or_else(|| {
                format!("remote endpoint {host}:{port} returned malformed handshake metadata")
            })?;
        if let Some(expected_server_identity) = expected_server_identity
            && server_identity_id.as_deref() != Some(expected_server_identity)
        {
            return Err(format!(
                "remote endpoint {host}:{port} reported daemon identity {:?}, expected {:?}",
                server_identity_id, expected_server_identity
            ));
        }

        Ok(Self {
            stream,
            host,
            port,
            protocol_version,
            capabilities,
            build_id,
            server_instance_id,
            server_identity_id,
        })
    }

    pub(crate) fn heartbeat_round_trip(&mut self, payload: &[u8]) -> Result<u64, String> {
        let heartbeat_start = std::time::Instant::now();
        self.stream
            .write_all(&encode_message(MessageType::Heartbeat, payload))
            .map_err(|error| {
                format!(
                    "failed to send heartbeat to {}:{}: {error}",
                    self.host, self.port
                )
            })?;
        let (_heartbeat_ty, heartbeat_reply) = read_probe_reply(
            &mut self.stream,
            &self.host,
            self.port,
            MessageType::HeartbeatAck,
        )?;
        if heartbeat_reply != payload {
            return Err(format!(
                "heartbeat payload mismatch from remote endpoint {}:{}",
                self.host, self.port
            ));
        }
        Ok(heartbeat_start.elapsed().as_millis() as u64)
    }

    pub(crate) fn list_tabs(&mut self) -> Result<Vec<RemoteDirectTabInfo>, String> {
        self.stream
            .write_all(&encode_message(MESSAGE_TYPE_LIST_TABS, &[]))
            .map_err(|error| {
                format!(
                    "failed to send list tabs request to {}:{}: {error}",
                    self.host, self.port
                )
            })?;
        let (_reply_ty, payload) = read_probe_reply(
            &mut self.stream,
            &self.host,
            self.port,
            MESSAGE_TYPE_TAB_LIST,
        )?;
        decode_tab_list_payload(&payload).map_err(|error| {
            format!(
                "failed to decode remote tab list from {}:{}: {error}",
                self.host, self.port
            )
        })
    }

    pub(crate) fn create_tab(&mut self, cols: u16, rows: u16) -> Result<u32, String> {
        let payload = serde_json::to_vec(&RuntimeAction::NewTab {
            view_id: 0,
            cols: Some(cols),
            rows: Some(rows),
        })
        .map_err(|error| {
            format!(
                "failed to encode create-tab runtime action for {}:{}: {error}",
                self.host, self.port
            )
        })?;
        self.stream
            .write_all(&encode_message(MessageType::RuntimeAction, &payload))
            .map_err(|error| {
                format!(
                    "failed to send create-tab runtime action to {}:{}: {error}",
                    self.host, self.port
                )
            })?;
        loop {
            let (ty, payload) = read_message(&mut self.stream).map_err(|error| {
                format!(
                    "failed to read create-tab reply from {}:{}: {error}",
                    self.host, self.port
                )
            })?;
            match ty {
                MessageType::UiRuntimeState => {
                    let runtime_state: crate::control::UiRuntimeState =
                        serde_json::from_slice(&payload).map_err(|error| {
                            format!(
                                "failed to decode runtime state from {}:{}: {error}",
                                self.host, self.port
                            )
                        })?;
                    if let Some(tab_id) = runtime_state.viewed_tab_id {
                        return Ok(tab_id);
                    }
                }
                MESSAGE_TYPE_TAB_CREATED => {
                    return parse_created_tab_id(&payload).ok_or_else(|| {
                        format!(
                            "invalid tab-created payload from remote endpoint {}:{}",
                            self.host, self.port
                        )
                    });
                }
                MESSAGE_TYPE_TAB_LIST => {}
                MessageType::Heartbeat => {
                    self.stream
                        .write_all(&encode_message(MessageType::HeartbeatAck, &payload))
                        .map_err(|error| {
                            format!(
                                "failed to send heartbeat ack to {}:{}: {error}",
                                self.host, self.port
                            )
                        })?;
                }
                MessageType::ErrorMsg => {
                    return Err(format!(
                        "remote endpoint {}:{} rejected create-tab request",
                        self.host, self.port
                    ));
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::remote_wire::{MessageType, encode_auth_ok_payload, encode_message, read_message};
    use std::io::{self, Cursor, Read, Write};

    pub(crate) struct ScriptedDirectStream {
        read: Cursor<Vec<u8>>,
        written: Vec<u8>,
    }

    impl ScriptedDirectStream {
        pub(crate) fn new(replies: Vec<Vec<u8>>) -> Self {
            Self {
                read: Cursor::new(replies.concat()),
                written: Vec::new(),
            }
        }

        pub(crate) fn written_frames(&self) -> Vec<(MessageType, Vec<u8>)> {
            let mut cursor = Cursor::new(self.written.as_slice());
            let mut frames = Vec::new();
            while (cursor.position() as usize) < self.written.len() {
                frames.push(read_message(&mut cursor).expect("decode scripted write"));
            }
            frames
        }
    }

    impl Read for ScriptedDirectStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.read.read(buf)
        }
    }

    impl Write for ScriptedDirectStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.written.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    pub(crate) fn auth_ok_reply(server_identity_id: &str, server_instance_id: &str) -> Vec<u8> {
        encode_message(
            MessageType::AuthOk,
            &encode_auth_ok_payload(server_identity_id, server_instance_id),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{UiMouseSelectionSnapshot, UiRuntimeState, UiTabSnapshot};
    use crate::remote_direct_transport::test_support::{ScriptedDirectStream, auth_ok_reply};
    use crate::status_components::UiStatusBarSnapshot;

    #[test]
    fn create_tab_uses_runtime_action_and_viewed_tab_runtime_state() {
        let stream = ScriptedDirectStream::new(vec![
            auth_ok_reply("test-daemon", "test-instance"),
            encode_message(
                MessageType::UiRuntimeState,
                &serde_json::to_vec(&UiRuntimeState {
                    active_tab: 0,
                    focused_pane: 5,
                    tabs: vec![UiTabSnapshot {
                        tab_id: 77,
                        index: 0,
                        active: true,
                        title: "Tab 1".to_string(),
                        pane_count: 1,
                        focused_pane: Some(5),
                        pane_ids: vec![5],
                    }],
                    visible_panes: vec![],
                    mouse_selection: UiMouseSelectionSnapshot::default(),
                    status_bar: UiStatusBarSnapshot::default(),
                    pwd: "/tmp".to_string(),
                    runtime_revision: 2,
                    view_revision: 2,
                    view_id: 4,
                    viewed_tab_id: Some(77),
                    viewport_cols: Some(120),
                    viewport_rows: Some(36),
                    visible_pane_ids: vec![5],
                })
                .expect("encode runtime state"),
            ),
        ]);
        let mut client =
            DirectTransportClient::connect_over_stream(stream, "127.0.0.1".into(), 43210, None)
                .expect("connect transport");
        let tab_id = client.create_tab(120, 36).expect("create tab");
        assert_eq!(tab_id, 77);

        let frames = client.stream.written_frames();
        assert_eq!(frames[0], (MessageType::Auth, Vec::new()));
        let (ty, payload) = &frames[1];
        assert_eq!(*ty, MessageType::RuntimeAction);
        let action: RuntimeAction = serde_json::from_slice(payload).expect("decode action");
        assert_eq!(
            action,
            RuntimeAction::NewTab {
                view_id: 0,
                cols: Some(120),
                rows: Some(36),
            }
        );
    }
}
