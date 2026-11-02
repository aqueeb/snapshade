#!/usr/bin/env bash
# scripts/spike-key-probe.sh — automated spike gate (c) key probe (Plan 01-10 Task 3; ROADMAP SC3;
# 01-SPIKE-RESULT open issue 2; re-run by Plan 01-12 on the release bundle).
#
# Launches Snapshade, presses ⌘⇧9 through System Events, waits until the app has logged a
# completed capture row (`latency {` — the overlay is up and its panel is key), then sends
# ⌘Z, ←, ⇧→ and Esc 300 ms apart and greps the app's stderr for the page's own `overlay_key`
# reports — the same probe lines the 01-08 hardware session looked for (RESEARCH Pitfall 5: the
# acceptance is the DOM keydown reaching the page, never a menu-event forward):
#   key <display> z meta=true shift=false
#   key <display> ArrowLeft meta=false shift=false
#   key <display> ArrowRight meta=false shift=true
#
# Launch path: `SNAPSHADE_BIN=<path>` names a signed binary to run directly (Plan 01-12 passes the
# release bundle's `Contents/MacOS/snapshade`); otherwise `cargo run -p snapshade` runs the debug
# binary through the signing runner (scripts/sign-and-run.sh), so no unsigned binary ever
# executes the capture path (PKG-02). Terminal-launched runs inherit the terminal's TCC
# attribution and are behaviour evidence only, never PKG-02 evidence (01-06 SUMMARY).
#
# Safety (T-01-30): synthetic keystrokes are system-wide, so none is sent before the app has
# logged `hotkey registered` AND a `latency {` row after the ⌘⇧9 press (the overlay panel is key);
# only fixed key codes are used; Esc is always sent and the launched app is terminated on every
# exit path (trap). The app's stderr goes to target/key-probe.err.
#
# Exit codes:
#   0  all three key lines found           — `key-probe: ok (z, ArrowLeft, Shift+ArrowRight)`
#   1  one or more missing                 — `key-probe: missing: <names>` + the `key ` lines found
#   3  osascript failed                    — Accessibility not granted to the terminal app
#   5  Screen Recording not granted for this launch path (preflight did not print `granted`)
#   6  the app never logged `hotkey registered` within 120 s (⌘⇧9 owned by another process, e.g.
#      another screenshot tool; or the app failed to start — see target/key-probe.err)
#   7  no capture row within 5 s of the press (overlay never came up) — no keys were sent
#
# Usage: scripts/spike-key-probe.sh            (debug binary via cargo run)
#        SNAPSHADE_BIN=target/release/bundle/macos/Snapshade.app/Contents/MacOS/snapshade \
#          scripts/spike-key-probe.sh          (a signed bundle binary)
set -euo pipefail

LOG="target/key-probe.err"
PREFLIGHT_LOG="target/key-probe-preflight.err"
HOTKEY_TIMEOUT_S=120
ROW_TIMEOUT_S=5
APP_PID=""
ESC_SENT=0

launch() {
  if [[ -n "${SNAPSHADE_BIN:-}" ]]; then
    "$SNAPSHADE_BIN" "$@"
  else
    cargo run -q -p snapshade "$@"
  fi
}

# System Events key press; the caller decides what a failure means.
press() {
  osascript -e "tell application \"System Events\" to $1" >/dev/null
}

send_escape() {
  if [[ "$ESC_SENT" -eq 0 ]]; then
    ESC_SENT=1
    press 'key code 53' 2>/dev/null || true
  fi
}

terminate_app() {
  if [[ -n "$APP_PID" ]] && kill -0 "$APP_PID" 2>/dev/null; then
    # The app runs in its own process group (set -m below): cargo + the exec'd binary, or the
    # bare binary. Fall back to the parent/child pair if the group kill is refused.
    kill -TERM -- "-$APP_PID" 2>/dev/null || {
      pkill -TERM -P "$APP_PID" 2>/dev/null || true
      kill -TERM "$APP_PID" 2>/dev/null || true
    }
    wait "$APP_PID" 2>/dev/null || true
  fi
  APP_PID=""
}

cleanup() {
  # Every exit path: restore the desktop (Esc) and stop the app we launched.
  send_escape
  terminate_app
}
trap cleanup EXIT

# Poll the log for an extended regex until it appears or the deadline passes (0 = found,
# 1 = timed out, 2 = the app exited early).
wait_for() {
  local pattern="$1" timeout_s="$2" deadline
  deadline=$(( $(date +%s) + timeout_s ))
  while :; do
    if grep -Eq "$pattern" "$LOG" 2>/dev/null; then
      return 0
    fi
    if ! kill -0 "$APP_PID" 2>/dev/null; then
      return 2
    fi
    if [[ $(date +%s) -ge $deadline ]]; then
      return 1
    fi
    sleep 0.2
  done
}

mkdir -p target

# Precheck: Screen Recording must be granted for THIS launch path, otherwise the press would
# only raise the TCC prompt and no row could ever appear.
if ! SNAPSHADE_SELFTEST=preflight launch 2>"$PREFLIGHT_LOG" | grep -q '^preflight: granted$'; then
  echo "key-probe: Screen Recording not granted for this launch path (see $PREFLIGHT_LOG)" >&2
  exit 5
fi

# Launch in its own process group (job control) with stderr captured.
: > "$LOG"
set -m
launch >/dev/null 2>"$LOG" &
APP_PID=$!
set +m

# Keys are never sent before the hotkey is registered …
if ! wait_for 'hotkey registered' "$HOTKEY_TIMEOUT_S"; then
  if grep -q 'hotkey registration failed' "$LOG" 2>/dev/null; then
    echo "key-probe: hotkey registration failed (another process owns ⌘⇧9?) — see $LOG" >&2
  else
    echo "key-probe: the app never logged 'hotkey registered' within ${HOTKEY_TIMEOUT_S} s — see $LOG" >&2
  fi
  exit 6
fi

# ⌘⇧9 (key code 25 with command + shift) — system-wide, like the 01-08 session.
if ! press 'key code 25 using {command down, shift down}'; then
  echo "key-probe: osascript failed (Accessibility not granted to the terminal?)" >&2
  exit 3
fi

# … and not before a completed capture row proves the overlay is up and its panel is key.
if ! wait_for 'latency \{' "$ROW_TIMEOUT_S"; then
  echo "key-probe: no capture row within ${ROW_TIMEOUT_S} s of the press (overlay not up) — no keys sent; see $LOG" >&2
  exit 7
fi

press 'key code 6 using {command down}'     # ⌘Z
sleep 0.3
press 'key code 123'                        # ←
sleep 0.3
press 'key code 124 using {shift down}'     # ⇧→
sleep 0.3
send_escape                                 # Esc — restores the desktop
sleep 1
terminate_app

missing=""
grep -Eq 'key [0-9]+ z meta=true shift=false' "$LOG" || missing="$missing z"
grep -Eq 'key [0-9]+ ArrowLeft meta=false shift=false' "$LOG" || missing="$missing ArrowLeft"
grep -Eq 'key [0-9]+ ArrowRight meta=false shift=true' "$LOG" || missing="$missing Shift+ArrowRight"

if [[ -z "$missing" ]]; then
  echo "key-probe: ok (z, ArrowLeft, Shift+ArrowRight)"
  exit 0
fi
echo "key-probe: missing:${missing}"
echo "key-probe: key lines found in $LOG:"
grep -E '^key ' "$LOG" || echo "  (none)"
exit 1
