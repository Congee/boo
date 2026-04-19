//! Sync direct-client session transport for remote daemon RPCs.

use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::io::{Read, Write};

#[cfg(test)]
use crate::remote_transport::{PinnedQuicConnector, PinnedTlsConnector, connect_with};
use crate::remote_transport::{QuicClientStream, TlsClientStream};
use crate::remote_types::{RemoteAttachedSummary, RemoteDirectSessionInfo};
use crate::remote_wire::{
    MessageType, RemoteFullState, decode_auth_ok_payload, decode_session_list_payload,
    encode_message, parse_session_id, read_attach_bootstrap, read_probe_auth_reply,
    read_probe_reply, validate_auth_ok_payload,
};

type HmacSha256 = Hmac<Sha256>;

pub(crate) trait DirectReadWrite: Read + Write {}
impl<T: Read + Write> DirectReadWrite for T {}

pub(crate) struct DirectTransportSession<S: DirectReadWrite> {
    pub(crate) stream: S,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) auth_required: bool,
    pub(crate) protocol_version: u16,
    pub(crate) capabilities: u32,
    pub(crate) build_id: Option<String>,
    pub(crate) server_instance_id: Option<String>,
    pub(crate) server_identity_id: Option<String>,
}

impl DirectTransportSession<QuicClientStream> {
    #[cfg(test)]
    pub(crate) fn connect_quic(
        host: &str,
        port: u16,
        auth_key: Option<&str>,
        expected_identity: &str,
    ) -> Result<Self, String> {
        connect_with(
            PinnedQuicConnector { expected_identity },
            host,
            port,
            auth_key,
            Some(expected_identity),
        )
    }
}

impl DirectTransportSession<TlsClientStream> {
    #[cfg(test)]
    pub(crate) fn connect_tls(
        host: &str,
        port: u16,
        auth_key: Option<&str>,
        expected_identity: &str,
    ) -> Result<Self, String> {
        connect_with(
            PinnedTlsConnector { expected_identity },
            host,
            port,
            auth_key,
            Some(expected_identity),
        )
    }
}

impl<S: DirectReadWrite> DirectTransportSession<S> {
    pub(crate) fn connect_over_stream(
        mut stream: S,
        host: String,
        port: u16,
        auth_key: Option<&str>,
        expected_server_identity: Option<&str>,
    ) -> Result<Self, String> {
        stream
            .write_all(&encode_message(MessageType::Auth, &[]))
            .map_err(|error| format!("failed to send auth request to {host}:{port}: {error}"))?;
        let (ty, auth_payload) = read_probe_auth_reply(&mut stream, &host, port)?;
        let (auth_required, auth_ok_payload) = match ty {
            MessageType::AuthOk => (false, auth_payload),
            MessageType::AuthChallenge => {
                let key = auth_key
                    .ok_or_else(|| format!("remote endpoint {host}:{port} requires --auth-key"))?;
                let mut mac = HmacSha256::new_from_slice(key.as_bytes()).expect("valid HMAC key");
                mac.update(&auth_payload);
                let response = mac.finalize().into_bytes().to_vec();
                stream.write_all(&encode_message(MessageType::Auth, &response)).map_err(
                    |error| format!("failed to send auth response to {host}:{port}: {error}"),
                )?;
                let (reply_ty, reply_payload) =
                    crate::remote_wire::read_message_retrying(&mut stream, 2).map_err(
                        |error| {
                            format!(
                                "failed to read authenticated reply from {host}:{port}: {error}"
                            )
                        },
                    )?;
                if reply_ty != MessageType::AuthOk {
                    return Err(format!(
                        "expected auth ok from {host}:{port}, got {reply_ty:?}"
                    ));
                }
                (true, reply_payload)
            }
            MessageType::AuthFail => {
                return Err(format!("authentication failed for remote endpoint {host}:{port}"));
            }
            other => {
                return Err(format!(
                    "unexpected auth reply from {host}:{port}: {other:?}"
                ));
            }
        };

        validate_auth_ok_payload(&auth_ok_payload, auth_required)?;
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
            auth_required,
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

    pub(crate) fn list_sessions(&mut self) -> Result<Vec<RemoteDirectSessionInfo>, String> {
        self.stream
            .write_all(&encode_message(MessageType::ListSessions, &[]))
            .map_err(|error| {
                format!(
                    "failed to send list sessions request to {}:{}: {error}",
                    self.host, self.port
                )
            })?;
        let (_reply_ty, payload) =
            read_probe_reply(&mut self.stream, &self.host, self.port, MessageType::SessionList)?;
        decode_session_list_payload(&payload).map_err(|error| {
            format!(
                "failed to decode remote session list from {}:{}: {error}",
                self.host, self.port
            )
        })
    }

    pub(crate) fn attach(
        &mut self,
        session_id: u32,
        attachment_id: Option<u64>,
        resume_token: Option<u64>,
    ) -> Result<(RemoteAttachedSummary, RemoteFullState), String> {
        let mut attach_payload = session_id.to_le_bytes().to_vec();
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

    pub(crate) fn create_session(&mut self, cols: u16, rows: u16) -> Result<u32, String> {
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
        parse_session_id(&payload).ok_or_else(|| {
            format!(
                "invalid session-created payload from remote endpoint {}:{}",
                self.host, self.port
            )
        })
    }
}
