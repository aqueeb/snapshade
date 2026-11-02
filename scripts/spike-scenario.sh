#!/usr/bin/env bash
# scripts/spike-scenario.sh — set the scenario tag for the next hotkey presses (Plan 01-08).
#
# The running Snapshade reads `~/Library/Logs/Snapshade/scenario.txt` on EVERY ⌘⇧9 press
# (src-tauri/src/latency.rs `scenario_tag`: SNAPSHADE_SCENARIO env > this file > "untagged"),
# so the human switches scenarios without relaunching the app (D-10). Self-test rows are
# tagged `selftest` by their trigger source and never by this file (T-01-19).
#
# Usage: scripts/spike-scenario.sh <tag>
#   e.g. scripts/spike-scenario.sh a-desktop
# Tags used by the 01-08 spike: a-desktop, b-fullscreen-safari, b-fullscreen-safari-nospaces,
# c-idle-battery, d-negative-origin, a-desktop-bmp (plus the release-profile re-runs
# a-desktop-release / a-desktop-release-3disp).
# Tags for the Plan 01-12 re-run on the release bundle (gate per topology with
# `spike-stats.ts --displays <n>`):
#   two-display Retina + one 1x rows (--displays 2): a-desktop, b-fullscreen-safari, c-idle-battery
#   three-display DisplayLink rows  (--displays 3): a-desktop-3disp, b-fullscreen-safari-3disp
#   negative-origin arrangement:                    d-negative-origin
# Reminder (01-SPIKE-RESULT open issue 7): set `c-idle-battery` BEFORE unplugging and starting
# the idle wait — the 01-08 battery rows were recorded under `c-idle-ac` because the tag was
# never switched, and a tag that selects no rows now FAILS the gate instead of passing vacuously.
set -euo pipefail
if [[ $# -ne 1 || -z "$1" ]]; then
  echo "usage: scripts/spike-scenario.sh <tag>   (writes ~/Library/Logs/Snapshade/scenario.txt)" >&2
  exit 1
fi
DIR="$HOME/Library/Logs/Snapshade"
mkdir -p "$DIR"
printf '%s\n' "$1" > "$DIR/scenario.txt"
echo "scenario: $1 -> $DIR/scenario.txt"
