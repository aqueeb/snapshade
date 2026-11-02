//! The regular (non-overlay) windows: Options, About and the Screen Recording guidance
//! (CAPTURE-03). Each is created on demand and re-used. Every call runs on the main thread.
//!
//! Activation: a regular window needs the app to be active to become key. Under the
//! Accessory policy macOS 14+ refuses programmatic activation that is not user-initiated,
//! so the policy is switched to Regular (Dock icon, ⌘-tab entry) while a regular window is
//! open — the pattern menu-bar apps use for their settings windows — and restored to
//! Accessory when the last one closes.

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent, Wry};

pub const OPTIONS: &str = "options";
pub const ABOUT: &str = "about";
pub const PERMISSION: &str = "permission";

struct Spec {
    label: &'static str,
    page: &'static str,
    title: &'static str,
    w: f64,
    h: f64,
    resizable: bool,
}

const SPECS: &[Spec] = &[
    Spec {
        label: OPTIONS,
        page: "options.html",
        title: "Options",
        w: 520.0,
        h: 460.0,
        resizable: false,
    },
    Spec {
        label: ABOUT,
        page: "about.html",
        title: "About Snapshade",
        w: 380.0,
        h: 320.0,
        resizable: false,
    },
    Spec {
        label: PERMISSION,
        page: "permission.html",
        title: "Snapshade",
        w: 480.0,
        h: 300.0,
        resizable: false,
    },
];

/// Show (creating if needed) the window `label`. Must run on the main thread.
pub fn show(app: &AppHandle<Wry>, label: &str) -> Result<(), String> {
    let spec = SPECS
        .iter()
        .find(|s| s.label == label)
        .ok_or_else(|| format!("unknown window {label}"))?;
    let window = match app.get_webview_window(label) {
        Some(w) => w,
        None => {
            let window =
                WebviewWindowBuilder::new(app, spec.label, WebviewUrl::App(spec.page.into()))
                    .title(spec.title)
                    .inner_size(spec.w, spec.h)
                    .resizable(spec.resizable)
                    .minimizable(false)
                    .center()
                    .visible(false)
                    .build()
                    .map_err(|e| format!("{label}: build: {e}"))?;
            // Back to the menu-bar-only policy once the last regular window is gone.
            let handle = app.clone();
            window.on_window_event(move |event| {
                if matches!(
                    event,
                    WindowEvent::CloseRequested { .. } | WindowEvent::Destroyed
                ) {
                    let inner = handle.clone();
                    let _ = handle.run_on_main_thread(move || {
                        // The closing window is still "visible" during CloseRequested;
                        // re-check on the next turn of the loop.
                        let again = inner.clone();
                        std::thread::spawn(move || {
                            std::thread::sleep(std::time::Duration::from_millis(150));
                            let final_handle = again.clone();
                            let _ = again.run_on_main_thread(move || {
                                restore_accessory_if_idle(&final_handle)
                            });
                        });
                    });
                }
            });
            window
        }
    };
    #[cfg(target_os = "macos")]
    {
        if let Err(err) = app.set_activation_policy(tauri::ActivationPolicy::Regular) {
            eprintln!("{label}: activation policy: {err}");
        }
        if let Err(err) = snapshade_platform_macos::activate_app() {
            eprintln!("{label}: activate: {err}");
        }
    }
    window.show().map_err(|e| format!("{label}: show: {e}"))?;
    window
        .set_focus()
        .map_err(|e| format!("{label}: focus: {e}"))?;
    Ok(())
}

/// Switch back to Accessory when no regular window is visible any more (main thread).
pub fn restore_accessory_if_idle(app: &AppHandle<Wry>) {
    let any_visible = SPECS.iter().any(|spec| {
        app.get_webview_window(spec.label)
            .and_then(|w| w.is_visible().ok())
            .unwrap_or(false)
    });
    #[cfg(target_os = "macos")]
    if !any_visible {
        let restored = app.set_activation_policy(tauri::ActivationPolicy::Accessory);
        if let Err(err) = restored {
            eprintln!("activation policy restore: {err}");
        }
    }
    // Nothing to restore elsewhere (no activation policy); the scan above keeps the
    // function's shape identical on every OS. Written this way because an early
    // `return` here is clippy's needless_return on non-macOS targets.
    #[cfg(not(target_os = "macos"))]
    let _ = any_visible;
}

/// Dispatch `show` to the main thread from anywhere.
pub fn show_later(app: &AppHandle<Wry>, label: &'static str) {
    let handle = app.clone();
    let result = app.run_on_main_thread(move || {
        if let Err(err) = show(&handle, label) {
            eprintln!("window {label}: {err}");
        }
    });
    if let Err(err) = result {
        eprintln!("window {label}: run_on_main_thread: {err}");
    }
}
