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
# after the build (simctl stalls, with no output). A try that's said nothing
# for five minutes after Xcode's build is done (a running test speaks within
# a minute), or that's still going after 15 minutes (the test takes 5), is
# stopped, the simulator restarted, and the test run again, up to three
# times; a test that fails isn't run again.
log=$(mktemp)
try() {
  : > "$log"
  set -m
  (flutter test integration_test/phone_test.dart -d "$device" ${defines[@]+"${defines[@]}"} 2>&1 | tee "$log") &
  local test=$! waited=0 quiet
  set +m
  while kill -0 "$test" 2> /dev/null; do
    quiet=$(($(date +%s) - $(stat -f %m "$log")))
    if [ "$waited" -ge 900 ] || { grep -q "Xcode build done" "$log" && [ "$quiet" -ge 300 ]; }; then
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
status=124
for _ in 1 2 3; do
  [ "$status" = 124 ] || break
  status=0
  try || status=$?
done
exit "$status"
