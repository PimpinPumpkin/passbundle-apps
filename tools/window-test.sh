#!/usr/bin/env bash
# Checks the app's window on Linux, under X (CI runs it on Xvfb, with Xfce's
# window manager, xfwm4, and its compositor, in a D-Bus session of its own):
#
# - the app draws the window's top (client-side decorations): the window
#   manager draws no title bar (_NET_FRAME_EXTENTS 0), and GTK draws only
#   the window's edges (_GTK_FRAME_EXTENTS);
# - the app's buttons, those GTK's gtk-decoration-layout asks for, where it
#   asks for them (the end by default, the start when it says so), and they
#   minimize, maximize and restore, and close;
# - dragging the band moves the window and a double click maximizes it,
#   but dragging the search field doesn't move it;
# - without a compositor, the window manager's own title bar instead;
# - it asks for at least 400 by 560, opens no bigger than the screen, and
#   opens again where and how big it was left, kept in its folder
#   (~/.local/share/PassBundle, here a throwaway XDG_DATA_HOME).
#
# Needs xdotool, xprop, and xfwm4, and a build with test hooks
# (tools/measure.sh says how), opened on the sample vault so the vault shows.
#
#   DISPLAY=:98 dbus-run-session -- tools/window-test.sh <executable>
set -euo pipefail

exe=$1
root="$(cd "$(dirname "$0")/.." && pwd)"
home=$(mktemp -d)
config=$(mktemp -d)
pid=""
wm=""
trap '[ -z "$pid" ] || kill "$pid" 2> /dev/null || true; [ -z "$wm" ] || kill "$wm" 2> /dev/null || true' EXIT

# Xfce's window manager, with its compositor on or off.
window_manager() {
  if [ -n "$wm" ]; then
    kill "$wm" 2> /dev/null || true
    wait "$wm" 2> /dev/null || true
  fi
  xfwm4 --compositor="$1" --replace > /dev/null 2>&1 &
  wm=$!
  for _ in $(seq 1 40); do
    xprop -root _NET_SUPPORTING_WM_CHECK 2> /dev/null | grep -q 'window id' && break
    sleep 0.25
  done
  sleep 2
}

launch() {
  local data
  data=$(mktemp -d)
  cp "$root/app/test/sample/settings.json" "$root/app/test/sample/sync.json" "$data/"
  local lines
  lines=$(mktemp)
  XDG_DATA_HOME="$home" XDG_CONFIG_HOME="$config" PB_DATA_DIR="$data" PB_SECRETS=memory \
    PB_MEASURE=1 PB_MEASURE_FILE="$lines" PB_MEASURE_PASSWORD='correct horse battery staple 1' \
    "$exe" > /dev/null 2>&1 &
  pid=$!
  window=""
  for _ in $(seq 1 120); do
    grep -q '^ready' "$lines" 2> /dev/null && break
    kill -0 "$pid" 2> /dev/null || { echo "The app quit before it was ready." >&2; exit 1; }
    sleep 0.5
  done
  grep -q '^ready' "$lines" || { echo "The app wasn't ready within a minute." >&2; exit 1; }
  window=$(xdotool search --onlyvisible --pid "$pid" 2> /dev/null | head -1 || true)
  [ -n "$window" ] || { echo "The app's window didn't show." >&2; exit 1; }
  xdotool windowactivate --sync "$window" 2> /dev/null || true
  sleep 1
}

quit() {
  kill "$pid"
  wait "$pid" 2> /dev/null || true
  pid=""
}

# The window's X geometry (with GTK's edges), as WIDTH HEIGHT X Y, and its
# content's: CW CH CX CY.
geometry() {
  eval "$(xdotool getwindowgeometry --shell "$window")"
  local extents
  extents=$(xprop -id "$window" _GTK_FRAME_EXTENTS 2> /dev/null | sed -n 's/.*= //p' | tr -d ' ')
  IFS=, read -r EL ER ET EB <<< "${extents:-0,0,0,0}"
  CW=$((WIDTH - EL - ER)) CH=$((HEIGHT - ET - EB)) CX=$((X + EL)) CY=$((Y + ET))
}

place() { geometry; echo "${WIDTH}x${HEIGHT}+${X}+${Y}"; }

state() { xprop -id "$window" _NET_WM_STATE 2> /dev/null || true; }

# A click at a point of the app's content.
click() {
  geometry
  xdotool mousemove --sync $((CX + $1)) $((CY + $2)) click "${3:-1}"
}

drag() {
  geometry
  xdotool mousemove --sync $((CX + $1)) $((CY + $2)) mousedown 1
  for _ in $(seq 1 10); do
    xdotool mousemove_relative --sync 8 6
    sleep 0.05
  done
  xdotool mouseup 1
  sleep 1
}

read -r screen_width screen_height < <(xdotool getdisplaygeometry)

window_manager on
launch
geometry
echo "Opened at $(place) on a ${screen_width}x${screen_height} screen, its content ${CW}x${CH}."

# The app draws the top: no title bar from the window manager, and GTK's
# own edges around the app.
frame=$(xprop -id "$window" _NET_FRAME_EXTENTS | sed -n 's/.*= //p' | tr -d ' ')
[ "$frame" = "0,0,0,0" ] || { echo "The window manager draws a frame ($frame): the app doesn't draw its own top." >&2; exit 1; }
xprop -id "$window" _GTK_FRAME_EXTENTS | grep -q '=' ||
  { echo "GTK doesn't draw the window's edges (no _GTK_FRAME_EXTENTS)." >&2; exit 1; }
echo "No title bar but the app's: client-side decorations, GTK's edges $EL,$ER,$ET,$EB."

[ "$CW" -ge 400 ] && [ "$CH" -ge 560 ] || { echo "Opened smaller than 400 by 560." >&2; exit 1; }
[ "$WIDTH" -le "$screen_width" ] && [ "$HEIGHT" -le "$screen_height" ] || { echo "Opened bigger than the screen." >&2; exit 1; }
# At least 400 by 560 for the app, and GTK's edges around it.
xprop -id "$window" WM_NORMAL_HINTS | tee /dev/stderr | grep -q "minimum size: $((400 + EL + ER)) by $((560 + ET + EB))" ||
  { echo "The window doesn't ask for at least 400 by 560 and GTK's edges." >&2; exit 1; }
# And made as small as it goes, the app has at least that (GTK's edges are
# outside it).
before=$(place)
xdotool windowsize "$window" 100 100
sleep 1
geometry
echo "Made as small as it goes: its content ${CW}x${CH}."
[ "$CW" -ge 400 ] && [ "$CH" -ge 560 ] || { echo "It went smaller than 400 by 560 (${CW}x${CH})." >&2; exit 1; }
IFS=x+ read -r w h _ _ <<< "$before"
xdotool windowsize "$window" "$w" "$h"
sleep 1

# Dragging the band where nothing is (the item's side, no item chosen)
# moves the window; dragging the search field doesn't.
before="$X,$Y"
drag $((CW - 150)) 20
geometry
[ "$X,$Y" != "$before" ] || { echo "Dragging the band didn't move the window." >&2; exit 1; }
echo "Dragging the band moved the window from $before to $X,$Y."
before="$X,$Y"
drag 320 36
geometry
[ "$X,$Y" = "$before" ] || { echo "Dragging the search field moved the window to $X,$Y." >&2; exit 1; }
echo "Dragging the search field didn't."

# A double click on the band maximizes, and again restores.
xdotool mousemove --sync $((CX + CW - 150)) $((CY + 20)) click --repeat 2 --delay 80 1
sleep 1
state | grep -q _NET_WM_STATE_MAXIMIZED_VERT || { echo "A double click on the band didn't maximize." >&2; exit 1; }
geometry
xdotool mousemove --sync $((CX + CW - 150)) $((CY + 20)) click --repeat 2 --delay 80 1
sleep 1
state | grep -q _NET_WM_STATE_MAXIMIZED_VERT && { echo "A second double click didn't restore." >&2; exit 1; }
echo "A double click on the band maximizes, and restores."

# The buttons GTK's default layout asks for, at the end: minimize,
# maximize, close, 24 across and 10 apart, 20 in from the right, their
# middles 36 down, in line with the search.
geometry
click $((CW - 20 - 12 - 34 * 2)) 36
sleep 1
state | grep -q _NET_WM_STATE_HIDDEN || { echo "Minimize didn't." >&2; exit 1; }
xdotool windowactivate --sync "$window" 2> /dev/null || xdotool windowmap --sync "$window"
sleep 1
state | grep -q _NET_WM_STATE_HIDDEN && { echo "It didn't come back from being minimized." >&2; exit 1; }
geometry
click $((CW - 20 - 12 - 34)) 36
sleep 1
state | grep -q _NET_WM_STATE_MAXIMIZED_VERT || { echo "Maximize didn't." >&2; exit 1; }
geometry
click $((CW - 20 - 12 - 34)) 36
sleep 1
state | grep -q _NET_WM_STATE_MAXIMIZED_VERT && { echo "Restore didn't." >&2; exit 1; }
geometry
click $((CW - 20 - 12)) 36
for _ in $(seq 1 20); do
  kill -0 "$pid" 2> /dev/null || break
  sleep 0.5
done
if kill -0 "$pid" 2> /dev/null; then echo "Close didn't close the window." >&2; exit 1; fi
pid=""
echo "Minimize, maximize, restore, and close."

# Where the desktop puts them at the start (elementary's and a Mac's way):
# there, and the first is close.
mkdir -p "$config/gtk-3.0"
printf '[Settings]\ngtk-decoration-layout=close,minimize,maximize:\n' > "$config/gtk-3.0/settings.ini"
launch
geometry
click $((26 + 12)) 36
for _ in $(seq 1 20); do
  kill -0 "$pid" 2> /dev/null || break
  sleep 0.5
done
if kill -0 "$pid" 2> /dev/null; then echo "Close wasn't at the start." >&2; exit 1; fi
pid=""
rm -r "$config/gtk-3.0"
echo "With the buttons at the start, close is the first."

# Moved and sized as someone would, then the app ends without closing its
# window (the place is kept as the window settles).
launch
xdotool windowsize "$window" 900 640
xdotool windowmove "$window" 37 41
sleep 2
left=$(place)
echo "Left at $left."
quit
echo "Kept:"
cat "$home/PassBundle/window.ini"
launch
again=$(place)
echo "Opened again at $again."
quit
[ "$again" = "$left" ] || { echo "It opened at $again, not where it was left ($left)." >&2; exit 1; }
echo "The window opens again where and how big it was left."

# Without a compositor, the window can't have GTK's edges: the window
# manager's own title bar, and none of the app's.
window_manager off
launch
frame=$(xprop -id "$window" _NET_FRAME_EXTENTS | sed -n 's/.*= //p' | tr -d ' ')
quit
IFS=, read -r _ _ frame_top _ <<< "$frame"
[ "${frame_top:-0}" -gt 0 ] || { echo "Without a compositor, the window manager drew no title bar ($frame)." >&2; exit 1; }
echo "Without a compositor, the window manager's title bar ($frame)."
