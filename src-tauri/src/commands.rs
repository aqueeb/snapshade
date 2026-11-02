//! IPC commands invoked by the overlay page. Payloads are tiny typed JSON
//! (T-01-14: serde-typed, no panics on bad input); pixels never travel here, and
//! neither does typed text — `overlay_input` carries a length only (T-01-18) and
//! `overlay_key` carries a key NAME or the class `<printable>`, never a character
//! (REVIEW WR-03, Plan 01-10).

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use snapshade_core::capture::CaptureFrame;
use snapshade_core::geometry::{DisplayId, PhysicalRect};
use snapshade_core::settings::Settings;
use tauri::{AppHandle, Emitter, Manager, Runtime, State, Wry};

use crate::export::{self, ExportKind, ExportOutcome};
use crate::frame_protocol::SharedFrameStore;
use crate::latency::{append_row, power_source, report, LatencyLog, Reporter, SharedTimings};
use crate::{aux_windows, hotkeys, settings_store, strings, tray};

/// Longest key name echoed to the log; longer (forged) names are truncated.
const MAX_KEY_LEN: usize = 32;
/// Longest fetch error echoed to the log (control chars stripped).
const MAX_ERROR_LEN: usize = 120;

/// Where the overlay page's lazy `frame://` fetch outcome is delivered besides stderr:
/// `(display_id, gen, bytes, error)`. Mirrors the shape of `latency::Reporter` (the
/// `Arc<Mutex<Option<Sender<_>>>>` alias in `src-tauri/src/latency.rs`): installed only
/// by the `SNAPSHADE_SELFTEST=<N>` loop, `None` in normal operation (messages dropped).
/// Declared here, next to its command, because `latency.rs` is Plan 01-11's file.
pub type FetchReporter = Arc<Mutex<Option<Sender<(DisplayId, u64, usize, Option<String>)>>>>;

/// The overlay for `display_id` has presented generation `gen` (sent from the
/// second nested `requestAnimationFrame` after the frame was drawn). `t_wall_ms` is
/// the webview's `performance.timeOrigin + performance.now()`.
///
/// The last paint of a generation completes the run: the JSONL row is appended
/// (and mirrored to stderr) BEFORE the self-test reporter is notified, so a
/// self-test that exits on the report always leaves its rows on disk.
#[tauri::command]
pub fn overlay_painted(app: AppHandle<Wry>, display_id: u32, gen: u64, t_wall_ms: f64) {
    record_paint(&app, display_id, gen, t_wall_ms, 0);
}

/// How often, and how far apart, a paint report is re-dispatched while the overlay host is
/// busy (Windows, a display-change resync in progress) before it is recorded unrevealed.
#[cfg(target_os = "windows")]
const PAINT_DEFER_MAX: u32 = 100;
#[cfg(target_os = "windows")]
const PAINT_DEFER_STEP: std::time::Duration = std::time::Duration::from_millis(10);

/// The body of `overlay_painted`, re-entrant through the event loop. On Windows (Plan
/// 999.4-03) the overlay windows are always shown but transparent, and this paint report
/// is the moment this display's window turns opaque — which needs the host lock, held on
/// this same thread while a display-change resync rebuilds the windows (the command then
/// runs inside WebView2's nested message pump). Rather than record the paint without its
/// reveal, the WHOLE handling is dispatched again from a later turn, 10 ms apart and at
/// most 100 times, so the deferred turn re-checks that the generation is still live and
/// stamps the reveal and the paint together (Plan 999.4-04). The reveal is stamped before
/// the paint so a completing row includes it.
fn record_paint(app: &AppHandle<Wry>, display_id: u32, gen: u64, t_wall_ms: f64, attempt: u32) {
    #[cfg(target_os = "windows")]
    let revealed = {
        use crate::overlay_win::{reveal_after_paint, RevealOutcome};
        match reveal_after_paint(app, display_id, gen) {
            RevealOutcome::Revealed => true,
            RevealOutcome::Skipped => false,
            RevealOutcome::Busy if attempt < PAINT_DEFER_MAX => {
                let app = app.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(PAINT_DEFER_STEP);
                    let handle = app.clone();
                    let dispatched = app.run_on_main_thread(move || {
                        record_paint(&handle, display_id, gen, t_wall_ms, attempt + 1);
                    });
                    if let Err(err) = dispatched {
                        eprintln!("deferred paint handling for display {display_id}: {err}");
                    }
                });
                return;
            }
            RevealOutcome::Busy => {
                eprintln!(
                    "reveal display {display_id} gen {gen}: host busy through {attempt} attempts; \
                     recorded painted without a reveal"
                );
                false
            }
        }
    };
    #[cfg(not(target_os = "windows"))]
    let revealed = false;
    if attempt > 0 {
        eprintln!(
            "deferred paint display {display_id} gen {gen}: attempt {attempt}, revealed={revealed}"
        );
    }
    let timings = app.state::<SharedTimings>();
    let reporter = app.state::<Reporter>();
    let log = app.state::<LatencyLog>();
    let completed = {
        let mut timings = timings.lock().unwrap_or_else(|p| p.into_inner());
        if revealed {
            timings.record_revealed(gen, display_id);
        }
        timings
            .record_painted(gen, display_id, t_wall_ms)
            .map(|run| {
                (
                    run,
                    timings.row(log.next_run(), log.hw(), log.os(), &power_source()),
                )
            })
    };
    if let Some((run, row)) = completed {
        match row {
            Some(row) => {
                if let Err(err) = append_row(&row) {
                    eprintln!("latency log append failed: {err}");
                }
            }
            None => eprintln!("latency row unavailable for gen {gen}"),
        }
        report(&reporter, Ok(run));
    }
}

/// The overlay page finished its lazy `frame://` fetch for generation `gen` (self-test
/// runs only — `FrameReady.fetch_after_paint`; Plan 01-10 Task 2). Prints
/// `frame fetched <display> gen=<g> bytes=<n>` or `frame fetch failed <display>: <error>`
/// (error truncated to 120 non-control chars) and forwards to the [`FetchReporter`].
#[tauri::command]
pub fn overlay_frame_fetched(
    reporter: State<'_, FetchReporter>,
    display_id: u32,
    gen: u64,
    bytes: usize,
    error: Option<String>,
    fetch_ms: Option<f64>,
    paint_ms: Option<f64>,
) {
    let error = error.map(|e| {
        e.chars()
            .filter(|c| !c.is_control())
            .take(MAX_ERROR_LEN)
            .collect::<String>()
    });
    // The Windows fetch path adds how long the page spent fetching and painting.
    let timing = match (fetch_ms, paint_ms) {
        (Some(f), Some(p)) => format!(" fetch_ms={f:.1} paint_ms={p:.1}"),
        (Some(f), None) => format!(" fetch_ms={f:.1}"),
        _ => String::new(),
    };
    match &error {
        Some(e) => eprintln!("frame fetch failed {display_id}: {e}"),
        None => eprintln!("frame fetched {display_id} gen={gen} bytes={bytes}{timing}"),
    }
    if let Some(tx) = reporter.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
        let _ = tx.send((display_id, gen, bytes, error));
    }
}

/// A key reached the overlay page (gate (c) evidence; Plan 01-07 Task 2 sends it).
/// Prints `key <display> <key> meta=<b> shift=<b>`; the key NAME or the literal class
/// `<printable>` (unmodified printable keys, WR-03) — never the typed character.
#[tauri::command]
pub fn overlay_key(display_id: u32, key: String, meta: bool, shift: bool) {
    let key: String = key
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_KEY_LEN)
        .collect();
    eprintln!("key {display_id} {key} meta={meta} shift={shift}");
}

/// The probe `<input>` on the overlay page changed; only its length is reported
/// (T-01-18 — the text never crosses IPC). Prints `input <display> len=<n>`.
#[tauri::command]
pub fn overlay_input(display_id: u32, len: u32) {
    eprintln!("input {display_id} len={len}");
}

/// The overlay page for `display_id` has attached its listeners (once per page load).
/// On Windows, if a capture is already showing on that display — the page loaded after
/// the `frame-ready` event (slow machine) or was reloaded after a renderer failure —
/// the pending payload is returned so the page paints it instead of staying black.
#[tauri::command]
pub fn overlay_ready(app: AppHandle<Wry>, display_id: u32) -> Option<crate::payloads::FrameReady> {
    eprintln!("overlay page ready: display {display_id}");
    #[cfg(target_os = "windows")]
    {
        // Plan 999.4-01: hand this page its display's shared frame buffer (the page attached
        // its listener before invoking us). DEFERRED through the event loop, never inline:
        // this command can run inside WebView2's nested message pump while
        // `WinOverlayHost::prepare` is still building the next window — with the host lock
        // held by that same thread. Locking here wedged the app (self-deadlock on a
        // non-reentrant Mutex, observed 2026-10-31 on the resync path). A proxy-delivered
        // closure runs only after the current main-thread turn has returned the lock.
        {
            let app = app.clone();
            std::thread::spawn(move || {
                let handle = app.clone();
                let dispatched = app.run_on_main_thread(move || {
                    use crate::overlay_win::SharedOverlayHost;
                    let host = handle.state::<SharedOverlayHost>();
                    let host = host.lock().unwrap_or_else(|p| p.into_inner());
                    match host.post_shared_buffer(display_id) {
                        Ok(true) => eprintln!("shared frame buffer posted to display {display_id}"),
                        Ok(false) => {}
                        Err(err) => {
                            eprintln!("shared frame buffer post for display {display_id}: {err}")
                        }
                    }
                });
                if let Err(err) = dispatched {
                    eprintln!("shared frame buffer post for display {display_id}: {err}");
                }
            });
        }
        crate::capture_controller_win::pending_frame_ready(&app, display_id)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = app;
        None
    }
}

/// Esc: hide every panel (order out, never destroy) and drop the frames.
#[tauri::command]
pub fn overlay_cancel<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    app.state::<SharedFrameStore>()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clear();
    hide_overlays(&app)
}

#[cfg(target_os = "macos")]
fn hide_overlays<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    use crate::overlay::SharedOverlayHost;
    use snapshade_platform_api::OverlayHost;

    let handle = app.clone();
    app.run_on_main_thread(move || {
        let host = handle.state::<SharedOverlayHost>();
        let mut host = host.lock().unwrap_or_else(|p| p.into_inner());
        if let Err(err) = host.hide_all() {
            eprintln!("overlay_cancel: hide_all failed: {err}");
        }
    })
    .map_err(|e| e.to_string())
}

#[cfg(target_os = "windows")]
fn hide_overlays<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    use crate::overlay_win::SharedOverlayHost;
    use snapshade_platform_api::OverlayHost;

    let handle = app.clone();
    app.run_on_main_thread(move || {
        let host = handle.state::<SharedOverlayHost>();
        let mut host = host.lock().unwrap_or_else(|p| p.into_inner());
        if let Err(err) = host.hide_all() {
            eprintln!("overlay_cancel: hide_all failed: {err}");
        }
    })
    .map_err(|e| e.to_string())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn hide_overlays<R: Runtime>(_app: &AppHandle<R>) -> Result<(), String> {
    Err("overlay host is not supported on this platform".to_string())
}

// ---------------------------------------------------------------------------------------
// Phase 2–4 commands: export, selection memory, cross-panel coordination, settings, shell.
// ---------------------------------------------------------------------------------------

/// Largest crop the page may hand over (a 6K display's full frame: 6016 × 3384 × 4 ≈ 81 MB).
const MAX_EXPORT_BYTES: usize = 96 * 1024 * 1024;

fn header_u32(request: &tauri::ipc::Request<'_>, name: &str) -> Result<u32, String> {
    request
        .headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u32>().ok())
        .ok_or_else(|| format!("missing or invalid header {name}"))
}

/// The flattened selection (raw RGBA body, `x-width` / `x-height` / `x-kind` headers) →
/// copy / save / save-as / print (EXPORT-01..05). Runs synchronously on the main thread
/// so the clipboard write is complete before the page's `await` resolves and it dismisses
/// the overlay (EXPORT-02). Save As and Print hide the overlay first: the panels sit at
/// screen-saver level and would cover the native dialog.
#[tauri::command]
pub fn export_image(
    app: AppHandle<Wry>,
    request: tauri::ipc::Request<'_>,
) -> Result<ExportOutcome, String> {
    let kind = request
        .headers()
        .get("x-kind")
        .and_then(|v| v.to_str().ok())
        .and_then(ExportKind::parse)
        .ok_or_else(|| "missing or invalid header x-kind".to_string())?;
    let width = header_u32(&request, "x-width")?;
    let height = header_u32(&request, "x-height")?;
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("export_image expects a raw RGBA body".to_string());
    };
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .filter(|n| *n > 0 && *n <= MAX_EXPORT_BYTES)
        .ok_or_else(|| format!("invalid export dimensions {width}x{height}"))?;
    if bytes.len() != expected {
        return Err(format!(
            "export body is {} bytes, expected {expected} for {width}x{height}",
            bytes.len()
        ));
    }
    let frame = CaptureFrame {
        display_id: 0,
        width,
        height,
        scale: 1.0,
        rgba: bytes.clone(),
    };
    let outcome = match kind {
        ExportKind::Copy => export::copy(&app, &frame),
        ExportKind::Save => export::save_instant(&app, &frame),
        ExportKind::SaveAs => {
            hide_overlays_now(&app);
            export::save_as(&app, &frame)
        }
        ExportKind::Print => {
            hide_overlays_now(&app);
            export::print(&app, &frame)
        }
    };
    // One stderr line per export so a failure is visible outside the page console.
    match &outcome {
        Ok(o) if o.cancelled => eprintln!("export {kind:?} {width}x{height}: cancelled"),
        Ok(o) => eprintln!(
            "export {kind:?} {width}x{height}: ok{}",
            o.path
                .as_deref()
                .map(|p| format!(" {p}"))
                .unwrap_or_default()
        ),
        Err(e) => eprintln!("export {kind:?} {width}x{height}: {e}"),
    }
    outcome
}

/// Hide the panels from the main thread (the export commands run there).
#[cfg(target_os = "macos")]
fn hide_overlays_now(app: &AppHandle<Wry>) {
    use crate::overlay::SharedOverlayHost;
    use snapshade_platform_api::OverlayHost;
    app.state::<SharedFrameStore>()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clear();
    let host = app.state::<SharedOverlayHost>();
    let mut host = host.lock().unwrap_or_else(|p| p.into_inner());
    if let Err(err) = host.hide_all() {
        eprintln!("export: hide_all failed: {err}");
    }
}

#[cfg(not(target_os = "macos"))]
fn hide_overlays_now(app: &AppHandle<Wry>) {
    let _ = hide_overlays(app);
}

/// The page committed a selection (SELECT-08 "Remember the selected area").
#[tauri::command]
pub fn overlay_selection_changed(app: AppHandle<Wry>, display_id: u32, rect: PhysicalRect) {
    #[cfg(target_os = "macos")]
    {
        let state = app.state::<std::sync::Arc<crate::capture_controller::CaptureState>>();
        state.remember_selection(display_id, rect);
    }
    #[cfg(target_os = "windows")]
    {
        let state = app.state::<std::sync::Arc<crate::capture_controller_win::WinCaptureState>>();
        state.remember_selection(display_id, rect);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (app, display_id, rect);
    }
}

/// A press on `display_id`'s panel: every other panel drops its selection (a selection
/// lives on exactly one display, spec §3.4).
#[tauri::command]
pub fn overlay_claim(app: AppHandle<Wry>, display_id: u32) {
    #[derive(serde::Serialize, Clone)]
    #[serde(rename_all = "camelCase")]
    struct HomeChanged {
        display_id: u32,
    }
    if let Err(err) = app.emit("overlay-home-changed", HomeChanged { display_id }) {
        eprintln!("overlay-home-changed emit failed: {err}");
    }
}

/// Persist the shared tool colour / marker colour / size so the next capture starts there.
#[tauri::command]
pub fn overlay_tool_changed(app: AppHandle<Wry>, color: String, marker_color: String, size: u32) {
    let valid = |c: &str| {
        c.len() == 7 && c.starts_with('#') && c[1..].bytes().all(|b| b.is_ascii_hexdigit())
    };
    if !valid(&color) || !valid(&marker_color) || !(1..=40).contains(&size) {
        return;
    }
    settings_store::update(&app, |s| {
        s.color = color;
        s.marker_color = marker_color;
        s.tool_size = size;
    });
}

#[tauri::command]
pub fn get_settings(app: AppHandle<Wry>) -> Settings {
    settings_store::current(&app)
}

/// Options → Save: validate + re-register hotkeys (HOTKEY-02; the validation message on
/// failure, nothing persisted then), apply start-at-login, refresh the tray tooltip, persist.
#[tauri::command]
pub fn set_settings(app: AppHandle<Wry>, settings: Settings) -> Result<(), String> {
    if let Err(err) = hotkeys::apply(&app, &settings) {
        // `apply` unregisters everything before it tries the new set, so a rejected
        // combo must not leave the app with no hotkey at all: put the last good set back.
        let previous = settings_store::current(&app);
        if let Err(restore) = hotkeys::apply(&app, &previous) {
            eprintln!("hotkeys: restoring the previous set failed: {restore}");
        }
        return Err(err);
    }
    apply_autostart(&app, settings.start_at_login);
    tray::update_tooltip(&app, &settings.capture_hotkey);
    settings_store::update(&app, |s| *s = settings);
    Ok(())
}

pub fn apply_autostart(app: &AppHandle<Wry>, enabled: bool) {
    use tauri_plugin_autostart::ManagerExt;
    let manager = app.autolaunch();
    let result = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    if let Err(err) = result {
        eprintln!("autostart ({enabled}): {err}");
    }
}

/// Options → Formats → Browse… (native folder picker; main thread).
#[tauri::command]
pub fn pick_folder(app: AppHandle<Wry>) -> Option<String> {
    let settings = settings_store::current(&app);
    let start = settings.save_folder(export::os_default_folder(&app));
    #[cfg(target_os = "macos")]
    let _ = snapshade_platform_macos::activate_app();
    rfd::FileDialog::new()
        .set_title(strings::FOLDER_DIALOG_TITLE)
        .set_directory(start)
        .set_can_create_directories(true)
        .pick_folder()
        .map(|p| p.display().to_string())
}

/// Open a URL with the OS (Help, the Screen Recording deep link). Allow-listed schemes only.
#[tauri::command]
pub fn open_url(app: AppHandle<Wry>, url: String) -> Result<(), String> {
    let allowed = url.starts_with("https://")
        || url.starts_with("x-apple.systempreferences:")
        || url.starts_with("ms-settings:");
    if !allowed {
        return Err(format!("refusing to open {url}"));
    }
    app.state::<export::Exporters>()
        .shell
        .open_url(&url)
        .map_err(|e| e.to_string())
}

/// Open the Options window (from About, the permission page, or a notification).
#[tauri::command]
pub fn open_options(app: AppHandle<Wry>) -> Result<(), String> {
    aux_windows::show(&app, aux_windows::OPTIONS)
}

/// Start a capture from the UI (tray fallback is handled in lib.rs).
#[tauri::command]
pub fn start_capture(app: AppHandle<Wry>) {
    crate::start_capture_from_ui(&app);
}

/// CAPTURE-03 "Quit and relaunch": the Screen Recording grant applies to a fresh process.
#[tauri::command]
pub fn relaunch_app(app: AppHandle<Wry>) {
    app.restart();
}

/// App version for About.
#[tauri::command]
pub fn app_version(app: AppHandle<Wry>) -> String {
    app.package_info().version.to_string()
}
