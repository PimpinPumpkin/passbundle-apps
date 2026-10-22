#!/usr/bin/env bash
# The app's tests on an iPhone simulator (docs/phones.md, "Order and tests"):
# builds the core for iOS (tools/ios-core.sh), makes the test's fixtures
# (tools/phone-test-fixtures.sh), and runs app/integration_test on a booted
# (or newly booted) iPhone simulator. Needs Xcode, Flutter, the Rust targets
# (rustup target add aarch64-apple-ios aarch64-apple-ios-sim), and openssl.
#
# With PB_TEST_SERVER set to a test server on this Mac (tools/test-server.sh,
# http://127.0.0.1:<port>), which the simulator reaches as the Mac does, the
# test also signs in to it with a new account.
set -euo pipefail
cd "$(dirname "$0")/.."

tools/ios-core.sh build
tools/phone-test-fixtures.sh
device=$(tools/ios-core.sh simulator)

defines=()
if [ -n "${PB_TEST_SERVER:-}" ]; then
  email="iphone-$(od -An -N4 -tx1 /dev/urandom | tr -d ' \n')@core.example.test"
  password="correct horse battery staple iphone"
  (cd core && cargo run --locked --quiet --example test_account -- "$PB_TEST_SERVER" "$email" "$password") > /dev/null
  defines=(--dart-define="PB_TEST_SERVER=$PB_TEST_SERVER" --dart-define="PB_TEST_EMAIL=$email" --dart-define="PB_TEST_PASSWORD=$password")
fi

cd app
# The simulator sometimes never finishes installing or launching the app
# after the build (simctl stalls, with no output). A try that's still going
# after 15 minutes (the test takes 5) is stopped, the simulator restarted,
# and the test run once more; a test that fails isn't run again.
try() {
  set -m
  flutter test integration_test/phone_test.dart -d "$device" ${defines[@]+"${defines[@]}"} &
  local test=$! waited=0
  set +m
  while kill -0 "$test" 2> /dev/null; do
    if [ "$waited" -ge 900 ]; then
      echo "The test stalled on the simulator; restarting it." >&2
      # The test's whole process group: Flutter, and the simctl it waits on.
      kill -- "-$test" 2> /dev/null || true
      wait "$test" 2> /dev/null || true
      xcrun simctl shutdown "$device" || true
      xcrun simctl boot "$device"
      xcrun simctl bootstatus "$device" -b > /dev/null
      return 124
    fi
    sleep 5
    waited=$((waited + 5))
  done
  wait "$test"
}
status=0
try || status=$?
if [ "$status" = 124 ]; then
  status=0
  try || status=$?
fi
[ "$status" = 0 ] || exit "$status"

# The AutoFill extension, inside the app the tests ran: iOS's credential
# provider, with the entitlements it needs (AutoFill, the App Group and
# Keychain group it shares with the app), and the core linked in.
built=$(find build/ios -path '*iphonesimulator*' -name Runner.app -type d | head -1)
[ -n "$built" ] || { echo "No built app under app/build/ios." >&2; exit 1; }
extension="$built/PlugIns/AutoFill.appex"
[ "$(plutil -extract NSExtension.NSExtensionPointIdentifier raw "$extension/Info.plist")" = com.apple.authentication-services-credential-provider-ui ] \
  || { echo "The extension isn't iOS's credential provider." >&2; exit 1; }
# In the code signature, where the simulator's Settings and AutoFill read
# them (ENTITLEMENTS_DESTINATION in the xcconfigs), for the app and the
# extension both.
for bundle in "$extension" "$built"; do
  entitlements=$(codesign -d --entitlements - --xml "$bundle" 2> /dev/null || true)
  for needed in com.apple.developer.authentication-services.autofill-credential-provider group.com.passbundle.app com.passbundle.app.shared; do
    grep -q "$needed" <<< "$entitlements" || {
      echo "$(basename "$bundle")'s signature is missing $needed:" >&2
      codesign -d --entitlements - "$bundle" >&2 || true
      exit 1
    }
  done
done
# Xcode's debug builds keep an executable's code in a .debug.dylib beside it.
binaries=("$extension/AutoFill")
[ -f "$extension/AutoFill.debug.dylib" ] && binaries+=("$extension/AutoFill.debug.dylib")
[ "$(nm -gU "${binaries[@]}" 2> /dev/null | grep -c '_pb_fillable$')" -gt 0 ] || { echo "The extension doesn't have the core." >&2; exit 1; }
echo "The AutoFill extension is in the app, with its entitlements and the core."
