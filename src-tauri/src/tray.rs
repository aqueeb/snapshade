//! Menu-bar / tray icon (SHELL-01, SHELL-02): a template icon whose tooltip names the
//! current capture hotkey, and the five menu items (docs/SPEC.md §2).

use snapshade_core::hotkey::KeyCombo;
use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Runtime};

use crate::strings;

pub const TRAY_ID: &str = "snapshade-tray";
pub const ID_TAKE: &str = "take";
pub const ID_OPTIONS: &str = "options";
pub const ID_ABOUT: &str = "about";
pub const ID_HELP: &str = "help";
pub const ID_EXIT: &str = "exit";

/// The template icon: black glyph on transparent, recoloured by the OS for light/dark menu bars.
const ICON_PNG: &[u8] = include_bytes!("../icons/tray.png");

/// Human-readable hotkey for the tooltip (`⇧⌘9` on macOS, `PrintScreen` on Windows).
pub fn hotkey_label(combo: &KeyCombo) -> String {
    combo.display(cfg!(target_os = "macos"))
}

pub fn build<R: Runtime>(
    app: &AppHandle<R>,
    hotkey: &KeyCombo,
    on_menu: impl Fn(&AppHandle<R>, MenuEvent) + Send + Sync + 'static,
) -> tauri::Result<TrayIcon<R>> {
    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(
                app,
                ID_TAKE,
                strings::MENU_TAKE_SCREENSHOT,
                true,
                None::<&str>,
            )?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, ID_OPTIONS, strings::MENU_OPTIONS, true, None::<&str>)?,
            &MenuItem::with_id(app, ID_ABOUT, strings::MENU_ABOUT, true, None::<&str>)?,
            &MenuItem::with_id(app, ID_HELP, strings::MENU_HELP, true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, ID_EXIT, strings::MENU_EXIT, true, None::<&str>)?,
        ],
    )?;
    let icon = tauri::image::Image::from_bytes(ICON_PNG)?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .icon_as_template(true)
        .tooltip(strings::tray_tooltip(&hotkey_label(hotkey)))
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(on_menu)
        .build(app)
}

/// Refresh the tooltip after the hotkey changed in Options.
pub fn update_tooltip<R: Runtime>(app: &AppHandle<R>, hotkey: &KeyCombo) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        if let Err(err) = tray.set_tooltip(Some(strings::tray_tooltip(&hotkey_label(hotkey)))) {
            eprintln!("tray: set_tooltip failed: {err}");
        }
    }
}
