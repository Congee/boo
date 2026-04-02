//! Safe wrappers around AppKit APIs used by boo.

use objc2::rc::Retained;
use objc2::{define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSEvent, NSEventMask, NSView, NSWindow, NSWindowOrderingMode,
};
use objc2_foundation::{MainThreadMarker, NSRect, NSSize, NSString};
use std::ffi::c_void;

// Custom NSView subclass that refuses first responder.
// This ensures keyboard events stay with winit's view so iced receives them.
define_class!(
    #[unsafe(super(NSView))]
    #[name = "BooPassthroughView"]
    struct PassthroughView;

    impl PassthroughView {
        #[unsafe(method(acceptsFirstResponder))]
        fn _accepts_first_responder(&self) -> bool {
            false
        }
    }
);

/// Get a MainThreadMarker. Safe to call from the main thread (which is always
/// the case for iced's update loop and AppKit callbacks).
fn mtm() -> MainThreadMarker {
    // SAFETY: all callers are on the main thread (iced event loop / AppKit callbacks)
    unsafe { MainThreadMarker::new_unchecked() }
}

/// Get the main NSWindow, if any.
pub fn main_window() -> Option<Retained<NSWindow>> {
    let app = NSApplication::sharedApplication(mtm());
    app.mainWindow()
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

/// Make the window transparent so terminal background-opacity works.
pub fn set_window_transparent() {
    if let Some(window) = main_window() {
        window.setOpaque(false);
        window.setHasShadow(false);
        let bg = objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(1.0, 1.0, 1.0, 0.001);
        window.setBackgroundColor(Some(&bg));
    }
}

/// Create a child NSView with the given frame, add it to the parent on top.
/// Uses a custom subclass that refuses first responder so keyboard events
/// stay with winit's view (iced receives them).
/// Returns a raw pointer — the parent NSView retains the child.
pub fn create_child_view(parent: &NSView, frame: NSRect) -> *mut c_void {
    unsafe {
        let child: Retained<PassthroughView> =
            msg_send![mtm().alloc::<PassthroughView>(), initWithFrame: frame];
        parent.addSubview_positioned_relativeTo(&child, NSWindowOrderingMode::Above, None);
        let ptr = Retained::as_ptr(&child) as *mut c_void;
        std::mem::forget(child);
        ptr
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

/// Set hidden state of a raw NSView.
pub fn set_view_hidden(view: *mut c_void, hidden: bool) {
    unsafe {
        let view = &*(view as *const NSView);
        view.setHidden(hidden);
    }
}

/// Remove a view from its parent.
pub fn remove_view(view: *mut c_void) {
    unsafe {
        let view = &*(view as *const NSView);
        view.removeFromSuperview();
    }
}

/// Install local event monitors for window drag and smooth scrolling.
pub fn install_event_monitors(scroll_tx: std::sync::mpsc::Sender<ScrollEvent>) {
    install_cmd_drag_monitor();
    install_scroll_monitor(scroll_tx);
}

pub struct ScrollEvent {
    pub dx: f64,
    pub dy: f64,
    pub precision: bool,
    pub momentum: u8,
}

fn install_scroll_monitor(tx: std::sync::mpsc::Sender<ScrollEvent>) {
    use std::ptr::NonNull;

    let block = block2::RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
        let event_ref = unsafe { event.as_ref() };
        let dx = event_ref.scrollingDeltaX();
        let dy = event_ref.scrollingDeltaY();
        let precision = event_ref.hasPreciseScrollingDeltas();
        let momentum = event_ref.momentumPhase().0 as u8;

        let (dx, dy) = if precision {
            (dx * 2.0, dy * 2.0) // 2x multiplier matches ghostty's feel
        } else {
            (dx, dy)
        };

        let _ = tx.send(ScrollEvent {
            dx,
            dy,
            precision,
            momentum,
        });

        // Consume the event — we handle it ourselves
        std::ptr::null_mut()
    });
    let monitor = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(
            NSEventMask::ScrollWheel,
            &block,
        )
    };
    std::mem::forget(monitor);
}

fn install_cmd_drag_monitor() {
    use std::ptr::NonNull;

    let block = block2::RcBlock::new(|event: NonNull<NSEvent>| -> *mut NSEvent {
        let event_ref = unsafe { event.as_ref() };
        let flags = event_ref.modifierFlags();
        if flags.contains(objc2_app_kit::NSEventModifierFlags::Command) {
            let mtm = unsafe { MainThreadMarker::new_unchecked() };
            if let Some(window) = event_ref.window(mtm) {
                let () = unsafe { msg_send![&*window, performWindowDragWithEvent: event_ref] };
                return std::ptr::null_mut();
            }
        }
        event.as_ptr()
    });
    let monitor = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(
            NSEventMask::LeftMouseDown,
            &block,
        )
    };
    // Retain the monitor for the process lifetime
    std::mem::forget(monitor);
}

/// Set a view's layer (and all sublayers) to non-opaque for transparency.
pub fn set_view_layer_transparent(view: *mut c_void) {
    unsafe {
        let view = &*(view as *const NSView);
        if let Some(layer) = view.layer() {
            layer.setOpaque(false);
            layer.setBackgroundColor(None);
            if let Some(sublayers) = layer.sublayers() {
                for sublayer in sublayers.iter() {
                    sublayer.setOpaque(false);
                }
            }
        }
    }
}

/// Create a scrollbar overlay CALayer on top of all content.
/// Returns a raw pointer to the layer (caller must retain).
pub fn create_scrollbar_layer() -> *mut c_void {
    use objc2_quartz_core::CALayer;

    let layer = CALayer::new();
    layer.setCornerRadius(3.0);
    layer.setOpacity(0.0);
    // Add to content view's layer so it composites above everything
    if let Some(cv) = content_view() {
        if let Some(parent_layer) = cv.layer() {
            parent_layer.addSublayer(&layer);
        }
    }
    let ptr = objc2::rc::Retained::as_ptr(&layer) as *mut c_void;
    std::mem::forget(layer);
    ptr
}

/// Update the scrollbar overlay layer's frame, color, and opacity.
pub fn update_scrollbar_layer(
    layer: *mut c_void,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    opacity: f32,
) {
    if layer.is_null() {
        return;
    }
    unsafe {
        use objc2_quartz_core::CALayer;
        let layer = &*(layer as *const CALayer);
        // Disable implicit animations for smooth updates
        objc2_quartz_core::CATransaction::begin();
        objc2_quartz_core::CATransaction::setDisableActions(true);
        layer.setFrame(NSRect::new(
            objc2_foundation::NSPoint::new(x, y),
            NSSize::new(width, height),
        ));
        layer.setOpacity(opacity);
        let color = objc2_core_graphics::CGColor::new_generic_rgb(
            0.6, 0.6, 0.6, opacity as f64,
        );
        layer.setBackgroundColor(Some(&color));
        objc2_quartz_core::CATransaction::commit();
    }
}

/// Create a highlight overlay layer for copy mode cursor/selection.
pub fn create_highlight_layer() -> *mut c_void {
    use objc2_quartz_core::CALayer;

    let layer = CALayer::new();
    layer.setOpacity(0.0);
    if let Some(cv) = content_view() {
        if let Some(parent) = cv.layer() {
            parent.addSublayer(&layer);
        }
    }
    let ptr = objc2::rc::Retained::as_ptr(&layer) as *mut c_void;
    std::mem::forget(layer);
    ptr
}

/// Update the highlight layer for copy mode (cursor bar or selection rect).
pub fn update_highlight_layer(
    layer: *mut c_void,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    visible: bool,
    is_selection: bool,
) {
    if layer.is_null() {
        return;
    }
    unsafe {
        use objc2_quartz_core::CALayer;
        let layer = &*(layer as *const CALayer);
        objc2_quartz_core::CATransaction::begin();
        objc2_quartz_core::CATransaction::setDisableActions(true);
        layer.setFrame(NSRect::new(
            objc2_foundation::NSPoint::new(x, y),
            NSSize::new(width, height),
        ));
        layer.setOpacity(if visible { 1.0 } else { 0.0 });
        let (r, g, b, a) = if is_selection {
            (0.2, 0.5, 1.0, 0.3) // blue selection
        } else {
            (0.9, 0.9, 0.9, 0.8) // white cursor bar
        };
        let color = objc2_core_graphics::CGColor::new_generic_rgb(r, g, b, a);
        layer.setBackgroundColor(Some(&color));
        objc2_quartz_core::CATransaction::commit();
    }
}

/// Request a redraw of the content view (called from wakeup on any thread).
pub fn request_redraw() {
    if let Some(view) = content_view() {
        view.setNeedsDisplay(true);
    }
}
