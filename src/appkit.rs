//! Safe wrappers around AppKit APIs used by boo.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::msg_send;
use objc2_app_kit::{NSApplication, NSView, NSWindow};
use objc2_foundation::{MainThreadMarker, NSRect, NSSize, NSString};
use std::ffi::c_void;

/// Get a MainThreadMarker. Safe to call from the main thread (which is always
/// the case for iced's update loop and AppKit callbacks).
fn mtm() -> MainThreadMarker {
    // SAFETY: all callers are on the main thread (iced event loop / AppKit callbacks)
    unsafe { MainThreadMarker::new_unchecked() }
}

/// Get the main NSWindow, if any.
pub fn main_window() -> Option<Retained<NSWindow>> {
    unsafe {
        let app = NSApplication::sharedApplication(mtm());
        app.mainWindow()
    }
}

/// Get the backing scale factor of the main window.
pub fn scale_factor() -> f64 {
    main_window()
        .map(|w| w.backingScaleFactor())
        .unwrap_or(2.0)
}

/// Get the content view of the main window.
pub fn content_view() -> Option<Retained<NSView>> {
    main_window().and_then(|w| w.contentView())
}

/// Create a child NSView with the given frame, add it to the parent on top.
pub fn create_child_view(parent: &NSView, frame: NSRect) -> Retained<NSView> {
    unsafe {
        let child = NSView::initWithFrame(mtm().alloc(), frame);
        // NSWindowAbove = 1
        let () = msg_send![parent, addSubview: &*child, positioned: 1usize, relativeTo: std::ptr::null::<AnyObject>()];
        child
    }
}

/// Get the bounds of a raw NSView pointer.
pub fn view_bounds(view: *mut c_void) -> NSRect {
    unsafe {
        let view = &*(view as *const NSView);
        view.bounds()
    }
}

/// Set the frame of a raw NSView pointer.
pub fn set_view_frame(view: *mut c_void, frame: NSRect) {
    unsafe {
        let view = &*(view as *const NSView);
        view.setFrame(frame);
    }
}

/// Set the window title.
pub fn set_window_title(title: &str) {
    if let Some(window) = main_window() {
        let ns_title = NSString::from_str(title);
        window.setTitle(&ns_title);
    }
}

/// Set content resize increments on the main window.
pub fn set_resize_increments(width: f64, height: f64) {
    if let Some(window) = main_window() {
        window.setContentResizeIncrements(NSSize::new(width, height));
    }
}

/// Request a redraw of the content view (called from wakeup on any thread).
pub fn request_redraw() {
    unsafe {
        if let Some(view) = content_view() {
            view.setNeedsDisplay(true);
        }
    }
}
