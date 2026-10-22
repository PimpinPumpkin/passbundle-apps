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
flutter test integration_test/phone_test.dart -d "$device" ${defines[@]+"${defines[@]}"}

# The AutoFill extension, inside the app the tests ran: iOS's credential
# provider, with the entitlements it needs (AutoFill, the App Group and
# Keychain group it shares with the app), and the core linked in.
built=$(find build/ios -path '*iphonesimulator*' -name Runner.app -type d | head -1)
[ -n "$built" ] || { echo "No built app under app/build/ios." >&2; exit 1; }
extension="$built/PlugIns/AutoFill.appex"
[ "$(plutil -extract NSExtension.NSExtensionPointIdentifier raw "$extension/Info.plist")" = com.apple.authentication-services-credential-provider-ui ] \
  || { echo "The extension isn't iOS's credential provider." >&2; exit 1; }
entitlements=$(codesign -d --entitlements - --xml "$extension" 2> /dev/null)
for needed in com.apple.developer.authentication-services.autofill-credential-provider group.com.passbundle.app com.passbundle.app.shared; do
  grep -q "$needed" <<< "$entitlements" || { echo "The extension is missing $needed." >&2; exit 1; }
done
[ "$(nm -gU "$extension/AutoFill" | grep -c '_pb_fillable$')" -gt 0 ] || { echo "The extension doesn't have the core." >&2; exit 1; }
echo "The AutoFill extension is in the app, with its entitlements and the core."
