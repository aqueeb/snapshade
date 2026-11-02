#!/usr/bin/env bash
# Local convenience cross-check (RESEARCH Q10, assumption A15): verify that the
# pure crates still type-check for the Windows target from this Mac. This is
# NOT a gate — `cargo check` does not link, so it says nothing about a Windows
# linker or resource compiler. The `snapshade` app crate (tauri-build needs a
# Windows resource compiler) is checked on `windows-latest` in CI only.
#
# Requires: `rustup target add x86_64-pc-windows-msvc` (done in Plan 01-01).
# Usage:    scripts/check-windows-core.sh [extra cargo check args]
set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/.."
cargo check --target x86_64-pc-windows-msvc \
  -p snapshade-core \
  -p snapshade-platform-api \
  -p snapshade-platform-fake \
  -p snapshade-platform-windows \
  "$@"
