#!/usr/bin/env bash
# Enrolls a fingerprint on an Android emulator, for the app's biometric
# tests (docs/phones.md, "Order and tests"): sets a screen lock PIN, walks
# Settings' own enrollment by what's on the screen, and touches the
# emulator's simulated sensor. For throwaway test emulators only: it changes
# the device's lock screen, so it refuses anything but an emulator.
#
#   ANDROID_SERIAL=emulator-5554 tools/android-enroll-fingerprint.sh
set -euo pipefail

serial=${ANDROID_SERIAL:-$(adb get-serialno)}
case "$serial" in
  emulator-*) ;;
  *) echo "Not an emulator ($serial): this changes the lock screen, so it's for test emulators only." >&2; exit 1 ;;
esac
a() { adb -s "$serial" "$@"; }
pin=1111

screen() {
  a exec-out uiautomator dump /dev/tty 2> /dev/null | sed 's/UI hierchary dumped to.*//'
}

# Taps the first thing on screen whose text or description matches the
# pattern; fails if nothing does.
tap() {
  local where
  where=$(screen | python3 -c '
import re, sys
xml = sys.stdin.read()
pattern = re.compile(sys.argv[1], re.I)
for node in re.finditer(r"<node [^>]*>", xml):
    n = node.group(0)
    label = " ".join(t for t in re.findall(r"(?:text|content-desc)=\"([^\"]*)\"", n) if t)
    if pattern.search(label):
        x1, y1, x2, y2 = map(int, re.search(r"bounds=\"\[(\d+),(\d+)\]\[(\d+),(\d+)\]\"", n).groups())
        print((x1 + x2) // 2, (y1 + y2) // 2)
        break
' "$1")
  [ -n "$where" ] || return 1
  # shellcheck disable=SC2086
  a shell input tap $where
}

# Waits up to 20 seconds for something matching the pattern, and taps it.
tap_when_shown() {
  for _ in $(seq 20); do
    tap "$1" && return 0
    sleep 1
  done
  echo "Never showed up on screen: $1" >&2
  screen | grep -o 'text="[^"]*"' | sort -u | head -40 >&2
  return 1
}

enrolled() {
  a shell dumpsys fingerprint 2> /dev/null | grep -qE '"count":[1-9]'
}

if enrolled; then
  echo "A fingerprint is already enrolled on $serial."
  exit 0
fi

a shell input keyevent KEYCODE_WAKEUP
a shell wm dismiss-keyguard
# A PIN set by an earlier run stays (setting it again needs the old one).
a shell locksettings set-pin "$pin" > /dev/null 2>&1 || true
a shell am start -a android.settings.FINGERPRINT_ENROLL > /dev/null
sleep 3
# Settings asks for the PIN first.
a shell input text "$pin"
a shell input keyevent KEYCODE_ENTER
sleep 3
# The introduction: "More" until "I agree" (its wording varies by version).
for _ in $(seq 6); do
  tap "^(I agree|Agree|Next)$" && break
  tap "^More$" || true
  sleep 1
done
# "Touch the sensor" (some versions have a "Next" on the way), then the touches.
tap "^(Next|Start)$" || true
sleep 2
for _ in $(seq 15); do
  a emu finger touch 1 > /dev/null
  sleep 1
  a emu finger remove 1 > /dev/null 2>&1 || true
  sleep 1
done
tap_when_shown "^(Done|Next)$"
sleep 1
a shell input keyevent KEYCODE_HOME
if enrolled; then
  echo "Enrolled a fingerprint on $serial."
else
  echo "Couldn't enroll a fingerprint on $serial." >&2
  a shell dumpsys fingerprint | head -20 >&2
  exit 1
fi
