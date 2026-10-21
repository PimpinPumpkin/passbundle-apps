#!/usr/bin/env bash
# PassBundle filling another app's sign-in form through Android's autofill
# (docs/phones.md, "Autofill"), on an emulator, the way someone would:
#
#   1. the app signs in to the test server (an account with a login for the
#      test app's androidapp:// link), turns on biometric unlock (debug builds'
#      stand-in for Android's prompt, which says yes), and locks
#      (integration_test/autofill_setup_test.dart);
#   2. PassBundle becomes the phone's autofill service;
#   3. in the test app (app/android/autofilltest), tapping the username offers
#      "Unlock PassBundle"; picking it unlocks, then offers the login; picking
#      that fills the form; signing in shows who signed in.
#
# Needs an emulator, a test server (PB_TEST_SERVER, tools/test-server.sh), and
# what tools/android-app-test.sh needs.
set -euo pipefail
cd "$(dirname "$0")/.."

serial=${ANDROID_SERIAL:-$(adb get-serialno)}
case "$serial" in
  emulator-*) ;;
  *) echo "Not an emulator ($serial): this changes the phone's autofill service." >&2; exit 1 ;;
esac
[ -n "${PB_TEST_SERVER:-}" ] || { echo "Set PB_TEST_SERVER to a test server (tools/test-server.sh)." >&2; exit 1; }
a() { adb -s "$serial" "$@"; }

screen() {
  a shell uiautomator dump /sdcard/pb-ui.xml > /dev/null 2>&1 || true
  a exec-out cat /sdcard/pb-ui.xml 2> /dev/null
}

# The center of the first thing on screen matching: by its text or
# description (a pattern), or by its resource id ("id=<id>").
where() {
  screen | python3 -c '
import re, sys
xml, want = sys.stdin.read(), sys.argv[1]
for node in re.finditer(r"<node [^>]*>", xml):
    n = node.group(0)
    if want.startswith("id="):
        found = ("resource-id=\"" + want[3:] + "\"") in n
    else:
        label = " ".join(t for t in re.findall(r"(?:text|content-desc)=\"([^\"]*)\"", n) if t)
        found = re.search(want, label) is not None
    if found:
        x1, y1, x2, y2 = map(int, re.search(r"bounds=\"\[(\d+),(\d+)\]\[(\d+),(\d+)\]\"", n).groups())
        print((x1 + x2) // 2, (y1 + y2) // 2)
        break
' "$1"
}

# Waits up to 30 seconds for something, then taps it.
tap() {
  local at
  for _ in $(seq 30); do
    at=$(where "$1")
    if [ -n "$at" ]; then
      # shellcheck disable=SC2086
      a shell input tap $at
      return 0
    fi
    sleep 1
  done
  echo "Never showed up on screen: $1" >&2
  screen | grep -oE '(text|resource-id)="[^"]+"' | sort -u | head -40 >&2
  return 1
}

# Waits up to 30 seconds for something to be on screen.
shown() {
  for _ in $(seq 30); do
    [ -n "$(where "$1")" ] && return 0
    sleep 1
  done
  echo "Never showed up on screen: $1" >&2
  screen | grep -oE '(text|resource-id)="[^"]+"' | sort -u | head -40 >&2
  return 1
}

echo "The test app"
(cd app/android && ./gradlew -q :autofilltest:assembleDebug)
a install -r app/build/autofilltest/outputs/apk/debug/autofilltest-debug.apk > /dev/null

echo "PassBundle, signed in with biometric unlock on, and locked"
port=${PB_TEST_SERVER##*:}
a reverse "tcp:$port" "tcp:$port" > /dev/null
email="autofill-$(od -An -N4 -tx1 /dev/urandom | tr -d ' \n')@core.example.test"
password="correct horse battery staple autofill"
(cd core && cargo run --locked --quiet --example test_account -- "$PB_TEST_SERVER" "$email" "$password") > /dev/null
# It waits, installed and locked, while this fills the other app, and ends
# when this leaves a file in its folder (the test runner uninstalls the app
# when its tests end).
setup_log=$(mktemp)
(cd app && flutter test integration_test/autofill_setup_test.dart -d "$serial" \
  --dart-define="PB_TEST_SERVER=$PB_TEST_SERVER" --dart-define="PB_TEST_EMAIL=$email" \
  --dart-define="PB_TEST_PASSWORD=$password" --dart-define=PB_AUTOFILL_SETUP=true) > "$setup_log" 2>&1 &
setup=$!
trap 'kill "$setup" 2> /dev/null || true' EXIT
for _ in $(seq 300); do
  grep -q PB-AUTOFILL-READY "$setup_log" && break
  kill -0 "$setup" 2> /dev/null || { cat "$setup_log" >&2; echo "The app's half stopped early." >&2; exit 1; }
  sleep 1
done
grep -q PB-AUTOFILL-READY "$setup_log" || { cat "$setup_log" >&2; exit 1; }

echo "PassBundle as the phone's autofill service"
a shell settings put secure autofill_service com.passbundle.app/.PassBundleAutofill
[ "$(a shell settings get secure autofill_service | tr -d '\r')" = com.passbundle.app/.PassBundleAutofill ]

echo "Filling the test app's sign-in form"
a shell input keyevent KEYCODE_WAKEUP
a shell wm dismiss-keyguard
a shell am start -W -n com.passbundle.autofilltest/.SignInActivity > /dev/null
tap "id=com.passbundle.autofilltest:id/username"
# Locked: Android's list offers to unlock, then the login.
tap "^Unlock PassBundle"
tap "^Autofill test app"
shown "^autofill@example.test$"
tap "id=com.passbundle.autofilltest:id/sign_in"
shown "^Signed in as autofill@example.test, password 28 characters$"
echo "Filled: signed in as autofill@example.test, with the 28-character password from the vault."

# The app's half can end now.
a shell run-as com.passbundle.app touch files/autofill-done
wait "$setup"
tail -3 "$setup_log"
