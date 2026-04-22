//! Outbound frame batcher for the remote write-side.
//!
//! The daemon reader/executor pushes per-client frames onto an `mpsc::Sender<OutboundMessage>`,
//! and a dedicated writer thread drains the other end via [`writer_loop`]. Under load this
//! lets us coalesce redundant control frames (multiple tab-list / UiRuntimeState /
//! UiAppearance updates queued while the socket was blocked collapse to the latest) and
//! optionally keep only the newest screen-update frame, which is how we avoid shipping
//! stale VT deltas to a slow client.
//!
//! The module is State-free: it consumes [`OutboundMessage`]s, inspects the on-the-wire
//! [`MessageType`] tag to decide coalescing, and hands bytes to a `Write`. The daemon
//! state graph in `remote.rs` owns the channel, we just drain it.

use std::io::Write;
use std::sync::mpsc;

use crate::remote_wire::MessageType;

pub(crate) enum OutboundMessage {
    Frame(Vec<u8>),
    ScreenUpdate(Vec<u8>),
}

pub(crate) fn writer_loop<W: Write>(
    mut stream: W,
    outbound_rx: mpsc::Receiver<OutboundMessage>,
    coalesce_screen_updates: bool,
    batch_messages: bool,
) {
    while let Ok(message) = outbound_rx.recv() {
        let mut scope =
            crate::profiling::scope("server.stream.batch_write", crate::profiling::Kind::Io);
        let batch = if batch_messages {
            collect_outbound_batch(message, &outbound_rx, coalesce_screen_updates)
        } else {
            collect_single_outbound_message(message)
        };
        crate::profiling::record_units(
            "server.stream.batch_write.frames",
            crate::profiling::Kind::Io,
            batch.frames.len() as u64,
        );
        crate::profiling::record_units(
            "server.stream.batch_write.messages",
            crate::profiling::Kind::Io,
            batch.message_count as u64,
        );
        crate::profiling::record_units(
            "server.stream.batch_write.coalesced_screen_updates",
            crate::profiling::Kind::Io,
            batch.coalesced_screen_updates as u64,
        );
        crate::profiling::record_units(
            "server.stream.batch_write.coalesced_control_frames",
            crate::profiling::Kind::Io,
            batch.coalesced_control_frames as u64,
        );
        let mut failed = false;
        for frame in batch.frames {
            scope.add_bytes(frame.len() as u64);
            if stream.write_all(&frame).is_err() {
                failed = true;
                break;
            }
        }
        if failed || stream.flush().is_err() {
            break;
        }
    }
}

fn collect_single_outbound_message(message: OutboundMessage) -> OutboundBatch {
    let (frames, coalesced_screen_updates, coalesced_control_frames) = match message {
        OutboundMessage::Frame(frame) => (vec![frame], 0, 0),
        OutboundMessage::ScreenUpdate(frame) => (vec![frame], 0, 0),
    };
    OutboundBatch {
        frames,
        message_count: 1,
        coalesced_screen_updates,
        coalesced_control_frames,
    }
}

struct OutboundBatch {
    frames: Vec<Vec<u8>>,
    message_count: usize,
    coalesced_screen_updates: usize,
    coalesced_control_frames: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CoalescibleFrameKind {
    TabList,
    UiRuntimeState,
    UiAppearance,
}

#[derive(Default)]
struct PendingOutboundFrames {
    order: Vec<CoalescibleFrameKind>,
    tab_list: Option<Vec<u8>>,
    ui_runtime_state: Option<Vec<u8>>,
    ui_appearance: Option<Vec<u8>>,
}

impl PendingOutboundFrames {
    fn push_kind_once(&mut self, kind: CoalescibleFrameKind) {
        if !self.order.contains(&kind) {
            self.order.push(kind);
        }
    }

    fn set(&mut self, kind: CoalescibleFrameKind, frame: Vec<u8>) {
        self.push_kind_once(kind);
        match kind {
            CoalescibleFrameKind::TabList => self.tab_list = Some(frame),
            CoalescibleFrameKind::UiRuntimeState => self.ui_runtime_state = Some(frame),
            CoalescibleFrameKind::UiAppearance => self.ui_appearance = Some(frame),
        }
    }

    fn take_all(&mut self) -> Vec<Vec<u8>> {
        let mut frames = Vec::with_capacity(self.order.len());
        for kind in self.order.drain(..) {
            let frame = match kind {
                CoalescibleFrameKind::TabList => self.tab_list.take(),
                CoalescibleFrameKind::UiRuntimeState => self.ui_runtime_state.take(),
                CoalescibleFrameKind::UiAppearance => self.ui_appearance.take(),
            };
            if let Some(frame) = frame {
                frames.push(frame);
            }
        }
        frames
    }
}

fn collect_outbound_batch(
    first: OutboundMessage,
    outbound_rx: &mpsc::Receiver<OutboundMessage>,
    coalesce_screen_updates: bool,
) -> OutboundBatch {
    let mut frames = Vec::new();
    let mut pending_screen = None;
    let mut pending_control = PendingOutboundFrames::default();
    let mut message_count = 0usize;
    let mut screen_updates = 0usize;
    let mut emitted_screen_frames = 0usize;
    let mut coalesced_control_frames = 0usize;

    let mut push = |message| match message {
        OutboundMessage::Frame(frame) => {
            message_count += 1;
            if let Some(kind) = coalescible_frame_kind(&frame) {
                let replaced = match kind {
                    CoalescibleFrameKind::TabList => pending_control.tab_list.is_some(),
                    CoalescibleFrameKind::UiRuntimeState => {
                        pending_control.ui_runtime_state.is_some()
                    }
                    CoalescibleFrameKind::UiAppearance => pending_control.ui_appearance.is_some(),
                };
                if replaced {
                    coalesced_control_frames += 1;
                }
                pending_control.set(kind, frame);
                return;
            }
            for pending in pending_control.take_all() {
                frames.push(pending);
            }
            if let Some(screen) = pending_screen.take() {
                frames.push(screen);
                emitted_screen_frames += 1;
            }
            frames.push(frame);
        }
        OutboundMessage::ScreenUpdate(frame) => {
            message_count += 1;
            screen_updates += 1;
            if coalesce_screen_updates {
                pending_screen = Some(frame);
            } else {
                for pending in pending_control.take_all() {
                    frames.push(pending);
                }
                if let Some(screen) = pending_screen.take() {
                    frames.push(screen);
                    emitted_screen_frames += 1;
                }
                frames.push(frame);
                emitted_screen_frames += 1;
            }
        }
    };

    push(first);
    while let Ok(message) = outbound_rx.try_recv() {
        push(message);
    }
    for pending in pending_control.take_all() {
        frames.push(pending);
    }
    if let Some(screen) = pending_screen {
        frames.push(screen);
        emitted_screen_frames += 1;
    }
    OutboundBatch {
        frames,
        message_count,
        coalesced_screen_updates: screen_updates.saturating_sub(emitted_screen_frames),
        coalesced_control_frames,
    }
}

fn coalescible_frame_kind(frame: &[u8]) -> Option<CoalescibleFrameKind> {
    let ty = frame.get(2).copied().and_then(|value| MessageType::try_from(value).ok())?;
    match ty {
        MessageType::SessionList => Some(CoalescibleFrameKind::TabList),
        MessageType::UiRuntimeState => Some(CoalescibleFrameKind::UiRuntimeState),
        MessageType::UiAppearance => Some(CoalescibleFrameKind::UiAppearance),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote_wire::{MessageType, encode_message};

    #[test]
    fn outbound_batch_coalesces_consecutive_screen_updates() {
        let (tx, rx) = mpsc::channel();
        tx.send(OutboundMessage::ScreenUpdate(vec![1])).unwrap();
        tx.send(OutboundMessage::ScreenUpdate(vec![2])).unwrap();
        tx.send(OutboundMessage::Frame(vec![9])).unwrap();
        tx.send(OutboundMessage::ScreenUpdate(vec![3])).unwrap();
        let first = rx.recv().unwrap();
        let batch = collect_outbound_batch(first, &rx, true);
        assert_eq!(batch.frames, vec![vec![2], vec![9], vec![3]]);
        assert_eq!(batch.message_count, 4);
        assert_eq!(batch.coalesced_screen_updates, 1);
    }

    #[test]
    fn outbound_batch_keeps_all_screen_updates_when_coalescing_disabled() {
        let (tx, rx) = mpsc::channel();
        tx.send(OutboundMessage::ScreenUpdate(vec![1])).unwrap();
        tx.send(OutboundMessage::ScreenUpdate(vec![2])).unwrap();
        tx.send(OutboundMessage::Frame(vec![9])).unwrap();
        tx.send(OutboundMessage::ScreenUpdate(vec![3])).unwrap();
        let first = rx.recv().unwrap();
        let batch = collect_outbound_batch(first, &rx, false);
        assert_eq!(batch.frames, vec![vec![1], vec![2], vec![9], vec![3]]);
        assert_eq!(batch.message_count, 4);
        assert_eq!(batch.coalesced_screen_updates, 0);
    }

    #[test]
    fn outbound_batch_coalesces_superseded_control_frames() {
        let (tx, rx) = mpsc::channel();
        let runtime_a = encode_message(MessageType::UiRuntimeState, b"runtime-a");
        let runtime_b = encode_message(MessageType::UiRuntimeState, b"runtime-b");
        let appearance_a = encode_message(MessageType::UiAppearance, b"appearance-a");
        let appearance_b = encode_message(MessageType::UiAppearance, b"appearance-b");
        let barrier = encode_message(MessageType::Attached, &7_u32.to_le_bytes());
        tx.send(OutboundMessage::Frame(runtime_a)).unwrap();
        tx.send(OutboundMessage::Frame(appearance_a)).unwrap();
        tx.send(OutboundMessage::Frame(runtime_b.clone())).unwrap();
        tx.send(OutboundMessage::Frame(appearance_b.clone())).unwrap();
        tx.send(OutboundMessage::Frame(barrier.clone())).unwrap();

        let first = rx.recv().unwrap();
        let batch = collect_outbound_batch(first, &rx, true);
        assert_eq!(batch.frames, vec![runtime_b, appearance_b, barrier]);
        assert_eq!(batch.message_count, 5);
        assert_eq!(batch.coalesced_control_frames, 2);
    }}
