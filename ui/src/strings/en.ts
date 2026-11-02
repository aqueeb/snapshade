// English string table (OPT-06). Every user-facing string in the webview layer goes
// through `t()`. Rust-side strings (tray menu, notifications, dialogs) live in
// `src-tauri/src/strings.rs` and mirror these keys.
export const en = {
  // Overlay
  "overlay.selectArea": "Select area",
  "overlay.sizeHint": "Scroll to change size",
  "overlay.frameFailed": "The screenshot could not be loaded. Press Esc and try again.",
  "overlay.menu.fullscreen": "Fullscreen",
  "overlay.menu.clear": "Clear selection",
  "overlay.menu.cancel": "Cancel",
  // Tools (docs/SPEC.md §4, in toolbar order)
  "tool.pen": "Pen",
  "tool.line": "Line",
  "tool.arrow": "Arrow",
  "tool.rect": "Rectangle",
  "tool.ellipse": "Circle",
  "tool.marker": "Marker",
  "tool.blur": "Blur",
  "tool.text": "Text",
  "tool.color": "Color",
  "tool.undo": "Undo",
  "tool.color.custom": "Custom color…",
  // Actions (docs/SPEC.md §5)
  "action.print": "Print",
  "action.copy": "Copy",
  "action.save": "Save",
  "action.close": "Close",
  // Options dialog (docs/SPEC.md §6)
  "options.title": "Options",
  "options.tab.general": "General",
  "options.tab.hotkeys": "Hotkeys",
  "options.tab.formats": "Formats",
  "options.general.notifications": "Show a notification after copying or saving",
  "options.general.cursor": "Include the mouse cursor",
  "options.general.keepSelection": "Remember the selected area for the next capture",
  "options.general.language": "Language",
  "options.general.startAtLogin": "Start Snapshade at login",
  "options.hotkeys.general": "Capture hotkey",
  "options.hotkeys.instantSave": "Instant full-screen save",
  "options.hotkeys.record": "Click, then press a key combination",
  "options.hotkeys.invalid": "That key combination cannot be used as a hotkey. Choose another one.",
  "options.hotkeys.duplicate": "Both functions would use the same key combination. Choose another one.",
  "options.hotkeys.reserved": "This key combination is reserved by the system. Please choose another key combination.",
  "options.hotkeys.failed": "Could not register the hotkey; another application is probably using it.",
  "options.formats.default": "Default save format",
  "options.formats.quality": "JPEG quality",
  "options.formats.optimize": "Optimize PNG files",
  "options.formats.never": "Never",
  "options.formats.always": "Always",
  "options.formats.folder": "Default save folder",
  "options.formats.browse": "Browse…",
  "options.saved": "Saved",
  // About
  "about.title": "About Snapshade",
  "about.tagline": "Hotkey, drag, annotate, copy. A local-only screenshot tool, open source.",
  "about.license": "MIT License",
  "about.noNetwork": "Snapshade never connects to the network.",
  // Permission guidance (CAPTURE-03)
  "permission.title": "Screen Recording permission needed",
  "permission.body":
    "Snapshade needs Screen Recording permission to capture your screen. Open System Settings → Privacy & Security → Screen & System Audio Recording, enable Snapshade, then quit and relaunch.",
  "permission.open": "Open System Settings",
  "permission.relaunch": "Quit and relaunch",
  "permission.later": "Later",
} as const;

export type StringKey = keyof typeof en;
