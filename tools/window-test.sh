#!/usr/bin/env bash
# Checks the app's window on Linux, under X (CI runs it on Xvfb): it asks for
# at least 400 by 560, opens no bigger than the screen, and opens again where
# and how big it was left, kept in its folder (~/.local/share/PassBundle,
# here a throwaway XDG_DATA_HOME). Needs xdotool and xprop, and a build with
# test hooks (tools/measure.sh says how), so it opens on an empty folder of
# its own.
#
#   DISPLAY=:98 tools/window-test.sh <executable>
set -euo pipefail

exe=$1
home=$(mktemp -d)
data=$(mktemp -d)
pid=""
trap '[ -z "$pid" ] || kill "$pid" 2> /dev/null || true' EXIT

launch() {
  XDG_DATA_HOME="$home" PB_DATA_DIR="$data" PB_SECRETS=memory "$exe" > /dev/null 2>&1 &
  pid=$!
  window=""
  for _ in $(seq 1 60); do
    window=$(xdotool search --onlyvisible --pid "$pid" 2> /dev/null | head -1 || true)
    [ -n "$window" ] && break
    kill -0 "$pid" 2> /dev/null || { echo "The app quit before its window showed." >&2; exit 1; }
    sleep 0.5
  done
  [ -n "$window" ] || { echo "The app's window didn't show within 30 seconds." >&2; exit 1; }
  # Its first frame, and its place settled.
  sleep 2
}

geometry() {
  eval "$(xdotool getwindowgeometry --shell "$window")"
  echo "${WIDTH}x${HEIGHT}+${X}+${Y}"
}

quit() {
  kill "$pid"
  wait "$pid" 2> /dev/null || true
  pid=""
}

read -r screen_width screen_height < <(xdotool getdisplaygeometry)

launch
first=$(geometry)
echo "Opened at $first on a ${screen_width}x${screen_height} screen."
eval "$(xdotool getwindowgeometry --shell "$window")"
[ "$WIDTH" -ge 400 ] && [ "$HEIGHT" -ge 560 ] || { echo "Opened smaller than 400 by 560." >&2; exit 1; }
[ "$WIDTH" -le "$screen_width" ] && [ "$HEIGHT" -le "$screen_height" ] || { echo "Opened bigger than the screen." >&2; exit 1; }
xprop -id "$window" WM_NORMAL_HINTS | tee /dev/stderr | grep -q "minimum size: 400 by 560" ||
  { echo "The window doesn't ask for at least 400 by 560." >&2; exit 1; }

# Moved and sized as someone would, then the app ends without closing its
# window (the place is kept as the window settles).
xdotool windowsize "$window" 900 640
xdotool windowmove "$window" 37 41
sleep 2
left=$(geometry)
echo "Left at $left."
quit
echo "Kept:"
cat "$home/PassBundle/window.ini"

launch
again=$(geometry)
echo "Opened again at $again."
quit
[ "$again" = "$left" ] || { echo "It opened at $again, not where it was left ($left)." >&2; exit 1; }
echo "The window opens again where and how big it was left."
