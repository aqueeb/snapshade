#![cfg(target_os = "macos")]
//! Snapshade macOS platform adapters.
//!
//! Implements the `snapshade-platform-api` traits with AppKit (`NSScreen`,
//! `NSWindow` frame helpers), ScreenCaptureKit (`SCScreenshotManager`), and
//! CoreGraphics (the Screen Recording permission probe). The whole crate is
//! `cfg(target_os = "macos")` so it compiles to nothing elsewhere (D-18, D-21).
//!
//! Boundary rules (ARCHITECTURE Pattern 1/2): OS types never escape this crate —
//! `NSRect`s are flipped once into `snapshade_core::geometry` space in
//! [`display`], and the capture path hands back `CaptureFrame`s only. Every
//! AppKit call demands the main thread (`MainThreadMarker`); the ScreenCaptureKit
//! capture runs on worker threads and must never touch the main thread.
//!
//! Plan B (Plan 01-09): the hotkey path returns [`NativeFrame`]s (retained `CGImage`s,
//! no RGBA copy) that [`FrameLayer`] installs as `CALayer.contents` beneath each
//! overlay's transparent webview; RGBA conversion is lazy (`NativeFrame::to_capture_frame`).
//! Self-exclusion (Plan 01-10): every hotkey capture excludes Snapshade's own application
//! through the `SCContentFilter` ([`SelfExclusion`]); panels keep the default sharing type.
//!
//! The overlay host itself (`OverlayHost` impl) lives in `src-tauri/src/overlay.rs`
//! because it is shaped by Tauri's `WebviewWindow`; this crate only provides the
//! raw-pointer `NSWindow` helpers it needs ([`window`]).

pub mod activity;
pub mod bitmap;
pub mod capture;
pub mod clipboard;
pub mod display;
pub mod frame_layer;
pub mod notify;
pub mod permission;
pub mod print;
pub mod window;
pub mod workspace;

pub use activity::{begin_latency_critical_activity, ActivityToken};
pub use capture::{MacCapturer, NativeFrame, SelfExclusion, SELF_EXCLUSION_UNAVAILABLE};
pub use clipboard::MacClipboard;
pub use display::{
    observe_screen_changes, raw_screens, MacDisplayProvider, RawScreen, ScreenObserver,
};
pub use frame_layer::FrameLayer;
pub use notify::{running_from_bundle, ClickHandler, MacNotifier};
pub use permission::{preflight, request};
pub use print::MacPrinter;
pub use window::{set_window_frame, window_frame};
pub use workspace::{activate_app, MacShell, SCREEN_RECORDING_SETTINGS_URL};
