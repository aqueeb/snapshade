//! `TauriOverlayHost`: one hidden `WebviewWindow` per display, converted exactly
//! once into a non-activating `NSPanel` at screen-saver level, placed from the
//! `NSScreen` frame (RESEARCH Q4; PITFALLS Pitfall 3; ARCHITECTURE Pattern 3), with a
//! native frame layer beneath a transparent webview (Plan 01-09 Plan B).
//!
//! Rules this file enforces (plan prohibitions):
//! * windows are created hidden at launch and re-used; `hide_all` orders them out
//!   with `panel.hide()` and never closes a converted window on the hotkey path
//!   (Cap's KVO/over-release hazards, RESEARCH Q4 line 354) — the ONE exception is
//!   [`TauriOverlayHost::reprepare`] on a display-parameter change, which hands each
//!   panel back to Tauri with `to_window()` and closes it (A8, measured in 01-08);
//! * the NSPanel conversion (`to_panel`) happens once per window, inside `prepare`;
//! * placement never reads tao/Tauri monitor geometry — the `NSWindow` frame is
//!   set to the `NSScreen` frame verbatim via objc2 (tauri#7890);
//! * panels keep the default sharing type; self-exclusion is an `SCContentFilter`
//!   concern (01-10) — `NSWindowSharingNone` made the overlay invisible on DisplayLink
//!   displays, which are rendered by screen capture (01-SPIKE-RESULT open issue 1), so
//!   Snapshade is kept out of its own capture by `MacCapturer`'s `SelfExclusion::Exclude`
//!   filter instead;
//! * **Plan B frame rule** (supersedes the former "no transparent flags" rule for the
//!   overlay windows only — RESEARCH Q5 line 461, 01-SPIKE-RESULT `## Plan B`): each
//!   window is `transparent(true)` and carries a [`FrameLayer`] below its WKWebView;
//!   the ScreenCaptureKit `CGImage` becomes the layer's `contents` zero-copy in
//!   [`TauriOverlayHost::present`] BEFORE the panel is ordered front, so the webview
//!   never fetches, decodes or paints pixels on the hotkey path. `fullscreen` stays
//!   forbidden, and `tauri.conf.json` still sets no `macOSPrivateApi` key (a no-op on
//!   Tauri ≥ 2.12.1 anyway);
//! * **capture-then-show** (Pitfall 7; ROADMAP SC2): `present` runs to completion for
//!   every display before `show_all`; a display without a panel aborts the run with no
//!   panel shown;
//! * **nothing stale is ever re-displayed (REVIEW CR-01 — fixed here; T-01-23)**: the
//!   layer contents are set before `show_and_make_key` and `hide_all` clears every
//!   layer's contents and emits `overlay-hidden` to every page, which detaches its drag
//!   handler and repaints the full dim. The "previous capture flashes on the next
//!   press" symptom cannot recur: the first frame composited after a show is always
//!   the new capture under a plain dim, and a press before the page even listens still
//!   shows a correct frame because the layer does not depend on the page.
//!
//! Every method must run on the main thread (`MainThreadRequired` otherwise); the
//! controller reaches it through `AppHandle::run_on_main_thread`.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex};

use snapshade_core::geometry::DisplayId;
use snapshade_core::topology::DisplayTopology;
use snapshade_platform_api::{OverlayHost, PlatformError};
use snapshade_platform_macos::{
    raw_screens, set_window_frame, window_frame, FrameLayer, NativeFrame, RawScreen,
};
use tauri::{AppHandle, Emitter, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Wry};
use tauri_nspanel::{
    tauri_panel, CollectionBehavior, PanelHandle, PanelLevel, StyleMask, WebviewWindowExt,
};

tauri_panel! {
    panel!(OverlayPanel {
        config: {
            // Esc / ⌘Z / arrows / text input need a key window (Pitfall 8) …
            can_become_key_window: true,
            // … but a non-activating panel must never become main.
            can_become_main_window: false,
            is_floating_panel: true,
            hides_on_deactivate: false
        }
    })
}

/// Env flag: when `1`, `show_all` activates the app (`NSApplication.activate`)
/// before ordering the panels front — the RESEARCH Q4 line 415 fallback for key
/// delivery. Off by default; Plan 01-08 Task 2 step 7 measures which path
/// delivers keys (A7). This file does not decide.
pub const ACTIVATE_ENV: &str = "SNAPSHADE_ACTIVATE";

/// Managed state: `Arc<Mutex<TauriOverlayHost>>`, touched only on the main thread.
pub type SharedOverlayHost = Arc<Mutex<TauriOverlayHost>>;

struct Overlay {
    id: DisplayId,
    label: String,
    window: WebviewWindow<Wry>,
    panel: PanelHandle<Wry>,
    screen: RawScreen,
    /// The native layer that holds this display's frozen frame (Plan B).
    frame_layer: FrameLayer,
}

/// `OverlayHost` for Tauri on macOS.
pub struct TauriOverlayHost {
    app: AppHandle<Wry>,
    overlays: Vec<Overlay>,
    /// Incremented by every `reprepare`. Tauri closes windows asynchronously on the
    /// event loop, so a re-created overlay cannot reuse the label of the window it
    /// replaces within the same turn; epoch > 0 suffixes the label (`overlay-<id>-<n>`).
    epoch: u64,
}

pub use crate::overlay_label;

impl TauriOverlayHost {
    pub fn new(app: AppHandle<Wry>) -> Self {
        Self {
            app,
            overlays: Vec::new(),
            epoch: 0,
        }
    }

    /// Display ids that currently have a prepared panel.
    pub fn display_ids(&self) -> Vec<DisplayId> {
        self.overlays.iter().map(|o| o.id).collect()
    }

    /// `(display id, window label)` for every prepared panel — the controller emits
    /// `frame-ready` to these labels. Safe to call off the main thread (no AppKit).
    pub fn labels(&self) -> Vec<(DisplayId, String)> {
        self.overlays
            .iter()
            .map(|o| (o.id, o.label.clone()))
            .collect()
    }

    /// Label of the overlay whose `NSWindow` is `ns_window` (pointer identity — the key
    /// window of an `NSEvent`), or `None` when the window is not an overlay panel. Main
    /// thread only (`ns_window()`); used by the ⌘Z key-equivalent monitor (Plan 01-10).
    pub fn label_for_ns_window(&self, ns_window: *mut std::ffi::c_void) -> Option<String> {
        self.overlays
            .iter()
            .find(|o| o.window.ns_window().is_ok_and(|p| p == ns_window))
            .map(|o| o.label.clone())
    }

    /// `true` when every panel's `NSWindow` frame equals its `NSScreen` frame to
    /// within 0.5 pt (gate: mixed-DPI placement with no 1-px gaps). Main thread only.
    pub fn placement_ok(&self) -> bool {
        if objc2::MainThreadMarker::new().is_none() {
            eprintln!("placement_ok: not on the main thread");
            return false;
        }
        self.overlays.iter().all(|o| match o.window.ns_window() {
            Ok(ns_window) => {
                // SAFETY: live NSWindow pointer from Tauri, on the main thread.
                let (x, y, w, h) = unsafe { window_frame(ns_window) };
                let s = o.screen;
                let ok = (x - s.x).abs() <= 0.5
                    && (y - s.y).abs() <= 0.5
                    && (w - s.w).abs() <= 0.5
                    && (h - s.h).abs() <= 0.5;
                if !ok {
                    eprintln!(
                        "placement mismatch display {}: window ({x}, {y}, {w}, {h}) vs screen ({}, {}, {}, {})",
                        o.id, s.x, s.y, s.w, s.h
                    );
                }
                ok
            }
            Err(err) => {
                eprintln!("placement_ok: ns_window for display {}: {err}", o.id);
                false
            }
        })
    }

    fn create_overlay(&self, screen: RawScreen) -> Result<Overlay, PlatformError> {
        let label = overlay_label(screen.id, self.epoch);
        let os =
            |what: &str, err: tauri::Error| PlatformError::Os(format!("{label}: {what}: {err}"));

        let window =
            WebviewWindowBuilder::new(&self.app, &label, WebviewUrl::App("overlay.html".into()))
                .visible(false)
                .decorations(false)
                .shadow(false)
                .resizable(false)
                .focused(false)
                .accept_first_mouse(true)
                // Plan B: clear window background and a WKWebView that draws no
                // background, so the FrameLayer beneath it shows through the page's
                // 50 % dim (the CLAUDE.md supersession recorded in the 01-09 objective).
                .transparent(true)
                .inner_size(screen.w, screen.h)
                .build()
                .map_err(|e| os("build", e))?;

        // Placement: bypass tao entirely (tauri#7890) — NSWindow frame = NSScreen frame.
        let ns_window = window.ns_window().map_err(|e| os("ns_window", e))?;
        // SAFETY: live NSWindow pointer from Tauri; prepare() runs on the main thread.
        unsafe { set_window_frame(ns_window, screen.x, screen.y, screen.w, screen.h) };

        // Exactly once per window (RESEARCH Q4 line 354).
        let panel = window
            .to_panel::<OverlayPanel>()
            .map_err(|e| os("to_panel", e))?;
        panel.set_level(PanelLevel::ScreenSaver.value());
        panel
            .add_style_mask(StyleMask::empty().nonactivating_panel().value())
            .map_err(|e| PlatformError::Os(format!("{label}: style mask: {e}")))?;
        panel.set_collection_behavior(
            CollectionBehavior::new()
                .can_join_all_spaces()
                .full_screen_auxiliary()
                .stationary()
                .ignores_cycle()
                .value(),
        );
        panel.set_hides_on_deactivate(false);
        panel.set_has_shadow(false);

        // Plan B: the layer-backed view below the WKWebView that will hold the CGImage.
        let ns_window = window.ns_window().map_err(|e| os("ns_window", e))?;
        // SAFETY: same live pointer (to_panel swizzles the class in place), main thread.
        let frame_layer = unsafe { FrameLayer::install(ns_window, screen.scale) }
            .map_err(|e| PlatformError::Os(format!("{label}: {e}")))?;

        // No sharing-type change (Plan 01-10): the window stays capturable so DisplayLink
        // displays show it; Snapshade's own capture excludes this app via SCContentFilter.

        Ok(Overlay {
            id: screen.id,
            label,
            window,
            panel,
            screen,
            frame_layer,
        })
    }

    /// Install every frame on its display's native layer (zero-copy), in the order
    /// given (the controller passes display-id order), BEFORE any panel is shown.
    /// `after_each(display_id)` runs right after that display's contents are set — the
    /// controller stamps D-09 `t2` there. A frame whose display has no panel is an
    /// error (`no overlay panel for display <id>`): every layer set so far is cleared
    /// again and nothing is shown (capture-then-show, Pitfall 7; T-01-23).
    pub fn present<'a, I, F>(&self, frames: I, mut after_each: F) -> Result<(), PlatformError>
    where
        I: IntoIterator<Item = &'a NativeFrame>,
        F: FnMut(DisplayId),
    {
        objc2::MainThreadMarker::new().ok_or(PlatformError::MainThreadRequired)?;
        let result = (|| {
            for frame in frames {
                let overlay = self
                    .overlays
                    .iter()
                    .find(|o| o.id == frame.display_id)
                    .ok_or_else(|| {
                        PlatformError::Os(format!(
                            "no overlay panel for display {}",
                            frame.display_id
                        ))
                    })?;
                overlay
                    .frame_layer
                    .set_contents(frame.image(), frame.scale)
                    .map_err(|e| PlatformError::Os(format!("{}: {e}", overlay.label)))?;
                after_each(frame.display_id);
            }
            Ok(())
        })();
        if result.is_err() {
            self.clear_layers();
        }
        result
    }

    /// `FrameLayer::clear` on every panel; failures are logged, never fatal.
    fn clear_layers(&self) {
        for overlay in &self.overlays {
            if let Err(err) = overlay.frame_layer.clear() {
                eprintln!("frame layer clear failed for {}: {err}", overlay.label);
            }
        }
    }

    /// Display parameters changed (hot-plug, resolution, arrangement): hide every
    /// panel, hand each back to Tauri with `to_window()` and `close()` it, forget
    /// them, bump the label epoch and `prepare(topo)` afresh — all on the main
    /// thread. Returns the number of panels replaced.
    ///
    /// A Rust panic or an Objective-C exception while tearing down (RESEARCH Q4
    /// line 416: WebKit KVO `NSRangeException` on Cap's older branch, A8) is caught
    /// and reported as `Err` so the process survives to log it; whether 2.1.1's
    /// `to_window()` avoids it is what Plan 01-08's hot-plug test records.
    pub fn reprepare(&mut self, topo: &DisplayTopology) -> Result<usize, PlatformError> {
        objc2::MainThreadMarker::new().ok_or(PlatformError::MainThreadRequired)?;
        self.hide_all()?;
        let old = std::mem::take(&mut self.overlays);
        let replaced = old.len();
        self.epoch += 1;

        let teardown = catch_unwind(AssertUnwindSafe(|| {
            objc2::exception::catch(AssertUnwindSafe(|| {
                for overlay in old {
                    match overlay.panel.to_window() {
                        Some(window) => {
                            if let Err(err) = window.close() {
                                eprintln!("screens changed: close {}: {err}", overlay.label);
                            }
                        }
                        None => eprintln!(
                            "screens changed: to_window returned None for {}",
                            overlay.label
                        ),
                    }
                }
            }))
        }));
        match teardown {
            Ok(Ok(())) => {}
            Ok(Err(exception)) => {
                return Err(PlatformError::Os(format!(
                    "objc exception while closing the old panels: {}",
                    exception
                        .map(|e| format!("{e:?}"))
                        .unwrap_or_else(|| "<nil>".to_string())
                )));
            }
            Err(panic) => {
                let msg = panic
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "<non-string panic>".to_string());
                return Err(PlatformError::Os(format!(
                    "panic while closing the old panels: {msg}"
                )));
            }
        }

        self.prepare(topo)?;
        Ok(replaced)
    }
}

impl OverlayHost for TauriOverlayHost {
    /// Create a hidden panel for every display of `topo` that has an `NSScreen`.
    /// A display that already has a panel keeps it (its frame is re-applied);
    /// panels for displays no longer in `topo` stay hidden until `reprepare`.
    fn prepare(&mut self, topo: &DisplayTopology) -> Result<(), PlatformError> {
        objc2::MainThreadMarker::new().ok_or(PlatformError::MainThreadRequired)?;
        for screen in raw_screens()? {
            if topo.get(screen.id).is_none() {
                continue;
            }
            if let Some(existing) = self.overlays.iter_mut().find(|o| o.id == screen.id) {
                let ns_window = existing
                    .window
                    .ns_window()
                    .map_err(|e| PlatformError::Os(format!("ns_window: {e}")))?;
                // SAFETY: live NSWindow pointer from Tauri, on the main thread.
                unsafe { set_window_frame(ns_window, screen.x, screen.y, screen.w, screen.h) };
                existing.screen = screen;
                continue;
            }
            let overlay = self.create_overlay(screen)?;
            self.overlays.push(overlay);
        }
        if self.overlays.is_empty() {
            return Err(PlatformError::Os(
                "no overlay could be prepared".to_string(),
            ));
        }
        Ok(())
    }

    /// Order every panel front and make it key, in one main-thread turn, AFTER every
    /// frame has been captured and installed on its layer by [`TauriOverlayHost::present`]
    /// (capture-then-show, Pitfall 7).
    fn show_all(&mut self) -> Result<(), PlatformError> {
        let mtm = objc2::MainThreadMarker::new().ok_or(PlatformError::MainThreadRequired)?;
        if self.overlays.is_empty() {
            return Err(PlatformError::Os("show_all before prepare".to_string()));
        }
        if std::env::var(ACTIVATE_ENV)
            .map(|v| v == "1")
            .unwrap_or(false)
        {
            // RESEARCH Q4 line 415 fallback — measured, not assumed (A7). `NSApplication
            // activate` is the macOS 14+ replacement for `activateIgnoringOtherApps:` /
            // `NSApplicationActivateIgnoringOtherApps`, which the SDK marks "no effect" on
            // 14+ (RESEARCH State of the Art); PKG-03's minimum is 14, so no fallback path.
            objc2_app_kit::NSApplication::sharedApplication(mtm).activate();
            eprintln!("activate fallback requested ({ACTIVATE_ENV}=1)");
        }
        for overlay in &self.overlays {
            overlay.panel.show_and_make_key();
        }
        Ok(())
    }

    /// Order every panel out, drop every layer's frame and tell every page
    /// (`overlay-hidden`) to reset its dim and drag state (CR-01 / T-01-23: the
    /// 24–64 MB `CGImage`s are released and nothing stale can be the first frame of
    /// the next show). An emit failure is logged, never fatal — a label may be
    /// mid-close during `reprepare`. Windows are kept alive — never closed on this path.
    fn hide_all(&mut self) -> Result<(), PlatformError> {
        objc2::MainThreadMarker::new().ok_or(PlatformError::MainThreadRequired)?;
        for overlay in &self.overlays {
            overlay.panel.hide();
        }
        self.clear_layers();
        for overlay in &self.overlays {
            if let Err(err) = self
                .app
                .emit_to(overlay.label.as_str(), "overlay-hidden", ())
            {
                eprintln!("overlay-hidden emit to {} failed: {err}", overlay.label);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::overlay_label;

    #[test]
    fn labels_keep_the_plain_form_until_the_first_reprepare() {
        assert_eq!(overlay_label(1, 0), "overlay-1");
        assert_eq!(overlay_label(69733382, 0), "overlay-69733382");
        assert_eq!(overlay_label(1, 1), "overlay-1-1");
        assert_eq!(overlay_label(2, 7), "overlay-2-7");
    }
}
