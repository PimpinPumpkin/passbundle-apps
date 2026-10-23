#!/usr/bin/env bash
# Checks that a release APK still has the classes and methods the core calls
# by name through JNI (app/android/app/proguard-rules.pro). Release builds
# shrink code, and the tests run debug builds with a test certificate, so
# nothing else would notice them gone: on a phone, signing in fails with
# "failed to call native verifier".
#
#   tools/android-release-check.sh [app.apk]   # default: the release APK flutter builds
set -euo pipefail
cd "$(dirname "$0")/.."

apk=${1:-app/build/app/outputs/flutter-apk/app-release.apk}
[ -f "$apk" ] || { echo "No APK at $apk: run flutter build apk --release in app/ first." >&2; exit 1; }
sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Library/Android/sdk}}"
dexdump=$(ls "$sdk"/build-tools/*/dexdump 2> /dev/null | sort -V | tail -1)
[ -x "$dexdump" ] || { echo "No dexdump in $sdk/build-tools." >&2; exit 1; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
unzip -q -o "$apk" 'classes*.dex' -d "$work"
for dex in "$work"/classes*.dex; do "$dexdump" "$dex"; done > "$work/dump" 2> /dev/null

# "Class" "method" pairs: each class must be there, unrenamed, with the method.
missing=0
while read -r class method; do
  if ! awk -v c="'L${class//.//};'" -v m="'$method'" '
      /Class descriptor/ { inside = ($NF == c) }
      inside && /name/ && $NF == m { found = 1 }
      END { exit !found }' "$work/dump"; then
    echo "Missing from $apk: $class.$method" >&2
    missing=1
  fi
done <<'LIST'
org.rustls.platformverifier.CertificateVerifier verifyCertificateChain
org.rustls.platformverifier.VerificationResult <init>
com.passbundle.app.Keystore seal
com.passbundle.app.Keystore open
com.passbundle.app.Biometric remove
com.passbundle.app.Core init
LIST
[ "$missing" = 0 ] || exit 1
echo "The release APK keeps what the core calls through JNI."
