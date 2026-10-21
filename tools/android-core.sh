#!/usr/bin/env bash
# Builds the core for Android with the NDK (docs/phones.md, "The core on
# phones"), or builds its tests and runs them on a device or emulator over adb.
#
#   tools/android-core.sh build [abi...]   # libpassbundle_core.so into app/android/app/src/main/jniLibs/<abi>/
#   tools/android-core.sh test [abi]       # the core's tests, on the device adb sees
#
# ABIs: arm64-v8a (phones) and x86_64 (the emulator); both by default for a
# build, x86_64 for tests. Needs the NDK (ANDROID_NDK_HOME, or the newest in
# $ANDROID_HOME/ndk) and the Rust targets (rustup target add
# aarch64-linux-android x86_64-linux-android).
set -euo pipefail
cd "$(dirname "$0")/.."

# Android 10, the oldest the apps support.
API=29

ndk="${ANDROID_NDK_HOME:-}"
if [ -z "$ndk" ]; then
  sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Library/Android/sdk}}"
  ndk=$(ls -d "$sdk"/ndk/*/ 2> /dev/null | sort -V | tail -1)
fi
[ -d "$ndk" ] || { echo "No Android NDK: set ANDROID_NDK_HOME." >&2; exit 1; }
case "$(uname -s)" in
  Darwin) host=darwin-x86_64 ;;
  *) host=linux-x86_64 ;;
esac
toolchain="$ndk/toolchains/llvm/prebuilt/$host/bin"

triple() {
  case "$1" in
    arm64-v8a) echo aarch64-linux-android ;;
    x86_64) echo x86_64-linux-android ;;
    *) echo "Unknown ABI: $1 (arm64-v8a or x86_64)" >&2; exit 1 ;;
  esac
}

# The NDK's compiler and linker for one target, as cargo and cc-rs read them.
use_ndk() {
  local t=$1 upper
  upper=$(echo "$t" | tr 'a-z-' 'A-Z_')
  export "CC_${t//-/_}=$toolchain/$t$API-clang"
  export "AR_${t//-/_}=$toolchain/llvm-ar"
  export "CARGO_TARGET_${upper}_LINKER=$toolchain/$t$API-clang"
}

command=${1:-build}
shift || true
case "$command" in
  build)
    abis=("$@")
    [ ${#abis[@]} -gt 0 ] || abis=(arm64-v8a x86_64)
    for abi in "${abis[@]}"; do
      t=$(triple "$abi")
      use_ndk "$t"
      (cd core && cargo build --locked --release --features ffi --target "$t")
      out="app/android/app/src/main/jniLibs/$abi"
      mkdir -p "$out"
      cp "core/target/$t/release/libpassbundle_core.so" "$out/"
      echo "$out/libpassbundle_core.so: $(wc -c < "$out/libpassbundle_core.so") bytes"
    done
    ;;
  test)
    abi=${1:-x86_64}
    t=$(triple "$abi")
    use_ndk "$t"
    # The test programs, as cargo lists them.
    programs=$(cd core && cargo test --locked --features ffi --target "$t" --no-run --message-format=json 2> /dev/null |
      python3 -c 'import json, sys
for line in sys.stdin:
    m = json.loads(line)
    if m.get("reason") == "compiler-artifact" and m.get("executable") and m["profile"]["test"]:
        print(m["executable"])')
    dir=/data/local/tmp/passbundle-core-tests
    adb shell "rm -rf $dir && mkdir -p $dir"
    failed=0
    for program in $programs; do
      name=$(basename "$program")
      adb push "$program" "$dir/$name" > /dev/null
      echo "== $name"
      # Run from the core's folder layout: the tests read their vectors at build time.
      if ! adb shell "cd $dir && TMPDIR=$dir ./$name"; then
        failed=1
      fi
    done
    adb shell "rm -rf $dir"
    exit $failed
    ;;
  *)
    echo "Usage: $0 build [abi...] | test [abi]" >&2
    exit 1
    ;;
esac
