# Changelog

All notable changes to Snapshade are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## Unreleased

## 0.1.0 - 2026-11-02

First release.

- Global hotkey (⇧⌘9 on macOS, PrintScreen on Windows, configurable) opens a frozen, dimmed
  overlay on every display at native resolution.
- Selection with eight handles, move, nudge and resize by keyboard, whole-display toggle,
  size badge in physical pixels, quick copy by holding ⌘ / Ctrl at the start of the drag.
- Annotation tools: pen, line, arrow, rectangle, circle, marker, blur, text, colour picker,
  undo; one shared size and colour, mouse-wheel and keyboard size control.
- Copy to clipboard, instant save and Save As (PNG / JPG / BMP, collision-safe file names),
  print through the system dialog, optional notifications that reveal the saved file.
- Options: notifications, include the mouse cursor, remember the selected area, start at
  login, hotkeys with a recorder and validation, instant full-screen save, default format,
  JPEG quality, PNG optimisation, default save folder.
- macOS: Screen Recording permission guidance, Developer ID signed, notarized and stapled
  `.dmg`, team-scoped signature so permissions survive updates.
- Windows: per-user NSIS installer signed through Azure Artifact Signing, DXGI capture,
  Snipping Tool hotkey detection.
- No networking code anywhere in the binary.
