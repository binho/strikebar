#!/usr/bin/env bash
# Builds a self-contained target/Strikebar.app (macOS).
#
# The app links against a minimal libheif (HEIC decoding only, see build-libheif.sh)
# and libde265. Both are copied into Contents/Frameworks and relinked through @rpath,
# so the app runs on Macs without Homebrew. Their licenses go in Contents/Resources.
set -euo pipefail
cd "$(dirname "$0")/.."

LIBHEIF_PREFIX="$PWD/target/libheif-min"
if [[ ! -f "$LIBHEIF_PREFIX/lib/libheif.1.dylib" || ! -f "$LIBHEIF_PREFIX/lib/libde265.0.dylib" ]]; then
  ./scripts/build-libheif.sh
fi

# Separate target dir, so switching libheif here doesn't rebuild normal dev builds.
export CARGO_TARGET_DIR="$PWD/target/bundle"
PKG_CONFIG_PATH="$LIBHEIF_PREFIX/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}" \
  cargo build --release

APP=target/Strikebar.app
BIN="$APP/Contents/MacOS/strikebar"
FW="$APP/Contents/Frameworks"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$FW" "$APP/Contents/Resources/licenses"
cp assets/Strikebar.icns "$APP/Contents/Resources/Strikebar.icns"
cp "$CARGO_TARGET_DIR/release/strikebar" "$BIN"
chmod u+w "$BIN"

is_system() { [[ "$1" == /usr/lib/* || "$1" == /System/* ]]; }

# Where a dependency actually lives. @rpath/* libraries come from our own build.
resolve() {
  case "$1" in
    @rpath/*) echo "$LIBHEIF_PREFIX/lib/${1#@rpath/}" ;;
    *) echo "$1" ;;
  esac
}

# Copy every non-system dylib the binary needs (recursively) and point all
# references at @rpath/<name>.
install_name_tool -add_rpath @executable_path/../Frameworks "$BIN"
queue=("$BIN")
while ((${#queue[@]})); do
  file="${queue[0]}"
  queue=("${queue[@]:1}")
  # First line of otool -L is the file itself; for dylibs the second is its own id.
  while read -r dep _; do
    [[ -z "$dep" ]] && continue
    name=$(basename "$dep")
    [[ "$file" == "$FW/$name" ]] && continue # its own install name
    if [[ "$dep" == @rpath/* ]] && [[ -f "$FW/$name" ]]; then
      continue
    fi
    if is_system "$dep"; then
      continue
    fi
    src=$(resolve "$dep")
    if [[ ! -f "$FW/$name" ]]; then
      cp -L "$src" "$FW/$name"
      chmod u+w "$FW/$name"
      install_name_tool -id "@rpath/$name" "$FW/$name"
      install_name_tool -add_rpath @loader_path "$FW/$name" 2>/dev/null || true
      queue+=("$FW/$name")
      echo "bundled $name  <-  $src"
    fi
    [[ "$dep" != "@rpath/$name" ]] && install_name_tool -change "$dep" "@rpath/$name" "$file"
  done < <(otool -L "$file" | tail -n +2 | awk '{print $1}')
done

# Licenses of the bundled LGPL libraries.
cp target/libheif-src/COPYING "$APP/Contents/Resources/licenses/libheif-COPYING.txt"
cp target/libde265-src/COPYING "$APP/Contents/Resources/licenses/libde265-COPYING.txt"

# Minimum macOS = the highest one any bundled binary was built for.
MIN_OS=$(for f in "$BIN" "$FW"/*.dylib; do
  otool -l "$f" | awk '/LC_BUILD_VERSION/{b=1} b&&/minos/{print $2; b=0}'
done | sort -t. -k1,1n -k2,2n | tail -1)

VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Strikebar</string>
  <key>CFBundleDisplayName</key><string>Strikebar</string>
  <key>CFBundleIdentifier</key><string>dev.cleber.strikebar</string>
  <key>CFBundleExecutable</key><string>strikebar</string>
  <key>CFBundleIconFile</key><string>Strikebar</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>LSMinimumSystemVersion</key><string>${MIN_OS}</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>CFBundleDocumentTypes</key>
  <array><dict>
    <key>CFBundleTypeRole</key><string>Editor</string>
    <key>LSItemContentTypes</key><array><string>public.image</string><string>public.heic</string></array>
  </dict></array>
</dict>
</plist>
PLIST

# Relinking invalidates signatures; re-sign ad hoc (inner code first).
for lib in "$FW"/*.dylib; do codesign --force --sign - "$lib" >/dev/null; done
codesign --force --sign - "$APP" >/dev/null

# Fail if anything still points outside the bundle.
if otool -L "$BIN" "$FW"/*.dylib | grep -E '^\s+/(opt|usr/local)/'; then
  echo "error: bundle still references libraries outside the app" >&2
  exit 1
fi
codesign --verify --strict "$APP"
echo "Built $APP (macOS ${MIN_OS}+, $(du -sh "$APP" | cut -f1))"
