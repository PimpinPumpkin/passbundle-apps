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
