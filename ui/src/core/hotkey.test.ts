import { describe, expect, it } from "vitest";
import { comboFromEvent, displayCombo, isValidBinding, sameCombo } from "./hotkey";

const ev = (code: string, m: Partial<{ metaKey: boolean; ctrlKey: boolean; shiftKey: boolean; altKey: boolean }> = {}) => ({
  code,
  metaKey: false,
  ctrlKey: false,
  shiftKey: false,
  altKey: false,
  ...m,
});

describe("KeyCombo mirror of crates/core/src/hotkey.rs", () => {
  it("builds from a keydown and rejects modifier-only / unsupported keys", () => {
    expect(comboFromEvent(ev("Digit9", { metaKey: true, shiftKey: true }))).toEqual({
      code: "Digit9",
      mods: { command: true, control: false, shift: true, alt: false },
    });
    expect(comboFromEvent(ev("ShiftLeft", { shiftKey: true }))).toBeNull();
    expect(comboFromEvent(ev("Escape"))).toBeNull();
    expect(comboFromEvent(ev("F25"))).toBeNull();
    expect(comboFromEvent(ev("F24"))).not.toBeNull();
    expect(comboFromEvent(ev("PrintScreen"))).not.toBeNull();
  });

  it("displays like the Rust side", () => {
    const c = comboFromEvent(ev("Digit9", { metaKey: true, shiftKey: true }))!;
    expect(displayCombo(c, true)).toBe("⇧⌘9");
    expect(displayCombo(c, false)).toBe("Win+Shift+9");
    const d = comboFromEvent(ev("KeyA", { ctrlKey: true, altKey: true, shiftKey: true }))!;
    expect(displayCombo(d, true)).toBe("⌃⌥⇧A");
    expect(displayCombo(d, false)).toBe("Ctrl+Alt+Shift+A");
  });

  it("validity matches the Rust rule", () => {
    expect(isValidBinding(comboFromEvent(ev("Digit9", { shiftKey: true }))!)).toBe(false);
    expect(isValidBinding(comboFromEvent(ev("KeyA"))!)).toBe(false);
    expect(isValidBinding(comboFromEvent(ev("F5"))!)).toBe(true);
    expect(isValidBinding(comboFromEvent(ev("PrintScreen"))!)).toBe(true);
    expect(isValidBinding(comboFromEvent(ev("KeyA", { altKey: true }))!)).toBe(true);
  });

  it("sameCombo compares structurally", () => {
    const a = comboFromEvent(ev("Digit9", { metaKey: true }))!;
    const b = comboFromEvent(ev("Digit9", { metaKey: true }))!;
    const c = comboFromEvent(ev("Digit9", { ctrlKey: true }))!;
    expect(sameCombo(a, b)).toBe(true);
    expect(sameCombo(a, c)).toBe(false);
  });
});
