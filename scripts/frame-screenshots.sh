#!/usr/bin/env bash
# Turns raw window captures (see src/screenshots.rs) into:
#   <out>/appstore/NAME.png  2560×1600, opaque: the Mac App Store size
#   <out>/readme/NAME.png    rounded corners and a shadow, 1400 px wide, for the README
# macOS draws the traffic lights itself, so they're missing from the captures;
# this paints them back at their real position (centres 16/39/62 pt, 16 pt down).
#
# Usage: scripts/frame-screenshots.sh target/shots [out-dir]
# Needs ImageMagick (brew install imagemagick).
set -euo pipefail
IN=${1:?usage: frame-screenshots.sh <captures-dir> [out-dir]}
OUT=${2:-$IN}
mkdir -p "$OUT/appstore" "$OUT/readme"

lights=(
  -strokewidth 1.5
  -fill '#FF5F57' -stroke '#E0443E' -draw 'circle 32,32 32,44'
  -fill '#FEBC2E' -stroke '#DEA123' -draw 'circle 78,32 78,44'
  -fill '#28C840' -stroke '#1AAB29' -draw 'circle 124,32 124,44'
)

for src in "$IN"/*.png; do
  name=$(basename "$src")
  read -r w h < <(magick identify -format '%w %h\n' "$src")
  if [[ "$w" != 2560 || "$h" != 1600 ]]; then
    echo "skip $name: ${w}×${h}, expected 2560×1600" >&2
    continue
  fi

  magick "$src" "${lights[@]}" -alpha off "$OUT/appstore/$name"

  # Rounded window (radius 24 px) with a soft shadow on a transparent canvas.
  magick "$OUT/appstore/$name" \
    \( +clone -alpha extract -fill black -colorize 100 \
       -fill white -draw "roundrectangle 0,0 $((w - 1)),$((h - 1)) 24,24" \) \
    -alpha off -compose copy_opacity -composite \
    \( +clone -background black -shadow 45x36+0+18 \) \
    +swap -background none -compose over -layers merge +repage \
    -resize 1400x "$OUT/readme/$name"
  echo "framed $name"
done
