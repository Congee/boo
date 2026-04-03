//! Linux platform backend — Wayland subsurface compositing + EGL rendering.
//!
//! Mirrors the macOS architecture: each terminal pane gets a native compositor
//! surface (wl_subsurface on Wayland, child window on X11) positioned over the
//! iced window. Ghostty renders into an EGL window surface backed by that
//! compositor surface. The display server composites it — zero-copy.

use super::{LayerHandle, Rect, ScrollEvent, ViewHandle};
use crate::ffi;
use std::ffi::c_void;

// --- Wayland display state (shared across all panes) ---

/// Shared Wayland/EGL state initialized once from iced's window handles.
pub struct WaylandDisplay {
    /// The wl_display pointer (borrowed from winit — NOT owned by us).
    wl_display: *mut c_void,
    /// The parent wl_surface (iced's window surface, NOT owned by us).
    parent_surface: *mut c_void,
    /// Our wl_compositor binding.
    compositor: *mut c_void,
    /// Our wl_subcompositor binding.
    subcompositor: *mut c_void,
    /// EGL display, config, and library handle.
    egl_display: khronos_egl::Display,
    egl_config: khronos_egl::Config,
    egl_lib: khronos_egl::DynamicInstance<khronos_egl::EGL1_4>,
}

/// Per-pane state: Wayland subsurface + EGL rendering context.
pub struct WaylandPane {
    /// Child wl_surface (owned).
    child_surface: *mut c_void,
    /// wl_subsurface of the parent (owned).
    subsurface: *mut c_void,
    /// Native EGL window handle from libwayland-egl (owned).
    egl_window: *mut c_void,
    /// EGL surface for rendering (owned).
    egl_surface: *mut c_void,
    /// EGL context (owned).
    egl_context: *mut c_void,
    /// Shared EGL display (borrowed from WaylandDisplay).
    egl_display: *mut c_void,
    /// Current size in pixels.
    width: i32,
    height: i32,
}

// --- Raw Wayland FFI (minimal, via wayland-sys) ---

mod wl {
    use std::ffi::c_void;
    use std::os::raw::c_int;

    // These are dlopen'd from libwayland-client.so via wayland-sys
    unsafe extern "C" {
        // wl_proxy functions
        pub fn wl_proxy_get_display(proxy: *mut c_void) -> *mut c_void;
        pub fn wl_proxy_marshal_flags(
            proxy: *mut c_void,
            opcode: u32,
            interface: *const c_void,
            version: u32,
            flags: u32,
            ...
        ) -> *mut c_void;
        pub fn wl_proxy_destroy(proxy: *mut c_void);
        pub fn wl_proxy_add_listener(
            proxy: *mut c_void,
            implementation: *const c_void,
            data: *mut c_void,
        ) -> c_int;

        // wl_display functions
        pub fn wl_display_roundtrip(display: *mut c_void) -> c_int;

        // Protocol interfaces (extern symbols from libwayland-client)
        pub static wl_registry_interface: c_void;
        pub static wl_compositor_interface: c_void;
        pub static wl_subcompositor_interface: c_void;
        pub static wl_surface_interface: c_void;
        pub static wl_subsurface_interface: c_void;
    }

    // wl_egl_window functions (from libwayland-egl.so)
    unsafe extern "C" {
        pub fn wl_egl_window_create(
            surface: *mut c_void,
            width: c_int,
            height: c_int,
        ) -> *mut c_void;
        pub fn wl_egl_window_destroy(egl_window: *mut c_void);
        pub fn wl_egl_window_resize(
            egl_window: *mut c_void,
            width: c_int,
            height: c_int,
            dx: c_int,
            dy: c_int,
        );
    }

    // wl_registry opcodes
    pub const WL_REGISTRY_BIND: u32 = 0;

    // wl_compositor opcodes
    pub const WL_COMPOSITOR_CREATE_SURFACE: u32 = 0;

    // wl_subcompositor opcodes
    pub const WL_SUBCOMPOSITOR_GET_SUBSURFACE: u32 = 1;

    // wl_subsurface opcodes
    pub const WL_SUBSURFACE_SET_POSITION: u32 = 1;
    pub const WL_SUBSURFACE_SET_DESYNC: u32 = 4;

    // wl_surface opcodes
    pub const WL_SURFACE_COMMIT: u32 = 6;
}

/// State passed to the wl_registry listener.
struct RegistryState {
    compositor: *mut c_void,
    subcompositor: *mut c_void,
}

/// wl_registry::global event handler.
unsafe extern "C" fn registry_global(
    data: *mut c_void,
    registry: *mut c_void,
    name: u32,
    interface: *const std::os::raw::c_char,
    version: u32,
) {
    unsafe {
        let state = &mut *(data as *mut RegistryState);
        let iface = std::ffi::CStr::from_ptr(interface);

        if iface.to_bytes() == b"wl_compositor" && state.compositor.is_null() {
            state.compositor = wl::wl_proxy_marshal_flags(
                registry,
                wl::WL_REGISTRY_BIND,
                &wl::wl_compositor_interface as *const _ as *const c_void,
                version.min(6),
                0,
                name,
                &wl::wl_compositor_interface as *const _ as *const c_void,
                version.min(6),
                std::ptr::null_mut::<c_void>(),
            );
        } else if iface.to_bytes() == b"wl_subcompositor" && state.subcompositor.is_null() {
            state.subcompositor = wl::wl_proxy_marshal_flags(
                registry,
                wl::WL_REGISTRY_BIND,
                &wl::wl_subcompositor_interface as *const _ as *const c_void,
                version.min(1),
                0,
                name,
                &wl::wl_subcompositor_interface as *const _ as *const c_void,
                version.min(1),
                std::ptr::null_mut::<c_void>(),
            );
        }
    }
}

/// wl_registry::global_remove event handler (no-op).
unsafe extern "C" fn registry_global_remove(
    _data: *mut c_void,
    _registry: *mut c_void,
    _name: u32,
) {
}

/// Registry listener function type.
type RegistryGlobalFn = unsafe extern "C" fn(*mut c_void, *mut c_void, u32, *const std::os::raw::c_char, u32);
type RegistryGlobalRemoveFn = unsafe extern "C" fn(*mut c_void, *mut c_void, u32);

/// Registry listener vtable.
#[repr(C)]
struct WlRegistryListener {
    global: RegistryGlobalFn,
    global_remove: RegistryGlobalRemoveFn,
}
// SAFETY: function pointers are inherently thread-safe
unsafe impl Sync for WlRegistryListener {}

static REGISTRY_LISTENER: WlRegistryListener = WlRegistryListener {
    global: registry_global,
    global_remove: registry_global_remove,
};

impl WaylandDisplay {
    /// Initialize from iced's raw window handles.
    /// `wl_surface_ptr` is the parent surface (from RawWindowHandle::Wayland).
    /// We derive the wl_display from it via wl_proxy_get_display.
    pub fn new(parent_surface_ptr: *mut c_void) -> Option<Self> {
        if parent_surface_ptr.is_null() {
            return None;
        }

        let wl_display = unsafe { wl::wl_proxy_get_display(parent_surface_ptr) };
        log::info!("wl_proxy_get_display: surface={:?} -> display={:?}", parent_surface_ptr, wl_display);
        if wl_display.is_null() {
            log::error!("wl_proxy_get_display returned null");
            return None;
        }

        // Bind compositor and subcompositor via registry
        let mut state = RegistryState {
            compositor: std::ptr::null_mut(),
            subcompositor: std::ptr::null_mut(),
        };

        unsafe {
            // wl_display.get_registry() — opcode 1
            let registry = wl::wl_proxy_marshal_flags(
                wl_display,
                1, // WL_DISPLAY_GET_REGISTRY
                &wl::wl_registry_interface as *const _ as *const c_void,
                1, // version
                0,
                std::ptr::null_mut::<c_void>(),
            );
            if registry.is_null() {
                log::error!("wl_display_get_registry failed");
                return None;
            }
            wl::wl_proxy_add_listener(
                registry,
                &REGISTRY_LISTENER as *const WlRegistryListener as *const c_void,
                &mut state as *mut RegistryState as *mut c_void,
            );
            wl::wl_display_roundtrip(wl_display);
            wl::wl_proxy_destroy(registry);
        }

        log::info!("registry roundtrip done: compositor={:?} subcompositor={:?}",
            state.compositor, state.subcompositor);
        if state.compositor.is_null() || state.subcompositor.is_null() {
            log::error!(
                "Wayland globals missing: compositor={} subcompositor={}",
                !state.compositor.is_null(),
                !state.subcompositor.is_null()
            );
            return None;
        }

        // Create EGL display from the Wayland display
        let egl_lib =
            unsafe { khronos_egl::DynamicInstance::<khronos_egl::EGL1_4>::load_required() }
                .ok()?;

        let egl_display = Self::create_egl_display(&egl_lib, wl_display)
            .or_else(|| { log::error!("EGL: create_egl_display failed"); None })?;
        // eglInitialize may fail if wgpu already initialized this display.
        // That's OK — the display is already usable.
        if let Err(e) = egl_lib.initialize(egl_display) {
            log::info!("EGL: initialize returned {e} (display may already be initialized by wgpu)");
        }

        egl_lib
            .bind_api(khronos_egl::OPENGL_API)
            .map_err(|e| log::error!("EGL bind_api(OPENGL_API) failed: {e}"))
            .ok()?;

        let attribs = [
            khronos_egl::RENDERABLE_TYPE,
            khronos_egl::OPENGL_BIT,
            khronos_egl::SURFACE_TYPE,
            khronos_egl::WINDOW_BIT,
            khronos_egl::RED_SIZE, 8,
            khronos_egl::GREEN_SIZE, 8,
            khronos_egl::BLUE_SIZE, 8,
            khronos_egl::ALPHA_SIZE, 8,
            khronos_egl::NONE,
        ];
        let egl_config = egl_lib
            .choose_first_config(egl_display, &attribs)
            .ok()
            .and_then(|c| c)
            .or_else(|| {
                log::error!("EGL: no suitable config with OPENGL_BIT + WINDOW_BIT");
                None
            })?;

        log::info!("Wayland display initialized (compositor + subcompositor + EGL)");

        Some(WaylandDisplay {
            wl_display,
            parent_surface: parent_surface_ptr,
            compositor: state.compositor,
            subcompositor: state.subcompositor,
            egl_display,
            egl_config,
            egl_lib,
        })
    }

    fn create_egl_display(
        lib: &khronos_egl::DynamicInstance<khronos_egl::EGL1_4>,
        wl_display: *mut c_void,
    ) -> Option<khronos_egl::Display> {
        const EGL_PLATFORM_WAYLAND_KHR: khronos_egl::Enum = 0x31D8;

        type GetPlatformDisplayFn = unsafe extern "C" fn(
            khronos_egl::Enum,
            *mut c_void,
            *const khronos_egl::Attrib,
        ) -> *mut c_void;

        let get_platform_display: GetPlatformDisplayFn = unsafe {
            std::mem::transmute(lib.get_proc_address("eglGetPlatformDisplay")?)
        };

        let raw = unsafe {
            get_platform_display(EGL_PLATFORM_WAYLAND_KHR, wl_display, std::ptr::null())
        };
        if raw.is_null() {
            log::error!("eglGetPlatformDisplay(WAYLAND) returned null");
            return None;
        }
        Some(unsafe { khronos_egl::Display::from_ptr(raw) })
    }

    /// Create a new pane: subsurface + EGL window surface + GL context.
    pub fn create_pane(&self, frame: Rect, scale: f64) -> Option<Box<WaylandPane>> {
        let w = (frame.size.width * scale) as i32;
        let h = (frame.size.height * scale) as i32;
        if w <= 0 || h <= 0 {
            return None;
        }

        unsafe {
            // Create child wl_surface
            let child_surface = wl::wl_proxy_marshal_flags(
                self.compositor,
                wl::WL_COMPOSITOR_CREATE_SURFACE,
                &wl::wl_surface_interface as *const _ as *const c_void,
                6, // version
                0,
                std::ptr::null_mut::<c_void>(),
            );
            if child_surface.is_null() {
                log::error!("wl_compositor.create_surface failed");
                return None;
            }

            // Create subsurface
            let subsurface = wl::wl_proxy_marshal_flags(
                self.subcompositor,
                wl::WL_SUBCOMPOSITOR_GET_SUBSURFACE,
                &wl::wl_subsurface_interface as *const _ as *const c_void,
                1, // version
                0,
                std::ptr::null_mut::<c_void>(),
                child_surface,
                self.parent_surface,
            );
            if subsurface.is_null() {
                log::error!("wl_subcompositor.get_subsurface failed");
                wl::wl_proxy_destroy(child_surface);
                return None;
            }

            // Set desync mode so the renderer thread can present independently
            wl::wl_proxy_marshal_flags(
                subsurface,
                wl::WL_SUBSURFACE_SET_DESYNC,
                std::ptr::null(),
                1,
                0,
            );

            // Position the subsurface
            wl::wl_proxy_marshal_flags(
                subsurface,
                wl::WL_SUBSURFACE_SET_POSITION,
                std::ptr::null(),
                1,
                0,
                frame.origin.x as i32,
                frame.origin.y as i32,
            );

            // Create wl_egl_window
            let egl_window = wl::wl_egl_window_create(child_surface, w, h);
            if egl_window.is_null() {
                log::error!("wl_egl_window_create failed");
                wl::wl_proxy_destroy(subsurface);
                wl::wl_proxy_destroy(child_surface);
                return None;
            }

            // Create EGL surface
            let egl_surface = self
                .egl_lib
                .create_window_surface(
                    self.egl_display,
                    self.egl_config,
                    egl_window,
                    None,
                )
                .map_err(|e| log::error!("eglCreateWindowSurface failed: {e}"))
                .ok()?;

            // Create EGL context (OpenGL 4.3 core)
            let ctx_attribs = [
                khronos_egl::CONTEXT_MAJOR_VERSION, 4,
                khronos_egl::CONTEXT_MINOR_VERSION, 3,
                khronos_egl::CONTEXT_OPENGL_PROFILE_MASK,
                khronos_egl::CONTEXT_OPENGL_CORE_PROFILE_BIT,
                khronos_egl::NONE,
            ];
            let egl_context = self
                .egl_lib
                .create_context(self.egl_display, self.egl_config, None, &ctx_attribs)
                .map_err(|e| log::error!("eglCreateContext failed: {e}"))
                .ok()?;

            // Make current so ghostty can init GL in surfaceInit
            self.egl_lib
                .make_current(
                    self.egl_display,
                    Some(egl_surface),
                    Some(egl_surface),
                    Some(egl_context),
                )
                .ok()?;

            // Commit the parent surface to make the subsurface visible
            wl::wl_proxy_marshal_flags(
                self.parent_surface,
                wl::WL_SURFACE_COMMIT,
                std::ptr::null(),
                6,
                0,
            );
            wl::wl_display_roundtrip(self.wl_display);

            log::info!("Wayland pane created: {w}x{h} at ({}, {})", frame.origin.x as i32, frame.origin.y as i32);

            Some(Box::new(WaylandPane {
                child_surface,
                subsurface,
                egl_window,
                egl_surface: egl_surface.as_ptr() as *mut c_void,
                egl_context: egl_context.as_ptr() as *mut c_void,
                egl_display: self.egl_display.as_ptr() as *mut c_void,
                width: w,
                height: h,
            }))
        }
    }

    /// Release EGL context from the current thread.
    pub fn release_current(&self) {
        let _ = self.egl_lib.make_current(self.egl_display, None, None, None);
    }
}

impl WaylandPane {
    /// Reposition and resize this pane.
    pub fn set_frame(&mut self, frame: Rect, scale: f64) {
        let x = frame.origin.x as i32;
        let y = frame.origin.y as i32;
        let w = (frame.size.width * scale) as i32;
        let h = (frame.size.height * scale) as i32;

        unsafe {
            wl::wl_proxy_marshal_flags(
                self.subsurface,
                wl::WL_SUBSURFACE_SET_POSITION,
                std::ptr::null(),
                1,
                0,
                x,
                y,
            );

            if w != self.width || h != self.height {
                wl::wl_egl_window_resize(self.egl_window, w, h, 0, 0);
                self.width = w;
                self.height = h;
            }
        }
    }

    /// Get the EGL state for ghostty surface creation.
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
            wl::wl_egl_window_destroy(self.egl_window);
            wl::wl_proxy_destroy(self.subsurface);
            wl::wl_proxy_destroy(self.child_surface);
        }
    }
}

// --- Platform API (matches macos.rs surface) ---

pub fn scale_factor() -> f64 {
    1.0
}

pub fn content_view_handle() -> ViewHandle {
    // Placeholder — actual initialization happens via WaylandDisplay::new()
    // after iced provides the raw window handle.
    std::ptr::null_mut()
}

pub fn set_window_transparent() {}

pub fn create_child_view(_parent_handle: ViewHandle, _frame: Rect) -> ViewHandle {
    // Not used on Linux — panes are created via WaylandDisplay::create_pane()
    std::ptr::null_mut()
}

pub fn view_bounds(_view: ViewHandle) -> Rect {
    Rect::default()
}

pub fn set_view_frame(view: ViewHandle, frame: Rect) {
    if view.is_null() {
        return;
    }
    let pane = unsafe { &mut *(view as *mut WaylandPane) };
    // Use scale 1.0 here — the caller already provides pixel coords
    pane.set_frame(frame, 1.0);
}

pub fn set_window_title(_title: &str) {}
pub fn set_resize_increments(_width: f64, _height: f64) {}

pub fn set_view_hidden(_view: ViewHandle, _hidden: bool) {
    // TODO: attach/detach subsurface or use zero-size
}

pub fn remove_view(view: ViewHandle) {
    if !view.is_null() {
        let _ = unsafe { Box::from_raw(view as *mut WaylandPane) };
    }
}

pub fn set_view_layer_transparent(_view: ViewHandle) {}
pub fn request_redraw() {}

// --- Event monitors ---

pub fn install_event_monitors(_scroll_tx: std::sync::mpsc::Sender<ScrollEvent>) {}

// --- Overlay layers (no-ops — will be iced widgets in future) ---

pub fn create_scrollbar_layer() -> LayerHandle { std::ptr::null_mut() }
pub fn update_scrollbar_layer(_: LayerHandle, _: f64, _: f64, _: f64, _: f64, _: f32) {}
pub fn create_highlight_layer() -> LayerHandle { std::ptr::null_mut() }
pub fn update_highlight_layer(_: LayerHandle, _: f64, _: f64, _: f64, _: f64, _: bool, _: bool) {}

// --- Clipboard (arboard) ---

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

// --- Platform config ---

pub fn platform_tag() -> i32 {
    ffi::ghostty_platform_e::GHOSTTY_PLATFORM_EGL as i32
}

pub fn platform_config(pane: &WaylandPane) -> ffi::ghostty_platform_u {
    pane.egl_platform_config()
}
