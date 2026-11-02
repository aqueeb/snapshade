//! User settings (the Options dialog: General / Hotkeys / Formats).
//!
//! Pure data in Phase 1 (D-01): serde (de)serialisation and defaults only, no
//! file I/O. Persistence arrives with the Options dialog in Phase 4.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::encode::ImageFormat;
use crate::hotkey::KeyCombo;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub show_notifications: bool,
    pub capture_cursor: bool,
    pub keep_selection_position: bool,
    pub capture_hotkey: KeyCombo,
    pub instant_save_enabled: bool,
    pub instant_save_hotkey: Option<KeyCombo>,
    pub default_format: ImageFormat,
    pub jpeg_quality: u8,
    pub optimize_png: bool,
    pub default_folder: Option<PathBuf>,
    /// Folder of the last Save As (EXPORT-03: ⌘S saves here first, then `default_folder`).
    pub last_save_folder: Option<PathBuf>,
    /// SHELL-04: login item / Run key.
    pub start_at_login: bool,
    /// OPT-01 Language (BCP-47); only `en` ships in v1 (OPT-06).
    pub language: String,
    /// The current annotation colour and marker colour (one colour shared by all tools).
    pub color: String,
    pub marker_color: String,
    /// Stroke size (wheel-adjusted), persisted so the next capture starts where you left off.
    pub tool_size: u32,
}

impl Settings {
    /// The folder ⌘S saves to: the last Save As folder, else the configured default, else
    /// `fallback` (the OS default the app computes at runtime).
    pub fn save_folder(&self, fallback: PathBuf) -> PathBuf {
        self.last_save_folder
            .clone()
            .or_else(|| self.default_folder.clone())
            .unwrap_or(fallback)
    }

    /// The file extension of `default_format`.
    pub fn default_extension(&self) -> &'static str {
        match self.default_format {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg { .. } => "jpg",
            ImageFormat::Bmp => "bmp",
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            show_notifications: true,
            capture_cursor: false,
            keep_selection_position: false,
            // ⌘⇧9 on macOS (kVK_ANSI_9 + Shift + Command); PrintScreen on Windows, the key
            // screenshot tools conventionally own there. The macOS combo was hard-coded here
            // until the 999.3 hands-on run: Win+Shift+9 is a taskbar shortcut on Windows and
            // `RegisterHotKey` refused it, so a fresh install had no hotkey at all.
            capture_hotkey: KeyCombo::platform_default(),
            instant_save_enabled: false,
            instant_save_hotkey: None,
            default_format: ImageFormat::Png,
            jpeg_quality: 90,
            optimize_png: false,
            default_folder: None,
            last_save_folder: None,
            start_at_login: false,
            language: "en".to_string(),
            color: "#ff0000".to_string(),
            marker_color: "#ffff00".to_string(),
            tool_size: 3,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Settings;

    #[test]
    fn defaults_round_trip_through_json() {
        let settings = Settings::default();
        let json = serde_json::to_string(&settings).expect("serialize");
        let back: Settings = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(settings, back);
    }

    #[test]
    fn missing_keys_fall_back_to_defaults() {
        let partial: Settings = serde_json::from_str(r#"{"jpeg_quality": 75}"#).expect("parse");
        assert_eq!(partial.jpeg_quality, 75);
        assert_eq!(partial.capture_hotkey, Settings::default().capture_hotkey);
    }

    #[test]
    fn save_folder_prefers_last_then_default_then_fallback() {
        use std::path::PathBuf;
        let mut s = Settings::default();
        assert_eq!(s.save_folder(PathBuf::from("/fb")), PathBuf::from("/fb"));
        s.default_folder = Some(PathBuf::from("/def"));
        assert_eq!(s.save_folder(PathBuf::from("/fb")), PathBuf::from("/def"));
        s.last_save_folder = Some(PathBuf::from("/last"));
        assert_eq!(s.save_folder(PathBuf::from("/fb")), PathBuf::from("/last"));
        assert_eq!(s.default_extension(), "png");
    }
}
