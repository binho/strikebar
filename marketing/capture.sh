#!/usr/bin/env bash
# Builds the Mac App Store screenshots end to end:
#   1. renders the fictional documents (render-documents.sh)
#   2. opens them in Strikebar's capture mode and runs each scene in scenes/
#   3. adds the traffic lights (scripts/frame-screenshots.sh)
#   4. puts each capture in an App Store frame with its headline (frame.html)
#
# Output: target/marketing/appstore/NN-name.png, 2560×1600.
# Needs: target/Strikebar.app (scripts/bundle-macos.sh), Google Chrome, ImageMagick.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$PWD
OUT=$ROOT/target/marketing
DOCS=$OUT/Confidential # folder name shows in the app's breadcrumb
SHOTS=$OUT/shots
CHROME=${CHROME:-"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"}
APP=$ROOT/target/Strikebar.app/Contents/MacOS/strikebar

# Headline and subtitle per scene.
title() {
  case $1 in
    01-blackout) echo "Black out anything confidential.|Draw a box. The saved copy has it burned in, for good." ;;
    02-pixelate) echo "Pixelate chats and screenshots.|Large blocks plus random noise, not a blur that can be sharpened." ;;
    03-pills) echo "Rectangles or pills, at any angle.|Boxes tilt to match photographed documents." ;;
    04-batch) echo "A whole folder at a time.|Open many images, redact each one, save every copy at once." ;;
    05-size) echo "Clean copies, smaller files.|No metadata, nothing uploaded. Scale down for email and chat." ;;
  esac
}

rm -rf "$OUT"
mkdir -p "$SHOTS" "$OUT/appstore"
./marketing/render-documents.sh "$DOCS"

pkill -x strikebar 2>/dev/null || true
STRIKEBAR_SCREENSHOTS=$SHOTS "$APP" "$DOCS/board-memo.png" "$DOCS/chat-screenshot.png" \
  "$DOCS/lab-report.png" >"$OUT/app.log" 2>&1 &
APP_PID=$!
trap 'kill $APP_PID 2>/dev/null || true' EXIT
sleep 3

for scene in marketing/scenes/*.scene; do
  name=$(basename "$scene" .scene)
  # Replace "@box" with "cx cy w h angle=A" from the current document's boxes.
  python3 - "$scene" "$DOCS" >"$SHOTS/take.tmp" <<'PY'
import sys, pathlib
scene, docs = sys.argv[1], pathlib.Path(sys.argv[2])
boxes = {}
for line in open(scene):
    words = line.split("#")[0].split()
    if words[:1] == ["doc"]:
        stem = pathlib.Path(words[1]).stem
        boxes = {}
        for b in (docs / f"{stem}.boxes").read_text().split("\n"):
            if b.strip():
                n, cx, cy, w, h, a = b.split()
                boxes[n] = f"{cx} {cy} {w} {h} angle={a}"
    words = [boxes[w[1:]] if w.startswith("@") else w for w in words]
    print(" ".join(words))
PY
  mv "$SHOTS/take.tmp" "$SHOTS/take"
  for _ in $(seq 50); do
    [[ -f "$SHOTS/$name.png" ]] && break
    sleep 0.2
  done
  [[ -f "$SHOTS/$name.png" ]] || { echo "no screenshot for $name" >&2; cat "$OUT/app.log" >&2; exit 1; }
  sleep 0.3
  echo "captured $name"
done
grep "^scene:" "$OUT/app.log" >&2 || true

./scripts/frame-screenshots.sh "$SHOTS" "$OUT/framed" >/dev/null
for shot in "$OUT"/framed/appstore/*.png; do
  name=$(basename "$shot" .png)
  IFS='|' read -r t s <<<"$(title "$name")"
  url="file://$ROOT/marketing/frame.html?img=$(python3 -c 'import sys, urllib.parse as u; print(u.quote(sys.argv[1]))' "file://$shot")&title=$(python3 -c 'import sys, urllib.parse as u; print(u.quote(sys.argv[1]))' "$t")&sub=$(python3 -c 'import sys, urllib.parse as u; print(u.quote(sys.argv[1]))' "$s")"
  "$CHROME" --headless --disable-gpu --hide-scrollbars --allow-file-access-from-files \
    --window-size=2560,1600 --force-device-scale-factor=1 \
    --screenshot="$OUT/appstore/$name.png" "$url" 2>/dev/null
  magick "$OUT/appstore/$name.png" -alpha off "$OUT/appstore/$name.png"
  echo "framed $name"
done
echo "App Store screenshots in $OUT/appstore"
