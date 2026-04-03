//! Linux platform backend — EGL surface embedding + arboard clipboard.

use super::{LayerHandle, Rect, ScrollEvent, ViewHandle};
use crate::ffi;
use std::ffi::c_void;

// --- EGL state ---

/// Holds the EGL display, surface, and context for a ghostty terminal surface.
pub struct EglState {
    pub display: *mut c_void,
    pub surface: *mut c_void,
    pub context: *mut c_void,
    lib: khronos_egl::DynamicInstance<khronos_egl::EGL1_4>,
}

impl EglState {
    /// Create an EGL context for rendering. Uses the platform display and
    /// creates a pbuffer surface (ghostty manages its own framebuffer).
    pub fn new() -> Option<Self> {
        let lib = unsafe { khronos_egl::DynamicInstance::<khronos_egl::EGL1_4>::load_required() }
            .ok()?;

        // Try platform-specific display, then fall back to default.
        // get_platform_display already calls eglInitialize on success.
        let display = Self::get_platform_display(&lib)
            .or_else(|| {
                let d = unsafe { lib.get_display(khronos_egl::DEFAULT_DISPLAY) }?;
                lib.initialize(d).ok()?;
                log::info!("EGL: using default display");
                Some(d)
            })
            .or_else(|| {
                log::error!("EGL: no display available");
                None
            })?;

        // Ghostty requires OpenGL 4.3 core profile.
        // Try full OpenGL first, fall back to trying without pbuffer.
        lib.bind_api(khronos_egl::OPENGL_API)
            .map_err(|e| log::error!("EGL: bind_api(OPENGL_API) failed: {e}"))
            .ok()?;

        let attribs = [
            khronos_egl::RENDERABLE_TYPE,
            khronos_egl::OPENGL_BIT,
            khronos_egl::SURFACE_TYPE,
            khronos_egl::PBUFFER_BIT,
            khronos_egl::RED_SIZE, 8,
            khronos_egl::GREEN_SIZE, 8,
            khronos_egl::BLUE_SIZE, 8,
            khronos_egl::ALPHA_SIZE, 8,
            khronos_egl::NONE,
        ];
        let config = lib
            .choose_first_config(display, &attribs)
            .ok()
            .and_then(|c| c)
            .or_else(|| {
                // Retry without PBUFFER_BIT — some drivers only support window surfaces
                log::warn!("EGL: pbuffer config unavailable, trying window surface");
                let attribs2 = [
                    khronos_egl::RENDERABLE_TYPE,
                    khronos_egl::OPENGL_BIT,
                    khronos_egl::RED_SIZE, 8,
                    khronos_egl::GREEN_SIZE, 8,
                    khronos_egl::BLUE_SIZE, 8,
                    khronos_egl::ALPHA_SIZE, 8,
                    khronos_egl::NONE,
                ];
                lib.choose_first_config(display, &attribs2).ok().and_then(|c| c)
            })
            .or_else(|| {
                log::error!("EGL: no suitable config found");
                None
            })?;

        let ctx_attribs = [
            khronos_egl::CONTEXT_MAJOR_VERSION,
            4,
            khronos_egl::CONTEXT_MINOR_VERSION,
            3,
            khronos_egl::CONTEXT_OPENGL_PROFILE_MASK,
            khronos_egl::CONTEXT_OPENGL_CORE_PROFILE_BIT,
            khronos_egl::NONE,
        ];
        let context = lib
            .create_context(display, config, None, &ctx_attribs)
            .map_err(|e| log::error!("EGL: create_context failed: {e}"))
            .ok()?;

        let pbuf_attribs = [
            khronos_egl::WIDTH,
            1,
            khronos_egl::HEIGHT,
            1,
            khronos_egl::NONE,
        ];
        let surface = lib
            .create_pbuffer_surface(display, config, &pbuf_attribs)
            .ok()?;

        // Make current so ghostty can initialize GL in surfaceInit
        lib.make_current(display, Some(surface), Some(surface), Some(context))
            .ok()?;

        log::info!("EGL context created for ghostty surface");

        Some(EglState {
            display: display.as_ptr() as *mut c_void,
            surface: surface.as_ptr() as *mut c_void,
            context: context.as_ptr() as *mut c_void,
            lib,
        })
    }

    /// Get an EGL display with desktop OpenGL support via EGL device
    /// enumeration. This uses the DRM render node directly and works
    /// on both Wayland and X11 — no display server dependency.
    fn get_platform_display(
        lib: &khronos_egl::DynamicInstance<khronos_egl::EGL1_4>,
    ) -> Option<khronos_egl::Display> {
        const EGL_PLATFORM_DEVICE_EXT: khronos_egl::Enum = 0x313F;

        type GetPlatformDisplayFn = unsafe extern "C" fn(
            khronos_egl::Enum,
            *mut c_void,
            *const khronos_egl::Attrib,
        ) -> *mut c_void;

        type QueryDevicesFn = unsafe extern "C" fn(
            max_devices: i32,
            devices: *mut *mut c_void,
            num_devices: *mut i32,
        ) -> u32;

        let get_platform_display: GetPlatformDisplayFn = unsafe {
            std::mem::transmute(lib.get_proc_address("eglGetPlatformDisplay")?)
        };
        let query_devices: QueryDevicesFn = unsafe {
            std::mem::transmute(lib.get_proc_address("eglQueryDevicesEXT")?)
        };

        // Enumerate EGL devices (GPU render nodes)
        let mut num_devices: i32 = 0;
        let mut devices = [std::ptr::null_mut::<c_void>(); 8];
        let ok = unsafe { query_devices(8, devices.as_mut_ptr(), &mut num_devices) };
        if ok == 0 || num_devices == 0 {
            log::debug!("EGL: no devices found");
            return None;
        }

        // Try each device until we find one with desktop OpenGL
        for i in 0..num_devices as usize {
            let raw = unsafe {
                get_platform_display(EGL_PLATFORM_DEVICE_EXT, devices[i], std::ptr::null())
            };
            if raw.is_null() {
                continue;
            }
            let display = unsafe { khronos_egl::Display::from_ptr(raw) };
            if let Err(e) = lib.initialize(display) {
                log::debug!("EGL: device {i} initialize failed: {e}");
                continue;
            }
            let _ = lib.bind_api(khronos_egl::OPENGL_API);
            let test_attribs = [
                khronos_egl::RENDERABLE_TYPE, khronos_egl::OPENGL_BIT,
                khronos_egl::NONE,
            ];
            if let Ok(Some(_)) = lib.choose_first_config(display, &test_attribs) {
                log::info!("EGL: using device {i} (desktop OpenGL available)");
                return Some(display);
            }
            log::debug!("EGL: device {i} has no desktop OpenGL configs");
        }

        None
    }

    /// Release the EGL context from the current thread (so the renderer
    /// thread can claim it).
    pub fn release_current(&self) {
        let display = unsafe { khronos_egl::Display::from_ptr(self.display) };
        let _ = self.lib.make_current(display, None, None, None);
    }
}

// --- Window / view management ---

pub fn scale_factor() -> f64 {
    1.0
}

pub fn content_view_handle() -> ViewHandle {
    // Return a non-null sentinel so init_surface proceeds.
    // On Linux we don't have a native content view — EGL handles rendering.
    1usize as ViewHandle
}

pub fn set_window_transparent() {}

#[allow(dead_code)]
pub fn create_child_view(_parent_handle: ViewHandle, _frame: Rect) -> ViewHandle {
    std::ptr::null_mut()
}

pub fn view_bounds(_view: ViewHandle) -> Rect {
    Rect::default()
}

pub fn set_view_frame(_view: ViewHandle, _frame: Rect) {}
pub fn set_window_title(_title: &str) {}
pub fn set_resize_increments(_width: f64, _height: f64) {}
pub fn set_view_hidden(_view: ViewHandle, _hidden: bool) {}
pub fn remove_view(_view: ViewHandle) {}
pub fn set_view_layer_transparent(_view: ViewHandle) {}
pub fn request_redraw() {}

// --- Event monitors ---

pub fn install_event_monitors(_scroll_tx: std::sync::mpsc::Sender<ScrollEvent>) {}

// --- Overlay layers (no-ops) ---

pub fn create_scrollbar_layer() -> LayerHandle {
    std::ptr::null_mut()
}

pub fn update_scrollbar_layer(
    _layer: LayerHandle,
    _x: f64,
    _y: f64,
    _width: f64,
    _height: f64,
    _opacity: f32,
) {
}

pub fn create_highlight_layer() -> LayerHandle {
    std::ptr::null_mut()
}

pub fn update_highlight_layer(
    _layer: LayerHandle,
    _x: f64,
    _y: f64,
    _width: f64,
    _height: f64,
    _visible: bool,
    _is_selection: bool,
) {
}

// --- Clipboard (arboard) ---

pub fn clipboard_read() -> Option<String> {
    arboard::Clipboard::new()
        .ok()
        .and_then(|mut cb| cb.get_text().ok())
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

// --- Platform config for ghostty surface creation ---

pub fn platform_tag() -> i32 {
    ffi::ghostty_platform_e::GHOSTTY_PLATFORM_EGL as i32
}

pub fn platform_config(egl_state: &EglState) -> ffi::ghostty_platform_u {
    ffi::ghostty_platform_u {
        egl: ffi::ghostty_platform_egl_s {
            display: egl_state.display,
            surface: egl_state.surface,
            context: egl_state.context,
        },
    }
}
