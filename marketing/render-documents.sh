#!/usr/bin/env bash
# Renders the fictional documents in marketing/documents to PNGs with headless
# Chrome, plus each one's redaction boxes (see documents/boxes.js).
#
# Usage: marketing/render-documents.sh <out-dir>
#   <out-dir>/<name>.png     the document image
#   <out-dir>/<name>.boxes   "name cx cy w h angle" per line, in image pixels
set -euo pipefail
cd "$(dirname "$0")/documents"
OUT=${1:?usage: render-documents.sh <out-dir>}
mkdir -p "$OUT"
CHROME=${CHROME:-"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"}

# name  css-width  css-height  scale
docs=(
  "board-memo 816 1056 3"
  "chat-screenshot 390 844 3"
  "lab-report 1100 1300 2"
)
for doc in "${docs[@]}"; do
  read -r name w h scale <<<"$doc"
  url="file://$PWD/$name.html"
  "$CHROME" --headless --disable-gpu --hide-scrollbars --window-size="$w,$h" \
    --force-device-scale-factor="$scale" --screenshot="$OUT/$name.png" "$url" 2>/dev/null
  "$CHROME" --headless --disable-gpu --window-size="$w,$h" --dump-dom "$url" 2>/dev/null |
    python3 -c 'import sys, re, html; m = re.search(r"<pre id=\"boxes\">(.*?)</pre>", sys.stdin.read(), re.S); print(html.unescape(m.group(1)) if m else "")' \
    >"$OUT/$name.boxes"
  echo "$name: $(sips -g pixelWidth -g pixelHeight "$OUT/$name.png" | awk '/pixel/{printf "%s ", $2}')px, $(grep -c . "$OUT/$name.boxes") boxes"
done
