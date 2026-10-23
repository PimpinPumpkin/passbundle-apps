#!/usr/bin/env bash
# The app's tests on an Android phone or emulator (docs/phones.md, "Order and
# tests"): builds the core for the device adb sees, makes the test's fixtures
# (tools/phone-test-fixtures.sh), and runs app/integration_test on it. Needs what android-core.sh
# needs, plus Flutter and openssl. ANDROID_SERIAL picks the device.
#
# With PB_TEST_SERVER set to a test server on this computer
# (tools/test-server.sh, http://127.0.0.1:<port>), the phone reaches it
# through adb, and the test also signs in to it with a new account.
set -euo pipefail
cd "$(dirname "$0")/.."

serial=${ANDROID_SERIAL:-$(adb get-serialno)}
abi=$(adb -s "$serial" shell getprop ro.product.cpu.abi | tr -d '\r')
tools/android-core.sh build "$abi"

tools/phone-test-fixtures.sh

defines=()
if [ -n "${PB_TEST_SERVER:-}" ]; then
  port=${PB_TEST_SERVER##*:}
  # The phone's own 127.0.0.1:<port> reaches this computer's.
  adb -s "$serial" reverse "tcp:$port" "tcp:$port" > /dev/null
  email="phone-$(od -An -N4 -tx1 /dev/urandom | tr -d ' \n')@core.example.test"
  password="correct horse battery staple phone"
  (cd core && cargo run --locked --quiet --example test_account -- "$PB_TEST_SERVER" "$email" "$password") > /dev/null
  defines=(--dart-define="PB_TEST_SERVER=$PB_TEST_SERVER" --dart-define="PB_TEST_EMAIL=$email" --dart-define="PB_TEST_PASSWORD=$password")
fi

# Android's log while the tests run, kept on this computer when
# PB_TEST_LOGCAT names a file: if the phone or emulator drops off adb in the
# middle, it's what's left to read (CI prints it).
if [ -n "${PB_TEST_LOGCAT:-}" ]; then
  adb -s "$serial" logcat -c || true
  adb -s "$serial" logcat -v time > "$PB_TEST_LOGCAT" 2>&1 &
  logcat=$!
  trap 'kill "$logcat" 2> /dev/null || true' EXIT
fi

cd app
# The phone tests; autofill_setup_test.dart is tools/android-autofill-test.sh's.
# Each test's own output: a summary alone says nothing about one that didn't
# finish.
flutter test integration_test/phone_test.dart -d "$serial" --reporter expanded ${defines[@]+"${defines[@]}"}
