# Snapshade behaviour specification

Snapshade is a local-only screenshot tool for macOS and Windows: press a hotkey, the screen
freezes and dims, drag a region, annotate it in place, then copy, save or print. This document
is the behaviour the code implements; comments and tests cite its sections (`spec §3.4`).
Where a detail was a judgement call rather than a requirement it is marked `DEFAULT`.

Principles, in order: the hotkey → drag → annotate → copy loop is instant and reliable;
nothing leaves the machine; both operating systems behave the same way.

## 1. Product summary

- One hotkey opens a full-screen selection overlay on every display with the frozen frame.
- Drag a rectangle, draw on it, then Copy, Save or Print. Under two seconds end to end.
- A resident app with a tray / menu-bar icon and no other windows until you need them.
- No upload, no accounts, no telemetry, no update checks: the binary contains no HTTP client.
- Small: the macOS disk image is about 2.5 MB, the Windows installer about 1.5 MB.

## 2. App lifecycle and shell

- Runs in the background. Windows: system-tray icon. macOS: menu-bar template icon.
- Tooltip: `Press <hotkey> to take a screenshot`, kept current when the hotkey changes.
- Menu items: **Take a screenshot**, **Options**, **About**, **Help**, **Exit**.
- Single instance: launching a second copy hands over to the running one.
- Start at login (Options → General): a LaunchAgent on macOS, an `HKCU\...\Run` value on Windows.
- First run on macOS: the first capture asks for Screen Recording permission and opens a
  guidance window with **Open System Settings** and **Quit and relaunch** (macOS applies the
  grant to a fresh process only). Capture is attempted only after the permission is present.
- Hotkey ownership: on Windows 11, when the Print Screen key is routed to Snipping Tool,
  Snapshade posts a notification that opens the setting; the tray's **Take a screenshot** always
  works. On macOS a hotkey owned by another app yields the registration-failure notification (§6).

## 3. Capture flow

3.1 **Hotkey.** Default macOS ⇧⌘9, default Windows PrintScreen; configurable in Options with a
key recorder. A binding without a modifier is accepted only for dedicated keys (PrintScreen,
F-keys), so ordinary typing is never swallowed.

3.2 **Freeze.** Every display is captured at native resolution in one go, cursor excluded unless
the option is on. One overlay window per display shows its frozen frame dimmed. The overlay
sits above every other window and never animates in.

3.3 **Hint.** `Select area` is shown near the cursor until a drag starts.

3.4 **Selection.** Click-drag creates a selection on the display where the drag started; the
region inside is shown un-dimmed with a white border and eight square handles (corners and
edge midpoints). The W×H badge shows the size in physical pixels, above the top-left corner
(below it when the selection touches the top edge, clamped inside the display). Dragging a
handle resizes, dragging inside moves (the rectangle keeps its offset from the grab point),
clicking outside starts a new selection, a plain click leaves no selection (`DEFAULT`). Arrow
keys nudge by exactly 1 px; ⇧+arrows resize by 1 px on the right/bottom edge (`DEFAULT`).
A selection lives on exactly one display; a press on another display re-homes the capture
there and drops the previous selection and its annotations.

3.5 **Toolbars.** Once a selection exists, a vertical tool strip hugs the selection's right
edge, top-aligned, and a horizontal action strip hugs the bottom edge, right-aligned, each a
small gap away. A strip that does not fit on its side flips to the opposite side; when neither
side fits it moves inside the selection. Strips never leave the display.

3.6 **Quick copy.** Holding ⌘ (Ctrl on Windows) when the drag *starts* copies the selection
to the clipboard on release and closes the overlay without showing the toolbars.

3.7 **Cancel.** Esc, ⌘X / Ctrl+X, the Close button or the context menu's **Cancel** closes the
overlay and leaves the desktop unchanged.

3.8 **Whole display.** ⌘A / Ctrl+A, a double-click, or the context menu's **Fullscreen** selects
the whole display; a second ⌘A restores the rectangle it replaced (nothing to restore when
there was none, `DEFAULT`). **Clear selection** returns to the no-selection state.

3.9 **Pixels.** Selections, annotations and exports are in physical pixels; mixed-DPI setups
and Retina displays work without scaling artefacts.

3.10 **Remember the selected area** (Options): the next capture starts with the previous
rectangle pre-selected on the same display.

3.11 **Include the mouse cursor** (Options): the cursor is drawn into the frozen frame.

## 4. Editing tools (vertical strip, in order)

| Tool | Behaviour |
|---|---|
| Pen | Freehand stroke. |
| Line | Straight line, click-drag. |
| Arrow | Straight line with an arrowhead at the end point. |
| Rectangle | Outline rectangle, not filled. |
| Circle | Outline ellipse inside the dragged rectangle. |
| Marker | Semi-transparent highlighter stroke with its own colour (default yellow). |
| Blur | Pixelates the dragged rectangle. |
| Text | Click to place a text box, type inline, Enter commits; one size per box. |
| Color | Palette swatches plus a custom colour picker (default red). |
| Undo | Removes the last annotation; ⌘Z / Ctrl+Z does the same. |

- One colour and one size are shared by every tool on every display within a capture; the
  marker keeps its own colour.
- Size changes with the mouse wheel while a tool is active (wheel down = bigger, wheel up =
  smaller) and with ⌘+ / ⌘- (Ctrl on Windows): step 1, range 1–40, initial 3. Text is drawn
  at 4 px per size unit (size 3 → 12 px). A hint says `Scroll to change size`.
- Undo is a snapshot stack with no redo (v2). Annotations are not editable after placement (v1).
- Annotations are drawn in the selection's physical pixels and are part of every export.

## 5. Actions (horizontal strip, left to right)

| Action | Keys (macOS / Windows) | Behaviour |
|---|---|---|
| Print | ⌘P / Ctrl+P | The OS print dialog with the image scaled to fit the page. |
| Copy | ⌘C / Ctrl+C | PNG to the clipboard. Notification: `Screenshot copied to the clipboard`. |
| Save | ⌘S / Ctrl+S | The key saves instantly to the last used folder (the default save folder until then) with no dialog; the button opens Save As with PNG, JPG and BMP filters. File name `Screenshot at <MMM dd HH-mm-ss>.<ext>`; a second save within the same second gets ` (2)`, ` (3)`, …. Notification: `Saved as <file>. Click to show it in the folder`, and the click reveals the file. |
| Close | Esc / Ctrl+X | Cancel (§3.7). |

Notifications are shown only when Options → General has them on.

## 6. Options dialog

**General**

- Show a notification after copying or saving
- Include the mouse cursor
- Remember the selected area for the next capture
- Start Snapshade at login
- Language (English in v1; every string goes through a keyed table so a second language is a
  second table, not a code change)

**Hotkeys** (each with a key recorder: click, then press the combination)

- Capture hotkey
- Instant full-screen save (optional, off by default): saves every display to the default
  save folder without showing the overlay.
- Validation before anything touches the OS: a binding that would swallow ordinary typing is
  rejected (`That key combination cannot be used as a hotkey. Choose another one.`), the same
  combination on both functions is rejected (`Both functions would use the same key
  combination. Choose another one.`), a system-reserved combination is rejected. A rejected
  combination is not saved and the recorder reverts.
- Registration failure at startup or after a change keeps the app running without the hotkey
  and posts `Could not register the hotkey; another application is probably using it. Click
  this message to choose a different one in Options.` The click opens Options.

**Formats**

- Default save format: PNG / JPEG / BMP
- JPEG quality (1–100)
- Optimize PNG files: Never / Always (slowest zlib level plus adaptive filtering)
- Default save folder (default: Desktop)

Settings are one JSON file: `~/Library/Application Support/io.github.aqueeb.snapshade/settings.json`
on macOS, `%APPDATA%\io.github.aqueeb.snapshade\settings.json` on Windows. Every change in
Options is saved immediately.

## 7. Deliberately absent

No upload, short links, sharing, cloud storage, accounts or gallery; no auto-update and no
update check; no analytics; no proxy settings. **Help** and **About → GitHub** hand a URL to
the operating system's default browser; the app itself opens no network connection.

## 8. Non-functional requirements

- Hotkey-to-overlay feels instant: under 100 ms on the frozen frame (measured around 50 ms on
  a two-monitor 4K Windows machine).
- Multi-monitor and mixed-DPI correctness on both operating systems.
- Follows the system light / dark appearance.
- macOS: Developer ID signed, hardened runtime, notarized and stapled, shipped as a `.dmg`,
  macOS 14 or later. Windows: signed NSIS installer (`.exe`, per-user, no admin rights),
  Windows 10 1903 or later.
- Small footprint; no bundled browser engine (the system webview renders the overlay).

## 9. Out of scope for v1

Redo and editing placed annotations, auto-update, Linux, browser extensions, a translated UI,
portable zip builds.

## 10. Stack

Tauri 2. A pure Rust core (`crates/core`: geometry, display topology, capture types, encoders,
hotkey model, settings, file names), platform traits (`crates/platform-api`) with macOS and
Windows adapters, the app composition root in `src-tauri`, and a TypeScript overlay / Options
layer (`ui`) whose selection, annotation and rendering core is framework-free and tested
headlessly. See the README's project layout.
