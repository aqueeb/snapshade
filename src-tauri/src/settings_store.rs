//! Settings persistence (OPT-04): one JSON file in the app config dir, written whole on
//! every change. Missing or unreadable file → defaults (never a crash); unknown keys are
//! ignored and missing keys default (`#[serde(default)]` on `Settings`).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use snapshade_core::settings::Settings;
use tauri::{AppHandle, Manager, Runtime};

pub const FILE_NAME: &str = "settings.json";

/// Managed state: the live settings, shared by the hotkey handler, the exporters and the
/// Options window.
pub type SharedSettings = Arc<RwLock<Settings>>;

/// Where the settings file lives (`~/Library/Application Support/<bundle id>/settings.json`
/// on macOS, `%APPDATA%\<bundle id>\settings.json` on Windows).
pub fn settings_path<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join(FILE_NAME))
}

pub fn load(path: &Path) -> Settings {
    match fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<Settings>(&text) {
            Ok(s) => s,
            Err(err) => {
                eprintln!(
                    "settings: {} unreadable ({err}); using defaults",
                    path.display()
                );
                Settings::default()
            }
        },
        Err(_) => Settings::default(),
    }
}

pub fn save(path: &Path, settings: &Settings) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    // Write-then-rename so a crash mid-write never leaves a truncated file.
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json)?;
    fs::rename(&tmp, path)
}

/// Read-locked clone of the live settings.
pub fn current<R: Runtime>(app: &AppHandle<R>) -> Settings {
    app.state::<SharedSettings>()
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
}

/// Mutate the live settings and persist them. Errors are logged, never fatal.
pub fn update<R: Runtime>(app: &AppHandle<R>, f: impl FnOnce(&mut Settings)) -> Settings {
    let state = app.state::<SharedSettings>();
    let snapshot = {
        let mut guard = state.write().unwrap_or_else(|p| p.into_inner());
        f(&mut guard);
        guard.clone()
    };
    if let Some(path) = settings_path(app) {
        if let Err(err) = save(&path, &snapshot) {
            eprintln!("settings: save to {} failed: {err}", path.display());
        }
    }
    snapshot
}

#[cfg(test)]
mod tests {
    use super::{load, save};
    use snapshade_core::settings::Settings;

    #[test]
    fn round_trips_and_defaults_on_garbage() {
        let dir = std::env::temp_dir().join(format!("snapshade-settings-{}", std::process::id()));
        let path = dir.join("settings.json");
        let s = Settings {
            jpeg_quality: 42,
            show_notifications: false,
            ..Settings::default()
        };
        save(&path, &s).unwrap();
        assert_eq!(load(&path), s);
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(load(&path), Settings::default());
        assert_eq!(load(&dir.join("missing.json")), Settings::default());
        let _ = std::fs::remove_dir_all(dir);
    }
}
