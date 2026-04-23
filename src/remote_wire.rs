//! Wire-format primitives for the remote protocol.
//!
//! Pure serde helpers with no server state: frame header layout, protocol
//! version / capability flags, the `MessageType` opcode set, and the encode /
//! decode / parse functions that translate between rust types and the byte
//! stream used over local stream and QUIC transports.
//!
//! Anything that mutates `State` or touches the socket lifecycle lives in
//! `remote.rs`; this module is intentionally dependency-free so it can be
//! reused by direct-client RPCs, the daemon, and test harnesses alike.

use std::io::{self, Read};
use std::time::{Duration, Instant};

use crate::remote_types::{RemoteDirectTabInfo, RemoteTabInfo};

pub(crate) const MAGIC: [u8; 2] = [0x47, 0x53];
pub(crate) const HEADER_LEN: usize = 7;

pub const REMOTE_PROTOCOL_VERSION: u16 = 1;
pub const REMOTE_CAPABILITY_SCREEN_DELTAS: u32 = 1 << 1;
pub const REMOTE_CAPABILITY_UI_STATE: u32 = 1 << 2;
pub const REMOTE_CAPABILITY_IMAGES: u32 = 1 << 3;
pub const REMOTE_CAPABILITY_HEARTBEAT: u32 = 1 << 4;
pub const REMOTE_CAPABILITY_DAEMON_IDENTITY: u32 = 1 << 6;
pub const REMOTE_CAPABILITY_QUIC_DIRECT_TRANSPORT: u32 = 1 << 7;
pub const REMOTE_CAPABILITIES: u32 = REMOTE_CAPABILITY_SCREEN_DELTAS
    | REMOTE_CAPABILITY_UI_STATE
    | REMOTE_CAPABILITY_IMAGES
    | REMOTE_CAPABILITY_HEARTBEAT
    | REMOTE_CAPABILITY_DAEMON_IDENTITY
    | REMOTE_CAPABILITY_QUIC_DIRECT_TRANSPORT;

pub(crate) const LOCAL_INPUT_SEQ_LEN: usize = 8;
pub(crate) const REMOTE_FULL_STATE_HEADER_LEN: usize = 14;
pub(crate) const REMOTE_DELTA_HEADER_LEN: usize = 13;
#[cfg(test)]
pub(crate) const LOCAL_DELTA_HEADER_LEN: usize = LOCAL_INPUT_SEQ_LEN + REMOTE_DELTA_HEADER_LEN;
pub(crate) const REMOTE_CELL_ENCODED_LEN: usize = 12;

pub const STYLE_FLAG_BOLD: u8 = 0x01;
pub const STYLE_FLAG_ITALIC: u8 = 0x02;
pub const STYLE_FLAG_HYPERLINK: u8 = 0x04;
pub const STYLE_FLAG_EXPLICIT_FG: u8 = 0x20;
pub const STYLE_FLAG_EXPLICIT_BG: u8 = 0x40;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageType {
    Auth = 0x01,
    ListTabs = 0x02,
    Create = 0x05,
    Input = 0x06,
    Resize = 0x07,
    Destroy = 0x08,
    Scroll = 0x0a,
    Key = 0x0b,
    ExecuteCommand = 0x0c,
    AppAction = 0x0d,
    AppKeyEvent = 0x0e,
    FocusPane = 0x0f,
    AppMouseEvent = 0x10,
    Heartbeat = 0x11,
    RuntimeAction = 0x12,

    AuthOk = 0x80,
    AuthFail = 0x81,
    TabList = 0x82,
    FullState = 0x83,
    Delta = 0x84,
    ErrorMsg = 0x87,
    TabCreated = 0x88,
    TabExited = 0x89,
    ScrollData = 0x8a,
    Clipboard = 0x8b,
    Image = 0x8c,
    UiRuntimeState = 0x8d,
    UiAppearance = 0x8e,
    UiPaneFullState = 0x90,
    UiPaneDelta = 0x91,
    HeartbeatAck = 0x92,
}

pub const MESSAGE_TYPE_LIST_TABS: MessageType = MessageType::ListTabs;
pub const MESSAGE_TYPE_TAB_LIST: MessageType = MessageType::TabList;
pub const MESSAGE_TYPE_TAB_CREATED: MessageType = MessageType::TabCreated;
pub const MESSAGE_TYPE_TAB_EXITED: MessageType = MessageType::TabExited;

#[repr(u16)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteErrorCode {
    Unknown = 0,
    AuthenticationFailed = 1,
    UnknownTab = 2,
    FailedCreateTab = 3,
    NoActiveTab = 4,
    CannotDestroyLastTab = 5,
    HeartbeatTimeout = 11,
}

impl TryFrom<u16> for RemoteErrorCode {
    type Error = ();

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        let code = match value {
            0 => Self::Unknown,
            1 => Self::AuthenticationFailed,
            2 => Self::UnknownTab,
            3 => Self::FailedCreateTab,
            4 => Self::NoActiveTab,
            5 => Self::CannotDestroyLastTab,
            11 => Self::HeartbeatTimeout,
            _ => return Err(()),
        };
        Ok(code)
    }
}

impl RemoteErrorCode {
    pub const fn default_message(self) -> &'static str {
        match self {
            Self::Unknown => "remote error",
            Self::AuthenticationFailed => "Authentication failed",
            Self::UnknownTab => "unknown tab",
            Self::FailedCreateTab => "failed to create tab",
            Self::NoActiveTab => "no active tab",
            Self::CannotDestroyLastTab => "cannot destroy last tab",
            Self::HeartbeatTimeout => "heartbeat timeout",
        }
    }
}

pub fn encode_error_payload(code: RemoteErrorCode, message: &str) -> Vec<u8> {
    let mut payload = Vec::with_capacity(2 + message.len());
    payload.extend_from_slice(&(code as u16).to_le_bytes());
    payload.extend_from_slice(message.as_bytes());
    payload
}

pub fn decode_error_payload(payload: &[u8]) -> Result<(RemoteErrorCode, String), String> {
    if payload.len() < 2 {
        return Err("payload too short".to_string());
    }
    let raw = u16::from_le_bytes(
        payload[..2]
            .try_into()
            .map_err(|_| "invalid error code".to_string())?,
    );
    let code = RemoteErrorCode::try_from(raw).unwrap_or(RemoteErrorCode::Unknown);
    let message = std::str::from_utf8(&payload[2..])
        .map_err(|_| "invalid utf-8".to_string())?
        .to_string();
    Ok((code, message))
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogicalChannel {
    Control,
    RuntimeStream,
    InputControl,
    Health,
}

#[cfg_attr(not(test), allow(dead_code))]
pub const fn logical_channel_for_message_type(message_type: MessageType) -> LogicalChannel {
    match message_type {
        MessageType::Auth
        | MessageType::AuthOk
        | MessageType::AuthFail
        | MessageType::ListTabs
        | MessageType::TabList
        | MessageType::Create
        | MessageType::TabCreated
        | MessageType::Destroy
        | MessageType::TabExited
        | MessageType::ErrorMsg => LogicalChannel::Control,
        MessageType::FullState
        | MessageType::Delta
        | MessageType::ScrollData
        | MessageType::UiRuntimeState
        | MessageType::UiAppearance
        | MessageType::UiPaneFullState
        | MessageType::UiPaneDelta => LogicalChannel::RuntimeStream,
        MessageType::Input
        | MessageType::Resize
        | MessageType::Scroll
        | MessageType::Key
        | MessageType::ExecuteCommand
        | MessageType::AppAction
        | MessageType::AppKeyEvent
        | MessageType::AppMouseEvent
        | MessageType::FocusPane
        | MessageType::RuntimeAction => LogicalChannel::InputControl,
        MessageType::Heartbeat | MessageType::HeartbeatAck => LogicalChannel::Health,
        MessageType::Clipboard | MessageType::Image => LogicalChannel::RuntimeStream,
    }
}

impl TryFrom<u8> for MessageType {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        let message = match value {
            0x01 => Self::Auth,
            0x02 => Self::ListTabs,
            0x05 => Self::Create,
            0x06 => Self::Input,
            0x07 => Self::Resize,
            0x08 => Self::Destroy,
            0x0a => Self::Scroll,
            0x0b => Self::Key,
            0x0c => Self::ExecuteCommand,
            0x0d => Self::AppAction,
            0x0e => Self::AppKeyEvent,
            0x0f => Self::FocusPane,
            0x10 => Self::AppMouseEvent,
            0x11 => Self::Heartbeat,
            0x12 => Self::RuntimeAction,
            0x80 => Self::AuthOk,
            0x81 => Self::AuthFail,
            0x82 => Self::TabList,
            0x83 => Self::FullState,
            0x84 => Self::Delta,
            0x87 => Self::ErrorMsg,
            0x88 => Self::TabCreated,
            0x89 => Self::TabExited,
            0x8a => Self::ScrollData,
            0x8b => Self::Clipboard,
            0x8c => Self::Image,
            0x8d => Self::UiRuntimeState,
            0x8e => Self::UiAppearance,
            0x90 => Self::UiPaneFullState,
            0x91 => Self::UiPaneDelta,
            0x92 => Self::HeartbeatAck,
            _ => return Err(()),
        };
        Ok(message)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteCell {
    pub codepoint: u32,
    pub fg: [u8; 3],
    pub bg: [u8; 3],
    pub style_flags: u8,
    pub wide: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteFullState {
    pub rows: u16,
    pub cols: u16,
    pub cursor_x: u16,
    pub cursor_y: u16,
    pub cursor_visible: bool,
    pub cursor_blinking: bool,
    pub cursor_style: i32,
    pub cells: Vec<RemoteCell>,
}

pub(crate) const UI_PANE_UPDATE_HEADER_LEN: usize = 28;

pub fn encode_ui_pane_update_payload(
    tab_id: u32,
    pane_id: u64,
    pane_revision: u64,
    runtime_revision: u64,
    payload: &[u8],
) -> Vec<u8> {
    let mut prefixed = Vec::with_capacity(UI_PANE_UPDATE_HEADER_LEN + payload.len());
    prefixed.extend_from_slice(&tab_id.to_le_bytes());
    prefixed.extend_from_slice(&pane_id.to_le_bytes());
    prefixed.extend_from_slice(&pane_revision.to_le_bytes());
    prefixed.extend_from_slice(&runtime_revision.to_le_bytes());
    prefixed.extend_from_slice(payload);
    prefixed
}

pub fn encode_message(ty: MessageType, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(HEADER_LEN + payload.len());
    frame.extend_from_slice(&MAGIC);
    frame.push(ty as u8);
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(payload);
    frame
}

pub fn read_message(stream: &mut impl Read) -> io::Result<(MessageType, Vec<u8>)> {
    let mut header = [0u8; HEADER_LEN];
    stream.read_exact(&mut header)?;
    if header[..2] != MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid remote magic",
        ));
    }
    let ty = MessageType::try_from(header[2])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "unknown remote message"))?;
    let payload_len = u32::from_le_bytes([header[3], header[4], header[5], header[6]]) as usize;
    let mut payload = vec![0u8; payload_len];
    if payload_len > 0 {
        stream.read_exact(&mut payload)?;
    }
    Ok((ty, payload))
}

fn read_exact_retrying(
    stream: &mut impl Read,
    buf: &mut [u8],
    max_idle_errors: usize,
) -> io::Result<()> {
    let mut offset = 0usize;
    let mut idle_errors = 0usize;
    while offset < buf.len() {
        match stream.read(&mut buf[offset..]) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "unexpected EOF while reading remote message",
                ));
            }
            Ok(n) => {
                offset += n;
                idle_errors = 0;
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                if offset == 0 {
                    return Err(error);
                }
                idle_errors += 1;
                if idle_errors > max_idle_errors {
                    return Err(error);
                }
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub(crate) fn read_message_retrying(
    stream: &mut impl Read,
    max_idle_errors: usize,
) -> io::Result<(MessageType, Vec<u8>)> {
    let mut header = [0u8; HEADER_LEN];
    read_exact_retrying(stream, &mut header, max_idle_errors)?;
    if header[..2] != MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid remote magic",
        ));
    }
    let ty = MessageType::try_from(header[2])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "unknown remote message"))?;
    let payload_len = u32::from_le_bytes([header[3], header[4], header[5], header[6]]) as usize;
    let mut payload = vec![0u8; payload_len];
    if payload_len > 0 {
        read_exact_retrying(stream, &mut payload, max_idle_errors)?;
    }
    Ok((ty, payload))
}

pub(crate) fn encode_auth_ok_payload(
    server_identity_id: &str,
    server_instance_id: &str,
) -> Vec<u8> {
    let build_id = env!("CARGO_PKG_VERSION").as_bytes();
    let server_identity_id = server_identity_id.as_bytes();
    let server_instance_id = server_instance_id.as_bytes();
    let mut payload = Vec::with_capacity(
        12 + build_id.len() + server_identity_id.len() + server_instance_id.len(),
    );
    payload.extend_from_slice(&REMOTE_PROTOCOL_VERSION.to_le_bytes());
    payload.extend_from_slice(&REMOTE_CAPABILITIES.to_le_bytes());
    payload.extend_from_slice(&(build_id.len() as u16).to_le_bytes());
    payload.extend_from_slice(build_id);
    payload.extend_from_slice(&(server_instance_id.len() as u16).to_le_bytes());
    payload.extend_from_slice(server_instance_id);
    payload.extend_from_slice(&(server_identity_id.len() as u16).to_le_bytes());
    payload.extend_from_slice(server_identity_id);
    payload
}

type AuthOkPayloadParts = (u16, u32, Option<String>, Option<String>, Option<String>);

#[cfg_attr(not(test), allow(dead_code))]
pub fn decode_auth_ok_payload(payload: &[u8]) -> Option<AuthOkPayloadParts> {
    if payload.is_empty() {
        return None;
    }
    if payload.len() < 6 {
        return None;
    }
    let version = u16::from_le_bytes([payload[0], payload[1]]);
    let capabilities = u32::from_le_bytes([payload[2], payload[3], payload[4], payload[5]]);
    if payload.len() < 8 {
        return Some((version, capabilities, None, None, None));
    }
    let build_len = u16::from_le_bytes([payload[6], payload[7]]) as usize;
    if payload.len() < 8 + build_len {
        return None;
    }
    let build_id = String::from_utf8(payload[8..8 + build_len].to_vec()).ok();
    if payload.len() < 10 + build_len {
        return Some((version, capabilities, build_id, None, None));
    }
    let instance_offset = 8 + build_len;
    let instance_len =
        u16::from_le_bytes([payload[instance_offset], payload[instance_offset + 1]]) as usize;
    if payload.len() < instance_offset + 2 + instance_len {
        return None;
    }
    let server_instance_id = String::from_utf8(
        payload[instance_offset + 2..instance_offset + 2 + instance_len].to_vec(),
    )
    .ok();
    let identity_offset = instance_offset + 2 + instance_len;
    if payload.len() < identity_offset + 2 {
        return Some((version, capabilities, build_id, server_instance_id, None));
    }
    let identity_len =
        u16::from_le_bytes([payload[identity_offset], payload[identity_offset + 1]]) as usize;
    if payload.len() < identity_offset + 2 + identity_len {
        return None;
    }
    let server_identity_id = String::from_utf8(
        payload[identity_offset + 2..identity_offset + 2 + identity_len].to_vec(),
    )
    .ok();
    Some((
        version,
        capabilities,
        build_id,
        server_instance_id,
        server_identity_id,
    ))
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn validate_auth_ok_payload(payload: &[u8]) -> Result<(), String> {
    let Some((version, capabilities, build_id, server_instance_id, server_identity_id)) =
        decode_auth_ok_payload(payload)
    else {
        return Err("Remote handshake is malformed".to_string());
    };
    if version != REMOTE_PROTOCOL_VERSION {
        return Err(format!("Unsupported remote protocol version: {version}"));
    }
    if (capabilities & REMOTE_CAPABILITY_HEARTBEAT) == 0 {
        return Err("Remote server does not advertise heartbeat support".to_string());
    }
    if (capabilities & REMOTE_CAPABILITY_DAEMON_IDENTITY) == 0 {
        return Err("Remote server does not advertise daemon identity support".to_string());
    }
    if (capabilities & REMOTE_CAPABILITY_QUIC_DIRECT_TRANSPORT) == 0 {
        return Err("Remote server does not advertise QUIC direct transport".to_string());
    }
    if build_id.as_deref().is_none_or(str::is_empty) {
        return Err("Remote handshake is missing server build metadata".to_string());
    }
    if server_instance_id.as_deref().is_none_or(str::is_empty) {
        return Err("Remote handshake is missing server instance metadata".to_string());
    }
    if server_identity_id.as_deref().is_none_or(str::is_empty) {
        return Err("Remote handshake is missing server identity metadata".to_string());
    }
    Ok(())
}

pub(crate) fn read_probe_reply(
    stream: &mut impl Read,
    host: &str,
    port: u16,
    expected: MessageType,
) -> Result<(MessageType, Vec<u8>), String> {
    for _ in 0..8 {
        let (ty, payload) = match read_message_retrying(stream, 2) {
            Ok(message) => message,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                continue;
            }
            Err(error) => {
                return Err(format!(
                    "failed to read probe reply from {host}:{port}: {error}"
                ));
            }
        };
        if ty == expected {
            return Ok((ty, payload));
        }
        match ty {
            MessageType::TabList
            | MessageType::FullState
            | MessageType::Delta
            | MessageType::UiRuntimeState
            | MessageType::UiAppearance
            | MessageType::UiPaneFullState
            | MessageType::UiPaneDelta => continue,
            MessageType::AuthFail => {
                return Err(format!(
                    "authentication failed for remote endpoint {host}:{port}"
                ));
            }
            MessageType::ErrorMsg => {
                let (_, message) = decode_error_payload(&payload)
                    .unwrap_or((RemoteErrorCode::Unknown, "remote error".to_string()));
                return Err(format!(
                    "remote endpoint {host}:{port} reported probe error: {message}"
                ));
            }
            other => {
                return Err(format!(
                    "expected {expected:?} from {host}:{port}, got {other:?}"
                ));
            }
        }
    }
    Err(format!(
        "timed out waiting for {expected:?} from remote endpoint {host}:{port}"
    ))
}

pub(crate) fn read_probe_auth_reply(
    stream: &mut impl Read,
    host: &str,
    port: u16,
) -> Result<(MessageType, Vec<u8>), String> {
    for _ in 0..8 {
        let (ty, payload) = match read_message_retrying(stream, 2) {
            Ok(message) => message,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                continue;
            }
            Err(error) => {
                return Err(format!(
                    "failed to read auth reply from {host}:{port}: {error}"
                ));
            }
        };
        match ty {
            MessageType::AuthOk | MessageType::AuthFail => {
                return Ok((ty, payload));
            }
            MessageType::TabList
            | MessageType::FullState
            | MessageType::Delta
            | MessageType::UiRuntimeState
            | MessageType::UiAppearance
            | MessageType::UiPaneFullState
            | MessageType::UiPaneDelta => continue,
            other => {
                return Err(format!(
                    "unexpected auth reply from {host}:{port}: {other:?}"
                ));
            }
        }
    }
    Err(format!(
        "timed out waiting for auth reply from remote endpoint {host}:{port}"
    ))
}

pub(crate) fn decode_tab_list_payload(payload: &[u8]) -> Result<Vec<RemoteDirectTabInfo>, String> {
    if payload.len() < 4 {
        return Err("payload too short".to_string());
    }
    let mut offset = 0usize;
    let count = u32::from_le_bytes(
        payload[offset..offset + 4]
            .try_into()
            .map_err(|_| "invalid tab count".to_string())?,
    ) as usize;
    offset += 4;

    fn read_u32(payload: &[u8], offset: &mut usize) -> Result<u32, String> {
        if payload.len() < *offset + 4 {
            return Err("payload truncated".to_string());
        }
        let value = u32::from_le_bytes(
            payload[*offset..*offset + 4]
                .try_into()
                .map_err(|_| "invalid u32".to_string())?,
        );
        *offset += 4;
        Ok(value)
    }

    fn read_u16(payload: &[u8], offset: &mut usize) -> Result<u16, String> {
        if payload.len() < *offset + 2 {
            return Err("payload truncated".to_string());
        }
        let value = u16::from_le_bytes(
            payload[*offset..*offset + 2]
                .try_into()
                .map_err(|_| "invalid u16".to_string())?,
        );
        *offset += 2;
        Ok(value)
    }

    fn read_string(payload: &[u8], offset: &mut usize) -> Result<String, String> {
        let len = read_u16(payload, offset)? as usize;
        if payload.len() < *offset + len {
            return Err("payload truncated".to_string());
        }
        let value = std::str::from_utf8(&payload[*offset..*offset + len])
            .map_err(|_| "invalid utf-8".to_string())?
            .to_string();
        *offset += len;
        Ok(value)
    }

    let mut tabs = Vec::with_capacity(count);
    for _ in 0..count {
        let id = read_u32(payload, &mut offset)?;
        let name = read_string(payload, &mut offset)?;
        let title = read_string(payload, &mut offset)?;
        let pwd = read_string(payload, &mut offset)?;
        let flags = *payload
            .get(offset)
            .ok_or_else(|| "payload truncated".to_string())?;
        offset += 1;
        tabs.push(RemoteDirectTabInfo {
            id,
            name,
            title,
            pwd,
            active: (flags & 0x01) != 0,
            child_exited: (flags & 0x02) != 0,
        });
    }
    if offset != payload.len() {
        return Err("payload has trailing bytes".to_string());
    }
    Ok(tabs)
}

#[cfg(test)]
pub(crate) fn decode_remote_full_state_payload(payload: &[u8]) -> Result<RemoteFullState, String> {
    if payload.len() < REMOTE_FULL_STATE_HEADER_LEN {
        return Err("payload too short".to_string());
    }
    let rows = u16::from_le_bytes(
        payload[0..2]
            .try_into()
            .map_err(|_| "invalid rows".to_string())?,
    );
    let cols = u16::from_le_bytes(
        payload[2..4]
            .try_into()
            .map_err(|_| "invalid cols".to_string())?,
    );
    let cursor_x = u16::from_le_bytes(
        payload[4..6]
            .try_into()
            .map_err(|_| "invalid cursor_x".to_string())?,
    );
    let cursor_y = u16::from_le_bytes(
        payload[6..8]
            .try_into()
            .map_err(|_| "invalid cursor_y".to_string())?,
    );
    let cursor_visible = payload[8] != 0;
    let cursor_blinking = payload[9] != 0;
    let cursor_style = i32::from_le_bytes(
        payload[10..14]
            .try_into()
            .map_err(|_| "invalid cursor_style".to_string())?,
    );
    let cell_count = rows as usize * cols as usize;
    let expected_len = REMOTE_FULL_STATE_HEADER_LEN + cell_count * REMOTE_CELL_ENCODED_LEN;
    if payload.len() != expected_len {
        return Err("payload length does not match grid size".to_string());
    }
    let mut cells = Vec::with_capacity(cell_count);
    let mut offset = REMOTE_FULL_STATE_HEADER_LEN;
    for _ in 0..cell_count {
        let codepoint = u32::from_le_bytes(
            payload[offset..offset + 4]
                .try_into()
                .map_err(|_| "invalid codepoint".to_string())?,
        );
        let fg = [
            payload[offset + 4],
            payload[offset + 5],
            payload[offset + 6],
        ];
        let bg = [
            payload[offset + 7],
            payload[offset + 8],
            payload[offset + 9],
        ];
        let style_flags = payload[offset + 10];
        let wide = payload[offset + 11] != 0;
        cells.push(RemoteCell {
            codepoint,
            fg,
            bg,
            style_flags,
            wide,
        });
        offset += REMOTE_CELL_ENCODED_LEN;
    }
    Ok(RemoteFullState {
        rows,
        cols,
        cursor_x,
        cursor_y,
        cursor_visible,
        cursor_blinking,
        cursor_style,
        cells,
    })
}

pub(crate) fn parse_tab_id(payload: &[u8]) -> Option<u32> {
    (payload.len() >= 4)
        .then(|| u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]))
}

pub(crate) fn parse_created_tab_id(payload: &[u8]) -> Option<u32> {
    parse_tab_id(payload)
}

pub(crate) fn parse_pane_id(payload: &[u8]) -> Option<u64> {
    (payload.len() >= 8).then(|| {
        u64::from_le_bytes([
            payload[0], payload[1], payload[2], payload[3], payload[4], payload[5], payload[6],
            payload[7],
        ])
    })
}

pub(crate) fn parse_resize(payload: &[u8]) -> Option<(u16, u16)> {
    (payload.len() >= 4).then(|| {
        (
            u16::from_le_bytes([payload[0], payload[1]]),
            u16::from_le_bytes([payload[2], payload[3]]),
        )
    })
}

pub(crate) fn parse_input_payload(
    payload: &[u8],
    is_local: bool,
) -> Option<(Option<u64>, Vec<u8>)> {
    if is_local {
        if payload.len() < 8 {
            return None;
        }
        let input_seq = u64::from_le_bytes(payload[..8].try_into().ok()?);
        return Some((Some(input_seq), payload[8..].to_vec()));
    }
    Some((None, payload.to_vec()))
}

pub(crate) fn parse_key_payload(payload: &[u8], is_local: bool) -> Option<(Option<u64>, Vec<u8>)> {
    parse_input_payload(payload, is_local)
}

pub fn encode_tab_list(tabs: &[RemoteTabInfo]) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&(tabs.len() as u32).to_le_bytes());
    for tab in tabs {
        payload.extend_from_slice(&tab.id.to_le_bytes());
        push_string(&mut payload, &tab.name);
        push_string(&mut payload, &tab.title);
        push_string(&mut payload, &tab.pwd);
        let mut flags = 0u8;
        if tab.active {
            flags |= 0x01;
        }
        if tab.child_exited {
            flags |= 0x02;
        }
        payload.push(flags);
    }
    payload
}

pub fn encode_full_state(
    state: &RemoteFullState,
    latest_input_seq: Option<u64>,
    local: bool,
) -> Vec<u8> {
    let prefix_len = if local { LOCAL_INPUT_SEQ_LEN } else { 0 };
    let mut payload = Vec::with_capacity(
        prefix_len + REMOTE_FULL_STATE_HEADER_LEN + state.cells.len() * REMOTE_CELL_ENCODED_LEN,
    );
    if local {
        payload.extend_from_slice(&latest_input_seq.unwrap_or(0).to_le_bytes());
    }
    payload.extend_from_slice(&state.rows.to_le_bytes());
    payload.extend_from_slice(&state.cols.to_le_bytes());
    payload.extend_from_slice(&state.cursor_x.to_le_bytes());
    payload.extend_from_slice(&state.cursor_y.to_le_bytes());
    payload.push(u8::from(state.cursor_visible));
    payload.push(u8::from(state.cursor_blinking));
    payload.extend_from_slice(&state.cursor_style.to_le_bytes());
    for cell in &state.cells {
        payload.extend_from_slice(&cell.codepoint.to_le_bytes());
        payload.extend_from_slice(&cell.fg);
        payload.extend_from_slice(&cell.bg);
        payload.push(cell.style_flags);
        payload.push(u8::from(cell.wide));
    }
    crate::profiling::record_bytes_and_units(
        if local {
            "server.stream.encode_full_state.local"
        } else {
            "server.stream.encode_full_state.remote"
        },
        crate::profiling::Kind::Cpu,
        Duration::ZERO,
        payload.len() as u64,
        state.cells.len() as u64,
    );
    payload
}

pub(crate) fn encode_delta(
    previous: &RemoteFullState,
    current: &RemoteFullState,
    latest_input_seq: Option<u64>,
    local: bool,
) -> Option<Vec<u8>> {
    if previous.rows != current.rows || previous.cols != current.cols {
        return None;
    }
    if previous == current {
        let prefix_len = if local { LOCAL_INPUT_SEQ_LEN } else { 0 };
        let mut payload = Vec::with_capacity(prefix_len + REMOTE_DELTA_HEADER_LEN);
        if local {
            payload.extend_from_slice(&latest_input_seq.unwrap_or(0).to_le_bytes());
        }
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload.extend_from_slice(&current.cursor_x.to_le_bytes());
        payload.extend_from_slice(&current.cursor_y.to_le_bytes());
        payload.push(u8::from(current.cursor_visible));
        payload.push(u8::from(current.cursor_blinking));
        payload.push(0);
        payload.extend_from_slice(&current.cursor_style.to_le_bytes());
        return Some(payload);
    }

    let cols = current.cols as usize;
    let rows = current.rows as usize;
    let mut changed_rows = Vec::new();
    for row in 0..rows {
        let start = row * cols;
        let end = start + cols;
        if previous.cells[start..end] != current.cells[start..end] {
            changed_rows.push((
                row as u16,
                changed_segment(&previous.cells[start..end], &current.cells[start..end]),
            ));
        }
    }

    if changed_rows.is_empty() {
        let prefix_len = if local { LOCAL_INPUT_SEQ_LEN } else { 0 };
        let mut payload = Vec::with_capacity(prefix_len + REMOTE_DELTA_HEADER_LEN);
        if local {
            payload.extend_from_slice(&latest_input_seq.unwrap_or(0).to_le_bytes());
        }
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload.extend_from_slice(&current.cursor_x.to_le_bytes());
        payload.extend_from_slice(&current.cursor_y.to_le_bytes());
        payload.push(u8::from(current.cursor_visible));
        payload.push(u8::from(current.cursor_blinking));
        payload.push(0);
        payload.extend_from_slice(&current.cursor_style.to_le_bytes());
        return Some(payload);
    }

    let scroll_rows = if local {
        None
    } else {
        detect_scroll_rows(previous, current)
    };
    if changed_rows.len() == rows
        && scroll_rows.is_none()
        && changed_rows
            .iter()
            .all(|(_, (start_col, cells))| *start_col == 0 && cells.len() == cols)
    {
        return None;
    }

    let mut payload = Vec::new();
    if local {
        payload.extend_from_slice(&latest_input_seq.unwrap_or(0).to_le_bytes());
    }
    let rows_to_encode = if let Some(scroll_rows) = scroll_rows {
        rows_changed_after_scroll(current.rows as usize, scroll_rows)
            .into_iter()
            .map(|row| {
                let start = row as usize * cols;
                let end = start + cols;
                (
                    row,
                    changed_segment(&previous.cells[start..end], &current.cells[start..end]),
                )
            })
            .collect::<Vec<_>>()
    } else {
        changed_rows
    };
    let encoded_rows = rows_to_encode.len() as u64;
    let encoded_cells = rows_to_encode
        .iter()
        .map(|(_, (_, cells))| cells.len() as u64)
        .sum::<u64>();
    payload.extend_from_slice(&(rows_to_encode.len() as u16).to_le_bytes());
    payload.extend_from_slice(&current.cursor_x.to_le_bytes());
    payload.extend_from_slice(&current.cursor_y.to_le_bytes());
    payload.push(u8::from(current.cursor_visible));
    payload.push(u8::from(current.cursor_blinking));
    let mut flags = 0u8;
    if scroll_rows.is_some() {
        flags |= 0x01;
    }
    payload.push(flags);
    payload.extend_from_slice(&current.cursor_style.to_le_bytes());
    if let Some(scroll_rows) = scroll_rows {
        payload.extend_from_slice(&scroll_rows.to_le_bytes());
    }
    for (row, (start_col, cells)) in rows_to_encode {
        payload.extend_from_slice(&row.to_le_bytes());
        payload.extend_from_slice(&(start_col as u16).to_le_bytes());
        payload.extend_from_slice(&(cells.len() as u16).to_le_bytes());
        for cell in &cells {
            payload.extend_from_slice(&cell.codepoint.to_le_bytes());
            payload.extend_from_slice(&cell.fg);
            payload.extend_from_slice(&cell.bg);
            payload.push(cell.style_flags);
            payload.push(u8::from(cell.wide));
        }
    }
    crate::profiling::record_bytes_and_units(
        if local {
            "server.stream.encode_delta.local"
        } else {
            "server.stream.encode_delta.remote"
        },
        crate::profiling::Kind::Cpu,
        Duration::ZERO,
        payload.len() as u64,
        encoded_cells,
    );
    crate::profiling::record_units(
        if local {
            "server.stream.encode_delta_rows.local"
        } else {
            "server.stream.encode_delta_rows.remote"
        },
        crate::profiling::Kind::Cpu,
        encoded_rows,
    );
    Some(payload)
}

fn changed_segment(previous: &[RemoteCell], current: &[RemoteCell]) -> (usize, Vec<RemoteCell>) {
    debug_assert_eq!(previous.len(), current.len());
    let first = previous
        .iter()
        .zip(current.iter())
        .position(|(a, b)| a != b);
    let Some(first) = first else {
        return (0, Vec::new());
    };
    let last = previous
        .iter()
        .zip(current.iter())
        .rposition(|(a, b)| a != b)
        .unwrap_or(first);
    (first, current[first..=last].to_vec())
}

pub(crate) fn detect_scroll_rows(
    previous: &RemoteFullState,
    current: &RemoteFullState,
) -> Option<i16> {
    if previous.rows != current.rows || previous.cols != current.cols || current.rows <= 1 {
        return None;
    }
    let rows = current.rows as usize;
    let cols = current.cols as usize;
    if previous.cells[cols..rows * cols] == current.cells[..(rows - 1) * cols] {
        return Some(1);
    }
    if previous.cells[..(rows - 1) * cols] == current.cells[cols..rows * cols] {
        return Some(-1);
    }
    let previous_rows = row_fingerprints(previous);
    let current_rows = row_fingerprints(current);

    let positive_overlap = longest_prefix_suffix_overlap(&current_rows, &previous_rows);
    if positive_overlap > 0 {
        let shift = rows - positive_overlap;
        if previous.cells[shift * cols..rows * cols] == current.cells[..positive_overlap * cols] {
            return Some(shift as i16);
        }
    }

    let negative_overlap = longest_prefix_suffix_overlap(&previous_rows, &current_rows);
    if negative_overlap > 0 {
        let shift = rows - negative_overlap;
        if previous.cells[..negative_overlap * cols] == current.cells[shift * cols..rows * cols] {
            return Some(-(shift as i16));
        }
    }
    None
}

pub(crate) fn longest_prefix_suffix_overlap(prefix: &[u64], suffix_source: &[u64]) -> usize {
    if prefix.is_empty() || suffix_source.is_empty() {
        return 0;
    }

    let mut sequence = Vec::with_capacity(prefix.len() + 1 + suffix_source.len());
    sequence.extend(prefix.iter().copied().map(Some));
    sequence.push(None);
    sequence.extend(suffix_source.iter().copied().map(Some));

    let mut prefix_function = vec![0usize; sequence.len()];
    for index in 1..sequence.len() {
        let mut matched = prefix_function[index - 1];
        while matched > 0 && sequence[index] != sequence[matched] {
            matched = prefix_function[matched - 1];
        }
        if sequence[index] == sequence[matched] {
            matched += 1;
        }
        prefix_function[index] = matched;
    }

    prefix_function
        .last()
        .copied()
        .unwrap_or(0)
        .min(prefix.len())
}

fn row_fingerprints(state: &RemoteFullState) -> Vec<u64> {
    use std::hash::Hasher;

    let cols = state.cols as usize;
    state
        .cells
        .chunks(cols)
        .map(|row| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            for cell in row {
                hasher.write_u32(cell.codepoint);
                hasher.write(&cell.fg);
                hasher.write(&cell.bg);
                hasher.write_u8(cell.style_flags);
                hasher.write_u8(u8::from(cell.wide));
            }
            hasher.finish()
        })
        .collect()
}

fn rows_changed_after_scroll(rows: usize, scroll_rows: i16) -> Vec<u16> {
    if scroll_rows > 0 {
        let shift = scroll_rows as usize;
        ((rows.saturating_sub(shift))..rows)
            .map(|row| row as u16)
            .collect()
    } else {
        let shift = (-scroll_rows) as usize;
        (0..shift.min(rows)).map(|row| row as u16).collect()
    }
}

pub(crate) fn push_string(payload: &mut Vec<u8>, text: &str) {
    let bytes = text.as_bytes();
    let len = bytes.len().min(u16::MAX as usize);
    payload.extend_from_slice(&(len as u16).to_le_bytes());
    payload.extend_from_slice(&bytes[..len]);
}

pub(crate) fn random_challenge() -> [u8; 32] {
    let mut challenge = [0u8; 32];
    if let Ok(mut file) = std::fs::File::open("/dev/urandom") {
        let _ = file.read_exact(&mut challenge);
        if challenge.iter().any(|byte| *byte != 0) {
            return challenge;
        }
    }
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    for (idx, byte) in challenge.iter_mut().enumerate() {
        *byte = (seed.wrapping_shr((idx % 8) as u32) as u8) ^ (idx as u8).wrapping_mul(17);
    }
    challenge
}

pub(crate) fn random_instance_id() -> String {
    let challenge = random_challenge();
    let mut output = String::with_capacity(16);
    for byte in &challenge[..8] {
        use std::fmt::Write as _;
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

pub(crate) fn elapsed_ms(now: Instant, earlier: Instant) -> u64 {
    now.saturating_duration_since(earlier).as_millis() as u64
}

pub(crate) fn remaining_ms(now: Instant, deadline: Instant) -> u64 {
    deadline.saturating_duration_since(now).as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote_types::{RemoteDirectTabInfo, RemoteTabInfo};

    #[test]
    fn tab_list_encoding_matches_client_layout() {
        let payload = encode_tab_list(&[RemoteTabInfo {
            id: 7,
            name: "Tab 1".to_string(),
            title: "shell".to_string(),
            pwd: "/tmp".to_string(),
            active: true,
            child_exited: false,
        }]);
        assert_eq!(u32::from_le_bytes(payload[0..4].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(payload[4..8].try_into().unwrap()), 7);
        assert_eq!(u16::from_le_bytes(payload[8..10].try_into().unwrap()), 5);
        assert_eq!(&payload[10..15], b"Tab 1");
        assert_eq!(*payload.last().unwrap(), 0x01);
    }

    #[test]
    fn full_state_encoding_uses_12_byte_cells() {
        let payload = encode_full_state(
            &RemoteFullState {
                rows: 1,
                cols: 2,
                cursor_x: 1,
                cursor_y: 0,
                cursor_visible: true,
                cursor_blinking: true,
                cursor_style: 5,
                cells: vec![
                    RemoteCell {
                        codepoint: u32::from('A'),
                        fg: [1, 2, 3],
                        bg: [4, 5, 6],
                        style_flags: 0x21,
                        wide: false,
                    },
                    RemoteCell {
                        codepoint: u32::from('好'),
                        fg: [7, 8, 9],
                        bg: [10, 11, 12],
                        style_flags: 0x42,
                        wide: true,
                    },
                ],
            },
            None,
            false,
        );
        assert_eq!(
            payload.len(),
            REMOTE_FULL_STATE_HEADER_LEN + 2 * REMOTE_CELL_ENCODED_LEN
        );
        assert_eq!(u16::from_le_bytes(payload[0..2].try_into().unwrap()), 1);
        assert_eq!(u16::from_le_bytes(payload[2..4].try_into().unwrap()), 2);
        assert_eq!(
            u32::from_le_bytes(
                payload[REMOTE_FULL_STATE_HEADER_LEN..REMOTE_FULL_STATE_HEADER_LEN + 4]
                    .try_into()
                    .unwrap()
            ),
            u32::from('A')
        );
        let second_offset = REMOTE_FULL_STATE_HEADER_LEN + REMOTE_CELL_ENCODED_LEN;
        assert_eq!(payload[REMOTE_FULL_STATE_HEADER_LEN + 10], 0x21);
        assert_eq!(payload[REMOTE_FULL_STATE_HEADER_LEN + 11], 0);
        assert_eq!(
            u32::from_le_bytes(
                payload[second_offset..second_offset + 4]
                    .try_into()
                    .unwrap()
            ),
            u32::from('好')
        );
        assert_eq!(payload[second_offset + 10], 0x42);
        assert_eq!(payload[second_offset + 11], 1);
    }

    #[test]
    fn local_full_state_encoding_prefixes_latest_input_seq() {
        let payload = encode_full_state(
            &RemoteFullState {
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
                    bg: [4, 5, 6],
                    style_flags: 0,
                    wide: false,
                }],
            },
            Some(42),
            true,
        );
        assert_eq!(u64::from_le_bytes(payload[0..8].try_into().unwrap()), 42);
        assert_eq!(u16::from_le_bytes(payload[8..10].try_into().unwrap()), 1);
        assert_eq!(u16::from_le_bytes(payload[10..12].try_into().unwrap()), 1);
    }

    #[test]
    fn auth_ok_payload_round_trips_protocol_version_and_capabilities() {
        let frame = encode_message(
            MessageType::AuthOk,
            &encode_auth_ok_payload("daemon-identity-01", "deadbeefcafebabe"),
        );
        let mut cursor = std::io::Cursor::new(frame);
        let (ty, payload) = read_message(&mut cursor).expect("auth ok frame");
        assert_eq!(ty, MessageType::AuthOk);
        assert_eq!(
            decode_auth_ok_payload(&payload),
            Some((
                REMOTE_PROTOCOL_VERSION,
                REMOTE_CAPABILITIES,
                Some(env!("CARGO_PKG_VERSION").to_string()),
                Some("deadbeefcafebabe".to_string()),
                Some("daemon-identity-01".to_string()),
            ))
        );
    }

    #[test]
    fn logical_channel_mapping_matches_current_message_families() {
        assert_eq!(
            logical_channel_for_message_type(MessageType::Auth),
            LogicalChannel::Control
        );
        assert_eq!(
            logical_channel_for_message_type(MessageType::TabList),
            LogicalChannel::Control
        );
        assert_eq!(
            logical_channel_for_message_type(MessageType::Delta),
            LogicalChannel::RuntimeStream
        );
        assert_eq!(
            logical_channel_for_message_type(MessageType::UiPaneDelta),
            LogicalChannel::RuntimeStream
        );
        assert_eq!(
            logical_channel_for_message_type(MessageType::Input),
            LogicalChannel::InputControl
        );
        assert_eq!(
            logical_channel_for_message_type(MessageType::Scroll),
            LogicalChannel::InputControl
        );
        assert_eq!(
            logical_channel_for_message_type(MessageType::ExecuteCommand),
            LogicalChannel::InputControl
        );
        assert_eq!(
            logical_channel_for_message_type(MessageType::Heartbeat),
            LogicalChannel::Health
        );
        assert_eq!(
            logical_channel_for_message_type(MessageType::HeartbeatAck),
            LogicalChannel::Health
        );
    }

    #[test]
    fn validate_auth_ok_payload_accepts_current_handshake_contract() {
        let payload = encode_auth_ok_payload("daemon-identity-01", "deadbeefcafebabe");
        assert_eq!(validate_auth_ok_payload(&payload), Ok(()));
    }

    #[test]
    fn validate_auth_ok_payload_rejects_missing_heartbeat_capability() {
        let mut payload = encode_auth_ok_payload("daemon-identity-01", "deadbeefcafebabe");
        payload[2..6]
            .copy_from_slice(&(REMOTE_CAPABILITIES & !REMOTE_CAPABILITY_HEARTBEAT).to_le_bytes());
        assert_eq!(
            validate_auth_ok_payload(&payload),
            Err("Remote server does not advertise heartbeat support".to_string())
        );
    }

    #[test]
    fn validate_auth_ok_payload_rejects_missing_daemon_identity_metadata() {
        let mut payload = encode_auth_ok_payload("daemon-identity-01", "deadbeefcafebabe");
        payload.truncate(payload.len() - "daemon-identity-01".len());
        assert_eq!(
            validate_auth_ok_payload(&payload),
            Err("Remote handshake is malformed".to_string())
        );
    }

    #[test]
    fn validate_auth_ok_payload_rejects_missing_direct_transport_capability() {
        let mut payload = encode_auth_ok_payload("daemon-identity-01", "deadbeefcafebabe");
        payload[2..6].copy_from_slice(
            &(REMOTE_CAPABILITIES & !REMOTE_CAPABILITY_QUIC_DIRECT_TRANSPORT).to_le_bytes(),
        );
        assert_eq!(
            validate_auth_ok_payload(&payload),
            Err("Remote server does not advertise QUIC direct transport".to_string())
        );
    }

    #[test]
    fn read_probe_auth_reply_skips_unsolicited_tab_list() {
        let mut frames = Vec::new();
        frames.extend_from_slice(&encode_message(MESSAGE_TYPE_TAB_LIST, b"[]"));
        frames.extend_from_slice(&encode_message(
            MessageType::AuthOk,
            &encode_auth_ok_payload("test-daemon", "test-instance"),
        ));
        let (ty, payload) =
            read_probe_auth_reply(&mut std::io::Cursor::new(frames), "127.0.0.1", 7359)
                .expect("auth reply");
        assert_eq!(ty, MessageType::AuthOk);
        assert!(validate_auth_ok_payload(&payload).is_ok());
    }

    #[test]
    fn decode_tab_list_payload_round_trips_encoded_tabs() {
        let payload = encode_tab_list(&[
            RemoteTabInfo {
                id: 7,
                name: "Tab 1".to_string(),
                title: "shell".to_string(),
                pwd: "/tmp".to_string(),
                active: true,
                child_exited: false,
            },
            RemoteTabInfo {
                id: 8,
                name: String::new(),
                title: "logs".to_string(),
                pwd: "/var/log".to_string(),
                active: false,
                child_exited: true,
            },
        ]);

        let decoded = decode_tab_list_payload(&payload).expect("decode tab list");
        assert_eq!(
            decoded,
            vec![
                RemoteDirectTabInfo {
                    id: 7,
                    name: "Tab 1".to_string(),
                    title: "shell".to_string(),
                    pwd: "/tmp".to_string(),
                    active: true,
                    child_exited: false,
                },
                RemoteDirectTabInfo {
                    id: 8,
                    name: String::new(),
                    title: "logs".to_string(),
                    pwd: "/var/log".to_string(),
                    active: false,
                    child_exited: true,
                },
            ]
        );
    }

    #[test]
    fn decode_remote_full_state_payload_round_trips_encoded_state() {
        let state = RemoteFullState {
            rows: 1,
            cols: 2,
            cursor_x: 1,
            cursor_y: 0,
            cursor_visible: true,
            cursor_blinking: false,
            cursor_style: 2,
            cells: vec![
                RemoteCell {
                    codepoint: u32::from('A'),
                    fg: [1, 2, 3],
                    bg: [4, 5, 6],
                    style_flags: STYLE_FLAG_BOLD,
                    wide: false,
                },
                RemoteCell {
                    codepoint: u32::from('B'),
                    fg: [7, 8, 9],
                    bg: [10, 11, 12],
                    style_flags: STYLE_FLAG_ITALIC,
                    wide: true,
                },
            ],
        };
        let payload = encode_full_state(&state, None, false);
        let decoded = decode_remote_full_state_payload(&payload).expect("decode full state");
        assert_eq!(decoded, state);
    }

    #[test]
    fn encode_delta_uses_scroll_delta_for_scrolling_output() {
        let row = |ch: char| -> Vec<RemoteCell> {
            vec![RemoteCell {
                codepoint: u32::from(ch),
                fg: [1, 2, 3],
                bg: [0, 0, 0],
                style_flags: 0,
                wide: false,
            }]
        };
        let previous = RemoteFullState {
            rows: 3,
            cols: 1,
            cursor_x: 0,
            cursor_y: 2,
            cursor_visible: true,
            cursor_blinking: false,
            cursor_style: 1,
            cells: [row('a'), row('b'), row('c')].concat(),
        };
        let current = RemoteFullState {
            rows: 3,
            cols: 1,
            cursor_x: 0,
            cursor_y: 2,
            cursor_visible: true,
            cursor_blinking: false,
            cursor_style: 1,
            cells: [row('b'), row('c'), row('d')].concat(),
        };

        let payload = encode_delta(&previous, &current, Some(7), false).expect("delta payload");
        assert_eq!(u16::from_le_bytes(payload[0..2].try_into().unwrap()), 1);
        assert_eq!(payload[8] & 0x01, 0x01);
        assert_eq!(
            i16::from_le_bytes(
                payload[REMOTE_DELTA_HEADER_LEN..REMOTE_DELTA_HEADER_LEN + 2]
                    .try_into()
                    .unwrap()
            ),
            1
        );
        assert_eq!(
            u16::from_le_bytes(
                payload[REMOTE_DELTA_HEADER_LEN + 2..REMOTE_DELTA_HEADER_LEN + 4]
                    .try_into()
                    .unwrap()
            ),
            2
        );
    }

    #[test]
    fn encode_delta_skips_scroll_optimization_for_local_clients() {
        let row = |ch: char| -> Vec<RemoteCell> {
            vec![RemoteCell {
                codepoint: u32::from(ch),
                fg: [1, 2, 3],
                bg: [0, 0, 0],
                style_flags: 0,
                wide: false,
            }]
        };
        let previous = RemoteFullState {
            rows: 4,
            cols: 1,
            cursor_x: 0,
            cursor_y: 3,
            cursor_visible: true,
            cursor_blinking: false,
            cursor_style: 1,
            cells: [row('a'), row('b'), row('c'), row('d')].concat(),
        };
        let current = RemoteFullState {
            rows: 4,
            cols: 1,
            cursor_x: 0,
            cursor_y: 3,
            cursor_visible: true,
            cursor_blinking: false,
            cursor_style: 1,
            cells: [row('b'), row('c'), row('d'), row('e')].concat(),
        };

        assert!(encode_delta(&previous, &current, Some(9), true).is_none());
    }

    #[test]
    fn encode_delta_trims_unchanged_prefix_and_suffix_within_row() {
        let cell = |ch: char| RemoteCell {
            codepoint: u32::from(ch),
            fg: [1, 2, 3],
            bg: [0, 0, 0],
            style_flags: 0,
            wide: false,
        };
        let previous = RemoteFullState {
            rows: 1,
            cols: 5,
            cursor_x: 2,
            cursor_y: 0,
            cursor_visible: true,
            cursor_blinking: false,
            cursor_style: 1,
            cells: vec![cell('a'), cell('b'), cell('c'), cell('d'), cell('e')],
        };
        let current = RemoteFullState {
            rows: 1,
            cols: 5,
            cursor_x: 2,
            cursor_y: 0,
            cursor_visible: true,
            cursor_blinking: false,
            cursor_style: 1,
            cells: vec![cell('a'), cell('b'), cell('X'), cell('d'), cell('e')],
        };

        let payload = encode_delta(&previous, &current, Some(5), true).expect("delta payload");
        let row_offset = LOCAL_DELTA_HEADER_LEN;
        assert_eq!(
            u16::from_le_bytes(payload[row_offset..row_offset + 2].try_into().unwrap()),
            0
        );
        assert_eq!(
            u16::from_le_bytes(payload[row_offset + 2..row_offset + 4].try_into().unwrap()),
            2
        );
        assert_eq!(
            u16::from_le_bytes(payload[row_offset + 4..row_offset + 6].try_into().unwrap()),
            1
        );
        assert_eq!(
            u32::from_le_bytes(payload[row_offset + 6..row_offset + 10].try_into().unwrap()),
            u32::from('X')
        );
    }

    #[test]
    fn longest_prefix_suffix_overlap_matches_scroll_overlap() {
        assert_eq!(longest_prefix_suffix_overlap(&[2, 3, 4], &[1, 2, 3, 4]), 3);
        assert_eq!(longest_prefix_suffix_overlap(&[1, 2, 3], &[1, 2, 3, 4]), 0);
        assert_eq!(longest_prefix_suffix_overlap(&[7, 8], &[5, 6, 7, 8]), 2);
    }

    #[test]
    fn detect_scroll_rows_handles_multi_row_scroll() {
        let row = |ch: char| -> Vec<RemoteCell> {
            vec![RemoteCell {
                codepoint: u32::from(ch),
                fg: [1, 2, 3],
                bg: [0, 0, 0],
                style_flags: 0,
                wide: false,
            }]
        };
        let previous = RemoteFullState {
            rows: 5,
            cols: 1,
            cursor_x: 0,
            cursor_y: 4,
            cursor_visible: true,
            cursor_blinking: false,
            cursor_style: 1,
            cells: [row('a'), row('b'), row('c'), row('d'), row('e')].concat(),
        };
        let current = RemoteFullState {
            rows: 5,
            cols: 1,
            cursor_x: 0,
            cursor_y: 4,
            cursor_visible: true,
            cursor_blinking: false,
            cursor_style: 1,
            cells: [row('c'), row('d'), row('e'), row('f'), row('g')].concat(),
        };

        assert_eq!(detect_scroll_rows(&previous, &current), Some(2));
    }
}
