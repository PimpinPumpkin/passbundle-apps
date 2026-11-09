#!/usr/bin/env bash
# The AutoFill extension's own code on an iPhone simulator (docs/phones.md,
# "Order and tests"): app/ios/AutoFillTests, in a bare host app, drive the
# extension's screens and the core under them as iOS's requests would, on two
# accounts kept as the app keeps them, and save the screens they draw into
# app/build/ios-screens. iOS hands the extension itself a request only in a
# build signed with an Apple Developer account (tools/ios-autofill-test.sh);
# these need none. Run it after tools/ios-app-test.sh, whose Flutter build
# makes the project's generated files, and with what that needs.
set -euo pipefail
cd "$(dirname "$0")/.."

device=$(tools/ios-core.sh simulator)
[ -d app/ios/PassBundleCore.xcframework ] || tools/ios-core.sh build
shots="$PWD/app/build/ios-screens"
mkdir -p "$shots"
result="$(mktemp -d)/AutoFillTests.xcresult"
status=0
# TEST_RUNNER_ variables reach the tests without the prefix: where to save
# the screens.
(cd app/ios && TEST_RUNNER_PB_SHOTS="$shots" xcodebuild test -project Runner.xcodeproj -scheme AutoFillTests \
  -destination "id=$device" -resultBundlePath "$result" -quiet) || status=$?
if [ "$status" != 0 ]; then
  # Why, with what was on screen (xcodebuild's quiet output leaves that out).
  xcrun xcresulttool get test-results tests --path "$result" --format json 2> /dev/null \
    | python3 -I -c 'import json, sys
def walk(node):
    for child in node.get("children", []):
        if child.get("nodeType") == "Failure Message":
            print(child.get("name", ""))
        walk(child)
for root in json.load(sys.stdin).get("testNodes", []):
    walk(root)' >&2 || xcrun xcresulttool get test-results summary --path "$result" >&2 || true
  exit "$status"
fi
echo "The AutoFill extension's tests passed; its screens:"
ls "$shots"
