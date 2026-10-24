#!/usr/bin/env bash
# Makes a Windows or Linux build into a release's folder (README, "Windows
# and Linux test releases"): copies the app's bundle into <out>/PassBundle
# with the core, the updater (passbundle-update), and on Windows the desktop
# agent, and writes release.json, which says which release it is and every
# name in the folder (the only ones an update may move; core/src/updates).
#
#   tools/release-folder.sh <bundle> <core folder> <agent folder or -> <out> <version> <platform>
#
# The release workflows (windows-release.yml, linux-release.yml) and
# tools/desktop-update-test.sh both make their folders with this.
set -euo pipefail

bundle=$1 core=$2 agent=$3 out=$4 version=$5 platform=$6
dir="$out/PassBundle"
rm -rf "$dir"
mkdir -p "$dir"
cp -R "$bundle"/. "$dir"/
case "$platform" in
  windows-*)
    cp "$core/passbundle_core.dll" "$core/passbundle-update.exe" "$dir"/
    if [ "$agent" != - ]; then
      cp "$agent/passbundle-agent.exe" "$agent/passbundle-agent-host.exe" "$dir"/
    fi
    test -f "$dir/passbundle.exe" ;;
  linux-*)
    mkdir -p "$dir/lib"
    cp "$core/libpassbundle_core.so" "$dir/lib/"
    cp "$core/passbundle-update" "$dir/"
    test -x "$dir/passbundle" && test -x "$dir/passbundle-update" ;;
  *) echo "No releases for $platform." >&2; exit 1 ;;
esac
# Every name in the folder, and release.json itself.
names=$( (ls -A "$dir"; echo release.json) | sort -u | jq -R -s -c 'split("\n") | map(select(length > 0))')
jq -n -c --arg version "$version" --arg platform "$platform" --argjson files "$names" \
  '{app: "com.passbundle.app", version: $version, platform: $platform, files: $files}' > "$dir/release.json"
cat "$dir/release.json"
