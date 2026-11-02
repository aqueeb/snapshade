//! Snapshade Windows platform adapters (Phase 5).
//!
//! The crate compiles on every host (D-18, D-21): on Windows the modules below implement
//! the `snapshade-platform-api` traits with Win32 / WinRT; elsewhere the same public types
//! exist and every call answers `Unsupported`, which is what the `windows-latest` CI job's
//! `cargo check --workspace` and this Mac's `scripts/check-windows-core.sh` rely on.
//!
//! | Trait | Windows implementation |
//! |---|---|
//! | `DisplayProvider` | `EnumDisplayMonitors` + `GetMonitorInfoW` (physical rects, primary flag) + `GetDpiForMonitor(MDT_EFFECTIVE_DPI)` → scale; logical rects are derived (`physical / scale`). The process must be Per-Monitor-V2 DPI aware before the first call — Tauri's generated application manifest declares it. Ids are 1-based enumeration indices. |
//! | `ScreenCapturer` | DXGI desktop duplication per monitor (`dxgi.rs`, Plan 999.4-02: the compositor's current image copied into a staging texture and read back; states primed at startup, `WAIT_TIMEOUT` reuses the last image, `ACCESS_LOST` recreates), with GDI `BitBlt` of the monitor's physical rect into a top-down 32-bit DIB (`CAPTUREBLT`) as the per-monitor fallback; BGRA → RGBA either way; the cursor is composited with `DrawIconEx` when requested. `SNAPSHADE_CAPTURE=gdi` forces the fallback. No permission model → `AlwaysGranted`. |
//! | `ImageClipboard` | `CF_DIB` (24-bit, opaque) + `CF_DIBV5` + registered `PNG`, in one `OpenClipboard` / `EmptyClipboard` / `SetClipboardData` sequence (PITFALLS Pitfall 6). |
//! | `Shell` | `explorer.exe /select,<file>` and `ShellExecuteW("open", url)`. |
//! | `Notifier` | WinRT `ToastNotification` under the app's AUMID with an `Activated` handler. |
//! | `Printer` | `Unsupported` here — the app prints through a hidden WebView2 page (`window.print()`). |
//!
//! The overlay host (per-monitor `WebviewWindow`s) lives in `src-tauri/src/overlay_win.rs`
//! because it is shaped by Tauri, exactly like the macOS host.

#[cfg(target_os = "windows")]
mod capture;
#[cfg(target_os = "windows")]
mod clipboard;
#[cfg(target_os = "windows")]
mod display;
#[cfg(target_os = "windows")]
mod dxgi;
#[cfg(target_os = "windows")]
mod notify;
mod pixels;
#[cfg(target_os = "windows")]
mod shell;
#[cfg(target_os = "windows")]
mod snipping;
#[cfg(target_os = "windows")]
pub mod window_fx;

#[cfg(target_os = "windows")]
pub use notify::ClickHandler;
#[cfg(target_os = "windows")]
pub use snipping::{print_screen_owned_by_snipping_tool, SNIPPING_TOOL_SETTINGS_URL};

use std::path::Path;

use snapshade_core::capture::{CaptureError, CaptureFrame, CaptureRequest, CaptureSet};
use snapshade_core::topology::DisplayTopology;
use snapshade_platform_api::{
    DisplayProvider, ImageClipboard, NotificationAction, Notifier, Permission, PlatformError,
    Printer, ScreenCapturer, Shell,
};

/// `EnumDisplayMonitors` + `GetDpiForMonitor` under PerMonitorV2.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WindowsDisplayProvider;

/// DXGI desktop duplication per monitor, GDI `BitBlt` as the fallback (cursor via
/// `DrawIconEx`). Holds the per-monitor duplication states, so one instance lives for the
/// whole process (`WinCaptureState` in the app).
#[derive(Debug, Default)]
pub struct WindowsCapturer {
    #[cfg(target_os = "windows")]
    dxgi: dxgi::Duplicator,
}

impl WindowsCapturer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create the duplication state for every monitor of `topo` and acquire its first frame
    /// (startup, background thread) so the first hotkey press pays neither. No-op off Windows.
    pub fn prime(&self, topo: &DisplayTopology) {
        #[cfg(target_os = "windows")]
        {
            capture::prime(&self.dxgi, topo);
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = topo;
        }
    }

    /// The next capture must not reuse a cached desktop image and must be a frame the
    /// compositor presented after this call (the overlay was just concealed for a
    /// press-while-up capture; the cache and any earlier frame still show it) — or GDI after
    /// 100 ms. No-op off Windows.
    pub fn invalidate(&self) {
        #[cfg(target_os = "windows")]
        {
            self.dxgi.invalidate_all();
        }
    }
}

/// `CF_DIB` + `CF_DIBV5` + `PNG` clipboard writer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WindowsClipboard;

/// Explorer reveal + `ShellExecuteW` URL opener.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WindowsShell;

/// Printing is done by the app's hidden WebView2 print page; this adapter only reports it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WindowsPrinter;

/// WinRT toast notifications (click → callback).
pub struct WindowsNotifier {
    #[cfg(target_os = "windows")]
    inner: notify::ToastNotifier,
}

impl std::fmt::Debug for WindowsNotifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WindowsNotifier")
    }
}

impl DisplayProvider for WindowsDisplayProvider {
    fn snapshot(&self) -> Result<DisplayTopology, PlatformError> {
        #[cfg(target_os = "windows")]
        {
            display::snapshot()
        }
        #[cfg(not(target_os = "windows"))]
        {
            Err(PlatformError::Unsupported)
        }
    }
}

impl ScreenCapturer for WindowsCapturer {
    fn permission(&self) -> Permission {
        Permission::AlwaysGranted
    }

    fn request_permission(&self) {}

    fn capture_all(
        &self,
        topo: &DisplayTopology,
        req: &CaptureRequest,
    ) -> Result<CaptureSet, CaptureError> {
        #[cfg(target_os = "windows")]
        {
            capture::capture_all(&self.dxgi, topo, req)
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = (topo, req);
            Err(CaptureError::Unsupported)
        }
    }
}

impl ImageClipboard for WindowsClipboard {
    fn write_image(&self, frame: &CaptureFrame) -> Result<(), PlatformError> {
        #[cfg(target_os = "windows")]
        {
            clipboard::write_image(frame)
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = frame;
            Err(PlatformError::Unsupported)
        }
    }
}

impl Shell for WindowsShell {
    fn reveal_file(&self, path: &Path) -> Result<(), PlatformError> {
        #[cfg(target_os = "windows")]
        {
            shell::reveal_file(path)
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = path;
            Err(PlatformError::Unsupported)
        }
    }

    fn open_url(&self, url: &str) -> Result<(), PlatformError> {
        #[cfg(target_os = "windows")]
        {
            shell::open_url(url)
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = url;
            Err(PlatformError::Unsupported)
        }
    }
}

impl Printer for WindowsPrinter {
    /// Always `Unsupported`: the app routes print through its WebView2 page.
    fn print_image(&self, _frame: &CaptureFrame) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported)
    }
}

impl WindowsNotifier {
    /// `aumid` is the Application User Model ID the installer's Start Menu shortcut carries
    /// (Tauri uses the bundle identifier); toasts from an unpackaged app need it.
    #[cfg(target_os = "windows")]
    pub fn new(aumid: &str, on_click: ClickHandler) -> Self {
        Self {
            inner: notify::ToastNotifier::new(aumid, on_click),
        }
    }

    #[cfg(not(target_os = "windows"))]
    pub fn new(_aumid: &str) -> Self {
        Self {}
    }
}

impl Notifier for WindowsNotifier {
    fn notify(
        &self,
        title: &str,
        body: &str,
        action: NotificationAction,
    ) -> Result<(), PlatformError> {
        #[cfg(target_os = "windows")]
        {
            self.inner.notify(title, body, action)
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = (title, body, action);
            Err(PlatformError::Unsupported)
        }
    }
}
