#!/usr/bin/env bash
# Screenshots of the app's real window on a Mac or on Linux, light and dark:
# the window as the system shows it, its top edge included, on the sample
# vault with an item chosen. CI keeps them with each run (window-shots-*);
# tools/window-shots.ps1 takes Windows'. On Linux it needs a display
# (DISPLAY, such as Xvfb's) and starts Xfce's window manager, xfwm4, with
# its compositor, in a D-Bus session (dbus-run-session), unless one is
# running. Needs a build with test hooks (tools/measure.sh says how).
#
#   tools/window-shots.sh <executable> <folder> <name>
#
# writes <folder>/<name>-light.png and <folder>/<name>-dark.png.
set -euo pipefail

exe=$1
out=$2
name=$3
root="$(cd "$(dirname "$0")/.." && pwd)"
mkdir -p "$out"
pid=""
wm=""
trap '[ -z "$pid" ] || kill "$pid" 2> /dev/null || true; [ -z "$wm" ] || kill "$wm" 2> /dev/null || true' EXIT

if [ "$(uname)" = Darwin ]; then
  helper=$(mktemp -d)/mac-window
  swiftc -O "$root/tools/mac-window.swift" -o "$helper"
elif ! xprop -root _NET_SUPPORTING_WM_CHECK 2> /dev/null | grep -q 'window id'; then
  xfwm4 --compositor=on --replace > /dev/null 2>&1 &
  wm=$!
  sleep 3
fi

for theme in light dark; do
  data=$(mktemp -d)
  cp "$root/app/test/sample/settings.json" "$root/app/test/sample/sync.json" "$data/"
  lines=$(mktemp)
  home=$(mktemp -d)
  XDG_DATA_HOME="$home" PB_THEME="$theme" PB_MEASURE=1 PB_MEASURE_FILE="$lines" \
    PB_MEASURE_PASSWORD='correct horse battery staple 1' PB_DATA_DIR="$data" PB_SECRETS=memory \
    "$exe" > /dev/null 2>&1 &
  pid=$!
  for _ in $(seq 1 120); do
    grep -q '^ready' "$lines" 2> /dev/null && break
    kill -0 "$pid" 2> /dev/null || { echo "The app quit before it was ready." >&2; exit 1; }
    sleep 0.5
  done
  grep -q '^ready' "$lines" || { echo "The app wasn't ready within a minute." >&2; exit 1; }
  shot="$out/$name-$theme.png"
  if [ "$(uname)" = Darwin ]; then
    osascript -e "tell application \"System Events\" to set frontmost of (first process whose unix id is $pid) to true" > /dev/null 2>&1 || true
    sleep 1
    eval "$("$helper" window "$pid" | python3 -c '
import json, sys
w = json.load(sys.stdin)
f = w["frame"]
print("X=%d Y=%d ID=%d" % (round(f["x"]), round(f["y"]), w["id"]))
')"
    # An item, the first in the list, chosen: its actions show along the
    # top (a title bar, where there is one, only moves the list down).
    "$helper" click $((X + 380)) $((Y + 150))
    sleep 1.5
    screencapture -x -o -l"$ID" "$shot"
  else
    window=$(xdotool search --onlyvisible --pid "$pid" 2> /dev/null | head -1)
    xdotool windowactivate --sync "$window" 2> /dev/null || true
    sleep 1
    # Where the window is on the screen (xdotool's own position counts a
    # window manager's frame twice), and how big.
    X=$(xwininfo -id "$window" | awk '/Absolute upper-left X/ { print $NF }')
    Y=$(xwininfo -id "$window" | awk '/Absolute upper-left Y/ { print $NF }')
    WIDTH=$(xwininfo -id "$window" | awk '/^ *Width:/ { print $NF }')
    HEIGHT=$(xwininfo -id "$window" | awk '/^ *Height:/ { print $NF }')
    extents() { xprop -id "$window" "$1" 2> /dev/null | sed -n 's/.*= //p' | tr -d ' '; }
    IFS=, read -r gl gr gt gb <<< "$(extents _GTK_FRAME_EXTENTS | grep . || echo 0,0,0,0)"
    IFS=, read -r fl fr ft fb <<< "$(extents _NET_FRAME_EXTENTS | grep . || echo 0,0,0,0)"
    xdotool mousemove --sync $((X + gl + 380)) $((Y + gt + 150)) click 1
    sleep 1.5
    # The window as it's seen: the window manager's frame, if it has one,
    # and none of GTK's shadow, if it draws its own.
    import -window root \
      -crop "$((WIDTH - gl - gr + fl + fr))x$((HEIGHT - gt - gb + ft + fb))+$((X + gl - fl))+$((Y + gt - ft))" \
      +repage "$shot"
  fi
  kill "$pid"
  wait "$pid" 2> /dev/null || true
  pid=""
  echo "$shot"
done
