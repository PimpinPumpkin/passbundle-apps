#!/usr/bin/env bash
# PassBundle filling a website's sign-in form in Safari through iOS's AutoFill
# (docs/phones.md, "Autofill"), on an iPhone simulator, the way someone would:
#
#   1. the app signs in to the test server (an account with a login for
#      https://phone.example.test) and turns on Face ID unlock (debug builds'
#      stand-in for the prompt) (integration_test/ios_autofill_setup_test.dart);
#   2. Settings turns PassBundle on for AutoFill (AutoFillUITests.swift);
#   3. the app gives iOS its suggestions;
#   4. in Safari, on a sign-in page for phone.example.test (served here over
#      HTTPS, with a throwaway authority the simulator trusts), the email
#      field offers PassBundle's login; picking it opens the extension, which
#      unlocks and fills; signing in shows who signed in.
#
# The two halves talk through files in the App Group folder, which this
# reaches from the Mac. CI only: it maps phone.example.test to this Mac in
# /etc/hosts. Needs a test server (PB_TEST_SERVER, tools/test-server.sh) and
# what tools/ios-app-test.sh needs.
set -euo pipefail
cd "$(dirname "$0")/.."

[ -n "${CI:-}" ] || { echo "CI only: this maps phone.example.test to this Mac in /etc/hosts." >&2; exit 1; }
[ -n "${PB_TEST_SERVER:-}" ] || { echo "Set PB_TEST_SERVER to a test server (tools/test-server.sh)." >&2; exit 1; }
device=$(tools/ios-core.sh simulator)
[ -d app/ios/PassBundleCore.xcframework ] || tools/ios-core.sh build
[ -f app/integration_test/fixtures.g.dart ] || tools/phone-test-fixtures.sh

echo "The sign-in page, over HTTPS, as phone.example.test"
work=$(mktemp -d)
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj "/CN=PassBundle AutoFill test authority" \
  -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign" \
  -keyout "$work/ca.key" -out "$work/ca.pem" > /dev/null 2>&1
openssl req -newkey rsa:2048 -nodes -subj "/CN=phone.example.test" -keyout "$work/key.pem" -out "$work/request.pem" > /dev/null 2>&1
printf 'subjectAltName=DNS:phone.example.test\nextendedKeyUsage=serverAuth\nbasicConstraints=CA:FALSE\n' > "$work/extensions"
openssl x509 -req -in "$work/request.pem" -CA "$work/ca.pem" -CAkey "$work/ca.key" -CAcreateserial -days 1 \
  -extfile "$work/extensions" -out "$work/cert.pem" > /dev/null 2>&1
xcrun simctl keychain "$device" add-root-cert "$work/ca.pem"
grep -q ' phone.example.test$' /etc/hosts || echo "127.0.0.1 phone.example.test" | sudo tee -a /etc/hosts > /dev/null
port=8462
python3 -I tools/ios-autofill/serve.py "$port" "$work/cert.pem" "$work/key.pem" &
page=$!

echo "PassBundle, signed in with Face ID unlock on"
email="iphone-autofill-$(od -An -N4 -tx1 /dev/urandom | tr -d ' \n')@core.example.test"
password="correct horse battery staple iphone autofill"
(cd core && cargo run --locked --quiet --example test_account -- "$PB_TEST_SERVER" "$email" "$password") > /dev/null
setup_log=$(mktemp)
(cd app && flutter test integration_test/ios_autofill_setup_test.dart -d "$device" \
  --dart-define="PB_TEST_SERVER=$PB_TEST_SERVER" --dart-define="PB_TEST_EMAIL=$email" \
  --dart-define="PB_TEST_PASSWORD=$password" --dart-define=PB_AUTOFILL_SETUP=true) > "$setup_log" 2>&1 &
setup=$!
trap 'kill "$setup" "$page" 2> /dev/null || true' EXIT

# The App Group folder, where the app keeps the vault: the files the halves leave.
folder() {
  local group
  group=$(xcrun simctl get_app_container "$device" com.passbundle.app group.com.passbundle.app 2> /dev/null) || return 1
  echo "$group/PassBundle"
}
# Waits up to ten minutes for the app's half to leave a file.
reached() {
  for _ in $(seq 600); do
    [ -f "$(folder)/$1" ] && return 0
    kill -0 "$setup" 2> /dev/null || break
    sleep 1
  done
  cat "$setup_log" >&2
  echo "$2" >&2
  exit 1
}
reached autofill-ready "The app's half never got ready."

# One of AutoFillUITests' tests, in Settings or Safari.
ui_test() {
  (cd app/ios && xcodebuild test -project Runner.xcodeproj -scheme AutoFillUITests -destination "id=$device" \
    -only-testing:"AutoFillUITests/AutoFillUITests/$1" -quiet)
}

echo "PassBundle on for AutoFill, in Settings"
ui_test testTurnOnInSettings
touch "$(folder)/autofill-enabled"
reached autofill-suggested "iOS never took PassBundle's suggestions."

echo "Filling the sign-in page in Safari"
xcrun simctl openurl "$device" "https://phone.example.test:$port/"
ui_test testFillInSafari
echo "Filled: signed in as phone@example.test, with the 25-character password from the vault."

# The app's half can end now.
touch "$(folder)/autofill-done"
wait "$setup"
tail -3 "$setup_log"
