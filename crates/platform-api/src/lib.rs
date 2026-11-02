//! Snapshade platform abstraction layer (PAL).
//!
//! One narrow trait per OS capability (ARCHITECTURE Pattern 1), reduced to what
//! Phase 1 needs: display enumeration, screen capture, and the overlay host.
//! Adapters convert OS types to [`snapshade_core::geometry`] types at the
//! boundary and never let an `NSRect` or `RECT` escape. This crate depends on
//! `snapshade-core` only; it must never depend on an OS binding or on `tauri`.

use std::path::{Path, PathBuf};

use snapshade_core::capture::{CaptureError, CaptureFrame, CaptureRequest, CaptureSet};
use snapshade_core::topology::DisplayTopology;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlatformError {
    /// The operation is not implemented on this platform (every
    /// `platform-windows` call in Phase 1).
    #[error("not supported on this platform")]
    Unsupported,
    /// AppKit work was attempted off the main thread.
    #[error("this call must run on the main thread")]
    MainThreadRequired,
    #[error("OS error: {0}")]
    Os(String),
}

/// Screen-recording permission state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    Granted,
    Denied,
    /// macOS: the user has not been asked yet.
    NotDetermined,
    /// Windows: no permission model exists for screen capture.
    AlwaysGranted,
}

/// Enumerates connected displays.
pub trait DisplayProvider {
    /// Snapshot the current topology: ids, global logical rects, scale, physical size.
    fn snapshot(&self) -> Result<DisplayTopology, PlatformError>;
}

/// Grabs every display at native pixel size.
pub trait ScreenCapturer {
    fn permission(&self) -> Permission;
    fn request_permission(&self);
    /// Capture every display in `topo`, one frame per display, concurrently.
    /// Must not touch the UI thread.
    fn capture_all(
        &self,
        topo: &DisplayTopology,
        req: &CaptureRequest,
    ) -> Result<CaptureSet, CaptureError>;
}

/// One pre-created, hidden overlay window per display (ARCHITECTURE Pattern 3).
pub trait OverlayHost {
    /// Create (or re-create after a display change) the hidden windows for `topo`.
    fn prepare(&mut self, topo: &DisplayTopology) -> Result<(), PlatformError>;
    /// Show every prepared window; must be sub-frame.
    fn show_all(&mut self) -> Result<(), PlatformError>;
    /// Hide every window; windows are kept, never destroyed.
    fn hide_all(&mut self) -> Result<(), PlatformError>;
}

/// Writes the flattened screenshot to the system clipboard in every format the
/// platform's paste targets expect (EXPORT-01: PNG + TIFF on macOS; CF_DIB + CF_DIBV5 +
/// PNG on Windows). Must complete before the overlay is dismissed (EXPORT-02).
pub trait ImageClipboard {
    fn write_image(&self, frame: &CaptureFrame) -> Result<(), PlatformError>;
}

/// Opens the native print dialog with the image scaled to fit the page (EXPORT-05).
pub trait Printer {
    fn print_image(&self, frame: &CaptureFrame) -> Result<(), PlatformError>;
}

/// OS shell integration: reveal a saved file, open a URL in the default browser /
/// settings app. The app itself has no networking; `open_url` hands off to the OS.
pub trait Shell {
    fn reveal_file(&self, path: &Path) -> Result<(), PlatformError>;
    fn open_url(&self, url: &str) -> Result<(), PlatformError>;
}

/// What a click on a notification should do (EXPORT-06 "Click to show it in the
/// folder"; HOTKEY-03 "Click this message to choose a different one in Options").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationAction {
    None,
    RevealFile(PathBuf),
    OpenOptions,
    /// Open a URL / settings deep link (PKG-05 "link to the setting that owns PrintScreen").
    OpenUrl(String),
}

/// Posts a user notification. Implementations deliver the click through the callback
/// handed to them at construction. A platform without a notification path (or a dev
/// binary that is not a bundle) logs and returns `Ok(())` — notifications are never fatal.
pub trait Notifier {
    fn notify(
        &self,
        title: &str,
        body: &str,
        action: NotificationAction,
    ) -> Result<(), PlatformError>;
}
