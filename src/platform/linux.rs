//! Linux platform backend for the VT-only runtime.
//!
//! Linux no longer embeds a Ghostty GL surface. Boo renders through iced and
//! uses only a minimal native-platform shim here.

#![allow(dead_code)]

use super::{LayerHandle, Rect, ScrollEvent, ViewHandle};

pub fn scale_factor() -> f64 { 1.0 }
pub fn content_view_handle() -> ViewHandle { 1usize as ViewHandle }
pub fn set_window_transparent() {}

pub fn view_bounds(_: ViewHandle) -> Rect { Rect::default() }
pub fn set_view_frame(_: ViewHandle, _: Rect) {}
pub fn set_window_title(_: &str) {}
pub fn set_resize_increments(_: f64, _: f64) {}
pub fn set_view_hidden(_: ViewHandle, _: bool) {}
pub fn remove_view(_: ViewHandle) {}
pub fn request_redraw() {}
pub fn install_event_monitors(_: std::sync::mpsc::Sender<ScrollEvent>) {}
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

pub fn clipboard_write_from_thread(text: String) {
    if let Ok(mut cb) = arboard::Clipboard::new() {
        let _ = cb.set_text(text);
    }
}
