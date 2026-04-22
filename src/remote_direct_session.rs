//! Sync direct-client tab transport for the remote daemon.

use std::io::{Read, Write};

use crate::remote_types::{RemoteAttachedSummary, RemoteDirectTabInfo};
use crate::remote_wire::{
    MessageType, RemoteFullState, decode_auth_ok_payload, decode_tab_list_payload,
    encode_message, parse_created_tab_id, read_attach_bootstrap, read_probe_auth_reply,
    read_probe_reply, validate_auth_ok_payload,
};

pub(crate) trait DirectReadWrite: Read + Write {}
impl<T: Read + Write> DirectReadWrite for T {}

pub(crate) struct DirectTransportSession<S: DirectReadWrite> {
    pub(crate) stream: S,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) protocol_version: u16,
    pub(crate) capabilities: u32,
    pub(crate) build_id: Option<String>,
    pub(crate) server_instance_id: Option<String>,
    pub(crate) server_identity_id: Option<String>,
}

impl<S: DirectReadWrite> DirectTransportSession<S> {
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
                return Err(format!("authentication failed for remote endpoint {host}:{port}"));
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
        if let Some(expected_server_identity) = expected_server_identity {
            if server_identity_id.as_deref() != Some(expected_server_identity) {
                return Err(format!(
                    "remote endpoint {host}:{port} reported daemon identity {:?}, expected {:?}",
                    server_identity_id, expected_server_identity
                ));
            }
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
        let (_heartbeat_ty, heartbeat_reply) =
            read_probe_reply(&mut self.stream, &self.host, self.port, MessageType::HeartbeatAck)?;
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
            .write_all(&encode_message(MessageType::ListSessions, &[]))
            .map_err(|error| {
                format!(
                    "failed to send list tabs request to {}:{}: {error}",
                    self.host, self.port
                )
            })?;
        let (_reply_ty, payload) =
            read_probe_reply(&mut self.stream, &self.host, self.port, MessageType::SessionList)?;
        decode_tab_list_payload(&payload).map_err(|error| {
            format!(
                "failed to decode remote tab list from {}:{}: {error}",
                self.host, self.port
            )
        })
    }

    #[allow(dead_code)]
    pub(crate) fn list_sessions(&mut self) -> Result<Vec<RemoteDirectTabInfo>, String> {
        self.list_tabs()
    }

    pub(crate) fn attach(
        &mut self,
        tab_id: u32,
        attachment_id: Option<u64>,
        resume_token: Option<u64>,
    ) -> Result<(RemoteAttachedSummary, RemoteFullState), String> {
        let mut attach_payload = tab_id.to_le_bytes().to_vec();
        if let Some(attachment_id) = attachment_id {
            attach_payload.extend_from_slice(&attachment_id.to_le_bytes());
        }
        if let Some(resume_token) = resume_token {
            if attachment_id.is_none() {
                return Err("resume token requires attachment id".to_string());
            }
            attach_payload.extend_from_slice(&resume_token.to_le_bytes());
        }
        self.stream
            .write_all(&encode_message(MessageType::Attach, &attach_payload))
            .map_err(|error| {
                format!(
                    "failed to send attach request to {}:{}: {error}",
                    self.host, self.port
                )
            })?;

        read_attach_bootstrap(&mut self.stream, &self.host, self.port)
    }

    pub(crate) fn create_tab(&mut self, cols: u16, rows: u16) -> Result<u32, String> {
        let mut payload = Vec::with_capacity(4);
        payload.extend_from_slice(&cols.to_le_bytes());
        payload.extend_from_slice(&rows.to_le_bytes());
        self.stream
            .write_all(&encode_message(MessageType::Create, &payload))
            .map_err(|error| {
                format!(
                    "failed to send create request to {}:{}: {error}",
                    self.host, self.port
                )
            })?;
        let (_reply_ty, payload) =
            read_probe_reply(&mut self.stream, &self.host, self.port, MessageType::SessionCreated)?;
        parse_created_tab_id(&payload).ok_or_else(|| {
            format!(
                "invalid tab-created payload from remote endpoint {}:{}",
                self.host, self.port
            )
        })
    }

    #[allow(dead_code)]
    pub(crate) fn create_session(&mut self, cols: u16, rows: u16) -> Result<u32, String> {
        self.create_tab(cols, rows)
    }
}
