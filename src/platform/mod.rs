//! Platform abstraction layer.
//!
//! Provides platform-neutral geometry types and re-exports the active
//! platform backend (macOS or Linux) which share an identical API surface.

use std::ffi::c_void;

// --- Geometry types (replace NSRect/NSPoint/NSSize) ---

#[derive(Debug, Clone, Copy, Default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Point { x, y }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

impl Size {
    pub fn new(width: f64, height: f64) -> Self {
        Size { width, height }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Rect {
    pub origin: Point,
    pub size: Size,
}

impl Rect {
    pub fn new(origin: Point, size: Size) -> Self {
        Rect { origin, size }
    }
}

// --- Scroll event (shared type, populated by platform-specific monitors) ---

pub struct ScrollEvent {
    pub dx: f64,
    pub dy: f64,
    pub precision: bool,
    pub momentum: u8,
}

// --- Platform handle types ---

/// Opaque handle to a native view (NSView* on macOS, widget pointer on Linux).
pub type ViewHandle = *mut c_void;

/// Opaque handle to a native overlay layer (CALayer* on macOS, unused on Linux).
pub type LayerHandle = *mut c_void;

// --- Platform backends ---

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;
