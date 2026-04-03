//! macOS platform backend — AppKit + Core Animation + NSPasteboard.

use super::{LayerHandle, Point, Rect, ScrollEvent, Size, ViewHandle};
use crate::ffi;
use objc2::rc::Retained;
use objc2::{define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSEvent, NSEventMask, NSView, NSWindow, NSWindowOrderingMode,
};
use objc2_foundation::{MainThreadMarker, NSRect, NSSize, NSString};
use std::ffi::c_void;

// --- NSRect <-> Rect conversions ---

fn to_nsrect(r: Rect) -> NSRect {
    NSRect::new(
        objc2_foundation::NSPoint::new(r.origin.x, r.origin.y),
        NSSize::new(r.size.width, r.size.height),
    )
}

fn from_nsrect(r: NSRect) -> Rect {
    Rect::new(
        Point::new(r.origin.x, r.origin.y),
        Size::new(r.size.width, r.size.height),
    )
}

// --- Custom NSView subclass (refuses first responder) ---

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

fn mtm() -> MainThreadMarker {
    unsafe { MainThreadMarker::new_unchecked() }
}

fn main_window() -> Option<Retained<NSWindow>> {
    let app = NSApplication::sharedApplication(mtm());
    app.mainWindow()
}

// --- Public API (matches linux.rs surface) ---

pub fn scale_factor() -> f64 {
    main_window()
        .map(|w| w.backingScaleFactor())
        .unwrap_or(2.0)
}

pub fn content_view() -> Option<Retained<NSView>> {
    main_window().and_then(|w| w.contentView())
}

/// Get the content view as an opaque pointer, or null.
pub fn content_view_handle() -> ViewHandle {
    content_view()
        .map(|cv| Retained::as_ptr(&cv) as ViewHandle)
        .unwrap_or(std::ptr::null_mut())
}

pub fn set_window_transparent() {
    if let Some(window) = main_window() {
        window.setOpaque(false);
        window.setHasShadow(false);
        let bg = objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(1.0, 1.0, 1.0, 0.001);
        window.setBackgroundColor(Some(&bg));
    }
}

pub fn create_child_view(parent: ViewHandle, frame: Rect) -> ViewHandle {
    let ns_frame = to_nsrect(frame);
    unsafe {
        let parent_view = &*(parent as *const NSView);
        let child: Retained<PassthroughView> =
            msg_send![mtm().alloc::<PassthroughView>(), initWithFrame: ns_frame];
        parent_view.addSubview_positioned_relativeTo(&child, NSWindowOrderingMode::Above, None);
        let ptr = Retained::as_ptr(&child) as *mut c_void;
        std::mem::forget(child);
        ptr
    }
}

pub fn view_bounds(view: ViewHandle) -> Rect {
    if view.is_null() {
        return Rect::default();
    }
    unsafe {
        let view = &*(view as *const NSView);
        from_nsrect(view.bounds())
    }
}

pub fn set_view_frame(view: ViewHandle, frame: Rect) {
    if view.is_null() {
        return;
    }
    unsafe {
        let view = &*(view as *const NSView);
        view.setFrame(to_nsrect(frame));
    }
}

pub fn set_window_title(title: &str) {
    if let Some(window) = main_window() {
        let ns_title = NSString::from_str(title);
        window.setTitle(&ns_title);
    }
}

pub fn set_resize_increments(width: f64, height: f64) {
    if let Some(window) = main_window() {
        window.setContentResizeIncrements(NSSize::new(width, height));
    }
}

pub fn set_view_hidden(view: ViewHandle, hidden: bool) {
    if view.is_null() {
        return;
    }
    unsafe {
        let view = &*(view as *const NSView);
        view.setHidden(hidden);
    }
}

pub fn remove_view(view: ViewHandle) {
    if view.is_null() {
        return;
    }
    unsafe {
        let view = &*(view as *const NSView);
        view.removeFromSuperview();
    }
}

pub fn set_view_layer_transparent(view: ViewHandle) {
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

pub fn request_redraw() {
    if let Some(view) = content_view() {
        view.setNeedsDisplay(true);
    }
}

// --- Event monitors ---

pub fn install_event_monitors(scroll_tx: std::sync::mpsc::Sender<ScrollEvent>) {
    install_cmd_drag_monitor();
    install_scroll_monitor(scroll_tx);
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
            (dx * 2.0, dy * 2.0)
        } else {
            (dx, dy)
        };

        let _ = tx.send(ScrollEvent {
            dx,
            dy,
            precision,
            momentum,
        });

        std::ptr::null_mut()
    });
    let monitor = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::ScrollWheel, &block)
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
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::LeftMouseDown, &block)
    };
    std::mem::forget(monitor);
}

// --- Overlay layers (CALayer) ---

pub fn create_scrollbar_layer() -> LayerHandle {
    use objc2_quartz_core::CALayer;

    let layer = CALayer::new();
    layer.setCornerRadius(3.0);
    layer.setOpacity(0.0);
    if let Some(cv) = content_view() {
        if let Some(parent_layer) = cv.layer() {
            parent_layer.addSublayer(&layer);
        }
    }
    let ptr = objc2::rc::Retained::as_ptr(&layer) as *mut c_void;
    std::mem::forget(layer);
    ptr
}

pub fn update_scrollbar_layer(
    layer: LayerHandle,
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
        objc2_quartz_core::CATransaction::begin();
        objc2_quartz_core::CATransaction::setDisableActions(true);
        layer.setFrame(NSRect::new(
            objc2_foundation::NSPoint::new(x, y),
            NSSize::new(width, height),
        ));
        layer.setOpacity(opacity);
        let color = objc2_core_graphics::CGColor::new_generic_rgb(0.6, 0.6, 0.6, opacity as f64);
        layer.setBackgroundColor(Some(&color));
        objc2_quartz_core::CATransaction::commit();
    }
}

pub fn create_highlight_layer() -> LayerHandle {
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

pub fn update_highlight_layer(
    layer: LayerHandle,
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
            (0.2, 0.5, 1.0, 0.3)
        } else {
            (0.9, 0.9, 0.9, 0.8)
        };
        let color = objc2_core_graphics::CGColor::new_generic_rgb(r, g, b, a);
        layer.setBackgroundColor(Some(&color));
        objc2_quartz_core::CATransaction::commit();
    }
}

// --- Clipboard ---

pub fn clipboard_read() -> Option<String> {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
    let pb = NSPasteboard::generalPasteboard();
    let text = pb.stringForType(unsafe { NSPasteboardTypeString });
    text.map(|ns_str| ns_str.to_string())
}

pub fn clipboard_write(text: &str) {
    use objc2_app_kit::NSPasteboard;
    let pb = NSPasteboard::generalPasteboard();
    pb.clearContents();
    let ns_str = NSString::from_str(text);
    let array = objc2_foundation::NSArray::from_slice(&[
        objc2::runtime::ProtocolObject::from_ref(&*ns_str),
    ]);
    pb.writeObjects(&array);
}

/// Write clipboard from a background thread (dispatches to main thread).
pub fn clipboard_write_from_thread(text: String) {
    let block = block2::RcBlock::new(move || {
        clipboard_write(&text);
    });
    unsafe {
        objc2_foundation::NSOperationQueue::mainQueue().addOperationWithBlock(&block);
    }
}

// --- Platform config for ghostty surface creation ---

pub fn platform_tag() -> i32 {
    ffi::ghostty_platform_e::GHOSTTY_PLATFORM_MACOS as i32
}

pub fn platform_config(view: ViewHandle) -> ffi::ghostty_platform_u {
    ffi::ghostty_platform_u {
        macos: ffi::ghostty_platform_macos_s { nsview: view },
    }
}
