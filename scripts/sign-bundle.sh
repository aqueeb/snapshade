#!/usr/bin/env bash
# scripts/sign-bundle.sh — re-sign a bundled Snapshade.app with the stable identity + team-scoped DR.
#
# `tauri build` signs the bundle with `bundle.macOS.signingIdentity` but has no option for a
# custom designated requirement, so the bundler's signature carries codesign's DEFAULT DR, which
# pins `certificate leaf[subject.CN]` (RESEARCH Pitfall 1). PKG-02 requires every build form —
# `cargo run`, `tauri dev`, and `tauri build --debug` — to carry the same team-scoped DR so the
# first Screen Recording grant survives certificate renewal and the Phase 4 Developer ID switch.
# Run this after `tauri build --debug --bundles app` (root script `bun run build:debug` does).
#
# Same identity/bundle/team defaults and env overrides as scripts/sign-and-run.sh (D-16).
# Nested code (Contents/MacOS/*) is signed first, then the bundle, as Apple requires.
# Phase 4 note: re-signing after notarization invalidates the ticket; the release pipeline must
# apply this DR BEFORE notarizing (e.g. bundler signing off, sign here, then notarytool).
set -euo pipefail
APP="${1:?usage: sign-bundle.sh <path/to/Snapshade.app>}"
IDENTITY="${SNAPSHADE_SIGN_IDENTITY:-}"
BUNDLE_ID="${SNAPSHADE_BUNDLE_ID:-io.github.aqueeb.snapshade}"
TEAM_ID="${SNAPSHADE_TEAM_ID:-}"
if [[ -z "$IDENTITY" || "${CI:-}" == "true" ]]; then
  echo "sign-bundle: no identity or CI=true, leaving $APP as the bundler signed it" >&2
  exit 0
fi
[[ -n "$TEAM_ID" ]] || { echo "sign-bundle: set SNAPSHADE_TEAM_ID (the 10-character Team ID in the identity)" >&2; exit 1; }
DR="=designated => identifier \"$BUNDLE_ID\" and anchor apple generic and certificate leaf[subject.OU] = \"$TEAM_ID\""
for exe in "$APP"/Contents/MacOS/*; do
  codesign --force --sign "$IDENTITY" --identifier "$BUNDLE_ID" --options runtime --timestamp=none -r "$DR" "$exe"
done
codesign --force --sign "$IDENTITY" --identifier "$BUNDLE_ID" --options runtime --timestamp=none -r "$DR" "$APP"
codesign --verify --deep --strict --verbose=2 "$APP"
