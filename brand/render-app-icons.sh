#!/usr/bin/env bash
# Draws the desktop app's icon from the mark (mark.svg, the same as
# passbundle-clients' brand/assets/mark.svg) and the brand color: the mark in
# white on a rounded square of the color. Run after changing either, then
# commit what it writes into app/. Needs rsvg-convert (librsvg) and python3
# with Pillow.
set -euo pipefail
cd "$(dirname "$0")"
APP=../app

COLOR=$(python3 -c "import json; print(json.load(open('brand.json'))['color'])")
PATH_D=$(sed -E 's/.* d="([^"]+)".*/\1/' mark.svg)

# One icon as SVG on a 1024 canvas: the square from `inset` to 1024 - inset,
# with corners of `radius`, and the mark `mark` units tall (it's 26x32), centered.
icon() { # inset radius mark shadow
  local side=$((1024 - 2 * $1))
  local scale height width x y
  scale=$(python3 -c "print($3 / 32)")
  height=$3
  width=$(python3 -c "print(26 * $3 / 32)")
  x=$(python3 -c "print(512 - $width / 2)")
  y=$(python3 -c "print(512 - $height / 2)")
  local filter="" use=""
  if [ "$4" = shadow ]; then
    # As Apple's icon template has it: a soft shadow under the square.
    filter='<filter id="s" x="-10%" y="-10%" width="120%" height="125%"><feGaussianBlur in="SourceAlpha" stdDeviation="10"/><feOffset dy="10"/><feComponentTransfer><feFuncA type="linear" slope="0.3"/></feComponentTransfer><feMerge><feMergeNode/><feMergeNode in="SourceGraphic"/></feMerge></filter>'
    use=' filter="url(#s)"'
  fi
  printf '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024"><defs>%s</defs><rect x="%s" y="%s" width="%s" height="%s" rx="%s" fill="%s"%s/><path transform="translate(%s %s) scale(%s)" fill="#FFFFFF" fill-rule="evenodd" d="%s"/></svg>' \
    "$filter" "$1" "$1" "$side" "$side" "$2" "$COLOR" "$use" "$x" "$y" "$scale" "$PATH_D"
}

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

# macOS: Apple's grid, an 824-point square inside 1024 with room for its shadow.
icon 100 185 430 shadow > "$TMP/macos.svg"
for s in 16 32 64 128 256 512 1024; do
  rsvg-convert -w "$s" -h "$s" -o "$APP/macos/Runner/Assets.xcassets/AppIcon.appiconset/app_icon_$s.png" "$TMP/macos.svg"
done

# Windows and Linux: the square fills the icon, and at small sizes the mark is
# bigger so it still reads.
icon 32 200 520 none > "$TMP/large.svg"
icon 0 180 640 none > "$TMP/small.svg"
for s in 16 24 32; do rsvg-convert -w "$s" -h "$s" -o "$TMP/$s.png" "$TMP/small.svg"; done
for s in 48 64 128 256 512; do rsvg-convert -w "$s" -h "$s" -o "$TMP/$s.png" "$TMP/large.svg"; done
python3 - "$TMP" "$APP/windows/runner/resources/app_icon.ico" << 'EOF'
import sys
from PIL import Image
tmp, out = sys.argv[1], sys.argv[2]
sizes = [16, 24, 32, 48, 64, 128, 256]
images = [Image.open(f"{tmp}/{s}.png").convert("RGBA") for s in sizes]
images[-1].save(out, format="ICO", sizes=[(s, s) for s in sizes], append_images=images[:-1])
EOF
mkdir -p "$APP/linux/icons"
cp "$TMP/256.png" "$APP/linux/icons/com.passbundle.app.png"
echo "Drew the app icon for macOS, Windows, and Linux."
