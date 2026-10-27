#!/usr/bin/env bash
# Checks the app's window on a Mac, as tools/window-test.sh does on Linux:
# no title bar over the app (its view fills the window), the traffic lights
# where the app's band has room for them (in the menu's top, lined up with
# the list's search), dragging the band moves the window but dragging the
# search field doesn't, a double click on the band zooms, the lights
# minimize, go full screen, and close, it can't be made smaller than 400 by
# 560, and it opens again where and how big it was left. Needs a build with
# test hooks (tools/measure.sh says how), opened on the sample vault so the
# vault shows, and the Accessibility permission CI's runners have.
#
#   tools/mac-window-test.sh <passbundle.app>
set -euo pipefail

bundle=$1
exe="$bundle/Contents/MacOS/passbundle"
root="$(cd "$(dirname "$0")/.." && pwd)"
work=$(mktemp -d)
helper="$work/mac-window"
swiftc -O "$root/tools/mac-window.swift" -o "$helper"

id=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$bundle/Contents/Info.plist")
defaults delete "$id" "NSWindow Frame PassBundle" 2> /dev/null || true
double_click=$(defaults read -g AppleActionOnDoubleClick 2> /dev/null || echo unset)
pid=""
cleanup() {
  [ -z "$pid" ] || kill "$pid" 2> /dev/null || true
  defaults delete "$id" "NSWindow Frame PassBundle" 2> /dev/null || true
  if [ "$double_click" = unset ]; then
    defaults delete -g AppleActionOnDoubleClick 2> /dev/null || true
  else
    defaults write -g AppleActionOnDoubleClick "$double_click"
  fi
}
trap cleanup EXIT

lines=""
launch() {
  local data
  data=$(mktemp -d)
  cp "$root/app/test/sample/settings.json" "$root/app/test/sample/sync.json" "$data/"
  lines=$(mktemp)
  PB_MEASURE=1 PB_MEASURE_FILE="$lines" PB_MEASURE_PASSWORD='correct horse battery staple 1' \
    PB_DATA_DIR="$data" PB_SECRETS=memory "$exe" > /dev/null 2>&1 &
  pid=$!
  for _ in $(seq 1 120); do
    grep -q '^ready' "$lines" 2> /dev/null && break
    kill -0 "$pid" 2> /dev/null || { echo "The app quit before it was ready." >&2; exit 1; }
    sleep 0.5
  done
  grep -q '^ready' "$lines" || { echo "The app wasn't ready within a minute." >&2; exit 1; }
  osascript -e "tell application \"System Events\" to set frontmost of (first process whose unix id is $pid) to true" > /dev/null 2>&1 || true
  sleep 1
}

quit() {
  kill "$pid" 2> /dev/null || true
  wait "$pid" 2> /dev/null || true
  pid=""
}

# What the helper says, as shell variables: X Y W H for the window, and
# CLOSE_X CLOSE_Y (the close button's left edge and middle, from the
# window's top left), MINIMIZED, FULL_SCREEN, and ID.
look() {
  eval "$("$helper" window "$pid" | python3 -c '
import json, sys
w = json.load(sys.stdin)
f, c = w["frame"], w["lights"]["close"]
print("X=%d Y=%d W=%d H=%d" % (round(f["x"]), round(f["y"]), round(f["width"]), round(f["height"])))
if c:
    print("CLOSE_X=%d CLOSE_Y=%d" % (round(c["x"] - f["x"]), round(c["y"] + c["height"] / 2 - f["y"])))
print("MINIMIZED=%s FULL_SCREEN=%s ID=%d" % (str(w["minimized"]).lower(), str(w["fullScreen"]).lower(), w["id"]))
')"
}

near() { python3 -c "import sys; sys.exit(0 if abs($1 - ($2)) <= ${3:-1} else 1)"; }

launch
look
echo "Opened at ${W}x${H}+${X}+${Y}."

# The app's view fills the window: there's no title bar over it.
view=$(awk '/^view/ { print $2 }' "$lines")
[ "$view" = "${W}x${H}" ] || { echo "The app's view is $view in a ${W}x${H} window: there's a title bar." >&2; exit 1; }
echo "No title bar: the app's view is the whole ${W}x${H} window."

# The traffic lights in the menu's top, lined up with the list's search.
near "$CLOSE_X" 26 || { echo "The close button is $CLOSE_X in, not 26." >&2; exit 1; }
near "$CLOSE_Y" 36 || { echo "The traffic lights' middle is $CLOSE_Y down, not 36." >&2; exit 1; }
echo "The traffic lights are at $CLOSE_X, $CLOSE_Y, in the menu's top."
# And still there after the window changes size, when AppKit lays its
# title bar out again.
"$helper" set "$pid" frame "$X" "$Y" $((W - 60)) $((H - 40))
sleep 1
look
near "$CLOSE_X" 26 && near "$CLOSE_Y" 36 || { echo "Resized, the lights went to $CLOSE_X, $CLOSE_Y." >&2; exit 1; }

# Dragging the band where nothing is moves the window (the item's side, no
# item chosen); dragging the search field doesn't.
before_x=$X before_y=$Y
"$helper" drag $((X + W - 150)) $((Y + 20)) $((X + W - 70)) $((Y + 80))
sleep 1
look
near "$X" $((before_x + 80)) 2 && near "$Y" $((before_y + 60)) 2 ||
  { echo "Dragging the band moved the window to $X, $Y from $before_x, $before_y." >&2; exit 1; }
echo "Dragging the band moved the window by 80, 60."
before_x=$X before_y=$Y
"$helper" drag $((X + 320)) $((Y + 36)) $((X + 400)) $((Y + 96))
sleep 1
look
[ "$X" = "$before_x" ] && [ "$Y" = "$before_y" ] ||
  { echo "Dragging the search field moved the window to $X, $Y." >&2; exit 1; }
echo "Dragging the search field didn't move the window."

# A double click on the band zooms, as System Settings says it does here,
# and again goes back.
defaults write -g AppleActionOnDoubleClick Maximize
before_w=$W
"$helper" click $((X + W - 150)) $((Y + 20)) 2
sleep 2
look
[ "$W" -gt "$before_w" ] || { echo "A double click on the band didn't zoom (${W} wide)." >&2; exit 1; }
"$helper" click $((X + W - 150)) $((Y + 20)) 2
sleep 2
look
near "$W" "$before_w" 2 || { echo "A second double click didn't zoom back (${W} wide, was $before_w)." >&2; exit 1; }
echo "A double click on the band zooms, and zooms back."

# The lights: minimize, and back; Option and zoom; full screen, where the
# app drops their room, and back; close.
"$helper" press "$pid" minimize
sleep 2
look
[ "$MINIMIZED" = true ] || { echo "Minimize didn't." >&2; exit 1; }
"$helper" set "$pid" minimized false
sleep 2
look
[ "$MINIMIZED" = false ] || { echo "It didn't come back from the Dock." >&2; exit 1; }
"$helper" press "$pid" fullscreen
for _ in $(seq 1 20); do
  sleep 0.5
  look
  [ "$FULL_SCREEN" = true ] && break
done
[ "$FULL_SCREEN" = true ] || { echo "The green button didn't go full screen." >&2; exit 1; }
"$helper" set "$pid" fullscreen false
for _ in $(seq 1 20); do
  sleep 0.5
  look
  [ "$FULL_SCREEN" = false ] && break
done
[ "$FULL_SCREEN" = false ] || { echo "It didn't come out of full screen." >&2; exit 1; }
sleep 1
look
near "$CLOSE_X" 26 && near "$CLOSE_Y" 36 ||
  { echo "Out of full screen, the lights are at $CLOSE_X, $CLOSE_Y." >&2; exit 1; }
echo "Minimize, full screen, and back; the lights where they were."
"$helper" press "$pid" close
for _ in $(seq 1 20); do
  kill -0 "$pid" 2> /dev/null || break
  sleep 0.5
done
if kill -0 "$pid" 2> /dev/null; then echo "Close didn't close the window." >&2; exit 1; fi
pid=""
echo "Close closed it."

# No smaller than 400 by 560; and where and how big it was left, next time.
launch
look
"$helper" set "$pid" frame "$X" "$Y" 100 100
sleep 1
look
[ "$W" -ge 400 ] && [ "$H" -ge 560 ] || { echo "It went smaller than 400 by 560 (${W}x${H})." >&2; exit 1; }
echo "Made as small as it goes: ${W}x${H}."
"$helper" set "$pid" frame 60 80 900 700
sleep 1
look
left="${W}x${H}+${X}+${Y}"
echo "Left at $left."
quit
launch
look
again="${W}x${H}+${X}+${Y}"
quit
[ "$again" = "$left" ] || { echo "It opened at $again, not where it was left ($left)." >&2; exit 1; }
echo "The window opens again where and how big it was left."
