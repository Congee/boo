//! Linux platform backend — GBM + EGL + Wayland subsurface compositing.
//!
//! Architecture (mirrors macOS NSView + Metal):
//! 1. Each terminal pane gets a Wayland subsurface (child of iced's window)
//! 2. A GBM surface provides the EGL window surface (desktop GL 4.3)
//! 3. ghostty renders → eglSwapBuffers → GBM front buffer
//! 4. GBM buffer exported as dmabuf → wl_buffer → attached to subsurface
//! 5. Wayland compositor composites — zero-copy

use super::{LayerHandle, Rect, ScrollEvent, ViewHandle};
use crate::ffi;
use std::ffi::c_void;
use std::os::unix::io::RawFd;

// ============================================================
// Raw FFI declarations (libwayland-client, libwayland-egl, libgbm)
// ============================================================

mod wl {
    use std::ffi::c_void;
    use std::os::raw::c_int;

    unsafe extern "C" {
        // wl_proxy
        pub fn wl_proxy_get_display(proxy: *mut c_void) -> *mut c_void;
        pub fn wl_proxy_marshal_flags(
            proxy: *mut c_void, opcode: u32, interface: *const c_void,
            version: u32, flags: u32, ...
        ) -> *mut c_void;
        pub fn wl_proxy_destroy(proxy: *mut c_void);
        pub fn wl_proxy_add_listener(
            proxy: *mut c_void, implementation: *const c_void, data: *mut c_void,
        ) -> c_int;

        // wl_display
        pub fn wl_display_roundtrip(display: *mut c_void) -> c_int;
        pub fn wl_display_flush(display: *mut c_void) -> c_int;

        // Protocol interfaces
        pub static wl_registry_interface: c_void;
        pub static wl_compositor_interface: c_void;
        pub static wl_subcompositor_interface: c_void;
        pub static wl_surface_interface: c_void;
        pub static wl_subsurface_interface: c_void;
        pub static wl_buffer_interface: c_void;
    }
}

mod gbm {
    use std::ffi::c_void;
    use std::os::raw::c_int;

    pub const GBM_FORMAT_ARGB8888: u32 = 0x34325241; // DRM_FORMAT_ARGB8888
    pub const GBM_BO_USE_RENDERING: u32 = 1 << 2;
    pub const GBM_BO_USE_LINEAR: u32 = 1 << 4;

    unsafe extern "C" {
        pub fn gbm_create_device(fd: c_int) -> *mut c_void;
        pub fn gbm_device_destroy(gbm: *mut c_void);
        pub fn gbm_surface_create(
            gbm: *mut c_void, width: u32, height: u32, format: u32, flags: u32,
        ) -> *mut c_void;
        pub fn gbm_surface_destroy(surface: *mut c_void);
        pub fn gbm_surface_lock_front_buffer(surface: *mut c_void) -> *mut c_void;
        pub fn gbm_surface_release_buffer(surface: *mut c_void, bo: *mut c_void);
        pub fn gbm_bo_get_fd(bo: *mut c_void) -> c_int;
        pub fn gbm_bo_get_stride(bo: *mut c_void) -> u32;
        pub fn gbm_bo_get_width(bo: *mut c_void) -> u32;
        pub fn gbm_bo_get_height(bo: *mut c_void) -> u32;
    }
}

// ============================================================
// Wayland registry binding
// ============================================================

struct RegistryState {
    compositor: *mut c_void,
    subcompositor: *mut c_void,
    dmabuf: *mut c_void,
}

type RegistryGlobalFn = unsafe extern "C" fn(*mut c_void, *mut c_void, u32, *const i8, u32);
type RegistryGlobalRemoveFn = unsafe extern "C" fn(*mut c_void, *mut c_void, u32);

#[repr(C)]
struct WlRegistryListener {
    global: RegistryGlobalFn,
    global_remove: RegistryGlobalRemoveFn,
}
unsafe impl Sync for WlRegistryListener {}

static REGISTRY_LISTENER: WlRegistryListener = WlRegistryListener {
    global: registry_global,
    global_remove: registry_global_remove,
};

// zwp_linux_dmabuf_v1 interface — not in libwayland-client, declare manually
#[repr(C)]
struct WlInterface {
    name: *const i8,
    version: i32,
    method_count: i32,
    methods: *const c_void,
    event_count: i32,
    events: *const c_void,
}
unsafe impl Sync for WlInterface {}

static ZWP_LINUX_DMABUF_V1_INTERFACE: WlInterface = WlInterface {
    name: b"zwp_linux_dmabuf_v1\0".as_ptr() as *const i8,
    version: 3,
    method_count: 0,
    methods: std::ptr::null(),
    event_count: 0,
    events: std::ptr::null(),
};

static ZWP_LINUX_BUFFER_PARAMS_V1_INTERFACE: WlInterface = WlInterface {
    name: b"zwp_linux_buffer_params_v1\0".as_ptr() as *const i8,
    version: 3,
    method_count: 0,
    methods: std::ptr::null(),
    event_count: 0,
    events: std::ptr::null(),
};

unsafe extern "C" fn registry_global(
    data: *mut c_void, registry: *mut c_void, name: u32,
    interface: *const i8, version: u32,
) {
    unsafe {
        let state = &mut *(data as *mut RegistryState);
        let iface = std::ffi::CStr::from_ptr(interface);
        let iface_bytes = iface.to_bytes();

        if iface_bytes == b"wl_compositor" && state.compositor.is_null() {
            state.compositor = wl::wl_proxy_marshal_flags(
                registry, 0, // WL_REGISTRY_BIND
                &wl::wl_compositor_interface as *const _ as *const c_void,
                version.min(6), 0,
                name,
                &wl::wl_compositor_interface as *const _ as *const c_void,
                version.min(6),
                std::ptr::null_mut::<c_void>(),
            );
        } else if iface_bytes == b"wl_subcompositor" && state.subcompositor.is_null() {
            state.subcompositor = wl::wl_proxy_marshal_flags(
                registry, 0,
                &wl::wl_subcompositor_interface as *const _ as *const c_void,
                version.min(1), 0,
                name,
                &wl::wl_subcompositor_interface as *const _ as *const c_void,
                version.min(1),
                std::ptr::null_mut::<c_void>(),
            );
        } else if iface_bytes == b"zwp_linux_dmabuf_v1" && state.dmabuf.is_null() {
            state.dmabuf = wl::wl_proxy_marshal_flags(
                registry, 0,
                &ZWP_LINUX_DMABUF_V1_INTERFACE as *const _ as *const c_void,
                version.min(3), 0,
                name,
                &ZWP_LINUX_DMABUF_V1_INTERFACE as *const _ as *const c_void,
                version.min(3),
                std::ptr::null_mut::<c_void>(),
            );
        }
    }
}

unsafe extern "C" fn registry_global_remove(
    _data: *mut c_void, _registry: *mut c_void, _name: u32,
) {}

// ============================================================
// WaylandDisplay — shared state across all panes
// ============================================================

pub struct WaylandDisplay {
    wl_display: *mut c_void,
    parent_surface: *mut c_void,
    compositor: *mut c_void,
    subcompositor: *mut c_void,
    dmabuf: *mut c_void,
    gbm_device: *mut c_void,
    egl_display: khronos_egl::Display,
    egl_config: khronos_egl::Config,
    egl_lib: khronos_egl::DynamicInstance<khronos_egl::EGL1_4>,
}

impl WaylandDisplay {
    pub fn new(parent_surface_ptr: *mut c_void) -> Option<Self> {
        if parent_surface_ptr.is_null() {
            return None;
        }

        let wl_display = unsafe { wl::wl_proxy_get_display(parent_surface_ptr) };
        if wl_display.is_null() {
            log::error!("wl_proxy_get_display returned null");
            return None;
        }

        // Bind compositor, subcompositor, and linux-dmabuf
        let mut state = RegistryState {
            compositor: std::ptr::null_mut(),
            subcompositor: std::ptr::null_mut(),
            dmabuf: std::ptr::null_mut(),
        };
        unsafe {
            let registry = wl::wl_proxy_marshal_flags(
                wl_display, 1, // WL_DISPLAY_GET_REGISTRY
                &wl::wl_registry_interface as *const _ as *const c_void,
                1, 0, std::ptr::null_mut::<c_void>(),
            );
            wl::wl_proxy_add_listener(
                registry,
                &REGISTRY_LISTENER as *const WlRegistryListener as *const c_void,
                &mut state as *mut RegistryState as *mut c_void,
            );
            wl::wl_display_roundtrip(wl_display);
            wl::wl_proxy_destroy(registry);
        }

        if state.compositor.is_null() || state.subcompositor.is_null() {
            log::error!("Missing Wayland globals: compositor={} subcompositor={}",
                !state.compositor.is_null(), !state.subcompositor.is_null());
            return None;
        }

        // Open DRM render node for GBM
        let drm_fd = Self::open_render_node()?;

        // Create GBM device
        let gbm_device = unsafe { gbm::gbm_create_device(drm_fd) };
        if gbm_device.is_null() {
            log::error!("gbm_create_device failed");
            return None;
        }

        // Create EGL display from GBM device (gives us desktop GL 4.3!)
        let egl_lib = unsafe {
            khronos_egl::DynamicInstance::<khronos_egl::EGL1_4>::load_required()
        }.ok()?;

        let egl_display = Self::create_gbm_egl_display(&egl_lib, gbm_device)?;
        egl_lib.initialize(egl_display)
            .map_err(|e| log::error!("EGL initialize failed: {e}"))
            .ok()?;

        egl_lib.bind_api(khronos_egl::OPENGL_API)
            .map_err(|e| log::error!("EGL bind_api(OPENGL_API) failed: {e}"))
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
            .or_else(|| { log::error!("EGL: no config with OPENGL_BIT + WINDOW_BIT on GBM"); None })?;

        log::info!("GBM + EGL display initialized (desktop GL available)");

        Some(WaylandDisplay {
            wl_display,
            parent_surface: parent_surface_ptr,
            compositor: state.compositor,
            subcompositor: state.subcompositor,
            dmabuf: state.dmabuf,
            gbm_device,
            egl_display,
            egl_config,
            egl_lib,
        })
    }

    fn open_render_node() -> Option<RawFd> {
        for i in 128..136 {
            let path = format!("/dev/dri/renderD{i}");
            match std::fs::OpenOptions::new().read(true).write(true).open(&path) {
                Ok(file) => {
                    use std::os::unix::io::IntoRawFd;
                    log::info!("Opened DRM render node: {path}");
                    return Some(file.into_raw_fd());
                }
                Err(_) => continue,
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

        type GetPlatformDisplayFn = unsafe extern "C" fn(
            khronos_egl::Enum, *mut c_void, *const khronos_egl::Attrib,
        ) -> *mut c_void;

        let get_platform_display: GetPlatformDisplayFn = unsafe {
            std::mem::transmute(lib.get_proc_address("eglGetPlatformDisplay")?)
        };

        let raw = unsafe {
            get_platform_display(EGL_PLATFORM_GBM_KHR, gbm_device, std::ptr::null())
        };
        if raw.is_null() {
            log::error!("eglGetPlatformDisplay(GBM) returned null");
            return None;
        }
        Some(unsafe { khronos_egl::Display::from_ptr(raw) })
    }

    /// Create a terminal pane: subsurface + GBM surface + EGL context.
    pub fn create_pane(&self, frame: Rect, scale: f64) -> Option<Box<WaylandPane>> {
        let w = ((frame.size.width * scale) as u32).max(1);
        let h = ((frame.size.height * scale) as u32).max(1);

        unsafe {
            // Create child wl_surface
            let child_surface = wl::wl_proxy_marshal_flags(
                self.compositor, 0, // WL_COMPOSITOR_CREATE_SURFACE
                &wl::wl_surface_interface as *const _ as *const c_void,
                6, 0, std::ptr::null_mut::<c_void>(),
            );
            if child_surface.is_null() {
                log::error!("wl_compositor.create_surface failed");
                return None;
            }

            // Create subsurface
            let subsurface = wl::wl_proxy_marshal_flags(
                self.subcompositor, 1, // WL_SUBCOMPOSITOR_GET_SUBSURFACE
                &wl::wl_subsurface_interface as *const _ as *const c_void,
                1, 0, std::ptr::null_mut::<c_void>(),
                child_surface, self.parent_surface,
            );
            if subsurface.is_null() {
                log::error!("wl_subcompositor.get_subsurface failed");
                wl::wl_proxy_destroy(child_surface);
                return None;
            }

            // Set desync mode + position
            wl::wl_proxy_marshal_flags(subsurface, 4, std::ptr::null(), 1, 0); // set_desync
            wl::wl_proxy_marshal_flags(subsurface, 1, std::ptr::null(), 1, 0, // set_position
                frame.origin.x as i32, frame.origin.y as i32);

            // Create GBM surface (the native EGL window)
            let gbm_surface = gbm::gbm_surface_create(
                self.gbm_device, w, h,
                gbm::GBM_FORMAT_ARGB8888,
                gbm::GBM_BO_USE_RENDERING,
            );
            if gbm_surface.is_null() {
                log::error!("gbm_surface_create failed");
                wl::wl_proxy_destroy(subsurface);
                wl::wl_proxy_destroy(child_surface);
                return None;
            }

            // Create EGL window surface from GBM surface
            let egl_surface = self.egl_lib
                .create_window_surface(self.egl_display, self.egl_config, gbm_surface, None)
                .map_err(|e| log::error!("eglCreateWindowSurface(GBM) failed: {e}"))
                .ok()?;

            // Create EGL context (OpenGL 4.3 core)
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

            // Make current so ghostty can init GL
            self.egl_lib.make_current(
                self.egl_display,
                Some(egl_surface), Some(egl_surface), Some(egl_context),
            ).ok()?;

            log::info!("Wayland pane created: {w}x{h} at ({}, {})",
                frame.origin.x as i32, frame.origin.y as i32);

            Some(Box::new(WaylandPane {
                child_surface,
                subsurface,
                gbm_surface,
                egl_surface: egl_surface.as_ptr() as *mut c_void,
                egl_context: egl_context.as_ptr() as *mut c_void,
                egl_display: self.egl_display.as_ptr() as *mut c_void,
                wl_display: self.wl_display,
                dmabuf: self.dmabuf,
                prev_bo: std::ptr::null_mut(),
                width: w,
                height: h,
            }))
        }
    }

    pub fn release_current(&self) {
        let _ = self.egl_lib.make_current(self.egl_display, None, None, None);
    }
}

// ============================================================
// WaylandPane — per-pane state
// ============================================================

pub struct WaylandPane {
    child_surface: *mut c_void,
    subsurface: *mut c_void,
    gbm_surface: *mut c_void,
    egl_surface: *mut c_void,
    egl_context: *mut c_void,
    egl_display: *mut c_void,
    wl_display: *mut c_void,
    dmabuf: *mut c_void,
    prev_bo: *mut c_void,
    width: u32,
    height: u32,
}

impl WaylandPane {
    pub fn set_frame(&mut self, frame: Rect, _scale: f64) {
        unsafe {
            wl::wl_proxy_marshal_flags(
                self.subsurface, 1, std::ptr::null(), 1, 0, // set_position
                frame.origin.x as i32, frame.origin.y as i32,
            );
        }
        // TODO: handle resize (need new GBM surface + EGL surface)
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
        unsafe {
            if !self.prev_bo.is_null() {
                gbm::gbm_surface_release_buffer(self.gbm_surface, self.prev_bo);
            }
            gbm::gbm_surface_destroy(self.gbm_surface);
            wl::wl_proxy_destroy(self.subsurface);
            wl::wl_proxy_destroy(self.child_surface);
        }
    }
}

// ============================================================
// Platform API (matches macos.rs surface)
// ============================================================

pub fn scale_factor() -> f64 { 1.0 }

#[allow(dead_code)]
pub fn content_view_handle() -> ViewHandle { std::ptr::null_mut() }

pub fn set_window_transparent() {}

#[allow(dead_code)]
pub fn create_child_view(_parent_handle: ViewHandle, _frame: Rect) -> ViewHandle {
    std::ptr::null_mut()
}

pub fn view_bounds(_view: ViewHandle) -> Rect { Rect::default() }

pub fn set_view_frame(view: ViewHandle, frame: Rect) {
    if view.is_null() { return; }
    let pane = unsafe { &mut *(view as *mut WaylandPane) };
    pane.set_frame(frame, 1.0);
}

pub fn set_window_title(_title: &str) {}
pub fn set_resize_increments(_width: f64, _height: f64) {}
pub fn set_view_hidden(_view: ViewHandle, _hidden: bool) {}

pub fn remove_view(view: ViewHandle) {
    if !view.is_null() {
        let _ = unsafe { Box::from_raw(view as *mut WaylandPane) };
    }
}

pub fn set_view_layer_transparent(_view: ViewHandle) {}
pub fn request_redraw() {}

// --- Event monitors ---

pub fn install_event_monitors(_scroll_tx: std::sync::mpsc::Sender<ScrollEvent>) {}

// --- Overlay layers (no-ops) ---

pub fn create_scrollbar_layer() -> LayerHandle { std::ptr::null_mut() }
pub fn update_scrollbar_layer(_: LayerHandle, _: f64, _: f64, _: f64, _: f64, _: f32) {}
pub fn create_highlight_layer() -> LayerHandle { std::ptr::null_mut() }
pub fn update_highlight_layer(_: LayerHandle, _: f64, _: f64, _: f64, _: f64, _: bool, _: bool) {}

// --- Clipboard (arboard) ---

pub fn clipboard_read() -> Option<String> {
    arboard::Clipboard::new().ok().and_then(|mut cb| cb.get_text().ok())
}

pub fn clipboard_write(text: &str) {
    if let Ok(mut cb) = arboard::Clipboard::new() { let _ = cb.set_text(text.to_owned()); }
}

pub fn clipboard_write_from_thread(text: String) {
    if let Ok(mut cb) = arboard::Clipboard::new() { let _ = cb.set_text(text); }
}

// --- Platform config ---

pub fn platform_tag() -> i32 {
    ffi::ghostty_platform_e::GHOSTTY_PLATFORM_EGL as i32
}

pub fn platform_config(pane: &WaylandPane) -> ffi::ghostty_platform_u {
    pane.egl_platform_config()
}
