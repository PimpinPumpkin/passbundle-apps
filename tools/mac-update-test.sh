#!/usr/bin/env bash
# The Mac updater (app/macos/Runner/Updater.swift) on its own, without
# Flutter or Xcode: builds it with tools/mac-update-test/main.swift into a
# test program, makes the bundles it updates, signed the way
# .github/workflows/macos-release.yml signs a release (keep the two the
# same: sign_app below and the release's sign job), and runs it. An older
# copy updates to a newer one; a tampered download, a copy signed ad hoc or
# with another certificate, the wrong version, another app, and a broken
# signature are each refused, and the older copy stays as it was.
#
#   tools/mac-update-test.sh [--launch] [<a built passbundle.app>]
#
# With a built app (Flutter's, with the core and the agent in it:
# tools/bundle-mac.sh), it also updates that app, signed as 0.1.1 and as
# 0.1.2; with --launch it opens the signed copy as tools/measure.sh does
# (its own sample vault, the session in memory), to show it runs under the
# hardened runtime with a certificate like PassBundle's.
#
# It signs with PB_MAC_SIGN_P12 (a PKCS12 file) and PB_MAC_SIGN_PASSWORD
# when both are set (PassBundle's test certificate), else with a throwaway
# certificate it makes; another throwaway one stands for someone else's.
# They go in a keychain of its own, which is on the user's keychain search
# list (codesign finds identities only there) while the script runs. The
# list is put back and the keychain deleted when it ends. Its files are in a
# temporary folder, removed when everything passes.
set -euo pipefail

launch=""
if [ "${1:-}" = --launch ]; then
  launch=yes
  shift
fi
built=${1:-}
root="$(cd "$(dirname "$0")/.." && pwd)"
entitlements="$root/app/macos/Runner/Release.entitlements"
work=$(mktemp -d "${TMPDIR:-/tmp}/pb-update-test.XXXXXX")
fixtures="$work/fixtures"
mkdir -p "$fixtures" "$work/src" "$work/certs"

kc="$work/signing.keychain-db"
saved=()
while IFS= read -r line; do saved+=("$line"); done < <(security list-keychains -d user | sed 's/^ *"//; s/"$//')
[ "${#saved[@]}" -gt 0 ] || { echo "The keychain search list is empty; not changing it." >&2; exit 1; }
passed=""
cleanup() {
  security list-keychains -d user -s "${saved[@]}"
  security delete-keychain "$kc" 2> /dev/null || true
  if [ -n "$passed" ]; then
    rm -rf "${work:?}"
  else
    echo "Left for a look: $work" >&2
  fi
}
trap cleanup EXIT

kcpass=$(/usr/bin/openssl rand -hex 16)
security create-keychain -p "$kcpass" "$kc"
security set-keychain-settings "$kc"
security unlock-keychain -p "$kcpass" "$kc"
security list-keychains -d user -s "$kc" "${saved[@]}"

import_p12() { # <file> <password>
  security import "$1" -k "$kc" -f pkcs12 -P "$2" -T /usr/bin/codesign > /dev/null
}

# A throwaway code signing certificate, as PassBundle's test one is made
# (README, "Mac test releases"), but for two days.
throwaway() { # <name> <common name>
  local dir="$work/certs/$1" pass
  mkdir -p "$dir"
  cat > "$dir/req.cnf" << EOF
[req]
distinguished_name = dn
prompt = no
x509_extensions = ext
[dn]
CN = $2
[ext]
basicConstraints = critical, CA:FALSE
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
EOF
  /usr/bin/openssl req -x509 -newkey rsa:2048 -nodes -keyout "$dir/key.pem" -out "$dir/cert.pem" \
    -days 2 -sha256 -config "$dir/req.cnf" 2> /dev/null
  pass=$(/usr/bin/openssl rand -hex 16)
  /usr/bin/openssl pkcs12 -export -inkey "$dir/key.pem" -in "$dir/cert.pem" -name "$2" \
    -passout "pass:$pass" -out "$dir/id.p12"
  import_p12 "$dir/id.p12" "$pass"
}

if [ -n "${PB_MAC_SIGN_P12:-}" ] && [ -n "${PB_MAC_SIGN_PASSWORD:-}" ]; then
  import_p12 "$PB_MAC_SIGN_P12" "$PB_MAC_SIGN_PASSWORD"
  ours_name="PassBundle test builds"
else
  throwaway ours "PassBundle throwaway test"
  ours_name="PassBundle throwaway test"
fi
throwaway other "Someone else"
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$kcpass" "$kc" > /dev/null
identity() { # <common name>: its SHA-1, which names it alone
  security find-identity -p codesigning "$kc" | awk -v name="\"$1\"" 'index($0, name) { print $2; exit }'
}
ours=$(identity "$ours_name")
other=$(identity "Someone else")
[ -n "$ours" ] && [ -n "$other" ] || { echo "The signing identities aren't in the keychain." >&2; exit 1; }
echo "Signing as \"$ours_name\" ($ours)"

# As the release's sign job does: what's nested first, each on its own (the
# app's entitlements are the app's, never the agent's), then the app with
# its entitlements; the hardened runtime throughout.
sign_app() { # <app> <identity, or - for ad hoc>
  local app=$1 id=$2 item helper main program
  sign() { codesign --force --timestamp=none --options runtime --sign "$id" "$@"; }
  for item in "$app"/Contents/Frameworks/*; do
    case "$item" in *.framework | *.dylib) sign "$item" ;; esac
  done
  for helper in "$app"/Contents/Helpers/*.app; do
    [ -d "$helper" ] || continue
    main=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$helper/Contents/Info.plist")
    for program in "$helper"/Contents/MacOS/*; do
      [ "$(basename "$program")" = "$main" ] || sign "$program"
    done
    sign "$helper"
  done
  sign --entitlements "$entitlements" "$app"
  codesign --verify --deep --strict "$app"
}

# Small stand-ins for the app's parts: its program, the core, a framework
# like Flutter's (with its links), the agent's app with two programs.
cat > "$work/src/main.c" << 'EOF'
int main(void) { return 0; }
EOF
# A program that waits, for the agent the old copy leaves running.
cat > "$work/src/sleeper.c" << 'EOF'
#include <stdlib.h>
#include <unistd.h>
int main(int argc, char **argv) { sleep(argc > 1 ? atoi(argv[1]) : 1); return 0; }
EOF
cat > "$work/src/core.c" << 'EOF'
int pb_version(void) { return 1; }
EOF
# Universal, as the app is.
archs=(-arch arm64 -arch x86_64)
clang "${archs[@]}" -o "$work/src/program" "$work/src/main.c"
clang -o "$fixtures/sleeper" "$work/src/sleeper.c"
clang "${archs[@]}" -dynamiclib -o "$work/src/core.dylib" "$work/src/core.c"
clang "${archs[@]}" -dynamiclib -install_name @rpath/FlutterMacOS.framework/Versions/A/FlutterMacOS \
  -o "$work/src/framework" "$work/src/core.c"

plist() { # <file> <bundle id> <executable> <version> <package type>
  cat > "$1" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleIdentifier</key>
	<string>$2</string>
	<key>CFBundleExecutable</key>
	<string>$3</string>
	<key>CFBundleShortVersionString</key>
	<string>$4</string>
	<key>CFBundleVersion</key>
	<string>${4##*.}</string>
	<key>CFBundlePackageType</key>
	<string>$5</string>
</dict>
</plist>
EOF
}

make_app() { # <name> <bundle id> <version> <identity>
  local app="$fixtures/$1" fw helper
  fw="$app/Contents/Frameworks/FlutterMacOS.framework"
  helper="$app/Contents/Helpers/PassBundle Agent.app"
  mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources/flutter_assets" "$fw/Versions/A/Resources" "$helper/Contents/MacOS"
  plist "$app/Contents/Info.plist" "$2" passbundle "$3" APPL
  cp "$work/src/program" "$app/Contents/MacOS/passbundle"
  cp "$work/src/core.dylib" "$app/Contents/Frameworks/libpassbundle_core.dylib"
  cp "$work/src/framework" "$fw/Versions/A/FlutterMacOS"
  plist "$fw/Versions/A/Resources/Info.plist" io.flutter.flutter-macos FlutterMacOS 1.0.0 FMWK
  ln -s A "$fw/Versions/Current"
  ln -s Versions/Current/FlutterMacOS "$fw/FlutterMacOS"
  ln -s Versions/Current/Resources "$fw/Resources"
  echo '{}' > "$app/Contents/Resources/flutter_assets/AssetManifest.json"
  plist "$helper/Contents/Info.plist" com.passbundle.agent passbundle-agent "$3" APPL
  cp "$work/src/program" "$helper/Contents/MacOS/passbundle-agent"
  cp "$work/src/program" "$helper/Contents/MacOS/passbundle-agent-host"
  sign_app "$app" "$4"
}

make_app old.app com.passbundle.app 0.1.1 "$ours"
make_app new.app com.passbundle.app 0.1.2 "$ours"
make_app adhoc.app com.passbundle.app 0.1.2 -
make_app other.app com.passbundle.app 0.1.2 "$other"
make_app version.app com.passbundle.app 0.1.3 "$ours"
make_app wrongid.app com.example.notpassbundle 0.1.2 "$ours"
make_app broken.app com.passbundle.app 0.1.2 "$ours"
echo '{"changed": "after signing"}' > "$fixtures/broken.app/Contents/Resources/flutter_assets/AssetManifest.json"
make_app oldadhoc.app com.passbundle.app 0.1.1 -

if [ -n "$built" ]; then
  real() { # <name> <version> <identity>
    ditto "$built" "$fixtures/$1"
    /usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $2" -c "Set :CFBundleVersion ${2##*.}" \
      "$fixtures/$1/Contents/Info.plist"
    sign_app "$fixtures/$1" "$3"
  }
  real real-old.app 0.1.1 "$ours"
  real real-new.app 0.1.2 "$ours"
  real real-adhoc.app 0.1.2 -
  echo "The built app, signed:"
  codesign -dv "$fixtures/real-new.app" 2>&1 | grep -E '^(Identifier|CodeDirectory|Authority|TeamIdentifier)='
  codesign -d -r- "$fixtures/real-new.app" 2>&1 | grep designated
  if [ -n "$launch" ]; then
    data="$work/launch-data"
    lines="$work/launch-lines"
    mkdir -p "$data"
    cp "$root/app/test/sample/settings.json" "$root/app/test/sample/sync.json" "$data/"
    PB_MEASURE=1 PB_MEASURE_FILE="$lines" PB_MEASURE_PASSWORD='correct horse battery staple 1' \
      PB_DATA_DIR="$data" PB_SECRETS=memory "$fixtures/real-new.app/Contents/MacOS/passbundle" > /dev/null 2>&1 &
    pid=$!
    for _ in $(seq 1 120); do
      grep -q '^ready' "$lines" 2> /dev/null && break
      kill -0 "$pid" 2> /dev/null || break
      sleep 0.5
    done
    if ! grep -q '^ready' "$lines" 2> /dev/null; then
      echo "The signed app didn't open and unlock its vault:" >&2
      cat "$lines" >&2 2> /dev/null || true
      kill "$pid" 2> /dev/null || true
      exit 1
    fi
    kill "$pid"
    wait "$pid" 2> /dev/null || true
    echo "The signed app opened under the hardened runtime and unlocked its sample vault."
  fi
fi

# Command Line Tools without Xcode carry two module maps for one module;
# swiftc stops at that unless one is hidden.
overlay=()
clt=/Library/Developer/CommandLineTools/usr/include/swift
if [ "$(xcode-select -p)" = /Library/Developer/CommandLineTools ] && [ -f "$clt/module.modulemap" ] && [ -f "$clt/bridging.modulemap" ]; then
  : > "$work/empty.modulemap"
  cat > "$work/overlay.yaml" << EOF
{ "version": 0, "case-sensitive": "false", "roots": [
  { "type": "file", "name": "$clt/module.modulemap", "external-contents": "$work/empty.modulemap" } ] }
EOF
  overlay=(-vfsoverlay "$work/overlay.yaml" -Xcc -ivfsoverlay -Xcc "$work/overlay.yaml")
fi
swiftc -swift-version 5 -O ${overlay[@]+"${overlay[@]}"} -o "$work/update-test" \
  "$root/app/macos/Runner/Updater.swift" "$root/tools/mac-update-test/main.swift"
"$work/update-test" "$fixtures"
passed=yes
