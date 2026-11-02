//! Rust-side user-facing strings (OPT-06). The webview layer has its own table in
//! `ui/src/strings/en.ts`; both are English in v1 and keyed so a later language is a
//! second table, not a code change.

pub const APP_NAME: &str = "Snapshade";

/// Tray tooltip, e.g. `Press ⇧⌘9 to take a screenshot`.
pub fn tray_tooltip(hotkey: &str) -> String {
    format!("Press {hotkey} to take a screenshot")
}

pub const MENU_TAKE_SCREENSHOT: &str = "Take a screenshot";
pub const MENU_OPTIONS: &str = "Options";
pub const MENU_ABOUT: &str = "About";
pub const MENU_HELP: &str = "Help";
pub const MENU_EXIT: &str = "Exit";

pub const NOTIFY_COPIED: &str = "Screenshot copied to the clipboard";
/// `Saved as <filename>. Click to show it in the folder.`
pub fn notify_saved(filename: &str) -> String {
    format!("Saved as {filename}. Click to show it in the folder.")
}
pub const NOTIFY_HOTKEY_FAILED: &str =
    "Could not register the hotkey; another application is probably using it. Click this message to choose a different one in Options.";

/// PKG-05: Windows 11 routes PrintScreen to Snipping Tool when this setting is on.
pub const NOTIFY_PRINTSCREEN_OWNED: &str =
    "Windows is using the Print Screen key for Snipping Tool, so Snapshade cannot receive it. Click here to open the setting and turn off \"Use the Print screen key to open screen capture\", or choose another hotkey in Options.";

pub const HOTKEY_INVALID: &str =
    "That key combination cannot be used as a hotkey. Choose another one.";
pub const HOTKEY_DUPLICATE: &str =
    "Both functions would use the same key combination. Choose another one.";

pub const SAVE_DIALOG_TITLE: &str = "Save screenshot";
pub const FOLDER_DIALOG_TITLE: &str = "Default save folder";

/// README anchor opened by Help (the OS browser does the networking, not the app).
pub const HELP_URL: &str = "https://github.com/aqueeb/snapshade#readme";
