//! `trigger`: the hotkey path (RESEARCH Architecture diagram; Pitfall 1 and 7), Plan B
//! ordering (Plan 01-09; RESEARCH Q5 line 461; 01-SPIKE-RESULT `## Plan B`):
//!
//! preflight → (request + log + stop) | `capture_native` on a worker thread (retained
//! `CGImage` per display, NO RGBA conversion) → `t1` → `FrameStore::replace(gen + 1)`
//! with lazy [`FrameSource`]s → ONE main-thread closure: superseded-generation guard →
//! `host.present` (each `CGImage` onto its panel's `FrameLayer`, `t2[id]` stamped as
//! "frame on the layer") → `show_all` (`t_show`) → `frame-ready` to each page (metadata
//! only, AFTER show). The page never fetches pixels on this path; it is pre-dimmed and
//! reports `overlay_painted` (`t4`) after a double rAF, so the ordering
//! `t1 ≤ t2 ≤ t_show ≤ max t4` holds by construction (CAPTURE-02 ordering).
//!
//! D-09 point 3 is re-defined by this plan: `t2[id]` means the display's frame is
//! installed on its native layer (not "served over `frame://`"); point 4 is unchanged.
//! `FrameReady.fmt` and the latency row's `fmt` are `"native"`. `FrameReady.token` is the
//! generation's `frame://` capability token (Plan 01-10, T-01-28) and
//! `FrameReady.fetch_after_paint` is `true` only for self-test runs (lazy-fetch proof).
//!
//! The overlay is never shown without permission, with zero displays, or after any
//! per-display capture error (`aborted: <reason>`), and no latency row with `total_ms`
//! is produced in those cases (CAPTURE-02 empty). A capture whose generation has been
//! superseded by a newer one before its main-thread turn is skipped (`superseded: gen
//! <g> by <newer>`) so an older frame can never overwrite a newer one (REVIEW IN-11).
//!
//! The scenario tag is re-read on EVERY press (`latency::scenario_tag`) so the human
//! switches scenarios by editing `scenario.txt` without relaunching (D-10); a
//! self-test trigger is always tagged `selftest` with `selftest: true` (T-01-19).

use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Instant;

use snapshade_core::capture::{CaptureFrame, CaptureKind, CaptureRequest};
use snapshade_core::geometry::{DisplayId, PhysicalRect};
use snapshade_core::topology::DisplayTopology;
use snapshade_platform_api::{OverlayHost, ScreenCapturer};
use snapshade_platform_macos::{preflight, request, MacCapturer, NativeFrame, SelfExclusion};
use tauri::{AppHandle, Emitter, Manager, Wry};

pub use crate::payloads::NATIVE_FMT;
use crate::payloads::{epoch_us, frame_token, FrameReady, ToolPrefs};
use crate::{aux_windows, export, settings_store};

use crate::frame_protocol::{FrameMeta, FrameSource, SharedFrameStore};
use crate::latency::{
    ms_since, report, scenario_tag, DisplayRow, Reporter, RunContext, SharedTimings,
    SELFTEST_SCENARIO,
};
use crate::overlay::SharedOverlayHost;

/// The stderr line printed when Screen Recording is not granted (Pitfall 1).
pub const PERMISSION_MISSING: &str =
    "permission missing — grant Screen Recording, then fully quit and relaunch";

/// A captured display held for the `frame://` responder: the retained `CGImage` plus
/// an RGBA conversion that happens at most once, on the first `rgba()` call (the
/// responder thread), never on the hotkey path. A conversion error is returned, not
/// cached, so a later request may retry (T-01-25).
pub struct StoredNativeFrame {
    native: NativeFrame,
    rgba: OnceLock<Arc<CaptureFrame>>,
}

impl StoredNativeFrame {
    pub fn new(native: NativeFrame) -> Self {
        Self {
            native,
            rgba: OnceLock::new(),
        }
    }

    /// The frame as captured (for `present`).
    pub fn native(&self) -> &NativeFrame {
        &self.native
    }
}

impl FrameSource for StoredNativeFrame {
    fn meta(&self) -> FrameMeta {
        FrameMeta {
            display_id: self.native.display_id,
            width: self.native.width,
            height: self.native.height,
            scale: self.native.scale,
        }
    }

    fn rgba(&self) -> Result<Arc<CaptureFrame>, String> {
        if let Some(frame) = self.rgba.get() {
            return Ok(Arc::clone(frame));
        }
        // Two concurrent first requests may both convert; `get_or_init` keeps exactly
        // one result and the other copy is dropped — benign, and never on the hotkey path.
        let frame = self.native.to_capture_frame().map_err(|e| e.to_string())?;
        Ok(Arc::clone(self.rgba.get_or_init(|| Arc::new(frame))))
    }
}

/// Managed state: the current topology snapshot (taken on the main thread at launch
/// and again on every display-parameter change) and the capturer with its
/// prefetched shareable content.
pub struct CaptureState {
    pub topology: RwLock<DisplayTopology>,
    pub capturer: MacCapturer,
    /// SELECT-08: the last committed selection (display, rect), pre-selected on the next
    /// capture when "Remember the selected area" is on.
    last_selection: Mutex<Option<(DisplayId, PhysicalRect)>>,
}

impl CaptureState {
    pub fn new(topology: DisplayTopology, capturer: MacCapturer) -> Self {
        Self {
            topology: RwLock::new(topology),
            capturer,
            last_selection: Mutex::new(None),
        }
    }

    pub fn remember_selection(&self, display_id: DisplayId, rect: PhysicalRect) {
        *self
            .last_selection
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Some((display_id, rect));
    }

    pub fn last_selection(&self) -> Option<(DisplayId, PhysicalRect)> {
        *self
            .last_selection
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    /// A clone of the current topology (short read lock).
    pub fn topology(&self) -> DisplayTopology {
        self.topology
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Replace the topology after a display-parameter change.
    pub fn set_topology(&self, topology: DisplayTopology) {
        *self.topology.write().unwrap_or_else(|p| p.into_inner()) = topology;
    }
}

/// Automation hook (01-SPIKE-RESULT open issue 15): when set to `<ms>`, every shown
/// overlay is dismissed that many milliseconds after `frame-ready` without a key event.
pub const AUTO_DISMISS_ENV: &str = "SNAPSHADE_AUTO_DISMISS_MS";

/// Who pressed the (virtual) hotkey — decides the scenario tag and `selftest` flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerSource {
    /// A real ⌘⇧9 press: tagged by `SNAPSHADE_SCENARIO` / `scenario.txt` / `untagged`.
    Hotkey,
    /// The `SNAPSHADE_SELFTEST=<N>` loop: tagged `selftest`, `selftest: true`.
    SelfTest,
}

/// What `trigger` decided synchronously.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// Permission missing: `request()` was called, nothing else happens.
    PermissionMissing,
    /// The capture thread was started; the outcome arrives via the `Reporter`.
    Started,
}

/// Run the hotkey path once. Safe to call from the global-shortcut handler
/// thread; all AppKit work goes through `run_on_main_thread`.
pub fn trigger(app: AppHandle<Wry>, t0: Instant, source: TriggerSource) -> Trigger {
    let t0_epoch_us = epoch_us();
    // Read the tag at press time, before anything else, so the row describes the
    // scenario the human had selected when they pressed.
    let scenario = match source {
        TriggerSource::Hotkey => scenario_tag(),
        TriggerSource::SelfTest => SELFTEST_SCENARIO.to_string(),
    };
    eprintln!("t0 {t0_epoch_us} scenario={scenario}");

    if !preflight() {
        // First run: macOS shows the Screen Recording prompt; the human grants it on
        // this runner-signed binary and relaunches (PKG-02). Never show the overlay —
        // show the guidance window instead (CAPTURE-03).
        request();
        eprintln!("{PERMISSION_MISSING}");
        aux_windows::show_later(&app, aux_windows::PERMISSION);
        report(
            &app.state::<Reporter>(),
            Err(PERMISSION_MISSING.to_string()),
        );
        return Trigger::PermissionMissing;
    }

    let context = RunContext {
        scenario,
        selftest: source == TriggerSource::SelfTest,
        fmt: NATIVE_FMT.to_string(),
        displays: Vec::new(), // filled from the topology on the capture thread
    };
    std::thread::spawn(move || run_capture(app, t0, t0_epoch_us, context));
    Trigger::Started
}

fn run_capture(app: AppHandle<Wry>, t0: Instant, t0_epoch_us: u128, mut context: RunContext) {
    let reporter = app.state::<Reporter>().inner().clone();
    let abort = |reason: String| {
        eprintln!("aborted: {reason}");
        report(&reporter, Err(format!("aborted: {reason}")));
    };

    let state = app.state::<Arc<CaptureState>>();
    context.fmt = NATIVE_FMT.to_string();
    let topology = state.topology();
    context.displays = DisplayRow::from_topology(&topology);
    let settings = settings_store::current(&app);
    let capture_request = CaptureRequest {
        kind: CaptureKind::Interactive,
        include_cursor: settings.capture_cursor,
    };
    let initial_selection = if settings.keep_selection_position {
        state.last_selection()
    } else {
        None
    };
    let tool = ToolPrefs::from_settings(&settings);
    // Plan B: retained CGImages only — no RGBA conversion inside t1. SC2: Snapshade's own
    // windows are excluded by application (fail-closed: `aborted: … self-exclusion
    // unavailable` if the app is not in SCShareableContent).
    let frames =
        match state
            .capturer
            .capture_native(&topology, &capture_request, SelfExclusion::Exclude)
        {
            Ok(frames) => frames,
            Err(err) => return abort(err.to_string()),
        };
    let t1_ms = ms_since(t0);
    let fetch_after_paint = context.selftest;

    // The store and the main-thread closure share the same Arcs: the store serves
    // pixels lazily over frame://, the closure installs the CGImages on the layers.
    let stored: Vec<Arc<StoredNativeFrame>> = frames
        .into_iter()
        .map(|f| Arc::new(StoredNativeFrame::new(f)))
        .collect();
    let (generation, token) = {
        let store = app.state::<SharedFrameStore>();
        let mut store = store.lock().unwrap_or_else(|p| p.into_inner());
        let generation = store.generation() + 1;
        let token = frame_token(generation, epoch_us(), stored.len());
        let sources: Vec<Arc<dyn FrameSource>> = stored
            .iter()
            .map(|s| Arc::clone(s) as Arc<dyn FrameSource>)
            .collect();
        store.replace(generation, token.clone(), sources);
        (generation, token)
    };
    let ready: Vec<FrameReady> = stored
        .iter()
        .map(|s| {
            let f = s.native();
            FrameReady {
                gen: generation,
                display_id: f.display_id,
                width: f.width,
                height: f.height,
                scale: f.scale,
                fmt: NATIVE_FMT.to_string(),
                token: token.clone(),
                fetch_after_paint,
                initial_selection: initial_selection
                    .filter(|(id, _)| *id == f.display_id)
                    .map(|(_, rect)| rect),
                tool: tool.clone(),
            }
        })
        .collect();
    app.state::<SharedTimings>()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .begin(
            generation,
            t0,
            t0_epoch_us,
            t1_ms,
            ready.iter().map(|r| r.display_id),
            context,
        );

    // One main-thread turn: guard → present (t2 per display) → show (t_show) → frame-ready.
    let handle = app.clone();
    let reporter_main = reporter.clone();
    let shown = app.run_on_main_thread(move || {
        let abort = |reason: String| {
            eprintln!("aborted: {reason}");
            report(&reporter_main, Err(format!("aborted: {reason}")));
        };

        // IN-11: a newer capture already replaced this generation — never let the
        // older frames reach the layers.
        let newest = handle
            .state::<SharedFrameStore>()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .generation();
        if newest > generation {
            eprintln!("superseded: gen {generation} by {newest}");
            return;
        }

        let host = handle.state::<SharedOverlayHost>();
        let mut host = host.lock().unwrap_or_else(|p| p.into_inner());

        // (b) frame on the layer, t2 stamped right after each set.
        let timings = handle.state::<SharedTimings>().inner().clone();
        let presented = host.present(stored.iter().map(|s| s.native()), |display_id| {
            timings
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .record_served(generation, display_id, epoch_us());
        });
        if let Err(err) = presented {
            return abort(err.to_string());
        }

        // (c) capture-then-show: the panels are ordered front only now.
        if let Err(err) = host.show_all() {
            return abort(format!("show_all: {err}"));
        }
        let placement_ok = host.placement_ok();
        timings
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .record_shown(generation, placement_ok);

        // (d) metadata to each page AFTER show, so the page's double-rAF (t4) cannot
        // precede t_show. Labels carry a suffix after a re-prepare.
        let labels = host.labels();
        for payload in ready {
            let Some((_, label)) = labels.iter().find(|(id, _)| *id == payload.display_id) else {
                // present() already proved every display has a panel; defensive only.
                if let Err(err) = host.hide_all() {
                    eprintln!("hide_all after missing label: {err}");
                }
                return abort(format!(
                    "no overlay panel for display {}",
                    payload.display_id
                ));
            };
            if let Err(err) = handle.emit_to(label.as_str(), "frame-ready", payload) {
                if let Err(err) = host.hide_all() {
                    eprintln!("hide_all after emit failure: {err}");
                }
                return abort(format!("emit frame-ready to {label}: {err}"));
            }
        }
    });
    if let Err(err) = shown {
        abort(format!("run_on_main_thread: {err}"));
        return;
    }
    if let Some(ms) = std::env::var(AUTO_DISMISS_ENV)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
    {
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(ms));
            eprintln!("auto-dismiss after {ms} ms ({AUTO_DISMISS_ENV})");
            if let Err(err) = crate::commands::overlay_cancel(app) {
                eprintln!("auto-dismiss: {err}");
            }
        });
    }
}

/// HOTKEY-04 "Instant full-screen save": capture every display (eager RGBA, cursor
/// per settings) and save each to the default folder; no overlay is shown. Worker thread.
pub fn instant_save(app: AppHandle<Wry>) {
    if !preflight() {
        request();
        eprintln!("{PERMISSION_MISSING}");
        aux_windows::show_later(&app, aux_windows::PERMISSION);
        return;
    }
    std::thread::spawn(move || {
        let state = app.state::<Arc<CaptureState>>();
        let settings = settings_store::current(&app);
        let req = CaptureRequest {
            kind: CaptureKind::InstantFullscreen,
            include_cursor: settings.capture_cursor,
        };
        let set = match state.capturer.capture_all(&state.topology(), &req) {
            Ok(set) => set,
            Err(err) => {
                eprintln!("instant save: capture failed: {err}");
                return;
            }
        };
        match export::save_fullscreen(&app, &set.frames) {
            Ok(paths) => eprintln!("instant save: wrote {} file(s)", paths.len()),
            Err(err) => eprintln!("instant save: {err}"),
        }
    });
}
