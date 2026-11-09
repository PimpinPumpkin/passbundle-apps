#!/usr/bin/env bash
# Draws the app's icon from the mark (mark.svg, the same as
# passbundle-clients' brand/assets/mark.svg): the logo's jewel tile, as
# passbundle-clients' brand/wordmark.py draws it, with the mark in white. The
# tile is the logo's emerald with a soft highlight from the top left, a light
# edge along the top, and a shade at the bottom, like a cut stone. Run after
# changing either, then commit what it writes into app/. Needs rsvg-convert
# (librsvg) and python3 with Pillow.
set -euo pipefail
cd "$(dirname "$0")"
APP=../app

# The logo's emerald (passbundle-clients, brand/wordmark.py), not
# brand.json's color, which is the buttons' green.
EMERALD=#0E7A5A
PATH_D=$(sed -E 's/.* d="([^"]+)".*/\1/' mark.svg)

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

# One icon as SVG on a 1024 canvas: the tile from `inset` to 1024 - inset,
# with corners of `radius` (0: square, for iOS, which rounds it itself), the
# mark `mark` units tall (it's 26x32), centered, and on macOS the soft
# shadow Apple's template has under the tile.
icon() { # inset radius mark shadow
  python3 - "$EMERALD" "$PATH_D" "$@" << 'PY'
import sys
emerald, mark, inset, radius, height, shadow = sys.argv[1], sys.argv[2], *map(float, sys.argv[3:6]), sys.argv[6]
side = 1024 - 2 * inset
u = side / 42  # the logo's tile is 42 units
def rounded(x, y, w, h, r):
    if r <= 0:
        return f"M{x} {y}H{x + w}V{y + h}H{x}Z"
    return (f"M{x + r} {y}H{x + w - r}A{r} {r} 0 0 1 {x + w} {y + r}V{y + h - r}"
            f"A{r} {r} 0 0 1 {x + w - r} {y + h}H{x + r}A{r} {r} 0 0 1 {x} {y + h - r}"
            f"V{y + r}A{r} {r} 0 0 1 {x + r} {y}Z")
shape = rounded(inset, inset, side, side, radius)
top = rounded(inset, inset + u, side, side, radius)
bottom = rounded(inset, inset - 2 * u, side, side, radius)
width = 26 * height / 32
x, y = 512 - width / 2, 512 - height / 2
drop = ('<filter id="drop" x="-10%" y="-10%" width="120%" height="125%">'
        '<feGaussianBlur in="SourceAlpha" stdDeviation="10"/><feOffset dy="10"/>'
        '<feComponentTransfer><feFuncA type="linear" slope="0.3"/></feComponentTransfer>'
        '<feMerge><feMergeNode/><feMergeNode in="SourceGraphic"/></feMerge></filter>') if shadow == "shadow" else ""
use = ' filter="url(#drop)"' if shadow == "shadow" else ""
print(
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024"><defs>'
    '<radialGradient id="shine" cx=".28" cy=".18" r="1" '
    'gradientTransform="translate(.28 .18) scale(1.2 .9) translate(-.28 -.18)">'
    '<stop offset="0" stop-color="#fff" stop-opacity=".32"/>'
    '<stop offset=".58" stop-color="#fff" stop-opacity="0"/></radialGradient>'
    f'<clipPath id="clip"><path d="{shape}"/></clipPath>'
    f'<filter id="soft" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="{1.5 * u}"/></filter>'
    f'{drop}</defs>'
    f'<path fill="{emerald}" d="{shape}"{use}/>'
    f'<path fill="url(#shine)" d="{shape}"/>'
    '<g clip-path="url(#clip)">'
    # The light edge follows a rounded tile's top; a square one (iOS) has none.
    + (f'<path fill="#fff" fill-opacity=".35" fill-rule="evenodd" d="{shape}{top}"/>' if radius > 0 else '')
    + f'<path fill="#000" fill-opacity=".2" fill-rule="evenodd" d="{shape}{bottom}" filter="url(#soft)"/>'
    '</g>'
    f'<path transform="translate({x} {y}) scale({height / 32})" fill="#FFFFFF" fill-rule="evenodd" d="{mark}"/>'
    '</svg>'
)
PY
}

# macOS: Apple's grid, an 824-point square inside 1024 with room for its shadow.
icon 100 185 430 shadow > "$TMP/macos.svg"
for s in 16 32 64 128 256 512 1024; do
  rsvg-convert -w "$s" -h "$s" -o "$APP/macos/Runner/Assets.xcassets/AppIcon.appiconset/app_icon_$s.png" "$TMP/macos.svg"
done

# iOS: the whole square, which iOS rounds itself, with no transparency (the
# App Store's 1024 icon must have none).
icon 0 0 520 none > "$TMP/ios.svg"
IOS=$APP/ios/Runner/Assets.xcassets/AppIcon.appiconset
python3 - "$TMP" "$IOS" << 'PY'
import json, subprocess, sys
from PIL import Image
tmp, ios = sys.argv[1], sys.argv[2]
for image in json.load(open(f"{ios}/Contents.json"))["images"]:
    pixels = round(float(image["size"].split("x")[0]) * float(image["scale"].rstrip("x")))
    out = f"{ios}/{image['filename']}"
    subprocess.run(["rsvg-convert", "-w", str(pixels), "-h", str(pixels), "-o", out, f"{tmp}/ios.svg"], check=True)
    Image.open(out).convert("RGB").save(out)
PY

# Windows and Linux: the tile fills the icon, and at small sizes the mark is
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

# Android: an adaptive icon (app/android/app/src/main/res/mipmap-anydpi), the
# mark in white, 40 of the 108 units tall so it stays inside the 66-unit circle
# every launcher's shape keeps, on the tile: the emerald with its highlight
# from the top left. Launchers that tint icons to the wallpaper use the mark
# alone.
RES=$APP/android/app/src/main/res
cat > "$RES/drawable/ic_launcher_foreground.xml" << EOF
<?xml version="1.0" encoding="utf-8"?>
<!-- Drawn by brand/render-app-icons.sh from brand/mark.svg. -->
<vector xmlns:android="http://schemas.android.com/apk/res/android"
    android:width="108dp"
    android:height="108dp"
    android:viewportWidth="108"
    android:viewportHeight="108">
    <group
        android:scaleX="1.25"
        android:scaleY="1.25"
        android:translateX="37.75"
        android:translateY="34">
        <path
            android:fillColor="#FFFFFF"
            android:fillType="evenOdd"
            android:pathData="$PATH_D" />
    </group>
</vector>
EOF
cat > "$RES/drawable/ic_launcher_background.xml" << EOF
<?xml version="1.0" encoding="utf-8"?>
<!-- Drawn by brand/render-app-icons.sh: the logo's jewel tile, the emerald
     with a soft highlight from the top left and a shade at the bottom. -->
<vector xmlns:android="http://schemas.android.com/apk/res/android"
    xmlns:aapt="http://schemas.android.com/aapt"
    android:width="108dp"
    android:height="108dp"
    android:viewportWidth="108"
    android:viewportHeight="108">
    <path
        android:fillColor="$EMERALD"
        android:pathData="M0,0h108v108h-108z" />
    <path android:pathData="M0,0h108v108h-108z">
        <aapt:attr name="android:fillColor">
            <gradient
                android:type="radial"
                android:centerX="30"
                android:centerY="19"
                android:gradientRadius="75">
                <item android:offset="0" android:color="#52FFFFFF" />
                <item android:offset="0.58" android:color="#00FFFFFF" />
            </gradient>
        </aapt:attr>
    </path>
    <path android:pathData="M0,84h108v24h-108z">
        <aapt:attr name="android:fillColor">
            <gradient
                android:type="linear"
                android:startX="0"
                android:startY="84"
                android:endX="0"
                android:endY="108">
                <item android:offset="0" android:color="#00000000" />
                <item android:offset="1" android:color="#33000000" />
            </gradient>
        </aapt:attr>
    </path>
</vector>
EOF
cat > "$RES/values/colors.xml" << EOF
<?xml version="1.0" encoding="utf-8"?>
<!-- Drawn by brand/render-app-icons.sh: the logo's emerald. -->
<resources>
    <color name="ic_launcher_background">$EMERALD</color>
</resources>
EOF
echo "Drew the app icon for macOS, iOS, Windows, Linux, and Android."
