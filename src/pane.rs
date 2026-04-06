#![allow(dead_code)]

use crate::ffi;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};

pub type PaneId = u64;

static NEXT_PANE_ID: AtomicU64 = AtomicU64::new(1);

/// Backend-neutral pane handle.
///
/// Boo now uses backend-owned VT panes on both macOS and Linux. `surface`
/// remains for compatibility with any native-surface backends, while `view`
/// keeps the platform host view needed for focus and layout.
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

    pub fn id(self) -> PaneId {
        self.id
    }

    pub fn surface(self) -> ffi::ghostty_surface_t {
        self.surface
    }

    pub fn view(self) -> *mut c_void {
        self.view
    }

    pub fn is_null(self) -> bool {
        self.surface.is_null()
    }
}

#[cfg(test)]
mod tests {
    use super::PaneHandle;

    #[test]
    fn detached_panes_get_unique_ids() {
        let a = PaneHandle::detached();
        let b = PaneHandle::detached();

        assert_ne!(a.id(), 0);
        assert_ne!(b.id(), 0);
        assert_ne!(a.id(), b.id());
        assert!(a.is_null());
        assert!(b.is_null());
    }

    #[test]
    fn null_pane_is_zero_and_null() {
        let pane = PaneHandle::null();

        assert_eq!(pane.id(), 0);
        assert!(pane.is_null());
        assert!(pane.view().is_null());
    }
}
