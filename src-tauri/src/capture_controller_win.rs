//! The Windows hotkey path (Phase 5 port of `capture_controller.rs`): DXGI capture of every
//! monitor on a worker thread (GDI per monitor as the fallback, Plan 999.4-02) → `t1` →
//! each frame's RGBA copied into its overlay window's
//! WebView2 shared buffer (Plan 999.4-01, `t2`; parallel, ~5 ms per 4K display) and the
//! frames kept as eager sources in the `FrameStore` → one main-thread turn: `show_all` →
//! `frame-ready` to each page. `fmt: "shared"` means "paint from your buffer" (no transfer);
//! a display without a buffer gets `fmt: "fetch"` and the page fetches the frame over
//! `frame://` as in 999.3. A size mismatch (monitor arrangement changed since launch)
//! re-snapshots the topology, re-prepares the windows and retries once.
//!
//! Instrumentation mirrors the macOS controller: `t0` (hotkey receipt) → `t1` (frames
//! captured) → `t2` (pixels in the page's buffer) → `t_show` (windows shown, placement
//! checked) → `t4` (page painted) land in the same `latency {…}` stderr line / JSONL row,
//! `fmt` = `shared` | `fetch` | `mixed`, and `SNAPSHADE_AUTO_DISMISS_MS` dismisses the
//! overlay for unattended runs.

use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use snapshade_core::capture::{CaptureError, CaptureFrame, CaptureKind, CaptureRequest};
use snapshade_core::geometry::{DisplayId, PhysicalRect};
use snapshade_core::topology::DisplayTopology;
use snapshade_platform_api::{DisplayProvider, OverlayHost, ScreenCapturer};
use snapshade_platform_windows::{WindowsCapturer, WindowsDisplayProvider};
use tauri::{AppHandle, Emitter, Manager, Wry};

use crate::frame_protocol::{FrameSource, SharedFrameStore};
use crate::latency::{ms_since, scenario_tag, DisplayRow, RunContext, SharedTimings};
use crate::overlay_win::SharedOverlayHost;
use crate::payloads::{epoch_us, frame_token, FrameReady, ToolPrefs, FETCH_FMT, SHARED_FMT};
use crate::shared_frame_win::SharedFrameBuffer;
use crate::{export, settings_store};

/// One display's handoff outcome: whether its pixels are in the page's shared buffer and
/// when they got there (epoch µs; the row's `t2`).
struct Handoff {
    display_id: DisplayId,
    shared: bool,
    written_us: u128,
}

/// Copy every frame into its display's shared buffer, one thread per display (a 33 MB
/// memcpy each). A display without a buffer, or whose buffer is not the frame's size, is
/// reported `shared: false` and takes the fetch path.
fn hand_off(
    frames: &[Arc<CaptureFrame>],
    buffers: &[(DisplayId, Arc<SharedFrameBuffer>)],
) -> Vec<Handoff> {
    std::thread::scope(|scope| {
        let handles: Vec<_> = frames
            .iter()
            .map(|frame| {
                let buffer = buffers
                    .iter()
                    .find(|(id, _)| *id == frame.display_id)
                    .map(|(_, b)| Arc::clone(b));
                scope.spawn(move || {
                    let shared = buffer.is_some_and(|b| b.write(&frame.rgba));
                    Handoff {
                        display_id: frame.display_id,
                        shared,
                        written_us: epoch_us(),
                    }
                })
            })
            .collect();
        handles
            .into_iter()
            .zip(frames)
            .map(|(handle, frame)| {
                handle.join().unwrap_or(Handoff {
                    display_id: frame.display_id,
                    shared: false,
                    written_us: 0,
                })
            })
            .collect()
    })
}

pub struct WinCaptureState {
    pub topology: RwLock<DisplayTopology>,
    pub capturer: WindowsCapturer,
    last_selection: Mutex<Option<(DisplayId, PhysicalRect)>>,
    /// The `frame-ready` payloads of the newest capture, replayed to a page that
    /// reports ready after the event went out (`overlay_ready`).
    pending: Mutex<Vec<FrameReady>>,
}

/// How long the watchdog waits for the first page to paint before hiding the overlays.
const PAINT_WATCHDOG: std::time::Duration = std::time::Duration::from_secs(10);

impl WinCaptureState {
    pub fn new(topology: DisplayTopology) -> Self {
        Self {
            topology: RwLock::new(topology),
            capturer: WindowsCapturer::new(),
            last_selection: Mutex::new(None),
            pending: Mutex::new(Vec::new()),
        }
    }

    pub fn topology(&self) -> DisplayTopology {
        self.topology
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub fn set_topology(&self, topology: DisplayTopology) {
        *self.topology.write().unwrap_or_else(|p| p.into_inner()) = topology;
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
}

/// `overlay_ready` support: the pending payload for `display_id` if its capture is still
/// the live one (the store still holds that generation's frames under that token).
pub fn pending_frame_ready(app: &AppHandle<Wry>, display_id: DisplayId) -> Option<FrameReady> {
    let state = app.state::<Arc<WinCaptureState>>();
    let payload = state
        .pending
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .find(|p| p.display_id == display_id)
        .cloned()?;
    let live = app
        .state::<SharedFrameStore>()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(display_id, payload.gen, &payload.token)
        .is_some();
    if !live {
        return None;
    }
    eprintln!(
        "overlay page ready late: replaying frame-ready gen {} to display {display_id}",
        payload.gen
    );
    Some(payload)
}

/// Capture errors that mean the monitor arrangement changed since the topology snapshot —
/// the only case a resync + retry can fix. Anything else (`BitBlt failed` while the
/// workstation is locked, a GDI resource failure) is reported as is: re-preparing the
/// windows would not help, and it is not free (every overlay window is rebuilt).
fn is_topology_change(err: &CaptureError) -> bool {
    match err {
        CaptureError::SizeMismatch { .. } | CaptureError::NoDisplays => true,
        CaptureError::DisplayFailed { reason, .. } => {
            reason.contains("monitor no longer present") || reason.contains("EnumDisplayMonitors")
        }
        CaptureError::PermissionDenied | CaptureError::Unsupported => false,
    }
}

/// Re-snapshot the monitors and re-prepare the overlay windows (main thread).
fn resync(app: &AppHandle<Wry>) -> Result<DisplayTopology, String> {
    let topology = WindowsDisplayProvider
        .snapshot()
        .map_err(|e| e.to_string())?;
    let state = app.state::<Arc<WinCaptureState>>();
    state.set_topology(topology.clone());
    let host = app.state::<SharedOverlayHost>();
    let mut host = host.lock().unwrap_or_else(|p| p.into_inner());
    host.reprepare(&topology).map_err(|e| e.to_string())?;
    Ok(topology)
}

/// `true` while a capture is on screen (any overlay window opaque).
fn is_revealed(app: &AppHandle<Wry>) -> bool {
    app.state::<SharedOverlayHost>()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .is_revealed()
}

/// Dismiss the capture on screen the way Esc does (`commands::overlay_cancel`): the store
/// cleared, then `hide_all` on the main thread (conceal, restore the foreground,
/// `overlay-hidden` to every page) — awaited, so the capture that follows starts only once
/// the windows are transparent. Capture thread.
fn dismiss_previous(app: &AppHandle<Wry>) {
    app.state::<SharedFrameStore>()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clear();
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = app.clone();
    let dispatched = app.run_on_main_thread(move || {
        let host = handle.state::<SharedOverlayHost>();
        let mut host = host.lock().unwrap_or_else(|p| p.into_inner());
        if let Err(err) = host.hide_all() {
            eprintln!("press while up: hide_all: {err}");
        }
        let _ = tx.send(());
    });
    match dispatched {
        Ok(()) => {
            let _ = rx.recv();
        }
        Err(err) => eprintln!("press while up: run_on_main_thread: {err}"),
    }
}

/// Env flag (the macOS controller's name): dismiss the overlay this many ms after it was
/// shown; the QA hook for unattended runs, same name as the macOS controller uses.
pub const AUTO_DISMISS_ENV: &str = "SNAPSHADE_AUTO_DISMISS_MS";

/// Run the hotkey path once. `t0` is the hotkey handler's receipt time; the completed run
/// prints `latency {…}` (hotkey → every page painted) exactly like macOS.
pub fn trigger(app: AppHandle<Wry>, t0: Instant) {
    let t0_epoch_us = epoch_us();
    let context = RunContext {
        scenario: scenario_tag(),
        selftest: false,
        fmt: FETCH_FMT.to_string(),
        displays: Vec::new(), // filled from the topology on the capture thread
    };
    eprintln!("t0 {t0_epoch_us} scenario={}", context.scenario);
    std::thread::spawn(move || run_capture(app, t0, t0_epoch_us, context));
}

fn run_capture(app: AppHandle<Wry>, t0: Instant, t0_epoch_us: u128, mut context: RunContext) {
    let state = app.state::<Arc<WinCaptureState>>().inner().clone();
    let settings = settings_store::current(&app);
    let request = CaptureRequest {
        kind: CaptureKind::Interactive,
        include_cursor: settings.capture_cursor,
    };
    // Press while the overlay is up (Plan 999.4-03, redone in 999.4-04): the windows are
    // always shown, so the previous capture would be in the new frame. Dismiss it exactly
    // like Esc first — its frames leave the store (a late paint report for it can no longer
    // turn a window opaque over the new capture), its pages get `overlay-hidden`, the
    // windows are concealed and the foreground handed back — then invalidate the DXGI
    // cache: the next grab accepts only a frame the compositor presented after this point
    // (`LastPresentTime`), or the capture takes GDI after 100 ms.
    if is_revealed(&app) {
        let t = Instant::now();
        dismiss_previous(&app);
        state.capturer.invalidate();
        eprintln!(
            "press while up: previous capture dismissed in {:.1} ms",
            t.elapsed().as_secs_f64() * 1000.0
        );
    }
    let mut topology = state.topology();
    let mut set = state.capturer.capture_all(&topology, &request);
    if set.as_ref().is_err_and(is_topology_change) {
        // Monitor arrangement changed: resync on the main thread and retry once.
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = app.clone();
        if app
            .run_on_main_thread(move || {
                let _ = tx.send(resync(&handle));
            })
            .is_ok()
        {
            if let Ok(Ok(t)) = rx.recv() {
                topology = t;
                // The new monitors have no duplication state yet; this retry takes GDI for
                // them and priming runs off the hotkey path, as at startup (Plan 999.4-04).
                {
                    let state = Arc::clone(&state);
                    let topology = topology.clone();
                    std::thread::spawn(move || state.capturer.prime(&topology));
                }
                set = state.capturer.capture_all(&topology, &request);
            }
        }
    }
    let set = match set {
        Ok(set) => set,
        Err(err) => {
            let hint = match &err {
                CaptureError::DisplayFailed { reason, .. } if reason.contains("BitBlt") => {
                    " (workstation locked, or the display is unavailable)"
                }
                _ => "",
            };
            eprintln!("aborted: {err}{hint}");
            return;
        }
    };
    let t1_ms = ms_since(t0);
    context.displays = DisplayRow::from_topology(&topology);
    let initial_selection = if settings.keep_selection_position {
        state.last_selection()
    } else {
        None
    };
    let tool = ToolPrefs::from_settings(&settings);
    // The frames are moved, never cloned: a clone here cost a 33 MB copy per 4K display.
    let frames: Vec<Arc<CaptureFrame>> = set.frames.into_iter().map(Arc::new).collect();
    // Plan 999.4-01: pixels into each page's shared buffer (the host lock is held only to
    // clone the Arcs). `t2` = written; a display without a buffer takes the fetch path.
    let buffers = app
        .state::<SharedOverlayHost>()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .shared_buffers();
    let handoffs = hand_off(&frames, &buffers);
    let shared_count = handoffs.iter().filter(|h| h.shared).count();
    context.fmt = match shared_count {
        0 => FETCH_FMT,
        n if n == handoffs.len() => SHARED_FMT,
        _ => "mixed",
    }
    .to_string();
    let (generation, token) = {
        let store = app.state::<SharedFrameStore>();
        let mut store = store.lock().unwrap_or_else(|p| p.into_inner());
        let generation = store.generation() + 1;
        let token = frame_token(generation, epoch_us(), frames.len());
        // Eager sources: the fetch fallback serves them; the watchdog and `overlay_ready`
        // use their presence as "this generation is live".
        let sources: Vec<Arc<dyn FrameSource>> = frames
            .iter()
            .map(|f| Arc::new(Arc::clone(f)) as Arc<dyn FrameSource>)
            .collect();
        store.replace(generation, token.clone(), sources);
        (generation, token)
    };
    let ready: Vec<FrameReady> = frames
        .iter()
        .zip(&handoffs)
        .map(|(f, h)| FrameReady {
            gen: generation,
            display_id: f.display_id,
            width: f.width,
            height: f.height,
            scale: f.scale,
            fmt: if h.shared { SHARED_FMT } else { FETCH_FMT }.to_string(),
            token: token.clone(),
            fetch_after_paint: false,
            initial_selection: initial_selection
                .filter(|(id, _)| *id == f.display_id)
                .map(|(_, rect)| rect),
            tool: tool.clone(),
        })
        .collect();
    drop(frames);
    eprintln!(
        "handoff gen {generation}: {}",
        ready
            .iter()
            .map(|r| format!("{}={}", r.display_id, r.fmt))
            .collect::<Vec<_>>()
            .join(" ")
    );
    *state.pending.lock().unwrap_or_else(|p| p.into_inner()) = ready.clone();
    {
        let timings = app.state::<SharedTimings>();
        let mut timings = timings.lock().unwrap_or_else(|p| p.into_inner());
        timings.begin(
            generation,
            t0,
            t0_epoch_us,
            t1_ms,
            ready.iter().map(|r| r.display_id),
            context,
        );
        for h in handoffs.iter().filter(|h| h.shared) {
            timings.record_served(generation, h.display_id, h.written_us);
        }
    }
    let ready_ids: Vec<DisplayId> = ready.iter().map(|r| r.display_id).collect();
    let handle = app.clone();
    let result = app.run_on_main_thread(move || {
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
        if let Err(err) = host.show_all() {
            eprintln!("aborted: show_all: {err}");
            return;
        }
        let placement_ok = host.placement_ok();
        handle
            .state::<SharedTimings>()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .record_shown(generation, placement_ok);
        let labels = host.labels();
        for payload in ready {
            let Some((_, label)) = labels.iter().find(|(id, _)| *id == payload.display_id) else {
                eprintln!("no overlay window for display {}", payload.display_id);
                continue;
            };
            if let Err(err) = handle.emit_to(label.as_str(), "frame-ready", payload) {
                eprintln!("emit frame-ready to {label}: {err}");
            }
        }
    });
    if let Err(err) = result {
        eprintln!("aborted: run_on_main_thread: {err}");
        return;
    }
    // Watchdog: a page that never paints (renderer failure, lost event with no late
    // `overlay_ready`) must not leave black screens that Esc cannot dismiss.
    {
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(PAINT_WATCHDOG);
            let painted = app
                .state::<SharedTimings>()
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .painted_count(generation);
            let live = app
                .state::<SharedFrameStore>()
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get(ready_ids[0], generation, &token)
                .is_some();
            if painted == 0 && live {
                eprintln!(
                    "aborted: no overlay page painted gen {generation} within {:?}; hiding",
                    PAINT_WATCHDOG
                );
                if let Err(err) = crate::commands::overlay_cancel(app) {
                    eprintln!("watchdog: {err}");
                }
            }
        });
    }
    if let Some(ms) = std::env::var(AUTO_DISMISS_ENV)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
    {
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(ms));
            eprintln!("auto-dismiss after {ms} ms ({AUTO_DISMISS_ENV})");
            if let Err(err) = crate::commands::overlay_cancel(app) {
                eprintln!("auto-dismiss: {err}");
            }
        });
    }
}

/// `SNAPSHADE_SELFTEST` on Windows: `<N>` → N hotkey-path captures 3 s apart, each
/// dismissed after 1.5 s (rows on stderr), then exit 0; `save` → one instant full-screen
/// save (every monitor, cursor per settings), then exit 0; `pwu` → a capture, a second
/// one 400 ms later while the first is still up (the press-while-up path: the second frame
/// must be the desktop, not the dimmed first overlay), dismissed after 1.5 s, then exit 0.
pub fn schedule_selftest(app: AppHandle<Wry>, mode: String) {
    std::thread::spawn(move || {
        // Let the overlay pages load and report ready first.
        std::thread::sleep(std::time::Duration::from_secs(4));
        if mode == "save" {
            instant_save(app.clone());
            std::thread::sleep(std::time::Duration::from_secs(3));
        } else if mode == "pwu" {
            trigger(app.clone(), Instant::now());
            std::thread::sleep(std::time::Duration::from_millis(400));
            trigger(app.clone(), Instant::now());
            std::thread::sleep(std::time::Duration::from_millis(1500));
            if let Err(err) = crate::commands::overlay_cancel(app.clone()) {
                eprintln!("selftest: cancel: {err}");
            }
            std::thread::sleep(std::time::Duration::from_millis(1000));
        } else if let Ok(n) = mode.parse::<u32>() {
            for _ in 0..n {
                trigger(app.clone(), Instant::now());
                std::thread::sleep(std::time::Duration::from_millis(1500));
                if let Err(err) = crate::commands::overlay_cancel(app.clone()) {
                    eprintln!("selftest: cancel: {err}");
                }
                std::thread::sleep(std::time::Duration::from_millis(1500));
            }
        } else {
            eprintln!("selftest: unknown mode {mode:?} (use <N> or save)");
        }
        eprintln!("selftest: done");
        app.exit(0);
    });
}

/// HOTKEY-04 on Windows: capture every monitor and save, no overlay.
pub fn instant_save(app: AppHandle<Wry>) {
    std::thread::spawn(move || {
        let state = app.state::<Arc<WinCaptureState>>();
        let settings = settings_store::current(&app);
        let req = CaptureRequest {
            kind: CaptureKind::InstantFullscreen,
            include_cursor: settings.capture_cursor,
        };
        match state.capturer.capture_all(&state.topology(), &req) {
            Ok(set) => match export::save_fullscreen(&app, &set.frames) {
                Ok(paths) => eprintln!("instant save: wrote {} file(s)", paths.len()),
                Err(err) => eprintln!("instant save: {err}"),
            },
            Err(err) => eprintln!("instant save: capture failed: {err}"),
        }
    });
}
