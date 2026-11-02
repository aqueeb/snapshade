#!/usr/bin/env bash
# scripts/sign-and-run.sh — cargo runner: sign the app binary with a stable identity + bundle ID, then exec it.
#
# Cargo invokes this script (via .cargo/config.toml [target.<triple>].runner) with the
# executable path as $1 for `cargo run` AND for every `cargo test` binary. PKG-02 requires
# that no unsigned or ad-hoc-signed `snapshade` binary ever executes the capture path on a
# developer machine, because macOS TCC keys the Screen Recording grant to the bundle ID and
# the designated requirement (DR) of the signed code (CONTEXT.md D-13..D-17; PITFALLS.md 1).
#
# Three guards decide whether to sign; in every other case the binary is exec'd untouched:
#   1. basename "$BIN" is exactly `snapshade` — test binaries and other targets are never signed
#      (RESEARCH Q1 "Side effect to design for"; Pitfall 3).
#   2. IDENTITY is non-empty — `SNAPSHADE_SIGN_IDENTITY=""` disables signing explicitly (D-16).
#   3. CI is not "true" — CI builds are unsigned and never run the capture path (D-17).
#
# The DR is team-scoped (`certificate leaf[subject.OU] = "<TEAM_ID>"`), never CN-pinned, so a
# certificate renewal or the Phase 4 Developer ID switch keeps the same requirement string and
# the existing TCC grant (RESEARCH Pitfall 1). `--force` re-signs a rebuilt binary every time.
#
# Threat note (T-01-07): the SNAPSHADE_* overrides only affect the local developer's shell; a
# different identity/team simply fails the DR match and the grant does not apply (fails closed).
#
# This script must never print to stdout: test harnesses read the exec'd binary's stdout.
set -euo pipefail
BIN="$1"; shift
IDENTITY="${SNAPSHADE_SIGN_IDENTITY:-}"   # D-16: your "Apple Development: …" identity; empty = run unsigned
BUNDLE_ID="${SNAPSHADE_BUNDLE_ID:-io.github.aqueeb.snapshade}"
TEAM_ID="${SNAPSHADE_TEAM_ID:-}"
if [[ "$(basename "$BIN")" == "snapshade" && -n "$IDENTITY" && "${CI:-}" != "true" ]]; then
  [[ -n "$TEAM_ID" ]] || { echo "sign-and-run: set SNAPSHADE_TEAM_ID (the 10-character Team ID in the identity)" >&2; exit 1; }
  codesign --force --sign "$IDENTITY" --identifier "$BUNDLE_ID" --options runtime --timestamp=none \
    -r "=designated => identifier \"$BUNDLE_ID\" and anchor apple generic and certificate leaf[subject.OU] = \"$TEAM_ID\"" \
    "$BIN" >/dev/null
fi
exec "$BIN" "$@"
