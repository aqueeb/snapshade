//! `FrameLayer`: a layer-backed `NSView` inserted beneath the WKWebView of an overlay
//! panel whose `CALayer.contents` is the ScreenCaptureKit `CGImage` of that display
//! (Plan 01-09 Plan B; RESEARCH Q5 line 461; 01-SPIKE-RESULT `## Plan B`).
//!
//! Why: the webview handoff (`frame://` fetch → decode → `putImageData`) was the
//! largest latency term on three displays (release median 115 ms, `t4 − t2`). Setting
//! a `CGImageRef` as a layer's `contents` is zero-copy — Core Animation retains the
//! image and composites it directly — so the frozen frame is on screen the moment the
//! panel is ordered front. The webview above it is transparent and pre-dimmed; it
//! never paints pixels on the hotkey path.
//!
//! Geometry: the layer fills the window's content view (autoresizing width + height)
//! and `contentsScale` equals the display's `backingScaleFactor`, so an image whose
//! pixel size is the layer's point size × scale is composited 1:1 with no resampling
//! (`contentsGravity = resize` only matters if the sizes ever disagree).
//!
//! Threading: like [`crate::display::ScreenObserver`], the wrapped handle is `Send +
//! Sync` so Tauri managed state can hold it, but it is only ever dereferenced on the
//! main thread — every method checks `MainThreadMarker::new()` and returns
//! `PlatformError::MainThreadRequired` otherwise. Every `contents` change runs inside
//! `CATransaction::begin()` / `setDisableActions(true)` / `commit()` so Core
//! Animation's implicit 0.25 s crossfade never plays (a fade would blend the new
//! frame with the previous one — CR-01's symptom by another route).

use std::ffi::c_void;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSAutoresizingMaskOptions, NSView, NSWindow, NSWindowOrderingMode};
use objc2_quartz_core::{kCAGravityResize, CALayer, CATransaction};
use screencapturekit::screenshot_manager::CGImage;
use snapshade_platform_api::PlatformError;

/// The native frame layer of one overlay panel.
pub struct FrameLayer {
    /// The layer-backed view inserted below the WKWebView; its `layer()` holds the frame.
    view: Retained<NSView>,
}

// SAFETY: the view is only ever messaged on the main thread — every method below
// demands `MainThreadMarker::new()` first and fails otherwise — so holding the handle
// in `Send + Sync` managed state cannot cause a cross-thread AppKit call. Dropping the
// handle off the main thread only releases a reference the content view also holds.
unsafe impl Send for FrameLayer {}
unsafe impl Sync for FrameLayer {}

impl std::fmt::Debug for FrameLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FrameLayer(CALayer-backed NSView below the webview)")
    }
}

fn os(what: &str) -> PlatformError {
    PlatformError::Os(format!("frame layer: {what}"))
}

impl FrameLayer {
    /// Install a frame layer in `ns_window`: a layer-backed `NSView` the size of the
    /// content view, autoresizing with it, positioned BELOW the first existing subview
    /// (the WKWebView Tauri added), with an opaque layer at `contentsScale = scale`.
    ///
    /// # Safety
    /// `ns_window` must be a live `NSWindow` pointer obtained from
    /// `tauri::WebviewWindow::ns_window()` (the same contract as
    /// [`crate::window::set_window_frame`]), and the caller must be on the main thread
    /// (checked; `Err(MainThreadRequired)` otherwise).
    pub unsafe fn install(ns_window: *mut c_void, scale: f64) -> Result<FrameLayer, PlatformError> {
        let mtm = MainThreadMarker::new().ok_or(PlatformError::MainThreadRequired)?;
        // SAFETY: upheld by the caller contract above.
        let window = unsafe { &*(ns_window as *const NSWindow) };
        let content = window
            .contentView()
            .ok_or_else(|| os("window has no contentView"))?;

        let view = NSView::initWithFrame(mtm.alloc(), content.bounds());
        view.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        view.setWantsLayer(true);
        let layer = view
            .layer()
            .ok_or_else(|| os("setWantsLayer(true) produced no layer"))?;
        layer.setOpaque(true);
        // SAFETY: reading an extern static string constant QuartzCore defines.
        layer.setContentsGravity(unsafe { kCAGravityResize });
        layer.setContentsScale(scale);

        // Below the WKWebView (Tauri's subview of the content view); if no subview exists
        // yet the view simply becomes the first one and the webview lands above it.
        let first = content.subviews().firstObject();
        content.addSubview_positioned_relativeTo(
            &view,
            NSWindowOrderingMode::Below,
            first.as_deref(),
        );

        Ok(FrameLayer { view })
    }

    /// The backing layer, on the main thread only.
    fn layer(&self) -> Result<Retained<CALayer>, PlatformError> {
        MainThreadMarker::new().ok_or(PlatformError::MainThreadRequired)?;
        self.view
            .layer()
            .ok_or_else(|| os("backing view lost its layer"))
    }

    /// Make `image` the layer's contents (zero-copy: Core Animation retains the
    /// `CGImageRef`) at `scale` pixels per point, with implicit animations disabled.
    ///
    /// The image's pixel size should equal the layer's point size × `scale` (the
    /// capture is taken at the display's native pixel size, the layer fills a panel
    /// whose frame is the `NSScreen` frame) so no resampling happens. `scale` is
    /// re-applied on every call because the backing scale can change across a
    /// hot-plug / arrangement change.
    pub fn set_contents(&self, image: &CGImage, scale: f64) -> Result<(), PlatformError> {
        let layer = self.layer()?;
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        layer.setContentsScale(scale);
        // SAFETY: a `CGImageRef` is toll-free bridged to an Objective-C object, which is
        // exactly what `CALayer.contents` expects; the layer retains it, and `image`
        // stays alive for the duration of the call.
        unsafe { layer.setContents(Some(&*(image.as_ptr() as *const AnyObject))) };
        CATransaction::commit();
        Ok(())
    }

    /// Drop the layer's contents (Esc / hide — T-01-23: a dismissed frame is released
    /// and can never be the first thing composited on the next show).
    pub fn clear(&self) -> Result<(), PlatformError> {
        let layer = self.layer()?;
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        // SAFETY: `None` is a valid `contents` value (nil).
        unsafe { layer.setContents(None) };
        CATransaction::commit();
        Ok(())
    }
}
