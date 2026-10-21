#!/usr/bin/env bash
# Puts the core and the desktop agent into the Mac app that Flutter built,
# and signs it ad hoc (until code signing; the same order then):
#
#   <App>.app/Contents/Frameworks/libpassbundle_core.dylib
#   <App>.app/Contents/Helpers/<Name> Agent.app   the agent and the native-messaging host
#
# The agent is an app of its own (its own bundle ID, no Dock icon): inside
# the app's own folder of programs, macOS would take it for the app, and
# bring the agent forward when someone opens the app.
#
#   tools/bundle-mac.sh <passbundle.app> <libpassbundle_core.dylib> <folder with passbundle-agent and passbundle-agent-host>
set -euo pipefail

app=$1
core=$2
agent=$3
root="$(cd "$(dirname "$0")/.." && pwd)"
name=$(python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))["name"])' "$root/brand/brand.json")
domain=$(python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))["domain"])' "$root/brand/brand.json")
id="$(echo "$domain" | awk -F. '{ for (i = NF; i > 0; i--) printf "%s%s", $i, (i > 1 ? "." : "") }').agent"
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/agent/Cargo.toml" | head -1)

mkdir -p "$app/Contents/Frameworks"
cp "$core" "$app/Contents/Frameworks/"

helper="$app/Contents/Helpers/$name Agent.app"
rm -rf "$helper"
mkdir -p "$helper/Contents/MacOS"
cp "$agent/passbundle-agent" "$agent/passbundle-agent-host" "$helper/Contents/MacOS/"
cat > "$helper/Contents/Info.plist" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleIdentifier</key>
	<string>$id</string>
	<key>CFBundleName</key>
	<string>$name Agent</string>
	<key>CFBundleDisplayName</key>
	<string>$name Agent</string>
	<key>CFBundleExecutable</key>
	<string>passbundle-agent</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>$version</string>
	<key>CFBundleVersion</key>
	<string>$version</string>
	<key>LSMinimumSystemVersion</key>
	<string>11.0</string>
	<key>LSUIElement</key>
	<true/>
</dict>
</plist>
EOF
plutil -lint "$helper/Contents/Info.plist" > /dev/null

# Inside out: what was added, each on its own (the app's entitlements are the
# app's, never the agent's), then the app with its own.
codesign --force --sign - "$app/Contents/Frameworks/libpassbundle_core.dylib"
codesign --force --sign - "$helper/Contents/MacOS/passbundle-agent-host"
codesign --force --sign - "$helper"
codesign --force --sign - --entitlements "$root/app/macos/Runner/Release.entitlements" "$app"
codesign --verify --deep --strict "$app"
echo "$app: the core in Frameworks, $name Agent.app ($id $version) in Helpers, signed ad hoc"
