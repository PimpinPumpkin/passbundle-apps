#!/usr/bin/env bash
# Builds the core for iOS (docs/phones.md, "The core on phones"), or builds its
# tests and runs them on an iOS simulator.
#
#   tools/ios-core.sh build   # PassBundleCore.xcframework (iPhones and the simulator) into app/ios/
#   tools/ios-core.sh test    # the core's tests, on a booted (or newly booted) iPhone simulator
#
# Needs Xcode and the Rust targets (rustup target add aarch64-apple-ios
# aarch64-apple-ios-sim). The app links the core statically: iOS apps can't
# load a library from anywhere but their own frameworks.
set -euo pipefail
cd "$(dirname "$0")/.."

# iOS 17, the oldest the apps support.
export IPHONEOS_DEPLOYMENT_TARGET=17.0

command=${1:-build}
case "$command" in
  build)
    for t in aarch64-apple-ios aarch64-apple-ios-sim; do
      (cd core && cargo rustc --locked --release --lib --features ffi --target "$t" --crate-type staticlib)
    done
    out=app/ios/PassBundleCore.xcframework
    rm -rf "$out"
    xcodebuild -create-xcframework \
      -library core/target/aarch64-apple-ios/release/libpassbundle_core.a \
      -library core/target/aarch64-apple-ios-sim/release/libpassbundle_core.a \
      -output "$out" > /dev/null
    for a in "$out"/*/libpassbundle_core.a; do
      echo "$a: $(wc -c < "$a") bytes"
    done
    ;;
  test)
    t=aarch64-apple-ios-sim
    # The test programs, as cargo lists them.
    programs=$(cd core && cargo test --locked --features ffi --target "$t" --no-run --message-format=json 2> /dev/null |
      python3 -c 'import json, sys
for line in sys.stdin:
    m = json.loads(line)
    if m.get("reason") == "compiler-artifact" and m.get("executable") and m["profile"]["test"]:
        print(m["executable"])')
    # A booted iPhone simulator, or the newest one, booted now.
    device=$(xcrun simctl list devices booted -j | python3 -c 'import json, sys
devices = [d for ds in json.load(sys.stdin)["devices"].values() for d in ds if d["name"].startswith("iPhone")]
print(devices[0]["udid"] if devices else "")')
    if [ -z "$device" ]; then
      device=$(xcrun simctl list devices available -j | python3 -c 'import json, sys
runtimes = json.load(sys.stdin)["devices"]
ios = sorted((r for r in runtimes if ".iOS-" in r), key=lambda r: [int(p) for p in r.rsplit(".iOS-", 1)[1].split("-")])
devices = [d for d in runtimes[ios[-1]] if d["name"].startswith("iPhone")] if ios else []
print(devices[0]["udid"] if devices else "")')
      [ -n "$device" ] || { echo "No iPhone simulator." >&2; exit 1; }
      xcrun simctl boot "$device"
      xcrun simctl bootstatus "$device" > /dev/null
    fi
    xcrun simctl getenv "$device" SIMULATOR_RUNTIME_VERSION 2> /dev/null | sed 's/^/iOS /' || true
    failed=0
    for program in $programs; do
      echo "== $(basename "$program")"
      # The simulator runs the host's program as one of its own processes.
      if ! xcrun simctl spawn "$device" "$program"; then
        failed=1
      fi
    done
    exit $failed
    ;;
  *)
    echo "Usage: $0 build | test" >&2
    exit 1
    ;;
esac
