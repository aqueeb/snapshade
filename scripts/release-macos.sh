#!/usr/bin/env bash
# scripts/release-macos.sh — build the release .app + .dmg, re-sign with the team-scoped
# designated requirement (PKG-02), then notarize and staple BOTH artifacts (PKG-01, PKG-03).
#
# Usage on the dev Mac (Developer ID Application installed in the login keychain):
#   APPLE_SIGNING_IDENTITY="Developer ID Application: <Your Name> (<TEAMID>)" \
#   SNAPSHADE_TEAM_ID=<TEAMID> NOTARY_PROFILE=snapshade scripts/release-macos.sh
# Without APPLE_SIGNING_IDENTITY it signs with SNAPSHADE_SIGN_IDENTITY (an Apple
# Development identity, for a build that runs on your own Mac only). APPLE_SIGNING_IDENTITY=- (CI without a Developer
# ID) leaves the bundler's ad-hoc signature alone: no re-sign, no DMG signature. Without
# NOTARY_PROFILE / APPLE_ID+APPLE_PASSWORD+APPLE_TEAM_ID / APPLE_API_* it skips notarization
# (Gatekeeper rejects on other Macs).
#
# Order matters (sign-bundle.sh header): the team-scoped DR is applied BEFORE notarization,
# and nothing re-signs afterwards (re-signing invalidates the ticket).
set -euo pipefail
cd "$(dirname "$0")/.."

# CI passes every notarization variable from secrets, so an unset secret arrives as an
# EMPTY string — and the tauri bundler treats a present APPLE_ID / APPLE_TEAM_ID as "please
# notarize" ("Team ID must be at least 3 characters", first dispatch 2026-10-31). Empty
# means absent here.
for v in APPLE_SIGNING_IDENTITY APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID APPLE_API_ISSUER APPLE_API_KEY APPLE_API_KEY_PATH NOTARY_PROFILE; do
  if [[ -z "${!v:-}" ]]; then unset "$v"; fi
done

IDENTITY="${APPLE_SIGNING_IDENTITY:-${SNAPSHADE_SIGN_IDENTITY:-}}"
BUNDLE_ID="${SNAPSHADE_BUNDLE_ID:-io.github.aqueeb.snapshade}"
TEAM_ID="${SNAPSHADE_TEAM_ID:-${APPLE_TEAM_ID:-}}"
APP=target/release/bundle/macos/Snapshade.app

# 1. Build the .app only; the bundler's own signature is replaced below and the DMG is
#    built from the re-signed app so the two never disagree (PITFALLS Pitfall 9).
echo "==> tauri build (app)"
bun run tauri build --bundles app

# 2. Team-scoped DR on nested code and the bundle, with a secure timestamp for notarization.
if [[ "$IDENTITY" != "-" && ( ( -n "$IDENTITY" && "${CI:-}" != "true" ) || -n "${APPLE_SIGNING_IDENTITY:-}" ) ]]; then
  echo "==> codesign ($IDENTITY)"
  [[ -n "$TEAM_ID" ]] || { echo "release-macos: set SNAPSHADE_TEAM_ID (the 10-character Team ID in the identity)" >&2; exit 1; }
  DR="=designated => identifier \"$BUNDLE_ID\" and anchor apple generic and certificate leaf[subject.OU] = \"$TEAM_ID\""
  for exe in "$APP"/Contents/MacOS/*; do
    codesign --force --sign "$IDENTITY" --identifier "$BUNDLE_ID" --options runtime --timestamp -r "$DR" "$exe"
  done
  codesign --force --sign "$IDENTITY" --identifier "$BUNDLE_ID" --options runtime --timestamp -r "$DR" "$APP"
  codesign --verify --deep --strict --verbose=2 "$APP"
fi

# 3. Notarize + staple the .app when credentials exist. Locally the preferred form is a
#    keychain profile created once with
#      xcrun notarytool store-credentials snapshade --apple-id <id> --team-id <TEAMID>
#    and selected with NOTARY_PROFILE=snapshade (the password never touches a file or env).
NOTARIZE=0
if [[ -n "${NOTARY_PROFILE:-}" ]]; then
  NOTARY_ARGS=(--keychain-profile "$NOTARY_PROFILE"); NOTARIZE=1
elif [[ -n "${APPLE_API_KEY:-}" && -n "${APPLE_API_ISSUER:-}" && -n "${APPLE_API_KEY_PATH:-}" ]]; then
  NOTARY_ARGS=(--key "$APPLE_API_KEY_PATH" --key-id "$APPLE_API_KEY" --issuer "$APPLE_API_ISSUER"); NOTARIZE=1
elif [[ -n "${APPLE_ID:-}" && -n "${APPLE_PASSWORD:-}" && -n "${APPLE_TEAM_ID:-}" ]]; then
  NOTARY_ARGS=(--apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID"); NOTARIZE=1
fi
if [[ $NOTARIZE == 1 ]]; then
  echo "==> notarize .app"
  ditto -c -k --keepParent "$APP" target/release/bundle/macos/Snapshade.zip
  xcrun notarytool submit target/release/bundle/macos/Snapshade.zip "${NOTARY_ARGS[@]}" --wait
  xcrun stapler staple "$APP"
else
  echo "==> notarization skipped (no APPLE_API_* or APPLE_ID credentials)"
fi

# 4. DMG from the (stapled) app, then sign + notarize + staple the DMG itself.
echo "==> dmg"
mkdir -p target/release/bundle/dmg
VERSION=$(python3 -c 'import json;print(json.load(open("src-tauri/tauri.conf.json"))["version"])')
ARCH=$(uname -m); [[ "$ARCH" == "arm64" ]] && ARCH=aarch64
DMG="target/release/bundle/dmg/Snapshade_${VERSION}_${ARCH}.dmg"
rm -f "$DMG"
STAGE=$(mktemp -d)
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
hdiutil create -volname "Snapshade" -srcfolder "$STAGE" -ov -format UDZO "$DMG" >/dev/null
rm -rf "$STAGE"
if [[ "$IDENTITY" != "-" && -n "$IDENTITY" && ( "${CI:-}" != "true" || -n "${APPLE_SIGNING_IDENTITY:-}" ) ]]; then
  codesign --force --sign "$IDENTITY" --timestamp "$DMG"
fi
if [[ $NOTARIZE == 1 ]]; then
  echo "==> notarize .dmg"
  xcrun notarytool submit "$DMG" "${NOTARY_ARGS[@]}" --wait
  xcrun stapler staple "$DMG"
  xcrun stapler validate "$DMG"
  spctl --assess --type open --context context:primary-signature -vv "$DMG" || true
fi
ls -la "$APP/Contents/MacOS" "$DMG"
echo "done: $DMG"
