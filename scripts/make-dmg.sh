#!/usr/bin/env bash
# Packs target/Strikebar.app into a disk image for people downloading it:
# the app next to an Applications shortcut to drag it onto.
#
#   ./scripts/make-dmg.sh [out-dir]     # → <out-dir>/Strikebar-<version>-apple-silicon.dmg
#
# The version comes from the bundle's Info.plist. Apple silicon only, like the
# app. (In-app updates use the signed zip that release.sh makes, not this.)
set -euo pipefail
cd "$(dirname "$0")/.."

APP=target/Strikebar.app
OUT=${1:-target/release-assets}
[ -d "$APP" ] || { echo "make-dmg: no $APP (run scripts/bundle-macos.sh)" >&2; exit 1; }
VERSION=$(plutil -extract CFBundleShortVersionString raw -o - "$APP/Contents/Info.plist")
[ "$(lipo -archs "$APP/Contents/MacOS/strikebar")" = arm64 ] ||
  { echo "make-dmg: expected an arm64 build" >&2; exit 1; }
DMG="$OUT/Strikebar-$VERSION-apple-silicon.dmg"

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
ditto "$APP" "$STAGE/Strikebar.app"
ln -s /Applications "$STAGE/Applications"
mkdir -p "$OUT"
# ULFO: LZFSE-compressed, read-only; opens on macOS 10.11+ (the app needs 11).
hdiutil create -quiet -ov -fs HFS+ -format ULFO \
  -volname "Strikebar $VERSION" -srcfolder "$STAGE" "$DMG"
hdiutil verify -quiet "$DMG"
echo "$DMG ($(du -h "$DMG" | cut -f1))"
