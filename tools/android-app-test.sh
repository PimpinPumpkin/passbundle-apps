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

cd app
flutter test integration_test -d "$serial" ${defines[@]+"${defines[@]}"}
