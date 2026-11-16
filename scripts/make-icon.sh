#!/usr/bin/env bash
# Renders assets/icon.svg into assets/Strikebar.icns (bundle icon) and
# assets/icon-256.png (window icon for unbundled runs). Needs resvg (brew install resvg).
# The outputs are kept in the repo, so building the app doesn't need resvg.
set -euo pipefail
cd "$(dirname "$0")/.."

SET=$(mktemp -d)/Strikebar.iconset
mkdir -p "$SET"
for size in 16 32 128 256 512; do
  resvg -w "$size" assets/icon.svg "$SET/icon_${size}x${size}.png"
  resvg -w $((size * 2)) assets/icon.svg "$SET/icon_${size}x${size}@2x.png"
done
iconutil -c icns "$SET" -o assets/Strikebar.icns
cp "$SET/icon_256x256.png" assets/icon-256.png
rm -rf "$(dirname "$SET")"
echo "Wrote assets/Strikebar.icns and assets/icon-256.png"
