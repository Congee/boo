//! Linux platform backend — GBM + EGL + Wayland subsurface compositing.
//!
//! Architecture (mirrors macOS NSView + Metal):
//! 1. Each terminal pane gets a Wayland subsurface (child of iced's window)
//! 2. A GBM surface provides the EGL window surface (desktop GL 4.3)
//! 3. ghostty renders → eglSwapBuffers → GBM front buffer → compositor

use super::{LayerHandle, Rect, ScrollEvent, ViewHandle};
use crate::ffi;
use std::ffi::c_void;
use std::os::unix::io::IntoRawFd;
use std::sync::Arc;
use wayland_client::protocol::{wl_compositor, wl_subcompositor, wl_subsurface, wl_surface};
use wayland_client::{backend::Backend, Connection, Dispatch, Proxy};

// ============================================================
// GBM raw FFI (no Rust crate available)
// ============================================================

mod gbm {
    use std::ffi::c_void;
    use std::os::raw::c_int;

    pub const GBM_FORMAT_ARGB8888: u32 = 0x34325241;
    pub const GBM_BO_USE_RENDERING: u32 = 1 << 2;

    unsafe extern "C" {
        pub fn gbm_create_device(fd: c_int) -> *mut c_void;
        pub fn gbm_surface_create(
            gbm: *mut c_void, width: u32, height: u32, format: u32, flags: u32,
        ) -> *mut c_void;
        pub fn gbm_surface_destroy(surface: *mut c_void);
    }
}

// ============================================================
// Wayland state for registry binding
// ============================================================

struct WaylandState {
    compositor: Option<wl_compositor::WlCompositor>,
    subcompositor: Option<wl_subcompositor::WlSubcompositor>,
}

impl Dispatch<wayland_client::protocol::wl_registry::WlRegistry, ()> for WaylandState {
    fn event(
        state: &mut Self,
        registry: &wayland_client::protocol::wl_registry::WlRegistry,
        event: wayland_client::protocol::wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        qh: &wayland_client::QueueHandle<Self>,
    ) {
        if let wayland_client::protocol::wl_registry::Event::Global { name, interface, version } = event {
            match interface.as_str() {
                "wl_compositor" => {
                    state.compositor = Some(registry.bind(name, version.min(6), qh, ()));
                }
                "wl_subcompositor" => {
                    state.subcompositor = Some(registry.bind(name, version.min(1), qh, ()));
                }
                _ => {}
            }
        }
    }
}

// No-op dispatch for objects we create but don't listen to events on
impl Dispatch<wl_compositor::WlCompositor, ()> for WaylandState {
    fn event(_: &mut Self, _: &wl_compositor::WlCompositor, _: wl_compositor::Event, _: &(), _: &Connection, _: &wayland_client::QueueHandle<Self>) {}
}
impl Dispatch<wl_subcompositor::WlSubcompositor, ()> for WaylandState {
    fn event(_: &mut Self, _: &wl_subcompositor::WlSubcompositor, _: wl_subcompositor::Event, _: &(), _: &Connection, _: &wayland_client::QueueHandle<Self>) {}
}
impl Dispatch<wl_surface::WlSurface, ()> for WaylandState {
    fn event(_: &mut Self, _: &wl_surface::WlSurface, _: wl_surface::Event, _: &(), _: &Connection, _: &wayland_client::QueueHandle<Self>) {}
}
impl Dispatch<wl_subsurface::WlSubsurface, ()> for WaylandState {
    fn event(_: &mut Self, _: &wl_subsurface::WlSubsurface, _: wl_subsurface::Event, _: &(), _: &Connection, _: &wayland_client::QueueHandle<Self>) {}
}

// ============================================================
// WaylandDisplay — shared state across all panes
// ============================================================

pub struct WaylandDisplay {
    conn: Arc<Connection>,
    compositor: wl_compositor::WlCompositor,
    subcompositor: wl_subcompositor::WlSubcompositor,
    parent_surface: wl_surface::WlSurface,
    gbm_device: *mut c_void,
    egl_display: khronos_egl::Display,
    egl_config: khronos_egl::Config,
    egl_lib: khronos_egl::DynamicInstance<khronos_egl::EGL1_4>,
}

impl WaylandDisplay {
    /// Initialize from iced's raw wl_surface pointer.
    pub fn new(parent_surface_ptr: *mut c_void) -> Option<Self> {
        if parent_surface_ptr.is_null() {
            return None;
        }

        // Get the wl_display from iced's surface
        unsafe extern "C" {
            fn wl_proxy_get_display(proxy: *mut c_void) -> *mut c_void;
        }
        let wl_display_ptr = unsafe { wl_proxy_get_display(parent_surface_ptr) };
        if wl_display_ptr.is_null() {
            log::error!("wl_proxy_get_display returned null");
            return None;
        }

        // Wrap the foreign display in a wayland-client Connection
        let backend = unsafe { Backend::from_foreign_display(wl_display_ptr as *mut _) };
        let conn = Arc::new(Connection::from_backend(backend));
        let display = conn.display();

        // Create our own event queue for objects we create
        let mut event_queue = conn.new_event_queue::<WaylandState>();
        let qh = event_queue.handle();
        let mut state = WaylandState {
            compositor: None,
            subcompositor: None,
        };

        // Get registry and bind globals
        let _registry = display.get_registry(&qh, ());
        event_queue.roundtrip(&mut state).ok()?;

        let compositor = state.compositor.take()?;
        let subcompositor = state.subcompositor.take()?;

        // Wrap the parent wl_surface as a wayland-client proxy
        let parent_id = unsafe {
            wayland_client::backend::ObjectId::from_ptr(
                wl_surface::WlSurface::interface(),
                parent_surface_ptr as *mut _,
            )
        }.ok()?;
        let parent_surface = wl_surface::WlSurface::from_id(&conn, parent_id).ok()?;

        // Open DRM render node + create GBM device
        let drm_fd = Self::open_render_node()?;
        let gbm_device = unsafe { gbm::gbm_create_device(drm_fd) };
        if gbm_device.is_null() {
            log::error!("gbm_create_device failed");
            return None;
        }

        // Create EGL display from GBM device (desktop GL!)
        let egl_lib = unsafe {
            khronos_egl::DynamicInstance::<khronos_egl::EGL1_4>::load_required()
        }.ok()?;

        let egl_display = Self::create_gbm_egl_display(&egl_lib, gbm_device)?;
        egl_lib.initialize(egl_display)
            .map_err(|e| log::error!("EGL initialize failed: {e}"))
            .ok()?;

        egl_lib.bind_api(khronos_egl::OPENGL_API)
            .map_err(|e| log::error!("EGL bind_api failed: {e}"))
            .ok()?;

        let attribs = [
            khronos_egl::RENDERABLE_TYPE, khronos_egl::OPENGL_BIT,
            khronos_egl::SURFACE_TYPE, khronos_egl::WINDOW_BIT,
            khronos_egl::RED_SIZE, 8,
            khronos_egl::GREEN_SIZE, 8,
            khronos_egl::BLUE_SIZE, 8,
            khronos_egl::ALPHA_SIZE, 8,
            khronos_egl::NONE,
        ];
        let egl_config = egl_lib.choose_first_config(egl_display, &attribs)
            .ok().and_then(|c| c)
            .or_else(|| { log::error!("EGL: no config with GL + WINDOW on GBM"); None })?;

        log::info!("GBM + EGL + Wayland subsurface display initialized");

        Some(WaylandDisplay {
            conn,
            compositor,
            subcompositor,
            parent_surface,
            gbm_device,
            egl_display,
            egl_config,
            egl_lib,
        })
    }

    fn open_render_node() -> Option<i32> {
        for i in 128..136 {
            let path = format!("/dev/dri/renderD{i}");
            if let Ok(file) = std::fs::OpenOptions::new().read(true).write(true).open(&path) {
                log::info!("Opened {path}");
                return Some(file.into_raw_fd());
            }
        }
        log::error!("No DRM render node found");
        None
    }

    fn create_gbm_egl_display(
        lib: &khronos_egl::DynamicInstance<khronos_egl::EGL1_4>,
        gbm_device: *mut c_void,
    ) -> Option<khronos_egl::Display> {
        const EGL_PLATFORM_GBM_KHR: khronos_egl::Enum = 0x31D7;
        type Fn = unsafe extern "C" fn(khronos_egl::Enum, *mut c_void, *const khronos_egl::Attrib) -> *mut c_void;
        let f: Fn = unsafe { std::mem::transmute(lib.get_proc_address("eglGetPlatformDisplay")?) };
        let raw = unsafe { f(EGL_PLATFORM_GBM_KHR, gbm_device, std::ptr::null()) };
        if raw.is_null() { return None; }
        Some(unsafe { khronos_egl::Display::from_ptr(raw) })
    }

    pub fn create_pane(&self, frame: Rect, scale: f64) -> Option<Box<WaylandPane>> {
        let w = ((frame.size.width * scale) as u32).max(1);
        let h = ((frame.size.height * scale) as u32).max(1);

        // Create event queue for new objects
        let mut eq = self.conn.new_event_queue::<WaylandState>();
        let qh = eq.handle();
        let mut dummy = WaylandState { compositor: None, subcompositor: None };

        // Create child wl_surface
        let child_surface = self.compositor.create_surface(&qh, ());

        // Create subsurface
        let subsurface = self.subcompositor.get_subsurface(&child_surface, &self.parent_surface, &qh, ());
        subsurface.set_desync();
        subsurface.set_position(frame.origin.x as i32, frame.origin.y as i32);

        // Flush to make sure compositor sees the subsurface
        let _ = eq.roundtrip(&mut dummy);

        // Create GBM surface
        let gbm_surface = unsafe {
            gbm::gbm_surface_create(
                self.gbm_device, w, h,
                gbm::GBM_FORMAT_ARGB8888, gbm::GBM_BO_USE_RENDERING,
            )
        };
        if gbm_surface.is_null() {
            log::error!("gbm_surface_create failed");
            return None;
        }

        // EGL window surface from GBM
        let egl_surface = self.egl_lib
            .create_window_surface(self.egl_display, self.egl_config, gbm_surface, None)
            .map_err(|e| log::error!("eglCreateWindowSurface failed: {e}"))
            .ok()?;

        // EGL context (OpenGL 4.3 core)
        let ctx_attribs = [
            khronos_egl::CONTEXT_MAJOR_VERSION, 4,
            khronos_egl::CONTEXT_MINOR_VERSION, 3,
            khronos_egl::CONTEXT_OPENGL_PROFILE_MASK,
            khronos_egl::CONTEXT_OPENGL_CORE_PROFILE_BIT,
            khronos_egl::NONE,
        ];
        let egl_context = self.egl_lib
            .create_context(self.egl_display, self.egl_config, None, &ctx_attribs)
            .map_err(|e| log::error!("eglCreateContext failed: {e}"))
            .ok()?;

        // Make current for ghostty GL init
        self.egl_lib.make_current(
            self.egl_display, Some(egl_surface), Some(egl_surface), Some(egl_context),
        ).ok()?;

        log::info!("Wayland pane: {w}x{h} at ({}, {})", frame.origin.x as i32, frame.origin.y as i32);

        Some(Box::new(WaylandPane {
            child_surface,
            subsurface,
            gbm_surface,
            egl_surface: egl_surface.as_ptr() as *mut c_void,
            egl_context: egl_context.as_ptr() as *mut c_void,
            egl_display: self.egl_display.as_ptr() as *mut c_void,
        }))
    }

    pub fn release_current(&self) {
        let _ = self.egl_lib.make_current(self.egl_display, None, None, None);
    }
}

// ============================================================
// WaylandPane — per-pane state
// ============================================================

pub struct WaylandPane {
    child_surface: wl_surface::WlSurface,
    subsurface: wl_subsurface::WlSubsurface,
    gbm_surface: *mut c_void,
    egl_surface: *mut c_void,
    egl_context: *mut c_void,
    egl_display: *mut c_void,
}

impl WaylandPane {
    pub fn set_frame(&mut self, frame: Rect, _scale: f64) {
        self.subsurface.set_position(frame.origin.x as i32, frame.origin.y as i32);
    }

    pub fn egl_platform_config(&self) -> ffi::ghostty_platform_u {
        ffi::ghostty_platform_u {
            egl: ffi::ghostty_platform_egl_s {
                display: self.egl_display,
                surface: self.egl_surface,
                context: self.egl_context,
            },
        }
    }
}

impl Drop for WaylandPane {
    fn drop(&mut self) {
        unsafe { gbm::gbm_surface_destroy(self.gbm_surface); }
        self.subsurface.destroy();
        self.child_surface.destroy();
    }
}

// ============================================================
// Platform API (matches macos.rs)
// ============================================================

pub fn scale_factor() -> f64 { 1.0 }
#[allow(dead_code)]
pub fn content_view_handle() -> ViewHandle { std::ptr::null_mut() }
pub fn set_window_transparent() {}
#[allow(dead_code)]
pub fn create_child_view(_: ViewHandle, _: Rect) -> ViewHandle { std::ptr::null_mut() }
pub fn view_bounds(_: ViewHandle) -> Rect { Rect::default() }

pub fn set_view_frame(view: ViewHandle, frame: Rect) {
    if view.is_null() { return; }
    let pane = unsafe { &mut *(view as *mut WaylandPane) };
    pane.set_frame(frame, 1.0);
}

pub fn set_window_title(_: &str) {}
pub fn set_resize_increments(_: f64, _: f64) {}
pub fn set_view_hidden(_: ViewHandle, _: bool) {}

pub fn remove_view(view: ViewHandle) {
    if !view.is_null() { let _ = unsafe { Box::from_raw(view as *mut WaylandPane) }; }
}

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
pub fn platform_config(pane: &WaylandPane) -> ffi::ghostty_platform_u { pane.egl_platform_config() }
