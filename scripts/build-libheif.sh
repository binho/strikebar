#!/usr/bin/env bash
# Builds the HEIC libraries the app bundle ships, into target/libheif-min:
#
#   libde265  HEVC decoder, from its pinned upstream release (checksum-verified)
#   libheif   from the sources vendored in the libheif-sys crate, with only the
#             libde265 decoder enabled
#
# Both are built for macOS 11+ (Homebrew's bottles only target the macOS they were
# installed on), as dylibs with @rpath install names so the bundle can relink them.
#
# Why not Homebrew's libheif: it also links x265 (GPL-2.0) and aom, which we don't
# need for reading HEIC. This build has only LGPL-3.0 parts (libheif, libde265),
# linked dynamically and shipped as separate dylibs.
set -euo pipefail
cd "$(dirname "$0")/.."

MACOS_MIN=11.0
PREFIX="$PWD/target/libheif-min"
DOWNLOADS="$PWD/target/downloads"

LIBDE265_VERSION=1.1.3
LIBDE265_SHA256=554228bd17788c99a7e63b37ab5634722190e6e2bf60c1dcb01cef328e133905
LIBDE265_URL="https://github.com/strukturag/libde265/releases/download/v${LIBDE265_VERSION}/libde265-${LIBDE265_VERSION}.tar.gz"

COMMON=(
  -DCMAKE_BUILD_TYPE=Release
  -DCMAKE_INSTALL_PREFIX="$PREFIX"
  -DCMAKE_INSTALL_LIBDIR=lib
  -DCMAKE_INSTALL_NAME_DIR=@rpath
  -DCMAKE_OSX_DEPLOYMENT_TARGET="$MACOS_MIN"
  -DBUILD_SHARED_LIBS=ON
)

rm -rf "$PREFIX"
mkdir -p "$DOWNLOADS"

# --- libde265 -----------------------------------------------------------------
TARBALL="$DOWNLOADS/libde265-${LIBDE265_VERSION}.tar.gz"
if [[ ! -f "$TARBALL" ]]; then
  curl -fL --retry 3 -o "$TARBALL.part" "$LIBDE265_URL"
  mv "$TARBALL.part" "$TARBALL"
fi
echo "$LIBDE265_SHA256  $TARBALL" | shasum -a 256 -c - >/dev/null || {
  echo "error: checksum mismatch for $TARBALL" >&2
  exit 1
}
DE265_SRC="$PWD/target/libde265-src"
DE265_BUILD="$PWD/target/libde265-build"
rm -rf "$DE265_SRC" "$DE265_BUILD"
mkdir -p "$DE265_SRC"
tar -xzf "$TARBALL" -C "$DE265_SRC" --strip-components 1

cmake -S "$DE265_SRC" -B "$DE265_BUILD" "${COMMON[@]}" \
  -DENABLE_SDL=OFF -DENABLE_DECODER=OFF -DENABLE_ENCODER=OFF \
  -DENABLE_SHERLOCK265=OFF -DENABLE_SIMD=ON
cmake --build "$DE265_BUILD" --parallel
cmake --install "$DE265_BUILD"

# --- libheif ------------------------------------------------------------------
SRC=$(cargo metadata --format-version 1 | python3 -c '
import json, os, sys
pkgs = json.load(sys.stdin)["packages"]
sys_pkg = next(p for p in pkgs if p["name"] == "libheif-sys")
print(os.path.join(os.path.dirname(sys_pkg["manifest_path"]), "vendor", "libheif"))
')
echo "libheif source: $SRC"

# Work on a copy, and drop the `heifio` helper (only used by libheif's example tools,
# and not shipped in the vendored sources) — the same patch libheif-sys applies.
WORK="$PWD/target/libheif-src"
BUILD="$PWD/target/libheif-build"
rm -rf "$WORK" "$BUILD"
cp -R "$SRC" "$WORK"
chmod -R u+w "$WORK"
sed -i '' 's/^add_subdirectory(heifio)$//' "$WORK/CMakeLists.txt"

OFF=(X265 X264 KVAZAAR UVG266 VVDEC VVENC OpenH264_DECODER DAV1D AOM_DECODER AOM_ENCODER
     SvtEnc RAV1E JPEG_DECODER JPEG_ENCODER OpenJPEG_DECODER OpenJPEG_ENCODER
     FFMPEG_DECODER OPENJPH_ENCODER)
FLAGS=()
for codec in "${OFF[@]}"; do
  FLAGS+=("-DWITH_${codec}=OFF" "-DWITH_${codec}_PLUGIN=OFF")
done

# Point it at our libde265 explicitly so it can't pick up a system/Homebrew copy.
cmake -S "$WORK" -B "$BUILD" "${COMMON[@]}" \
  -DLIBDE265_INCLUDE_DIR="$PREFIX/include" \
  -DLIBDE265_LIBRARY="$PREFIX/lib/libde265.dylib" \
  -DENABLE_PLUGIN_LOADING=OFF \
  -DWITH_LIBDE265=ON -DWITH_LIBDE265_PLUGIN=OFF \
  -DWITH_LIBSHARPYUV=OFF \
  -DWITH_UNCOMPRESSED_CODEC=OFF \
  -DWITH_EXAMPLES=OFF -DWITH_GDK_PIXBUF=OFF \
  -DBUILD_DOCUMENTATION=OFF -DBUILD_TESTING=OFF \
  "${FLAGS[@]}"
cmake --build "$BUILD" --parallel
cmake --install "$BUILD"

echo "Built $PREFIX"
for lib in "$PREFIX"/lib/libheif.1.dylib "$PREFIX"/lib/libde265.0.dylib; do
  otool -L "$lib"
  otool -l "$lib" | awk '/LC_BUILD_VERSION/{b=1} b&&/minos/{print "  minos " $2; b=0}'
done
