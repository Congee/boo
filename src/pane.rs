#![allow(dead_code)]

use crate::ffi;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};

pub type PaneId = u64;

static NEXT_PANE_ID: AtomicU64 = AtomicU64::new(1);

/// Backend-neutral pane handle.
///
/// Today this still carries the native libghostty surface and platform view
/// used by the existing implementation. The key change is that tabs/splits
/// no longer traffic in raw `(surface, view)` tuples directly, which gives
/// Linux room to swap in a `libghostty-vt`-backed pane implementation later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneHandle {
    id: PaneId,
    surface: ffi::ghostty_surface_t,
    view: *mut c_void,
}

impl PaneHandle {
    pub fn new(surface: ffi::ghostty_surface_t, view: *mut c_void) -> Self {
        Self {
            id: NEXT_PANE_ID.fetch_add(1, Ordering::Relaxed),
            surface,
            view,
        }
    }

    pub fn detached() -> Self {
        Self {
            id: NEXT_PANE_ID.fetch_add(1, Ordering::Relaxed),
            surface: std::ptr::null_mut(),
            view: std::ptr::null_mut(),
        }
    }

    pub fn null() -> Self {
        Self {
            id: 0,
            surface: std::ptr::null_mut(),
            view: std::ptr::null_mut(),
        }
    }

    pub fn id(self) -> PaneId { self.id }

    pub fn surface(self) -> ffi::ghostty_surface_t { self.surface }

    pub fn view(self) -> *mut c_void { self.view }

    pub fn is_null(self) -> bool { self.surface.is_null() }
}
