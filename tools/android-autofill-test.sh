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
#      that fills the form; signing in shows who signed in;
#   3b. on the test app's screen that says it's a bank's website, only the
#      app's own login is offered, with an offer to trust the app as a
#      browser; trusting it in PassBundle's dialog offers the website's login,
#      which fills;
#   4. signing in there with something new, Android offers to save it to
#      PassBundle; accepting opens PassBundle's editor, filled in, and the
#      app's half saves it;
#   5. signing in again with a new password for it, accepting opens that
#      login, with the new password, and the app's half saves it.
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

# The device, back and booted (CI's emulator can drop off adb for a moment between installs).
a wait-for-device
for _ in $(seq 60); do
  [ "$(a shell getprop sys.boot_completed 2> /dev/null | tr -d '\r')" = 1 ] && break
  sleep 2
done

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

# A web page's Email field (WebSignInActivity), however this Android's
# WebView shows it, tapped.
web_field() {
  local at
  for _ in $(seq 30); do
    at=$(where "id=username")
    [ -n "$at" ] || at=$(where "^Email$")
    if [ -n "$at" ]; then
      # shellcheck disable=SC2086
      a shell input tap $at
      return 0
    fi
    sleep 1
  done
  echo "The page's Email field never showed up." >&2
  screen | grep -oE '(text|resource-id|content-desc)="[^"]+"' | sort -u | head -40 >&2
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

# Every browser on the list is visible to PassBundle (its manifest's <queries>).
python3 - app/android/app/src/main/assets/browsers.json app/android/app/src/main/AndroidManifest.xml <<'PY'
import json, re, sys
listed = {a["info"]["package_name"] for a in json.load(open(sys.argv[1]))["apps"] if a["type"] == "android"}
queried = set(re.findall(r'<package android:name="([^"]+)"', open(sys.argv[2]).read()))
missing = sorted(listed - queried)
if missing:
    sys.exit(f"Browsers missing from the manifest's <queries>: {', '.join(missing)}")
PY

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
# when its tests end). It says how far it got with files in its folder too,
# since the test runner can hold what it prints until the test ends.
marker() { [ "$(a shell run-as com.passbundle.app ls "files/$1" 2> /dev/null | tr -d '\r')" = "files/$1" ]; }
a shell run-as com.passbundle.app rm -f files/autofill-ready files/autofill-saved files/autofill-updated files/autofill-done > /dev/null 2>&1 || true
setup_log=$(mktemp)
(cd app && flutter test integration_test/autofill_setup_test.dart -d "$serial" \
  --dart-define="PB_TEST_SERVER=$PB_TEST_SERVER" --dart-define="PB_TEST_EMAIL=$email" \
  --dart-define="PB_TEST_PASSWORD=$password" --dart-define=PB_AUTOFILL_SETUP=true) > "$setup_log" 2>&1 &
setup=$!
trap 'kill "$setup" 2> /dev/null || true' EXIT
for _ in $(seq 300); do
  marker autofill-ready && break
  kill -0 "$setup" 2> /dev/null || { cat "$setup_log" >&2; echo "The app's half stopped early." >&2; exit 1; }
  sleep 1
done
marker autofill-ready || { cat "$setup_log" >&2; echo "The app's half never got ready." >&2; exit 1; }

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

echo "A sign-in form on a web page in an app's own WebView"
a shell am force-stop com.passbundle.autofilltest
a shell am start -W -n com.passbundle.autofilltest/.WebSignInActivity --es site https://elsewhere.test/ > /dev/null
web_field
# PassBundle reads the page's form, attributes without values and all: the
# app's own login, never a website's for a page an app shows.
shown "^Autofill test app"
[ -z "$(where '^Bank website')" ] || { echo "A website's login was offered to a page in an app's WebView." >&2; exit 1; }
[ -n "$(a shell pidof com.passbundle.app | tr -d '\r')" ] || { echo "PassBundle stopped while reading the page." >&2; exit 1; }
a shell am force-stop com.passbundle.autofilltest
echo "Offered: the app's own login on a web page in it, and PassBundle kept running."

# Android 10 doesn't pass autofill a web domain an app sets on its own views
# (there only a WebView gives one), so this screen can't say it's a website
# there: it runs from Android 11 (CI's Android 14 emulator).
if [ "$(a shell getprop ro.build.version.sdk | tr -d '\r')" -ge 30 ]; then
  echo "A screen that says it's a website, in an app that isn't a known browser"
  a shell am force-stop com.passbundle.autofilltest
  a shell am start -W -n com.passbundle.autofilltest/.ClaimsWebsiteActivity > /dev/null
  tap "id=com.passbundle.autofilltest:id/username"
  # The app's own login, and an offer to trust it as a browser; never the website's.
  shown "^Autofill test app"
  shown "^Trust Autofill test as a browser first"
  [ -z "$(where '^Bank website')" ] || { echo "The website's login was offered to an app that isn't a browser." >&2; exit 1; }
  # Trusted, in PassBundle's own dialog: then the website's login fills.
  tap "^Trust Autofill test as a browser first"
  shown "^Trust Autofill test as a browser\?"
  # Trust can't be tapped for its first moment.
  sleep 2
  tap "^(Trust|TRUST)$"
  tap "^Bank website"
  shown "^bank@example.test$"
  tap "id=com.passbundle.autofilltest:id/sign_in"
  shown "^Signed in as bank@example.test, password 24 characters$"
  echo "Filled: the website's login only once the app was trusted as a browser."
fi

# Signs in to the test app with something typed, as someone would (Android
# offers to save only what changed after its autofill started): the username,
# Tab to the password, Tab to the button, Enter. Each field opens Android's
# list, and typing closes it (nothing in it matches), losing what's typed
# while it closes: one letter, a wait, the rest. Then accepts Android's offer
# to save it, and opens PassBundle if Android didn't.
type_in() {
  sleep 2
  a shell input text "${1:0:1}"
  sleep 2
  a shell input text "${1:1}"
}
in_front() { [[ "$(a shell dumpsys activity activities | grep -E 'ResumedActivity[=:]')" == *com.passbundle.app/* ]]; }
sign_in_and_save() {
  a shell am force-stop com.passbundle.autofilltest
  a shell am start -W -n com.passbundle.autofilltest/.SignInActivity > /dev/null
  tap "id=com.passbundle.autofilltest:id/username"
  type_in "$1"
  a shell input keyevent KEYCODE_TAB
  type_in "$2"
  a shell input keyevent KEYCODE_TAB
  a shell input keyevent KEYCODE_ENTER
  # Android's offer, once the sign-in is done (it covers the test app's
  # screen), then PassBundle's editor, where the app's half checks every
  # letter came through and saves it.
  tap "^(Save|SAVE)$"
  for _ in $(seq 15); do
    in_front && break
    sleep 1
  done
  if ! in_front; then
    # Android 14 before its March 2024 update stops the app opening itself
    # here; the sign-in waits for the person to open it.
    [ "$(a shell getprop ro.build.version.sdk | tr -d '\r')" = 34 ] || { echo "PassBundle never opened to save." >&2; exit 1; }
    echo "Android 14 kept PassBundle from opening itself; opening it, as someone would."
    a shell am start -n com.passbundle.app/.MainActivity > /dev/null
  fi
}

# Waits up to two minutes for the app's half to leave a file.
reached() {
  for _ in $(seq 120); do
    marker "$1" && return 0
    kill -0 "$setup" 2> /dev/null || break
    sleep 1
  done
  marker "$1" || { cat "$setup_log" >&2; echo "$2" >&2; exit 1; }
}

echo "Saving a new sign-in"
sign_in_and_save new@example.test not-a-real-password-new
reached autofill-saved "PassBundle never saved the new sign-in."
echo "Saved: the new sign-in is in the vault."

echo "Saving a new password for it"
sign_in_and_save new@example.test not-a-real-password-changed
reached autofill-updated "PassBundle never updated the sign-in."
echo "Updated: the same login has the new password."

# The app's half can end now.
a shell run-as com.passbundle.app touch files/autofill-done
wait "$setup"
tail -3 "$setup_log"
