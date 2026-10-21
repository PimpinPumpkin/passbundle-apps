#!/usr/bin/env bash
# The app's tests on an Android phone or emulator (docs/phones.md, "Order and
# tests"): builds the core for the device adb sees, makes the test's fixtures
# (the sample vault, and a throwaway self-signed certificate for the HTTPS
# check), and runs app/integration_test on it. Needs what android-core.sh
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

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
openssl req -x509 -newkey rsa:2048 -nodes -days 2 -subj "/CN=localhost" \
  -addext "subjectAltName=DNS:localhost" -addext "basicConstraints=critical,CA:FALSE" \
  -addext "extendedKeyUsage=serverAuth" \
  -keyout "$tmp/key.pem" -out "$tmp/cert.pem" 2> /dev/null
python3 - "$tmp" > app/integration_test/fixtures.g.dart << 'EOF'
import json, sys
tmp = sys.argv[1]
def raw(text):
    assert "'''" not in text
    return "r'''" + text + "'''"
print("// Made by tools/android-app-test.sh; not committed.")
print("const sampleSettings = " + raw(open("app/test/sample/settings.json").read()) + ";")
print("const sampleSync = " + raw(open("app/test/sample/sync.json").read()) + ";")
# The sample vault's master password, and its Backup service item's password (the core's test vectors).
print("const samplePassword = 'correct horse battery staple 1';")
print("const samplePasswordOfBackupService = 'not-a-real-password-3';")
print("const certificatePem = " + raw(open(tmp + "/cert.pem").read()) + ";")
print("const privateKeyPem = " + raw(open(tmp + "/key.pem").read()) + ";")
EOF

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

# On an emulator, its simulated fingerprint sensor is touched every two
# seconds while the tests run, answering any fingerprint prompt they bring up
# (tools/android-enroll-fingerprint.sh enrolls the finger).
toucher=""
case "$serial" in
  emulator-*)
    (while true; do
      adb -s "$serial" emu finger touch 1 > /dev/null 2>&1 || true
      sleep 1
      adb -s "$serial" emu finger remove 1 > /dev/null 2>&1 || true
      sleep 1
    done) &
    toucher=$!
    defines+=(--dart-define=PB_TEST_FINGERPRINT=true)
    ;;
esac
trap 'rm -rf "$tmp"; [ -z "$toucher" ] || kill "$toucher" 2> /dev/null || true' EXIT

cd app
flutter test integration_test -d "$serial" ${defines[@]+"${defines[@]}"}
