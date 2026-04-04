//! macOS platform backend — AppKit + Core Animation + NSPasteboard.

use super::{KeyEvent, LayerHandle, Point, Rect, ScrollEvent, Size, TextInputCommand, TextInputEvent, ViewHandle};
use objc2::runtime::AnyObject;
use objc2::rc::Retained;
use objc2::{class, define_class, msg_send, sel, ClassType};
use objc2_app_kit::{
    NSApplication, NSEvent, NSEventMask, NSResponder, NSView, NSWindow, NSWindowOrderingMode,
};
use objc2_foundation::{NSArray, MainThreadMarker, NSAttributedString, NSObject, NSObjectProtocol, NSRange, NSRect, NSSize, NSString, NSNotFound};
use std::ffi::c_void;

static TEXT_INPUT_TX: std::sync::OnceLock<std::sync::mpsc::Sender<TextInputEvent>> =
    std::sync::OnceLock::new();
static KEY_EVENT_TX: std::sync::OnceLock<std::sync::mpsc::Sender<KeyEvent>> =
    std::sync::OnceLock::new();
static MARKED_TEXT: std::sync::OnceLock<std::sync::Mutex<String>> = std::sync::OnceLock::new();
static IME_RECT: std::sync::OnceLock<std::sync::Mutex<Rect>> = std::sync::OnceLock::new();

#[link(name = "UserNotifications", kind = "framework")]
unsafe extern "C" {}

fn send_text_input_event(event: TextInputEvent) {
    if let Some(tx) = TEXT_INPUT_TX.get() {
        let _ = tx.send(event);
    }
}

fn command_from_selector(selector: objc2::runtime::Sel) -> Option<TextInputCommand> {
    if selector == sel!(deleteBackward:)
        || selector == sel!(deleteBackwardByDecomposingPreviousCharacter:)
    {
        Some(TextInputCommand::Backspace)
    } else if selector == sel!(deleteForward:) {
        Some(TextInputCommand::DeleteForward)
    } else if selector == sel!(insertNewline:)
        || selector == sel!(insertLineBreak:)
        || selector == sel!(insertParagraphSeparator:)
        || selector == sel!(insertNewlineIgnoringFieldEditor:)
    {
        Some(TextInputCommand::InsertNewline)
    } else if selector == sel!(insertTab:) {
        Some(TextInputCommand::InsertTab)
    } else {
        None
    }
}

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

define_class!(
    #[unsafe(super(NSView))]
    #[name = "BooFocusableView"]
    struct FocusableView;

    impl FocusableView {
        #[unsafe(method(acceptsFirstResponder))]
        fn _accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(keyDown:))]
        fn _key_down(&self, event: &NSEvent) {
            if should_send_raw_key_event(event) {
                if let Some(tx) = KEY_EVENT_TX.get() {
                    let _ = tx.send(KeyEvent {
                        keycode: event.keyCode() as u32,
                        mods: event_mods(event),
                        repeat: event.isARepeat(),
                    });
                }
                return;
            }
            let responder: &NSResponder = unsafe { &*(self as *const Self as *const NSResponder) };
            let events = NSArray::from_slice(&[event]);
            responder.interpretKeyEvents(&events);
        }

        #[unsafe(method(insertText:))]
        fn _insert_text(&self, string: &AnyObject) {
            let object: &NSObject = unsafe { &*(string as *const AnyObject as *const NSObject) };
            let is_attributed = object.isKindOfClass(NSAttributedString::class());
            let is_string = object.isKindOfClass(NSString::class());
            let text = if is_attributed {
                let string: *const AnyObject = string;
                let string: *const NSAttributedString = string.cast();
                unsafe { &*string }.string().to_string()
            } else if is_string {
                let string: *const AnyObject = string;
                let string: *const NSString = string.cast();
                unsafe { &*string }.to_string()
            } else {
                return;
            };
            send_text_input_event(TextInputEvent::Commit(text));
        }

        #[unsafe(method(hasMarkedText))]
        fn _has_marked_text(&self) -> bool {
            MARKED_TEXT
                .get_or_init(|| std::sync::Mutex::new(String::new()))
                .lock()
                .map(|text| !text.is_empty())
                .unwrap_or(false)
        }

        #[unsafe(method(markedRange))]
        fn _marked_range(&self) -> NSRange {
            let len = MARKED_TEXT
                .get_or_init(|| std::sync::Mutex::new(String::new()))
                .lock()
                .map(|text| text.len())
                .unwrap_or(0);
            if len == 0 {
                NSRange::new(NSNotFound as usize, 0)
            } else {
                NSRange::new(0, len)
            }
        }

        #[unsafe(method(selectedRange))]
        fn _selected_range(&self) -> NSRange {
            NSRange::new(NSNotFound as usize, 0)
        }

        #[unsafe(method(setMarkedText:selectedRange:replacementRange:))]
        fn _set_marked_text(
            &self,
            string: &AnyObject,
            _selected_range: NSRange,
            _replacement_range: NSRange,
        ) {
            let object: &NSObject = unsafe { &*(string as *const AnyObject as *const NSObject) };
            let text = if object.isKindOfClass(NSAttributedString::class()) {
                let string: *const AnyObject = string;
                let string: *const NSAttributedString = string.cast();
                unsafe { &*string }.string().to_string()
            } else if object.isKindOfClass(NSString::class()) {
                let string: *const AnyObject = string;
                let string: *const NSString = string.cast();
                unsafe { &*string }.to_string()
            } else {
                return;
            };
            if let Ok(mut marked) = MARKED_TEXT
                .get_or_init(|| std::sync::Mutex::new(String::new()))
                .lock()
            {
                *marked = text.clone();
            }
            send_text_input_event(TextInputEvent::Preedit(text));
        }

        #[unsafe(method(unmarkText))]
        fn _unmark_text(&self) {
            if let Ok(mut marked) = MARKED_TEXT
                .get_or_init(|| std::sync::Mutex::new(String::new()))
                .lock()
            {
                marked.clear();
            }
            if let Some(input_context) = self.inputContext() {
                input_context.discardMarkedText();
            }
            send_text_input_event(TextInputEvent::PreeditClear);
        }

        #[unsafe(method(firstRectForCharacterRange:actualRange:))]
        fn _first_rect_for_character_range(
            &self,
            _range: NSRange,
            _actual_range: *mut NSRange,
        ) -> NSRect {
            let rect = IME_RECT
                .get_or_init(|| std::sync::Mutex::new(Rect::default()))
                .lock()
                .map(|rect| *rect)
                .unwrap_or_default();
            let rect = to_nsrect(rect);
            if let Some(window) = self.window() {
                window.convertRectToScreen(self.convertRect_toView(rect, None))
            } else {
                rect
            }
        }

        #[unsafe(method(doCommandBySelector:))]
        fn _do_command_by_selector(&self, selector: objc2::runtime::Sel) {
            if let Some(command) = command_from_selector(selector) {
                send_text_input_event(TextInputEvent::Command(command));
            }
        }

        #[unsafe(method(deleteBackward:))]
        fn _delete_backward(&self, _sender: &AnyObject) {
            send_text_input_event(TextInputEvent::Command(TextInputCommand::Backspace));
        }

        #[unsafe(method(deleteForward:))]
        fn _delete_forward(&self, _sender: &AnyObject) {
            send_text_input_event(TextInputEvent::Command(TextInputCommand::DeleteForward));
        }

        #[unsafe(method(insertNewline:))]
        fn _insert_newline(&self, _sender: &AnyObject) {
            send_text_input_event(TextInputEvent::Command(TextInputCommand::InsertNewline));
        }

        #[unsafe(method(insertTab:))]
        fn _insert_tab(&self, _sender: &AnyObject) {
            send_text_input_event(TextInputEvent::Command(TextInputCommand::InsertTab));
        }
    }
);

fn mtm() -> MainThreadMarker {
    unsafe { MainThreadMarker::new_unchecked() }
}

fn event_mods(event: &NSEvent) -> i32 {
    let flags = event.modifierFlags();
    let mut mods = 0;
    if flags.contains(objc2_app_kit::NSEventModifierFlags::Shift) {
        mods |= crate::ffi::GHOSTTY_MODS_SHIFT;
    }
    if flags.contains(objc2_app_kit::NSEventModifierFlags::Control) {
        mods |= crate::ffi::GHOSTTY_MODS_CTRL;
    }
    if flags.contains(objc2_app_kit::NSEventModifierFlags::Option) {
        mods |= crate::ffi::GHOSTTY_MODS_ALT;
    }
    if flags.contains(objc2_app_kit::NSEventModifierFlags::Command) {
        mods |= crate::ffi::GHOSTTY_MODS_SUPER;
    }
    mods
}

fn should_send_raw_key_event(event: &NSEvent) -> bool {
    let mods = event_mods(event);
    if mods & (crate::ffi::GHOSTTY_MODS_CTRL | crate::ffi::GHOSTTY_MODS_SUPER) != 0 {
        return true;
    }
    matches!(
        event.keyCode() as u32,
        0x35 | 0x7B | 0x7C | 0x7D | 0x7E | 0x73 | 0x77 | 0x74 | 0x79
    )
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

pub fn create_focusable_child_view(parent: ViewHandle, frame: Rect) -> ViewHandle {
    let ns_frame = to_nsrect(frame);
    unsafe {
        let parent_view = &*(parent as *const NSView);
        let child: Retained<FocusableView> =
            msg_send![mtm().alloc::<FocusableView>(), initWithFrame: ns_frame];
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

pub fn focus_view(view: ViewHandle) {
    if view.is_null() {
        return;
    }
    unsafe {
        let view = &*(view as *const NSView);
        let responder: &NSResponder = &*(view as *const NSView as *const NSResponder);
        let _ = responder.becomeFirstResponder();
        if let Some(window) = view.window() {
            let _ = window.makeFirstResponder(Some(responder));
        }
    }
}

pub fn set_text_input_cursor_rect(rect: Rect) {
    if let Ok(mut current) = IME_RECT
        .get_or_init(|| std::sync::Mutex::new(Rect::default()))
        .lock()
    {
        *current = rect;
    }
}

// --- Event monitors ---

pub fn install_event_monitors(
    scroll_tx: std::sync::mpsc::Sender<ScrollEvent>,
    key_event_tx: std::sync::mpsc::Sender<KeyEvent>,
    text_input_tx: std::sync::mpsc::Sender<TextInputEvent>,
) {
    let _ = KEY_EVENT_TX.set(key_event_tx);
    let _ = TEXT_INPUT_TX.set(text_input_tx);
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

pub fn send_desktop_notification(title: &str, body: &str) {
    let title = title.to_string();
    let body = body.to_string();
    std::thread::spawn(move || unsafe {
        if !send_user_notification(&title, &body) {
            send_apple_script_notification(&title, &body);
        }
    });
}

unsafe fn send_user_notification(title: &str, body: &str) -> bool {
    let center: *mut AnyObject = msg_send![class!(UNUserNotificationCenter), currentNotificationCenter];
    if center.is_null() {
        return false;
    }

    let () = msg_send![
        center,
        requestAuthorizationWithOptions: 0b11usize,
        completionHandler: std::ptr::null::<AnyObject>()
    ];

    let content: Retained<AnyObject> = msg_send![class!(UNMutableNotificationContent), new];
    let ns_title = NSString::from_str(title);
    let ns_body = NSString::from_str(body);
    let () = msg_send![&*content, setTitle: &*ns_title];
    let () = msg_send![&*content, setBody: &*ns_body];

    let sound: *mut AnyObject = msg_send![class!(UNNotificationSound), defaultSound];
    if !sound.is_null() {
        let () = msg_send![&*content, setSound: sound];
    }

    let identifier = NSString::from_str(&notification_identifier());
    let request: Retained<AnyObject> = msg_send![
        class!(UNNotificationRequest),
        requestWithIdentifier: &*identifier,
        content: &*content,
        trigger: std::ptr::null::<AnyObject>()
    ];

    let () = msg_send![
        center,
        addNotificationRequest: &*request,
        withCompletionHandler: std::ptr::null::<AnyObject>()
    ];
    true
}

fn notification_identifier() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    format!("boo-{}-{id}", std::process::id())
}

fn send_apple_script_notification(title: &str, body: &str) {
    let title = apple_script_literal(title);
    let body = apple_script_literal(body);
    let script = format!("display notification {body} with title {title}");
    let status = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .status();
    if let Err(error) = status {
        log::warn!("failed to send macOS notification: {}", error);
    }
}

fn apple_script_literal(text: &str) -> String {
    let escaped = text
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::{apple_script_literal, notification_identifier};

    #[test]
    fn apple_script_literal_escapes_quotes_and_newlines() {
        assert_eq!(
            apple_script_literal("a\"b\nc\\d"),
            "\"a\\\"b\\nc\\\\d\""
        );
    }

    #[test]
    fn notification_identifier_is_stable_and_prefixed() {
        let id = notification_identifier();
        assert!(id.starts_with("boo-"));
        assert!(id.len() > 4);
    }
}
