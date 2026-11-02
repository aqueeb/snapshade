//! Global hotkeys from `Settings` (HOTKEY-01..04): the main capture key and the optional
//! "Instant full-screen save" key, re-registered live when Options changes them
//! (HOTKEY-02) with the validation messages (OPT-02) and the "could not register the
//! hotkey" notification that opens Options on click (HOTKEY-03).

use std::str::FromStr;
use std::sync::{Arc, Mutex};

use snapshade_core::hotkey::KeyCombo;
use snapshade_core::settings::Settings;
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut};

use crate::strings;

/// Which registered hotkey fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyRole {
    Capture,
    InstantSave,
}

/// Managed state: the ids of the shortcuts currently registered, so the single plugin
/// handler can map a firing shortcut back to its role.
#[derive(Default)]
pub struct Registered {
    capture: Option<u32>,
    instant: Option<u32>,
}

pub type SharedRegistered = Arc<Mutex<Registered>>;

/// `KeyCombo` → plugin `Shortcut`. `Err` for a key code the plugin cannot bind (the
/// Options recorder only produces supported codes, so this is defensive).
pub fn shortcut_for(combo: &KeyCombo) -> Result<Shortcut, String> {
    let code =
        Code::from_str(&combo.code).map_err(|_| format!("unsupported key {}", combo.code))?;
    let mut mods = Modifiers::empty();
    if combo.mods.command {
        mods |= Modifiers::SUPER;
    }
    if combo.mods.control {
        mods |= Modifiers::CONTROL;
    }
    if combo.mods.shift {
        mods |= Modifiers::SHIFT;
    }
    if combo.mods.alt {
        mods |= Modifiers::ALT;
    }
    Ok(Shortcut::new(
        if mods.is_empty() { None } else { Some(mods) },
        code,
    ))
}

/// OPT-02 validation, before anything touches the OS: an invalid binding
/// (`HOTKEY_INVALID`) or the same combo on both functions (`HOTKEY_DUPLICATE`).
pub fn validate(settings: &Settings) -> Result<(), String> {
    if !settings.capture_hotkey.is_valid_binding() {
        return Err(strings::HOTKEY_INVALID.to_string());
    }
    shortcut_for(&settings.capture_hotkey).map_err(|_| strings::HOTKEY_INVALID.to_string())?;
    if settings.instant_save_enabled {
        let Some(instant) = &settings.instant_save_hotkey else {
            return Err(strings::HOTKEY_INVALID.to_string());
        };
        if !instant.is_valid_binding() {
            return Err(strings::HOTKEY_INVALID.to_string());
        }
        shortcut_for(instant).map_err(|_| strings::HOTKEY_INVALID.to_string())?;
        if *instant == settings.capture_hotkey {
            return Err(strings::HOTKEY_DUPLICATE.to_string());
        }
    }
    Ok(())
}

/// Role of a shortcut that just fired, by id.
pub fn role_of<R: Runtime>(app: &AppHandle<R>, shortcut: &Shortcut) -> Option<HotkeyRole> {
    let registered = app.state::<SharedRegistered>();
    let registered = registered.lock().unwrap_or_else(|p| p.into_inner());
    let id = shortcut.id();
    if registered.capture == Some(id) {
        Some(HotkeyRole::Capture)
    } else if registered.instant == Some(id) {
        Some(HotkeyRole::InstantSave)
    } else {
        None
    }
}

/// (Re)register everything from `settings`: unregister all, then the capture key and,
/// when enabled, the instant-save key. On an OS refusal the error text is returned (for
/// the Options UI) and nothing stays registered — the tray "Take a screenshot" is the
/// fallback the roadmap requires (Pitfall 5). The caller decides whether to notify.
pub fn apply<R: Runtime>(app: &AppHandle<R>, settings: &Settings) -> Result<(), String> {
    validate(settings)?;
    let gs = app.global_shortcut();
    if let Err(err) = gs.unregister_all() {
        eprintln!("hotkeys: unregister_all: {err}");
    }
    {
        let registered = app.state::<SharedRegistered>();
        *registered.lock().unwrap_or_else(|p| p.into_inner()) = Registered::default();
    }
    let capture = shortcut_for(&settings.capture_hotkey)?;
    gs.register(capture).map_err(|e| e.to_string())?;
    let mut instant_id = None;
    if settings.instant_save_enabled {
        if let Some(combo) = &settings.instant_save_hotkey {
            let instant = shortcut_for(combo)?;
            if let Err(err) = gs.register(instant) {
                let _ = gs.unregister_all();
                return Err(err.to_string());
            }
            instant_id = Some(instant.id());
        }
    }
    let registered = app.state::<SharedRegistered>();
    *registered.lock().unwrap_or_else(|p| p.into_inner()) = Registered {
        capture: Some(capture.id()),
        instant: instant_id,
    };
    eprintln!(
        "hotkeys: registered capture={} instant={}",
        settings.capture_hotkey.format(),
        settings
            .instant_save_hotkey
            .as_ref()
            .filter(|_| settings.instant_save_enabled)
            .map(|c| c.format())
            .unwrap_or_else(|| "off".to_string())
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{shortcut_for, validate};
    use snapshade_core::hotkey::KeyCombo;
    use snapshade_core::settings::Settings;
    use tauri_plugin_global_shortcut::{Code, Modifiers};

    #[test]
    fn default_combo_maps_to_super_shift_digit9_on_macos() {
        let s = shortcut_for(&KeyCombo::default_macos()).unwrap();
        assert_eq!(s.key, Code::Digit9);
        assert!(s.mods.contains(Modifiers::SUPER | Modifiers::SHIFT));
        let p = shortcut_for(&KeyCombo::default_windows()).unwrap();
        assert_eq!(p.key, Code::PrintScreen);
        assert!(p.mods.is_empty());
    }

    #[test]
    fn validation_returns_the_options_messages() {
        let mut s = Settings::default();
        assert!(validate(&s).is_ok());
        s.instant_save_enabled = true;
        s.instant_save_hotkey = Some(s.capture_hotkey.clone());
        assert_eq!(validate(&s).unwrap_err(), crate::strings::HOTKEY_DUPLICATE);
        s.instant_save_hotkey = Some(KeyCombo::parse("Shift+9").unwrap());
        assert_eq!(validate(&s).unwrap_err(), crate::strings::HOTKEY_INVALID);
        s.instant_save_hotkey = None;
        assert_eq!(validate(&s).unwrap_err(), crate::strings::HOTKEY_INVALID);
        s.instant_save_enabled = false;
        s.capture_hotkey = KeyCombo::parse("A").unwrap();
        assert_eq!(validate(&s).unwrap_err(), crate::strings::HOTKEY_INVALID);
    }
}
