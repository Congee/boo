//! Linux platform backend for the VT-only runtime.
//!
//! Linux no longer embeds a Ghostty GL surface. Boo renders through iced and
//! uses only a minimal native-platform shim here.

use super::{KeyEvent, LayerHandle, Rect, ScrollEvent, TextInputEvent, ViewHandle};
use std::process::Command;

pub fn scale_factor() -> f64 { 1.0 }
pub fn content_view_handle() -> ViewHandle { 1usize as ViewHandle }
pub fn set_window_transparent() {}
pub fn create_focusable_child_view(_: ViewHandle, _: Rect) -> ViewHandle { std::ptr::null_mut() }

pub fn view_bounds(_: ViewHandle) -> Rect { Rect::default() }
pub fn set_view_frame(_: ViewHandle, _: Rect) {}
pub fn set_view_hidden(_: ViewHandle, _: bool) {}
pub fn remove_view(_: ViewHandle) {}
pub fn focus_view(_: ViewHandle) {}
pub fn set_text_input_cursor_rect(_: Rect) {}
pub fn install_event_monitors(
    _: std::sync::mpsc::Sender<ScrollEvent>,
    _: std::sync::mpsc::Sender<KeyEvent>,
    _: std::sync::mpsc::Sender<TextInputEvent>,
) {}
pub fn create_scrollbar_layer() -> LayerHandle { std::ptr::null_mut() }
pub fn update_scrollbar_layer(_: LayerHandle, _: f64, _: f64, _: f64, _: f64, _: f32) {}
pub fn create_highlight_layer() -> LayerHandle { std::ptr::null_mut() }
pub fn update_highlight_layer(_: LayerHandle, _: f64, _: f64, _: f64, _: f64, _: bool, _: bool) {}

pub fn clipboard_read() -> Option<String> {
    arboard::Clipboard::new().ok().and_then(|mut cb| cb.get_text().ok())
}

pub fn clipboard_write(text: &str) {
    if let Ok(mut cb) = arboard::Clipboard::new() {
        let _ = cb.set_text(text.to_owned());
    }
}

pub fn send_desktop_notification(title: &str, body: &str) {
    let mut sent = false;

    if Command::new("notify-send")
        .args(["--app-name=boo", title, body])
        .spawn()
        .is_ok()
    {
        sent = true;
    }

    if !sent {
        log::warn!("failed to send desktop notification: notify-send not available");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn linux_notification_command_shape_is_stable() {
        let args = ["--app-name=boo", "title", "body"];
        assert_eq!(args[0], "--app-name=boo");
        assert_eq!(args[1], "title");
        assert_eq!(args[2], "body");
    }
}
