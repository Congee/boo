#![allow(dead_code)]
//! tmux control mode protocol parser.
//!
//! Parses the text-based protocol from `tmux -CC`. The parser receives raw
//! lines from tmux's stdout and emits structured events.
//!
//! Protocol overview:
//! - Notifications: `%output %42 data`, `%window-add @1`, `%exit`, etc.
//! - Command responses: wrapped in `%begin TS CMD FLAGS` / `%end` / `%error`
//! - Output data uses octal escaping: `\033` → ESC, `\\` → backslash

/// A parsed event from the tmux control protocol.
#[derive(Debug, Clone, PartialEq)]
pub enum TmuxEvent {
    /// Terminal output for a pane. Data is already decoded from octal escapes.
    Output { pane_id: u32, data: Vec<u8> },
    /// A new window was added.
    WindowAdd { window_id: u32 },
    /// A window was closed.
    WindowClose { window_id: u32 },
    /// A window was renamed.
    WindowRenamed { window_id: u32, name: String },
    /// The layout of a window changed.
    LayoutChanged { window_id: u32, layout: String },
    /// A pane mode changed (e.g., copy mode entered/exited).
    PaneModeChanged { pane_id: u32 },
    /// The active pane in a window changed.
    WindowPaneChanged { window_id: u32, pane_id: u32 },
    /// The client's session changed.
    SessionChanged { session_id: u32, name: String },
    /// Sessions were created or destroyed.
    SessionsChanged,
    /// A window was renamed.
    SessionRenamed { name: String },
    /// The tmux client is exiting.
    Exit { reason: String },
    /// A command response completed successfully.
    CommandOk { id: u32, output: String },
    /// A command response failed.
    CommandError { id: u32, error: String },
}

/// Tracks state for in-progress command responses.
#[derive(Debug)]
struct PendingCommand {
    id: u32,
    lines: Vec<String>,
}

/// tmux control mode protocol parser.
///
/// Feed lines from tmux stdout via `parse_line()`. Collects command
/// responses across multiple lines and emits events.
#[derive(Debug)]
pub struct TmuxParser {
    pending: Option<PendingCommand>,
}

impl TmuxParser {
    pub fn new() -> Self {
        TmuxParser { pending: None }
    }

    /// Parse a single line from tmux stdout. Returns an event if one is complete.
    pub fn parse_line(&mut self, line: &str) -> Option<TmuxEvent> {
        let line = line.trim_end_matches('\n').trim_end_matches('\r');

        // Command response guards
        if line.starts_with("%begin ") {
            let id = parse_guard_id(line);
            self.pending = Some(PendingCommand {
                id,
                lines: Vec::new(),
            });
            return None;
        }
        if line.starts_with("%end ") {
            if let Some(cmd) = self.pending.take() {
                return Some(TmuxEvent::CommandOk {
                    id: cmd.id,
                    output: cmd.lines.join("\n"),
                });
            }
            return None;
        }
        if line.starts_with("%error ") {
            if let Some(cmd) = self.pending.take() {
                return Some(TmuxEvent::CommandError {
                    id: cmd.id,
                    error: cmd.lines.join("\n"),
                });
            }
            return None;
        }

        // If we're inside a command response, accumulate lines
        if let Some(ref mut cmd) = self.pending {
            cmd.lines.push(line.to_owned());
            return None;
        }

        // Notifications
        if let Some(rest) = line.strip_prefix("%output ") {
            return parse_output(rest);
        }
        if let Some(rest) = line.strip_prefix("%window-add ") {
            return parse_id(rest, "@").map(|id| TmuxEvent::WindowAdd { window_id: id });
        }
        if let Some(rest) = line.strip_prefix("%window-close ") {
            return parse_id(rest, "@").map(|id| TmuxEvent::WindowClose { window_id: id });
        }
        if let Some(rest) = line.strip_prefix("%window-renamed ") {
            return parse_id_and_name(rest, "@")
                .map(|(id, name)| TmuxEvent::WindowRenamed { window_id: id, name });
        }
        if let Some(rest) = line.strip_prefix("%layout-change ") {
            return parse_layout_change(rest);
        }
        if let Some(rest) = line.strip_prefix("%window-pane-changed ") {
            return parse_window_pane_changed(rest);
        }
        if let Some(rest) = line.strip_prefix("%pane-mode-changed ") {
            return parse_id(rest, "%").map(|id| TmuxEvent::PaneModeChanged { pane_id: id });
        }
        if let Some(rest) = line.strip_prefix("%session-changed ") {
            return parse_session_changed(rest);
        }
        if line == "%sessions-changed" {
            return Some(TmuxEvent::SessionsChanged);
        }
        if let Some(rest) = line.strip_prefix("%session-renamed ") {
            return Some(TmuxEvent::SessionRenamed {
                name: rest.to_owned(),
            });
        }
        if let Some(rest) = line.strip_prefix("%exit") {
            let reason = rest.trim().to_owned();
            return Some(TmuxEvent::Exit { reason });
        }

        None
    }
}

/// Decode tmux octal escapes: `\033` → 0x1B, `\\` → `\`, `\NNN` → byte.
pub fn decode_octal(input: &str) -> Vec<u8> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            if bytes[i + 1] == b'\\' {
                out.push(b'\\');
                i += 2;
            } else if i + 3 < bytes.len()
                && bytes[i + 1].is_ascii_digit()
                && bytes[i + 2].is_ascii_digit()
                && bytes[i + 3].is_ascii_digit()
            {
                let val = (bytes[i + 1] - b'0') as u16 * 64
                    + (bytes[i + 2] - b'0') as u16 * 8
                    + (bytes[i + 3] - b'0') as u16;
                out.push(val as u8);
                i += 4;
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

/// Parse a tmux layout string into a split tree description.
///
/// tmux layout format: `checksum,WxH,X,Y[,pane_id]` or
/// `checksum,WxH,X,Y{child,child}` (horizontal) or
/// `checksum,WxH,X,Y[child,child]` (vertical)
///
/// Returns a tree of LayoutNode.
#[derive(Debug, Clone, PartialEq)]
pub enum LayoutNode {
    Leaf {
        width: u32,
        height: u32,
        x: u32,
        y: u32,
        pane_id: u32,
    },
    Split {
        horizontal: bool, // true = side-by-side ({}), false = stacked ([])
        width: u32,
        height: u32,
        children: Vec<LayoutNode>,
    },
}

pub fn parse_layout(layout: &str) -> Option<LayoutNode> {
    // Strip the checksum prefix (e.g., "b25f,")
    let layout = layout.find(',').map(|i| &layout[i + 1..]).unwrap_or(layout);
    parse_layout_node(layout).map(|(node, _)| node)
}

fn parse_layout_node(s: &str) -> Option<(LayoutNode, &str)> {
    // Parse WxH,X,Y
    let (width, rest) = parse_u32(s)?;
    let rest = rest.strip_prefix('x')?;
    let (height, rest) = parse_u32(rest)?;
    let rest = rest.strip_prefix(',')?;
    let (x, rest) = parse_u32(rest)?;
    let rest = rest.strip_prefix(',')?;
    let (y, rest) = parse_u32(rest)?;

    // Check what follows: '{' (h-split), '[' (v-split), ',' (pane_id), or end
    if let Some(rest) = rest.strip_prefix('{') {
        let (children, rest) = parse_children(rest, '}')?;
        Some((
            LayoutNode::Split {
                horizontal: true,
                width,
                height,
                children,
            },
            rest,
        ))
    } else if let Some(rest) = rest.strip_prefix('[') {
        let (children, rest) = parse_children(rest, ']')?;
        Some((
            LayoutNode::Split {
                horizontal: false,
                width,
                height,
                children,
            },
            rest,
        ))
    } else if let Some(rest) = rest.strip_prefix(',') {
        let (pane_id, rest) = parse_u32(rest)?;
        Some((
            LayoutNode::Leaf {
                width,
                height,
                x,
                y,
                pane_id,
            },
            rest,
        ))
    } else {
        // Leaf without explicit pane_id
        Some((
            LayoutNode::Leaf {
                width,
                height,
                x,
                y,
                pane_id: 0,
            },
            rest,
        ))
    }
}

fn parse_children(mut s: &str, close: char) -> Option<(Vec<LayoutNode>, &str)> {
    let mut children = Vec::new();
    loop {
        let (child, rest) = parse_layout_node(s)?;
        children.push(child);
        if let Some(rest) = rest.strip_prefix(close) {
            return Some((children, rest));
        }
        s = rest.strip_prefix(',')?;
    }
}

fn parse_u32(s: &str) -> Option<(u32, &str)> {
    let end = s
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(s.len());
    if end == 0 {
        return None;
    }
    let val: u32 = s[..end].parse().ok()?;
    Some((val, &s[end..]))
}

// --- Helper parsers for notifications ---

fn parse_guard_id(line: &str) -> u32 {
    // "%begin 1363006971 2 1" → command number is the second field
    line.split_whitespace()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn parse_output(rest: &str) -> Option<TmuxEvent> {
    // "%output %42 data..." → pane_id=42, data=decoded
    let rest = rest.strip_prefix('%')?;
    let space = rest.find(' ')?;
    let pane_id: u32 = rest[..space].parse().ok()?;
    let data = decode_octal(&rest[space + 1..]);
    Some(TmuxEvent::Output { pane_id, data })
}

fn parse_id(s: &str, prefix: &str) -> Option<u32> {
    let s = s.trim();
    let s = s.strip_prefix(prefix)?;
    s.split_whitespace().next()?.parse().ok()
}

fn parse_id_and_name(s: &str, prefix: &str) -> Option<(u32, String)> {
    let s = s.trim();
    let s = s.strip_prefix(prefix)?;
    let space = s.find(' ')?;
    let id: u32 = s[..space].parse().ok()?;
    let name = s[space + 1..].to_owned();
    Some((id, name))
}

fn parse_layout_change(rest: &str) -> Option<TmuxEvent> {
    // "%layout-change @1 b25f,80x24,0,0,2"
    let rest = rest.trim();
    let rest = rest.strip_prefix('@')?;
    let space = rest.find(' ')?;
    let window_id: u32 = rest[..space].parse().ok()?;
    let layout = rest[space + 1..].to_owned();
    Some(TmuxEvent::LayoutChanged { window_id, layout })
}

fn parse_window_pane_changed(rest: &str) -> Option<TmuxEvent> {
    // "%window-pane-changed @1 %3"
    let mut parts = rest.split_whitespace();
    let window_id: u32 = parts.next()?.strip_prefix('@')?.parse().ok()?;
    let pane_id: u32 = parts.next()?.strip_prefix('%')?.parse().ok()?;
    Some(TmuxEvent::WindowPaneChanged { window_id, pane_id })
}

fn parse_session_changed(rest: &str) -> Option<TmuxEvent> {
    // "%session-changed $1 mysession"
    let rest = rest.trim();
    let rest = rest.strip_prefix('$')?;
    let space = rest.find(' ')?;
    let session_id: u32 = rest[..space].parse().ok()?;
    let name = rest[space + 1..].to_owned();
    Some(TmuxEvent::SessionChanged { session_id, name })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_octal_esc() {
        assert_eq!(decode_octal(r"\033[31m"), vec![0x1B, b'[', b'3', b'1', b'm']);
    }

    #[test]
    fn test_decode_octal_backslash() {
        assert_eq!(decode_octal(r"a\\b"), vec![b'a', b'\\', b'b']);
    }

    #[test]
    fn test_decode_octal_mixed() {
        assert_eq!(
            decode_octal(r"hello\012world"),
            b"hello\nworld"
        );
    }

    #[test]
    fn test_decode_octal_plain() {
        assert_eq!(decode_octal("hello"), b"hello");
    }

    #[test]
    fn test_parse_output() {
        let mut parser = TmuxParser::new();
        let event = parser.parse_line("%output %42 hello\\012world");
        assert_eq!(
            event,
            Some(TmuxEvent::Output {
                pane_id: 42,
                data: b"hello\nworld".to_vec(),
            })
        );
    }

    #[test]
    fn test_parse_window_add() {
        let mut parser = TmuxParser::new();
        assert_eq!(
            parser.parse_line("%window-add @3"),
            Some(TmuxEvent::WindowAdd { window_id: 3 })
        );
    }

    #[test]
    fn test_parse_window_close() {
        let mut parser = TmuxParser::new();
        assert_eq!(
            parser.parse_line("%window-close @5"),
            Some(TmuxEvent::WindowClose { window_id: 5 })
        );
    }

    #[test]
    fn test_parse_window_renamed() {
        let mut parser = TmuxParser::new();
        assert_eq!(
            parser.parse_line("%window-renamed @1 my-window"),
            Some(TmuxEvent::WindowRenamed {
                window_id: 1,
                name: "my-window".to_owned(),
            })
        );
    }

    #[test]
    fn test_parse_layout_change() {
        let mut parser = TmuxParser::new();
        assert_eq!(
            parser.parse_line("%layout-change @2 b25f,80x24,0,0,2"),
            Some(TmuxEvent::LayoutChanged {
                window_id: 2,
                layout: "b25f,80x24,0,0,2".to_owned(),
            })
        );
    }

    #[test]
    fn test_parse_session_changed() {
        let mut parser = TmuxParser::new();
        assert_eq!(
            parser.parse_line("%session-changed $1 main"),
            Some(TmuxEvent::SessionChanged {
                session_id: 1,
                name: "main".to_owned(),
            })
        );
    }

    #[test]
    fn test_parse_exit() {
        let mut parser = TmuxParser::new();
        assert_eq!(
            parser.parse_line("%exit server exited"),
            Some(TmuxEvent::Exit {
                reason: "server exited".to_owned(),
            })
        );
    }

    #[test]
    fn test_parse_exit_empty() {
        let mut parser = TmuxParser::new();
        assert_eq!(
            parser.parse_line("%exit"),
            Some(TmuxEvent::Exit {
                reason: String::new(),
            })
        );
    }

    #[test]
    fn test_command_response() {
        let mut parser = TmuxParser::new();
        assert_eq!(parser.parse_line("%begin 1363006971 2 1"), None);
        assert_eq!(parser.parse_line("0: ksh* (1 panes) [80x24]"), None);
        assert_eq!(
            parser.parse_line("%end 1363006971 2 1"),
            Some(TmuxEvent::CommandOk {
                id: 2,
                output: "0: ksh* (1 panes) [80x24]".to_owned(),
            })
        );
    }

    #[test]
    fn test_command_error() {
        let mut parser = TmuxParser::new();
        assert_eq!(parser.parse_line("%begin 100 5 0"), None);
        assert_eq!(parser.parse_line("no such session"), None);
        assert_eq!(
            parser.parse_line("%error 100 5 0"),
            Some(TmuxEvent::CommandError {
                id: 5,
                error: "no such session".to_owned(),
            })
        );
    }

    #[test]
    fn test_parse_window_pane_changed() {
        let mut parser = TmuxParser::new();
        assert_eq!(
            parser.parse_line("%window-pane-changed @1 %3"),
            Some(TmuxEvent::WindowPaneChanged {
                window_id: 1,
                pane_id: 3,
            })
        );
    }

    #[test]
    fn test_sessions_changed() {
        let mut parser = TmuxParser::new();
        assert_eq!(
            parser.parse_line("%sessions-changed"),
            Some(TmuxEvent::SessionsChanged)
        );
    }

    // --- Layout parser tests ---

    #[test]
    fn test_parse_layout_single_pane() {
        let node = parse_layout("b25f,80x24,0,0,2").unwrap();
        assert_eq!(
            node,
            LayoutNode::Leaf {
                width: 80,
                height: 24,
                x: 0,
                y: 0,
                pane_id: 2,
            }
        );
    }

    #[test]
    fn test_parse_layout_horizontal_split() {
        // Two panes side by side
        let node = parse_layout("1234,160x48,0,0{80x48,0,0,1,80x48,80,0,2}").unwrap();
        match node {
            LayoutNode::Split {
                horizontal,
                width,
                height,
                children,
            } => {
                assert!(horizontal);
                assert_eq!(width, 160);
                assert_eq!(height, 48);
                assert_eq!(children.len(), 2);
            }
            _ => panic!("expected split"),
        }
    }

    #[test]
    fn test_parse_layout_vertical_split() {
        // Two panes stacked
        let node = parse_layout("abcd,80x48,0,0[80x24,0,0,1,80x24,0,24,2]").unwrap();
        match node {
            LayoutNode::Split {
                horizontal,
                children,
                ..
            } => {
                assert!(!horizontal);
                assert_eq!(children.len(), 2);
            }
            _ => panic!("expected split"),
        }
    }

    #[test]
    fn test_parse_layout_nested() {
        // Horizontal split where second child is a vertical split
        let node =
            parse_layout("ffff,160x48,0,0{80x48,0,0,1,80x48,80,0[80x24,80,0,2,80x24,80,24,3]}")
                .unwrap();
        match node {
            LayoutNode::Split {
                horizontal,
                children,
                ..
            } => {
                assert!(horizontal);
                assert_eq!(children.len(), 2);
                match &children[1] {
                    LayoutNode::Split {
                        horizontal, children, ..
                    } => {
                        assert!(!horizontal);
                        assert_eq!(children.len(), 2);
                    }
                    _ => panic!("expected nested split"),
                }
            }
            _ => panic!("expected split"),
        }
    }
}
