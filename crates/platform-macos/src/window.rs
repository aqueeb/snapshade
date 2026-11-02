//! Raw `NSWindow` helpers for the overlay host (RESEARCH Q4 lines 376-380).
//!
//! Tauri hands out the native window as `*mut c_void` (`WebviewWindow::ns_window`).
//! These helpers are the only place that pointer is dereferenced; the caller
//! (`src-tauri/src/overlay.rs`) owns the lifetime and the main-thread guarantee.

use std::ffi::c_void;

use objc2_app_kit::NSWindow;
use objc2_foundation::{NSPoint, NSRect, NSSize};

/// Set the window frame directly from an `NSScreen` frame (points, AppKit y-up),
/// bypassing tao's monitor geometry entirely (tauri#7890 mixed-DPI bug).
///
/// # Safety
/// `ns_window` must be a live `NSWindow` pointer obtained from
/// `tauri::WebviewWindow::ns_window()`, and the caller must be on the main thread.
pub unsafe fn set_window_frame(ns_window: *mut c_void, x: f64, y: f64, w: f64, h: f64) {
    // SAFETY: upheld by the caller contract above.
    let window = unsafe { &*(ns_window as *const NSWindow) };
    window.setFrame_display(NSRect::new(NSPoint::new(x, y), NSSize::new(w, h)), false);
}

/// Read the window frame back as `(x, y, w, h)` in points, AppKit y-up — used by
/// `placement_ok` to compare against the `NSScreen` frame.
///
/// # Safety
/// Same contract as [`set_window_frame`].
pub unsafe fn window_frame(ns_window: *mut c_void) -> (f64, f64, f64, f64) {
    // SAFETY: upheld by the caller contract above.
    let window = unsafe { &*(ns_window as *const NSWindow) };
    let frame = window.frame();
    (
        frame.origin.x,
        frame.origin.y,
        frame.size.width,
        frame.size.height,
    )
}

// There is deliberately no sharing-type helper here (Plan 01-10): overlay panels keep the
// default `NSWindow.sharingType`, because DisplayLink renders its virtual displays by
// capturing them and a sharing-excluded window never appears on one (01-SPIKE-RESULT open
// issue 1). Keeping Snapshade out of its own capture is `capture.rs`'s `SelfExclusion`
// (an `SCContentFilter` concern), not a window property.
