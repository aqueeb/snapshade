//! Platform-neutral hotkey model: parse and format the `'+'`-separated string
//! form used by `tauri-plugin-global-shortcut` (RESEARCH Q7) and the Options
//! dialog, with the W3C `KeyboardEvent.code` vocabulary for the key.

use serde::{Deserialize, Serialize};

/// `CommandOrControl` resolves to ⌘ on macOS and Ctrl everywhere else, and
/// `format` renders that platform-primary modifier back as `CommandOrControl`.
const PRIMARY_IS_COMMAND: bool = cfg!(target_os = "macos");

/// Modifier keys. `command` is ⌘ on macOS and Win on Windows; `alt` is ⌥/Alt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Modifiers {
    pub command: bool,
    pub control: bool,
    pub shift: bool,
    pub alt: bool,
}

/// A key plus modifiers.
///
/// `code` is a W3C `KeyboardEvent.code` value (e.g. `"Digit9"`, `"PrintScreen"`),
/// so the same combo round-trips between the webview and the OS adapters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyCombo {
    pub code: String,
    pub mods: Modifiers,
}

impl KeyCombo {
    /// Parse `"CommandOrControl+Shift+9"`-style strings.
    ///
    /// Grammar: `'+'`-separated tokens; every token but the last is a modifier
    /// (`Command|Cmd|Super`, `Control|Ctrl`, `CommandOrControl`, `Shift`,
    /// `Alt|Option`, case-insensitive, each at most once; `CommandOrControl`
    /// is ⌘ on macOS and Ctrl elsewhere); the last token is the key: a digit →
    /// `Digit<n>`, a letter → `Key<X>`, `PrintScreen`, or `F1`–`F24`.
    /// Anything else is `HotkeyError::Invalid`.
    pub fn parse(s: &str) -> Result<KeyCombo, HotkeyError> {
        let invalid = || HotkeyError::Invalid(s.to_string());
        let tokens: Vec<&str> = s.split('+').map(str::trim).collect();
        let (key, modifier_tokens) = tokens.split_last().ok_or_else(invalid)?;

        let mut mods = Modifiers::default();
        for token in modifier_tokens {
            let flag = match token.to_ascii_lowercase().as_str() {
                "command" | "cmd" | "super" => &mut mods.command,
                "control" | "ctrl" => &mut mods.control,
                "commandorcontrol" | "cmdorctrl" => {
                    if PRIMARY_IS_COMMAND {
                        &mut mods.command
                    } else {
                        &mut mods.control
                    }
                }
                "shift" => &mut mods.shift,
                "alt" | "option" => &mut mods.alt,
                _ => return Err(invalid()),
            };
            if *flag {
                return Err(invalid());
            }
            *flag = true;
        }

        let code = key_code(key).ok_or_else(invalid)?;
        Ok(KeyCombo { code, mods })
    }

    /// Canonical string form: platform primary modifier as `CommandOrControl`
    /// (⌘ on macOS, Ctrl on Windows), the other as `Control`/`Super`, then
    /// `Shift`, `Alt`, then the key with `Digit<n>` → `n` and `Key<X>` → `X`.
    pub fn format(&self) -> String {
        let (primary, secondary, secondary_name) = if PRIMARY_IS_COMMAND {
            (self.mods.command, self.mods.control, "Control")
        } else {
            (self.mods.control, self.mods.command, "Super")
        };
        let mut parts: Vec<&str> = Vec::with_capacity(5);
        if primary {
            parts.push("CommandOrControl");
        }
        if secondary {
            parts.push(secondary_name);
        }
        if self.mods.shift {
            parts.push("Shift");
        }
        if self.mods.alt {
            parts.push("Alt");
        }
        let key = self
            .code
            .strip_prefix("Digit")
            .or_else(|| self.code.strip_prefix("Key"))
            .unwrap_or(&self.code);
        parts.push(key);
        parts.join("+")
    }

    /// Human-readable form for the tray tooltip / Options recorder: macOS symbols
    /// (`⌃⌥⇧⌘9`, Apple's modifier order) when `macos_symbols`, else Windows words
    /// (`Ctrl+Shift+9`, `PrintScreen` → `PrtScr`-free: the spec's `%hotkey_main%`
    /// keeps the full word).
    pub fn display(&self, macos_symbols: bool) -> String {
        let key = self
            .code
            .strip_prefix("Digit")
            .or_else(|| self.code.strip_prefix("Key"))
            .unwrap_or(&self.code);
        if macos_symbols {
            let mut out = String::new();
            if self.mods.control {
                out.push('⌃');
            }
            if self.mods.alt {
                out.push('⌥');
            }
            if self.mods.shift {
                out.push('⇧');
            }
            if self.mods.command {
                out.push('⌘');
            }
            out.push_str(key);
            out
        } else {
            let mut parts: Vec<&str> = Vec::with_capacity(5);
            if self.mods.control {
                parts.push("Ctrl");
            }
            if self.mods.command {
                parts.push("Win");
            }
            if self.mods.alt {
                parts.push("Alt");
            }
            if self.mods.shift {
                parts.push("Shift");
            }
            parts.push(key);
            parts.join("+")
        }
    }

    /// The platform default: ⌘⇧9 on macOS, PrintScreen elsewhere.
    pub fn platform_default() -> KeyCombo {
        if PRIMARY_IS_COMMAND {
            Self::default_macos()
        } else {
            Self::default_windows()
        }
    }

    /// A combo with no modifier that is not a dedicated key (PrintScreen / F-keys) would
    /// swallow ordinary typing — the `HOTKEY_INVALID` case in Options.
    pub fn is_valid_binding(&self) -> bool {
        let dedicated = self.code == "PrintScreen" || self.code.starts_with('F');
        dedicated || self.mods.command || self.mods.control || self.mods.alt
    }

    /// ⌘⇧9 — the macOS default (docs/SPEC.md §3).
    pub fn default_macos() -> KeyCombo {
        KeyCombo {
            code: "Digit9".to_string(),
            mods: Modifiers {
                command: true,
                control: false,
                shift: true,
                alt: false,
            },
        }
    }

    /// PrintScreen — the Windows default (docs/SPEC.md §3).
    pub fn default_windows() -> KeyCombo {
        KeyCombo {
            code: "PrintScreen".to_string(),
            mods: Modifiers::default(),
        }
    }
}

/// Map the final token of a hotkey string to a W3C `KeyboardEvent.code`.
fn key_code(token: &str) -> Option<String> {
    match token.as_bytes() {
        [d @ b'0'..=b'9'] => Some(format!("Digit{}", *d as char)),
        [c] if c.is_ascii_alphabetic() => Some(format!("Key{}", c.to_ascii_uppercase() as char)),
        _ if token.eq_ignore_ascii_case("printscreen") => Some("PrintScreen".to_string()),
        _ => {
            let n = token.strip_prefix(['F', 'f'])?;
            if n.is_empty() || n.len() > 2 || !n.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let n: u8 = n.parse().ok()?;
            (1..=24).contains(&n).then(|| format!("F{n}"))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HotkeyError {
    #[error("invalid hotkey: {0}")]
    Invalid(String),
    #[error("the same hotkey is assigned to two functions")]
    Duplicate,
}

#[cfg(test)]
mod tests {
    use super::{HotkeyError, KeyCombo, Modifiers};

    #[test]
    fn command_or_control_round_trips() {
        let combo = KeyCombo::parse("CommandOrControl+Shift+9").expect("parses");
        assert_eq!(combo.code, "Digit9");
        assert!(combo.mods.shift);
        assert!(combo.mods.command || combo.mods.control);
        assert_eq!(combo.format(), "CommandOrControl+Shift+9");
    }

    #[test]
    fn command_shift_9_sets_command_and_shift() {
        let combo = KeyCombo::parse("Command+Shift+9").expect("parses");
        assert_eq!(combo.code, "Digit9");
        assert_eq!(
            combo.mods,
            Modifiers {
                command: true,
                control: false,
                shift: true,
                alt: false
            }
        );
    }

    #[test]
    fn modifier_aliases_and_case_are_accepted() {
        let combo = KeyCombo::parse("ctrl+option+a").expect("parses");
        assert_eq!(combo.code, "KeyA");
        assert!(combo.mods.control && combo.mods.alt);
        assert!(!combo.mods.command && !combo.mods.shift);
        let combo = KeyCombo::parse("Super+F12").expect("parses");
        assert_eq!(combo.code, "F12");
        assert!(combo.mods.command);
    }

    #[test]
    fn print_screen_alone_has_no_modifiers() {
        let combo = KeyCombo::parse("PrintScreen").expect("parses");
        assert_eq!(combo.code, "PrintScreen");
        assert_eq!(combo.mods, Modifiers::default());
        assert_eq!(combo.format(), "PrintScreen");
    }

    #[test]
    fn invalid_strings_are_rejected() {
        for s in [
            "Shift+Shift+9",
            "",
            "Command+",
            "+9",
            "Command+Shift",
            "Hyper+9",
            "F25",
            "99",
        ] {
            assert!(
                matches!(KeyCombo::parse(s), Err(HotkeyError::Invalid(_))),
                "{s:?} should be invalid"
            );
        }
    }

    #[test]
    fn formatted_keys_are_bare_digits_and_letters() {
        let combo = KeyCombo::parse("Shift+Alt+B").expect("parses");
        assert_eq!(combo.format(), "Shift+Alt+B");
        let combo = KeyCombo::parse("Control+Shift+F3").expect("parses");
        assert!(combo.format().ends_with("Shift+F3"));
    }

    #[test]
    fn display_renders_mac_symbols_and_windows_words() {
        let combo = KeyCombo::default_macos();
        assert_eq!(combo.display(true), "⇧⌘9");
        assert_eq!(combo.display(false), "Win+Shift+9");
        let combo = KeyCombo::parse("Control+Alt+Shift+A").expect("parses");
        assert_eq!(combo.display(true), "⌃⌥⇧A");
        assert_eq!(combo.display(false), "Ctrl+Alt+Shift+A");
        assert_eq!(KeyCombo::default_windows().display(false), "PrintScreen");
        assert_eq!(KeyCombo::parse("F13").unwrap().display(true), "F13");
    }

    #[test]
    fn bindings_without_a_modifier_are_invalid_unless_dedicated() {
        assert!(KeyCombo::default_macos().is_valid_binding());
        assert!(KeyCombo::default_windows().is_valid_binding());
        assert!(KeyCombo::parse("F5").unwrap().is_valid_binding());
        assert!(!KeyCombo::parse("Shift+9").unwrap().is_valid_binding());
        assert!(!KeyCombo::parse("A").unwrap().is_valid_binding());
        assert!(KeyCombo::parse("Alt+A").unwrap().is_valid_binding());
    }

    #[test]
    fn platform_defaults_match_the_spec() {
        assert_eq!(
            KeyCombo::default_macos(),
            KeyCombo {
                code: "Digit9".to_string(),
                mods: Modifiers {
                    command: true,
                    control: false,
                    shift: true,
                    alt: false
                }
            }
        );
        assert_eq!(
            KeyCombo::default_windows(),
            KeyCombo {
                code: "PrintScreen".to_string(),
                mods: Modifiers::default()
            }
        );
    }
}
