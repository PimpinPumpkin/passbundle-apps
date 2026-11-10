#!/usr/bin/env bash
# Touch ID on the Mac (app/macos/Runner/TouchID.swift) and the seal for what
# unlocking with a PIN keeps (DeviceSeal.swift) on their own, without
# Flutter or Xcode: builds them with tools/mac-touch-id-test/main.swift into
# a test program and runs it. Both seals against the meta repo's vectors
# (core/tests/vectors/mac-local-v1.json and pin-unlock-v1.json), the
# account's file and its modes, and, where the Mac has one, the real Secure
# Enclave: a key it keeps seals and opens a user key, one that needs Touch
# ID opens nothing without it, and a PIN's record opens only as its own
# account's. It never shows a prompt or touches a keychain. With the core
# (built with its ffi feature), it also checks the app finds the core's
# functions for both; with its test-hooks feature too, that the core seals
# and opens a PIN's record through the app's seal.
#
#   tools/mac-touch-id-test.sh [core/target/release/libpassbundle_core.dylib]
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
core=${1:-}
work=$(mktemp -d "${TMPDIR:-/tmp}/pb-touch-id-test.XXXXXX")
trap 'rm -rf "$work"' EXIT

sources=("$root/app/macos/Runner/TouchID.swift" "$root/app/macos/Runner/DeviceSeal.swift" "$root/tools/mac-touch-id-test/main.swift")
if ! swiftc -O -o "$work/touch-id-test" "${sources[@]}" 2> "$work/build.log"; then
  # Some Command Line Tools releases (without Xcode) define the SwiftBridging
  # module twice; hide their extra module map and build again.
  if grep -q "redefinition of module 'SwiftBridging'" "$work/build.log"; then
    : > "$work/empty.modulemap"
    cat > "$work/overlay.yaml" << EOF
{ "version": 0, "case-sensitive": "false", "roots": [ { "type": "directory",
  "name": "$(xcode-select -p)/usr/include/swift",
  "contents": [ { "type": "file", "name": "module.modulemap", "external-contents": "$work/empty.modulemap" } ] } ] }
EOF
    swiftc -O -vfsoverlay "$work/overlay.yaml" -Xcc -ivfsoverlay -Xcc "$work/overlay.yaml" \
      -o "$work/touch-id-test" "${sources[@]}"
  else
    cat "$work/build.log" >&2
    exit 1
  fi
fi
"$work/touch-id-test" "$root/core/tests/vectors/mac-local-v1.json" "$root/core/tests/vectors/pin-unlock-v1.json" ${core:+"$core"}
