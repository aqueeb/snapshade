//! Snapshade app composition root.
//!
//! Phase 1 (Plans 01-03 / 01-06 / 01-09): an accessory (menu-bar-less, dock-less) Tauri
//! app whose only windows are the hidden per-display overlay panels (non-activating
//! NSPanels, transparent webview over a native frame layer). ⌘⇧9 runs
//! `capture_controller::trigger` in the Plan B ordering: preflight → `capture_native`
//! (one retained `CGImage` per display, no pixel copy) → `FrameStore` (lazy sources) →
//! present (each `CGImage` onto its panel's native `FrameLayer`) → show → `frame-ready`
//! (metadata only, after show). `frame://` is the lazy raw-RGBA export path for Phase 2's
//! selection crop / export, off the critical path; Esc in the overlay hides the panels.
//! `SNAPSHADE_SELFTEST=preflight|<N>|exclusion` runs the in-process self-tests; every
//! completed run appends one JSONL row to `~/Library/Logs/Snapshade/spike-latency.jsonl`
//! (Plan 01-07). Every capture excludes Snapshade's own application through the
//! `SCContentFilter` (Plan 01-10, ROADMAP SC2) — panels keep the default sharing type so
//! DisplayLink displays show them. The default macOS menu is disabled in favour of a
//! Quit-only menu so ⌘Z (and the other overlay keys) reach the page as DOM
//! keydowns instead of being consumed by Edit → Undo (Plan 01-10, ROADMAP SC3).
//!
//! Robustness (Plan 01-07 Task 3): `NSApplicationDidChangeScreenParametersNotification`
//! re-snapshots the topology, re-fetches `SCShareableContent` and re-prepares the
//! panels; `SNAPSHADE_ACTIVITY=1` holds an `NSProcessInfo` latency-critical activity for
//! the idle-on-battery comparison (RESEARCH Q6 line 472 — measured in 01-08, not assumed).
//!
//! This is the only crate that names both OS adapters (D-18): the macOS modules are
//! `cfg(target_os = "macos")`; on Windows the shell registers the hotkey and logs `t0`
//! until Phase 5 ports the adapters.
//!
//! All diagnostic output goes to stderr; stdout stays free for self-test results.

use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use snapshade_platform_api::NotificationAction;
use tauri::Manager;
use tauri_plugin_global_shortcut::ShortcutState;

pub mod aux_windows;
#[cfg(target_os = "macos")]
pub mod capture_controller;
#[cfg(target_os = "windows")]
pub mod capture_controller_win;
pub mod commands;
pub mod export;
pub mod frame_protocol;
pub mod hotkeys;
pub mod latency;
#[cfg(target_os = "macos")]
pub mod overlay;
#[cfg(target_os = "windows")]
pub mod overlay_win;
pub mod payloads;
pub mod print_page;
#[cfg(target_os = "macos")]
pub mod selftest;
pub mod settings_store;
#[cfg(target_os = "windows")]
pub mod shared_frame_win;
pub mod strings;
pub mod tray;

/// Window label for `display_id` at `epoch` (shared by both overlay hosts; the capability
/// glob `overlay-*` and the page's label parser accept both forms).
pub fn overlay_label(display_id: snapshade_core::geometry::DisplayId, epoch: u64) -> String {
    if epoch == 0 {
        format!("overlay-{display_id}")
    } else {
        format!("overlay-{display_id}-{epoch}")
    }
}

use frame_protocol::{FrameStore, SharedFrameStore};
use latency::{LatencyLog, Reporter, SharedTimings, Timings};

// tauri-codegen embeds `Info.plist` (plus CFBundleName/Version) into the raw binary only for a
// dev build (`custom-protocol` off — what `tauri dev` runs via `--no-default-features`). A plain
// `cargo run` uses default features, so without this embed the raw binary would carry no
// CFBundleIdentifier and TCC could not key the Screen Recording grant to it (RESEARCH Q1).
// Gating on the same feature codegen checks guarantees exactly one `__info_plist` section.
#[cfg(all(target_os = "macos", feature = "custom-protocol"))]
tauri::embed_plist::embed_info_plist!("../Info.plist");

/// Env flag: when `1`, hold a `UserInitiated | LatencyCritical` process activity.
#[cfg(target_os = "macos")]
pub const ACTIVITY_ENV: &str = "SNAPSHADE_ACTIVITY";

/// Screen-parameter notifications arrive in bursts; the re-prepare runs once this long
/// after the first one.
#[cfg(target_os = "macos")]
const SCREEN_CHANGE_SETTLE: std::time::Duration = std::time::Duration::from_millis(300);

/// Builds and runs the Tauri application. Never returns on success (the event loop owns the
/// process); exits with status 2 if the hotkey cannot be registered.
pub fn run() {
    let builder = tauri::Builder::default()
        // SHELL-03: must be the first plugin so a second launch exits before anything else.
        .plugin(tauri_plugin_single_instance::init(|_app, _args, _cwd| {
            eprintln!("single instance: a second launch was redirected here");
        }))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    // Fires for Pressed AND Released; only a press acts.
                    if event.state != ShortcutState::Pressed {
                        return;
                    }
                    match hotkeys::role_of(app, shortcut) {
                        Some(hotkeys::HotkeyRole::Capture) => on_hotkey(app, Instant::now()),
                        Some(hotkeys::HotkeyRole::InstantSave) => on_instant_save(app),
                        None => eprintln!("hotkey: unknown shortcut {shortcut:?} fired"),
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(SharedFrameStore::new(Mutex::new(FrameStore::default())))
        .manage(SharedTimings::new(Mutex::new(Timings::default())))
        .manage(Reporter::default())
        .manage(commands::FetchReporter::default())
        .manage(LatencyLog::open())
        .manage(hotkeys::SharedRegistered::default())
        .invoke_handler(tauri::generate_handler![
            commands::overlay_painted,
            commands::overlay_cancel,
            commands::overlay_key,
            commands::overlay_input,
            commands::overlay_frame_fetched,
            commands::overlay_ready,
            commands::export_image,
            commands::overlay_selection_changed,
            commands::overlay_claim,
            commands::overlay_tool_changed,
            commands::get_settings,
            commands::set_settings,
            commands::pick_folder,
            commands::open_url,
            commands::open_options,
            commands::start_capture,
            commands::relaunch_app,
            commands::app_version
        ]);
    // tauri-nspanel 2.1.1 (approved at the Plan 01-06 Task 1 checkpoint) manages the
    // panel registry `to_panel` writes to; it is the only plugin besides global-shortcut.
    #[cfg(target_os = "macos")]
    let builder = builder.plugin(tauri_nspanel::init());

    // ⌘Z routing (Plan 01-10 Task 3; ROADMAP SC3; spike gate (c); 01-SPIKE-RESULT open
    // issue 2). Tauri's default macOS menu carries Edit → Undo with the ⌘Z key equivalent
    // (plus Redo, Cut, Copy, Paste, Select All). AppKit dispatches a matching key
    // equivalent to the main menu BEFORE the key window's WKWebView ever creates a DOM
    // `keydown`, which is why runs 34/35 in 01-08 saw arrows and text but never
    // `z meta=true`. With no Edit submenu, ⌘Z, ⌘C, ⌘A, ⌘X, ⌘S, ⌘P (the overlay keys,
    // docs/SPEC.md §3 and §5) all reach the page as keydowns. The explicit menu is
    // the application submenu with Quit only; the accessory app shows no menu bar anyway.
    // Phase 4 note: the Options window will want Edit items for its text fields — add
    // them WITHOUT a ⌘Z key equivalent while an overlay panel is key (e.g. swap menus on
    // window focus), never by re-enabling the default menu.
    #[cfg(target_os = "macos")]
    let builder = builder.enable_macos_default_menu(false).menu(|app| {
        use tauri::menu::{Menu, PredefinedMenuItem, Submenu};
        let app_menu = Submenu::with_items(
            app,
            "Snapshade",
            true,
            &[&PredefinedMenuItem::quit(app, None)?],
        )?;
        Menu::with_items(app, &[&app_menu])
    });
    let builder = frame_protocol::register(builder);
    let builder = print_page::register(builder).manage(print_page::PrintJob::default());

    builder
        .setup(move |app| {
            // Self-tests that need no app state run before anything else in setup so
            // `preflight` works even when ⌘⇧9 is owned by another process.
            #[cfg(target_os = "macos")]
            let selftest_mode = selftest::mode();
            #[cfg(target_os = "macos")]
            if let Some(mode) = selftest_mode
                .as_deref()
                .filter(|mode| !selftest::needs_app(mode))
            {
                selftest::run_preflight_or_unknown(mode);
            }

            // Accessory policy: no Dock icon, no menu bar, no app switcher entry. The
            // overlays are non-activating panels, so the app never needs to be the
            // frontmost application (RESEARCH Q4).
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            #[cfg(target_os = "macos")]
            eprintln!("menu: explicit (quit only), default macOS menu disabled");

            // Settings (OPT-04): loaded once; every change goes through settings_store::update.
            let settings_path = settings_store::settings_path(app.handle());
            let settings = settings_path
                .as_deref()
                .map(settings_store::load)
                .unwrap_or_default();
            eprintln!(
                "settings: {} ({})",
                settings_path
                    .as_deref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "<no config dir>".to_string()),
                if settings_path.as_deref().is_some_and(|p| p.exists()) {
                    "loaded"
                } else {
                    "defaults"
                }
            );
            app.manage(settings_store::SharedSettings::new(RwLock::new(
                settings.clone(),
            )));

            // Platform exporters + notification click routing (EXPORT-06, HOTKEY-03).
            app.manage(make_exporters(app.handle().clone()));

            // QA hook: `SNAPSHADE_OPEN_WINDOW=options|about|permission` opens that window at
            // startup (the tray menu cannot be driven by synthetic clicks in automation).
            if let Ok(label) = std::env::var("SNAPSHADE_OPEN_WINDOW") {
                let label: &'static str = match label.as_str() {
                    "options" => aux_windows::OPTIONS,
                    "about" => aux_windows::ABOUT,
                    "permission" => aux_windows::PERMISSION,
                    _ => "",
                };
                if !label.is_empty() {
                    aux_windows::show_later(app.handle(), label);
                }
            }

            // Tray (SHELL-01/02): tooltip names the live hotkey; menu drives the shell.
            match tray::build(app.handle(), &settings.capture_hotkey, on_menu_event) {
                Ok(_) => eprintln!("tray: created"),
                Err(err) => eprintln!("tray: creation failed: {err}"),
            }

            // macOS: snapshot the displays (main thread), prefetch shareable content, and
            // pre-create one hidden overlay panel per display (ARCHITECTURE Pattern 3).
            #[cfg(target_os = "macos")]
            {
                use snapshade_platform_api::{DisplayProvider, OverlayHost};
                use snapshade_platform_macos::{MacCapturer, MacDisplayProvider};

                let topology = MacDisplayProvider.snapshot()?;
                for d in &topology.displays {
                    eprintln!(
                        "display {} logical=({}, {}, {}, {}) scale={} physical={}x{} primary={}",
                        d.id,
                        d.logical.x,
                        d.logical.y,
                        d.logical.w,
                        d.logical.h,
                        d.scale,
                        d.physical.w,
                        d.physical.h,
                        d.primary
                    );
                }
                let capturer = MacCapturer::new();
                let mut host = overlay::TauriOverlayHost::new(app.handle().clone());
                host.prepare(&topology)?;
                eprintln!("overlays prepared for displays {:?}", host.display_ids());
                let capture_state =
                    Arc::new(capture_controller::CaptureState::new(topology, capturer));
                app.manage(Arc::clone(&capture_state));
                app.manage(overlay::SharedOverlayHost::new(Mutex::new(host)));

                // SC2 (Plan 01-10): the launch-time SCShareableContent fetch ran before the
                // panels existed, so Snapshade may not be in its `applications()` yet. Refresh
                // on a worker thread (slow, RESEARCH Q3 line 326) now that the windows exist,
                // so the first press's `self_application()` lookup takes the fast path.
                std::thread::spawn(move || {
                    let ok = capture_state.capturer.refresh();
                    eprintln!(
                        "shareable content refresh after prepare {}",
                        if ok { "ok" } else { "failed" }
                    );
                });

                // Display change → re-snapshot, refresh SCK content, re-prepare (A8).
                let handle = app.handle().clone();
                let pending = Arc::new(std::sync::atomic::AtomicBool::new(false));
                let observer =
                    snapshade_platform_macos::observe_screen_changes(Box::new(move || {
                        on_screens_changed(handle.clone(), pending.clone());
                    }))?;
                app.manage(observer);
                eprintln!("screen change observer registered");

                // ⌘Z fallback (Plan 01-10 Task 3 step 2): with the default menu gone the probe
                // still saw no `z meta=true` keydown, so the key equivalent is caught at the
                // NSEvent level and funnelled to the key overlay's page (same `overlay_key`
                // report). Measured, not assumed — the SUMMARY records which path was needed.
                install_key_equivalent_monitor(app.handle().clone());

                // Scenario (c) comparison: hold the activity only when asked (RESEARCH Q6).
                if std::env::var(ACTIVITY_ENV)
                    .map(|v| v == "1")
                    .unwrap_or(false)
                {
                    app.manage(snapshade_platform_macos::begin_latency_critical_activity(
                        "Snapshade hotkey latency",
                    ));
                    eprintln!("latency-critical activity held ({ACTIVITY_ENV}=1)");
                }
            }

            // Windows (Phase 5): enumerate monitors, pre-create one hidden overlay window per
            // monitor, and warn when Snipping Tool owns PrintScreen (PKG-05).
            #[cfg(target_os = "windows")]
            {
                use snapshade_platform_api::{DisplayProvider, OverlayHost};
                use snapshade_platform_windows::WindowsDisplayProvider;

                let topology = WindowsDisplayProvider.snapshot()?;
                for d in &topology.displays {
                    eprintln!(
                        "display {} logical=({}, {}, {}, {}) scale={} physical={}x{} primary={}",
                        d.id,
                        d.logical.x,
                        d.logical.y,
                        d.logical.w,
                        d.logical.h,
                        d.scale,
                        d.physical.w,
                        d.physical.h,
                        d.primary
                    );
                }
                let mut host = overlay_win::WinOverlayHost::new(app.handle().clone());
                host.prepare(&topology)?;
                eprintln!("overlays prepared for displays {:?}", host.display_ids());
                app.manage(Arc::new(capture_controller_win::WinCaptureState::new(
                    topology,
                )));
                // Plan 999.4-02: create the DXGI duplication per monitor and acquire its first
                // frame now, off the hotkey path (one `capture display N: dxgi primed` line each).
                {
                    let state = app.state::<Arc<capture_controller_win::WinCaptureState>>();
                    let state = Arc::clone(state.inner());
                    std::thread::spawn(move || state.capturer.prime(&state.topology()));
                }
                app.manage(overlay_win::SharedOverlayHost::new(Mutex::new(host)));
                // The hidden print page, created here and never inside a command (Pitfall 11).
                print_page::prepare(app.handle())?;
                eprintln!("print page prepared");
                // QA hook, the Windows counterpart of the macOS `SNAPSHADE_SELFTEST` loop:
                // `<N>` runs N captures 3 s apart (each dismissed after 1.5 s) and exits —
                // the `latency {…}` rows are the result; `save` runs one instant full-screen
                // save and exits. Needs no keyboard, so it measures unattended.
                if let Ok(mode) = std::env::var("SNAPSHADE_SELFTEST") {
                    capture_controller_win::schedule_selftest(app.handle().clone(), mode);
                }

                if settings.capture_hotkey.code == "PrintScreen"
                    && snapshade_platform_windows::print_screen_owned_by_snipping_tool()
                        == Some(true)
                {
                    let exporters = app.state::<export::Exporters>();
                    if let Err(e) = exporters.notifier.notify(
                        strings::APP_NAME,
                        strings::NOTIFY_PRINTSCREEN_OWNED,
                        NotificationAction::OpenUrl(
                            snapshade_platform_windows::SNIPPING_TOOL_SETTINGS_URL.to_string(),
                        ),
                    ) {
                        eprintln!("notify: {e}");
                    }
                }
            }

            // Hotkeys from Settings (HOTKEY-01/02/04). Registration is dispatched to the main
            // thread by the plugin itself; do not wrap it in `run_on_main_thread` (RESEARCH A12).
            // On failure the app keeps running with the tray fallback and posts the
            // "could not register the hotkey" notification, which opens Options (HOTKEY-03) —
            // except in self-test mode, where exit status 2 is the measurement contract.
            if let Err(err) = hotkeys::apply(app.handle(), &settings) {
                eprintln!("hotkey registration failed: {err}");
                #[cfg(target_os = "macos")]
                if selftest_mode.is_some() {
                    std::process::exit(2);
                }
                let exporters = app.state::<export::Exporters>();
                if let Err(e) = exporters.notifier.notify(
                    strings::APP_NAME,
                    strings::NOTIFY_HOTKEY_FAILED,
                    NotificationAction::OpenOptions,
                ) {
                    eprintln!("notify: {e}");
                }
            }

            #[cfg(target_os = "macos")]
            if let Some(mode) = selftest_mode
                .as_deref()
                .filter(|mode| selftest::needs_app(mode))
            {
                // `<N>` → schedule_run; `exclusion` → schedule_exclusion (SC2 proof).
                selftest::schedule(app.handle().clone(), mode);
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Snapshade");
}

/// `NSApplicationDidChangeScreenParametersNotification` (main thread, bursts).
/// Coalesces with `pending`, then after [`SCREEN_CHANGE_SETTLE`]: re-snapshot the
/// topology and `reprepare` the panels on the main thread, and re-fetch
/// `SCShareableContent` on a worker thread (RESEARCH Q3 line 326 — slow, never on
/// the main thread). Either outcome is logged under `screens changed:` for 01-08.
#[cfg(target_os = "macos")]
fn on_screens_changed(app: tauri::AppHandle, pending: Arc<std::sync::atomic::AtomicBool>) {
    use std::sync::atomic::Ordering;
    if pending.swap(true, Ordering::SeqCst) {
        return; // a re-prepare is already scheduled for this burst
    }
    eprintln!("screens changed: notification received, re-preparing in {SCREEN_CHANGE_SETTLE:?}");
    std::thread::spawn(move || {
        std::thread::sleep(SCREEN_CHANGE_SETTLE);
        pending.store(false, Ordering::SeqCst);
        let handle = app.clone();
        let result =
            app.run_on_main_thread(move || {
                use snapshade_platform_api::DisplayProvider;
                use snapshade_platform_macos::MacDisplayProvider;

                let topology = match MacDisplayProvider.snapshot() {
                    Ok(t) => t,
                    Err(err) => {
                        eprintln!(
                            "screens changed: re-prepare failed: snapshot: {err} — restart the app"
                        );
                        return;
                    }
                };
                for d in &topology.displays {
                    eprintln!(
                    "screens changed: display {} logical=({}, {}, {}, {}) scale={} physical={}x{}",
                    d.id, d.logical.x, d.logical.y, d.logical.w, d.logical.h, d.scale,
                    d.physical.w, d.physical.h
                );
                }
                let state = handle.state::<Arc<capture_controller::CaptureState>>();
                state.set_topology(topology.clone());
                let host = handle.state::<overlay::SharedOverlayHost>();
                let mut host = host.lock().unwrap_or_else(|p| p.into_inner());
                match host.reprepare(&topology) {
                    Ok(n) => eprintln!(
                        "screens changed: re-prepared {n} panels → {:?}",
                        host.display_ids()
                    ),
                    Err(err) => {
                        eprintln!("screens changed: re-prepare failed: {err} — restart the app")
                    }
                }
                // SCShareableContent must be re-fetched after a display change; it is slow.
                let state = state.inner().clone();
                std::thread::spawn(move || {
                    let ok = state.capturer.refresh();
                    eprintln!(
                        "screens changed: shareable content refresh {}",
                        if ok { "ok" } else { "failed" }
                    );
                });
            });
        if let Err(err) = result {
            eprintln!(
                "screens changed: re-prepare failed: run_on_main_thread: {err} — restart the app"
            );
        }
    });
}

/// `overlay-key-equivalent` payload: the same fields the page sends over `overlay_key`.
#[cfg(target_os = "macos")]
#[derive(serde::Serialize, Clone)]
struct KeyEquivalent {
    key: String,
    meta: bool,
    shift: bool,
}

/// The ⌘-letters the overlay owns (docs/SPEC.md §3 and §5): undo, copy, save, print,
/// select-all, close, size up / down. Anything else passes through untouched.
#[cfg(target_os = "macos")]
const OVERLAY_COMMAND_KEYS: &[&str] = &["z", "c", "s", "p", "a", "x", "=", "+", "-"];

/// Local `NSEvent` monitor for key-down events (main thread, process lifetime): a ⌘Z
/// (Command, no Control/Option; Shift reported) whose target window is one of the overlay
/// panels is emitted to that panel's page as `overlay-key-equivalent {key:"z", meta:true,
/// shift}` and consumed (the handler returns null), so the page funnels it into the SAME
/// `overlay_key` invoke and the stderr probe line is byte-identical to a DOM keydown's.
/// Every other event — any other key, ⌘Z while no overlay panel is key — passes through
/// untouched. Rationale: AppKit resolves a Command key equivalent through
/// `performKeyEquivalent:` BEFORE WebKit would create a DOM `keydown`; with no Edit menu
/// the probe (scripts/spike-key-probe.sh) still found no `z meta=true` line, while a local
/// monitor sees the event before that dispatch (01-10 Task 3, measured on this Mac).
/// Phase 2 note: the overlay's undo handler must listen to this event as well as `keydown`.
#[cfg(target_os = "macos")]
fn install_key_equivalent_monitor(app: tauri::AppHandle) {
    use std::ffi::c_void;
    use std::ptr::NonNull;

    use objc2::rc::Retained;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSEvent, NSEventMask, NSEventModifierFlags};
    use tauri::Emitter;

    let block = block2::RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
        // SAFETY: AppKit hands the monitor a live NSEvent for the duration of the call.
        let ev = unsafe { event.as_ref() };
        let flags = ev.modifierFlags();
        let is_command_only = flags.contains(NSEventModifierFlags::Command)
            && !flags.contains(NSEventModifierFlags::Control)
            && !flags.contains(NSEventModifierFlags::Option);
        if !is_command_only {
            return event.as_ptr();
        }
        let Some(chars) = ev
            .charactersIgnoringModifiers()
            .map(|s| s.to_string().to_ascii_lowercase())
        else {
            return event.as_ptr();
        };
        if !OVERLAY_COMMAND_KEYS.contains(&chars.as_str()) {
            return event.as_ptr();
        }
        let Some(mtm) = MainThreadMarker::new() else {
            return event.as_ptr();
        };
        let Some(window) = ev.window(mtm) else {
            return event.as_ptr();
        };
        let ns_window = Retained::as_ptr(&window) as *mut c_void;
        let label = {
            let host = app.state::<overlay::SharedOverlayHost>();
            let host = host.lock().unwrap_or_else(|p| p.into_inner());
            host.label_for_ns_window(ns_window)
        };
        let Some(label) = label else {
            return event.as_ptr();
        };
        let payload = KeyEquivalent {
            key: chars.clone(),
            meta: true,
            shift: flags.contains(NSEventModifierFlags::Shift),
        };
        match app.emit_to(label.as_str(), "overlay-key-equivalent", payload) {
            Ok(()) => eprintln!("key equivalent ⌘{chars} consumed for {label}"),
            Err(err) => eprintln!("key equivalent ⌘{chars} emit to {label} failed: {err}"),
        }
        std::ptr::null_mut()
    });
    // SAFETY: called on the main thread during setup; the handler returns the event
    // pointer it was given or null, as the contract requires. AppKit copies the block.
    let monitor = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &block)
    };
    match monitor {
        Some(monitor) => {
            // The monitor lives for the whole process; keeping the handle out of Tauri's
            // managed state avoids a Send + Sync wrapper for an object never touched again.
            std::mem::forget(monitor);
            std::mem::forget(block);
            eprintln!("key equivalent monitor installed (⌘Z/C/S/P/A/X/+/- → overlay page)");
        }
        None => eprintln!("key equivalent monitor could not be installed"),
    }
}

/// D-09 point (1): the hotkey handler fired at `t0`.
#[cfg(target_os = "macos")]
fn on_hotkey(app: &tauri::AppHandle, t0: Instant) {
    capture_controller::trigger(app.clone(), t0, capture_controller::TriggerSource::Hotkey);
}

/// Windows: GDI capture → per-monitor overlay windows (Phase 5).
#[cfg(target_os = "windows")]
fn on_hotkey(app: &tauri::AppHandle, t0: Instant) {
    capture_controller_win::trigger(app.clone(), t0);
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn on_hotkey(_app: &tauri::AppHandle, t0: Instant) {
    eprintln!("hotkey at {t0:?}: no overlay host on this platform");
}

/// HOTKEY-04: the instant-save-fullscreen hotkey.
#[cfg(target_os = "macos")]
fn on_instant_save(app: &tauri::AppHandle) {
    capture_controller::instant_save(app.clone());
}

#[cfg(target_os = "windows")]
fn on_instant_save(app: &tauri::AppHandle) {
    capture_controller_win::instant_save(app.clone());
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn on_instant_save(_app: &tauri::AppHandle) {
    eprintln!("instant save: not supported on this platform");
}

/// "Take a screenshot" from the tray menu or a UI button — the hotkey's fallback (Pitfall 5).
pub fn start_capture_from_ui(app: &tauri::AppHandle) {
    on_hotkey(app, Instant::now());
}

/// Tray menu (SHELL-02): Take a screenshot, Options, About, Help, Exit.
fn on_menu_event(app: &tauri::AppHandle, event: tauri::menu::MenuEvent) {
    match event.id().as_ref() {
        tray::ID_TAKE => start_capture_from_ui(app),
        tray::ID_OPTIONS => aux_windows::show_later(app, aux_windows::OPTIONS),
        tray::ID_ABOUT => aux_windows::show_later(app, aux_windows::ABOUT),
        tray::ID_HELP => {
            let exporters = app.state::<export::Exporters>();
            if let Err(err) = exporters.shell.open_url(strings::HELP_URL) {
                eprintln!("help: {err}");
            }
        }
        tray::ID_EXIT => app.exit(0),
        other => eprintln!("tray: unknown menu id {other}"),
    }
}

/// The platform exporters. A notification click is routed to the main thread where the
/// Finder reveal / Options window can run.
#[cfg(target_os = "macos")]
fn make_exporters(app: tauri::AppHandle) -> export::Exporters {
    use snapshade_platform_macos::{MacClipboard, MacNotifier, MacPrinter, MacShell};
    let click_app = app.clone();
    let on_click: snapshade_platform_macos::ClickHandler = Arc::new(move |action| {
        let handle = click_app.clone();
        if let Err(err) =
            click_app.run_on_main_thread(move || export::on_notification_click(&handle, action))
        {
            eprintln!("notification click: {err}");
        }
    });
    let notifier: Box<dyn snapshade_platform_api::Notifier + Send + Sync> =
        match MacNotifier::new(on_click) {
            Ok(n) => Box::new(n),
            Err(err) => {
                eprintln!("notifications unavailable: {err}");
                Box::new(export::LogNotifier)
            }
        };
    export::Exporters {
        clipboard: Box::new(MacClipboard),
        printer: Box::new(MacPrinter),
        shell: Box::new(MacShell),
        notifier,
    }
}

/// Windows exporters: Win32 clipboard, Explorer/ShellExecute shell, WinRT toasts under the
/// bundle identifier as AUMID (the NSIS Start Menu shortcut carries it), webview print page.
#[cfg(target_os = "windows")]
fn make_exporters(app: tauri::AppHandle) -> export::Exporters {
    use snapshade_platform_windows::{
        ClickHandler, WindowsClipboard, WindowsNotifier, WindowsShell,
    };
    let click_app = app.clone();
    let on_click: ClickHandler = Arc::new(move |action| {
        let handle = click_app.clone();
        if let Err(err) =
            click_app.run_on_main_thread(move || export::on_notification_click(&handle, action))
        {
            eprintln!("notification click: {err}");
        }
    });
    let aumid = app.config().identifier.clone();
    export::Exporters {
        clipboard: Box::new(WindowsClipboard),
        printer: Box::new(print_page::WebviewPrinter::new(app.clone())),
        shell: Box::new(WindowsShell),
        notifier: Box::new(WindowsNotifier::new(&aumid, on_click)),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn make_exporters(_app: tauri::AppHandle) -> export::Exporters {
    export::Exporters::unsupported()
}
