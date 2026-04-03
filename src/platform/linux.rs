//! Linux platform backend — EGL offscreen rendering + iced image display.
//!
//! Architecture:
//! 1. ghostty renders to an EGL pbuffer (offscreen, desktop GL 4.3)
//! 2. After each frame, glReadPixels copies pixels to shared memory
//! 3. iced displays the frame as an image widget
//! 4. Wayland/X11 compositing handled by iced's own wgpu renderer

use super::{LayerHandle, Rect, ScrollEvent, ViewHandle};
use crate::ffi;
use std::ffi::c_void;
use std::sync::{Arc, Mutex};

// ============================================================
// EGL state — offscreen rendering context
// ============================================================

/// Shared pixel buffer between ghostty's renderer thread and iced's main thread.
pub type SharedFrameBuffer = Arc<Mutex<FrameData>>;

pub struct FrameData {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub dirty: bool,
}

/// EGL context for offscreen rendering. ghostty renders here.
pub struct EglState {
    pub display: *mut c_void,
    pub surface: *mut c_void,
    pub context: *mut c_void,
    /// Shared frame buffer — ghostty writes, iced reads.
    pub frame_buffer: SharedFrameBuffer,
    lib: khronos_egl::DynamicInstance<khronos_egl::EGL1_4>,
}

impl EglState {
    pub fn new(width: u32, height: u32) -> Option<Self> {
        let lib = unsafe { khronos_egl::DynamicInstance::<khronos_egl::EGL1_4>::load_required() }
            .ok()?;

        // Try default display first (works on most Mesa setups)
        let display = unsafe { lib.get_display(khronos_egl::DEFAULT_DISPLAY) }
            .or_else(|| { log::warn!("EGL: default display unavailable"); None })?;
        lib.initialize(display).ok()?;

        lib.bind_api(khronos_egl::OPENGL_API)
            .map_err(|e| log::error!("EGL bind_api failed: {e}"))
            .ok()?;

        // Request pbuffer config with desktop OpenGL
        let attribs = [
            khronos_egl::RENDERABLE_TYPE, khronos_egl::OPENGL_BIT,
            khronos_egl::SURFACE_TYPE, khronos_egl::PBUFFER_BIT,
            khronos_egl::RED_SIZE, 8,
            khronos_egl::GREEN_SIZE, 8,
            khronos_egl::BLUE_SIZE, 8,
            khronos_egl::ALPHA_SIZE, 8,
            khronos_egl::NONE,
        ];
        let config = lib.choose_first_config(display, &attribs)
            .ok().and_then(|c| c)
            .or_else(|| {
                // Fallback: try without pbuffer requirement
                let attribs2 = [
                    khronos_egl::RENDERABLE_TYPE, khronos_egl::OPENGL_BIT,
                    khronos_egl::RED_SIZE, 8, khronos_egl::GREEN_SIZE, 8,
                    khronos_egl::BLUE_SIZE, 8, khronos_egl::ALPHA_SIZE, 8,
                    khronos_egl::NONE,
                ];
                lib.choose_first_config(display, &attribs2).ok().and_then(|c| c)
            })
            .or_else(|| { log::error!("EGL: no desktop GL config found"); None })?;

        // OpenGL 4.3 core context
        let ctx_attribs = [
            khronos_egl::CONTEXT_MAJOR_VERSION, 4,
            khronos_egl::CONTEXT_MINOR_VERSION, 3,
            khronos_egl::CONTEXT_OPENGL_PROFILE_MASK,
            khronos_egl::CONTEXT_OPENGL_CORE_PROFILE_BIT,
            khronos_egl::NONE,
        ];
        let context = lib.create_context(display, config, None, &ctx_attribs)
            .map_err(|e| log::error!("EGL create_context failed: {e}"))
            .ok()?;

        // Pbuffer surface
        let pbuf_attribs = [
            khronos_egl::WIDTH, width as i32,
            khronos_egl::HEIGHT, height as i32,
            khronos_egl::NONE,
        ];
        let surface = lib.create_pbuffer_surface(display, config, &pbuf_attribs)
            .map_err(|e| log::error!("EGL create_pbuffer_surface failed: {e}"))
            .ok()?;

        // Make current for ghostty GL init
        lib.make_current(display, Some(surface), Some(surface), Some(context)).ok()?;

        let frame_buffer = Arc::new(Mutex::new(FrameData {
            pixels: vec![0u8; (width * height * 4) as usize],
            width,
            height,
            dirty: false,
        }));

        log::info!("EGL pbuffer context created: {width}x{height}");

        Some(EglState {
            display: display.as_ptr() as *mut c_void,
            surface: surface.as_ptr() as *mut c_void,
            context: context.as_ptr() as *mut c_void,
            frame_buffer,
            lib,
        })
    }

    /// Release EGL context from current thread (renderer thread claims it).
    pub fn release_current(&self) {
        let display = unsafe { khronos_egl::Display::from_ptr(self.display) };
        let _ = self.lib.make_current(display, None, None, None);
    }
}

// ============================================================
// Platform API (matches macos.rs)
// ============================================================

pub fn scale_factor() -> f64 { 1.0 }
pub fn content_view_handle() -> ViewHandle { 1usize as ViewHandle } // non-null sentinel
pub fn set_window_transparent() {}

#[allow(dead_code)]
pub fn create_child_view(_: ViewHandle, _: Rect) -> ViewHandle { std::ptr::null_mut() }
pub fn view_bounds(_: ViewHandle) -> Rect { Rect::default() }
pub fn set_view_frame(_: ViewHandle, _: Rect) {}
pub fn set_window_title(_: &str) {}
pub fn set_resize_increments(_: f64, _: f64) {}
pub fn set_view_hidden(_: ViewHandle, _: bool) {}
pub fn remove_view(_: ViewHandle) {}
pub fn set_view_layer_transparent(_: ViewHandle) {}
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
    if let Ok(mut cb) = arboard::Clipboard::new() { let _ = cb.set_text(text.to_owned()); }
}
pub fn clipboard_write_from_thread(text: String) {
    if let Ok(mut cb) = arboard::Clipboard::new() { let _ = cb.set_text(text); }
}

pub fn platform_tag() -> i32 { ffi::ghostty_platform_e::GHOSTTY_PLATFORM_EGL as i32 }

pub fn platform_config(egl: &EglState) -> ffi::ghostty_platform_u {
    ffi::ghostty_platform_u {
        egl: ffi::ghostty_platform_egl_s {
            display: egl.display,
            surface: egl.surface,
            context: egl.context,
            frame_cb: Some(frame_callback),
            frame_cb_userdata: Arc::as_ptr(&egl.frame_buffer) as *mut c_void,
        },
    }
}

/// Called from ghostty's renderer thread after each frame.
/// The target FBO is bound for reading. We call glReadPixels (provided by
/// libghostty.so which already links GL) to capture pixels.
unsafe extern "C" fn frame_callback(userdata: *mut c_void, width: u32, height: u32) {
    unsafe extern "C" {
        fn glReadPixels(x: i32, y: i32, w: i32, h: i32, format: u32, type_: u32, data: *mut c_void);
    }
    const GL_RGBA: u32 = 0x1908;
    const GL_UNSIGNED_BYTE: u32 = 0x1401;

    if userdata.is_null() || width == 0 || height == 0 {
        return;
    }

    let frame_buffer = unsafe { &*(userdata as *const Mutex<FrameData>) };
    let Ok(mut frame) = frame_buffer.lock() else { return };

    let size = (width * height * 4) as usize;
    if frame.pixels.len() != size {
        frame.pixels.resize(size, 0);
    }
    frame.width = width;
    frame.height = height;

    unsafe {
        glReadPixels(0, 0, width as i32, height as i32, GL_RGBA, GL_UNSIGNED_BYTE,
            frame.pixels.as_mut_ptr() as *mut c_void);
    }
    frame.dirty = true;
    log::debug!("frame captured: {width}x{height}");
}
