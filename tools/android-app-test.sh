#!/usr/bin/env bash
# The app's tests on an Android phone or emulator (docs/phones.md, "Order and
# tests"): builds the core for the device adb sees, makes the test's fixtures
# (the sample vault, and a throwaway self-signed certificate for the HTTPS
# check), and runs app/integration_test on it. Needs what android-core.sh
# needs, plus Flutter and openssl. ANDROID_SERIAL picks the device.
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

cd app
flutter test integration_test -d "$serial"
