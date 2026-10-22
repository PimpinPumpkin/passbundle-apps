#!/usr/bin/env bash
# Makes app/integration_test/fixtures.g.dart, which the phone tests need and
# which isn't committed: the sample vault (app/test/sample), its passwords
# (the core's test vectors), and a throwaway self-signed certificate for the
# HTTPS check. tools/android-app-test.sh runs it, and so does CI before
# analyzing the app. Needs python3 and openssl.
set -euo pipefail
cd "$(dirname "$0")/.."

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
openssl req -x509 -newkey rsa:2048 -nodes -days 2 -subj "/CN=localhost" \
  -addext "subjectAltName=DNS:localhost" -addext "basicConstraints=critical,CA:FALSE" \
  -addext "extendedKeyUsage=serverAuth" \
  -keyout "$tmp/key.pem" -out "$tmp/cert.pem" 2> /dev/null
python3 - "$tmp" > app/integration_test/fixtures.g.dart << 'PY'
import sys
tmp = sys.argv[1]
def raw(text):
    assert "'''" not in text
    return "r'''" + text + "'''"
print("// Made by tools/phone-test-fixtures.sh; not committed.")
print("const sampleSettings = " + raw(open("app/test/sample/settings.json").read()) + ";")
print("const sampleSync = " + raw(open("app/test/sample/sync.json").read()) + ";")
# The sample vault's master password, and its Backup service item's password (the core's test vectors).
print("const samplePassword = 'correct horse battery staple 1';")
print("const samplePasswordOfBackupService = 'not-a-real-password-3';")
print("const certificatePem = " + raw(open(tmp + "/cert.pem").read()) + ";")
print("const privateKeyPem = " + raw(open(tmp + "/key.pem").read()) + ";")
PY
