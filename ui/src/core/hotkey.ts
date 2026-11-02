// KeyCombo — TypeScript mirror of `crates/core/src/hotkey.rs` for the Options recorder:
// the same `KeyboardEvent.code` vocabulary, the same display rules (`⇧⌘9` / `Ctrl+Shift+9`),
// the same validity rule (a bare letter/digit is not a hotkey; PrintScreen and F-keys are).
export interface Modifiers {
  command: boolean;
  control: boolean;
  shift: boolean;
  alt: boolean;
}

export interface KeyCombo {
  code: string;
  mods: Modifiers;
}

const KEY_CODE = /^(Digit[0-9]|Key[A-Z]|F([1-9]|1[0-9]|2[0-4])|PrintScreen)$/;

/** `true` for a `KeyboardEvent.code` the Rust side can bind. */
export function isSupportedCode(code: string): boolean {
  return KEY_CODE.test(code);
}

/** Build a combo from a keydown; `null` for a modifier-only or unsupported key. */
export function comboFromEvent(e: { code: string; metaKey: boolean; ctrlKey: boolean; shiftKey: boolean; altKey: boolean }): KeyCombo | null {
  if (!isSupportedCode(e.code)) return null;
  return { code: e.code, mods: { command: e.metaKey, control: e.ctrlKey, shift: e.shiftKey, alt: e.altKey } };
}

/** @see crates/core/src/hotkey.rs `KeyCombo::is_valid_binding` */
export function isValidBinding(c: KeyCombo): boolean {
  const dedicated = c.code === "PrintScreen" || c.code.startsWith("F");
  return dedicated || c.mods.command || c.mods.control || c.mods.alt;
}

/** @see crates/core/src/hotkey.rs `KeyCombo::display` */
export function displayCombo(c: KeyCombo, macosSymbols: boolean): string {
  const key = c.code.replace(/^Digit/, "").replace(/^Key/, "");
  if (macosSymbols) {
    return `${c.mods.control ? "⌃" : ""}${c.mods.alt ? "⌥" : ""}${c.mods.shift ? "⇧" : ""}${c.mods.command ? "⌘" : ""}${key}`;
  }
  const parts: string[] = [];
  if (c.mods.control) parts.push("Ctrl");
  if (c.mods.command) parts.push("Win");
  if (c.mods.alt) parts.push("Alt");
  if (c.mods.shift) parts.push("Shift");
  parts.push(key);
  return parts.join("+");
}

export function sameCombo(a: KeyCombo, b: KeyCombo): boolean {
  return (
    a.code === b.code &&
    a.mods.command === b.mods.command &&
    a.mods.control === b.mods.control &&
    a.mods.shift === b.mods.shift &&
    a.mods.alt === b.mods.alt
  );
}
