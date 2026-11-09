#!/usr/bin/env bash
# The Windows and Linux updater, end to end, with the real app (README,
# "Windows and Linux test releases"): installs an older build, serves a newer
# one, signed as the releases are, from a stand-in for GitHub on this
# computer (tools/github-stand-in.py), and checks the app updates itself and
# opens again as the new version, leaving nothing behind. Then a newer one
# that doesn't start: the app takes it out, and the older one opens again.
#
#   tools/desktop-update-test.sh <app bundle> <core folder>
#
# The bundle is a build with test hooks (the core's test-hooks feature and
# PB_TEST_HOOKS, as CI's are): it reads PB_UPDATE_SOURCE (the stand-in),
# PB_UPDATE_KEY (this run's throwaway key), and PB_UPDATE_NOW (update as it
# opens, without a click). A release reads none of them. The core folder has
# the core and passbundle-update (core/target/release). On Linux, it needs a
# display (DISPLAY); CI runs it under Xvfb.
set -Eeuo pipefail
trap 'echo "Failed at line $LINENO: $BASH_COMMAND" >&2' ERR
# Folders as a person's own are: theirs to change, and no one else's.
umask 022

bundle=$(cd "$1" && pwd)
core=$(cd "$2" && pwd)
root=$(cd "$(dirname "$0")/.." && pwd)

case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*)
    os=windows platform=windows-x64 program=passbundle.exe helper=passbundle-update.exe python=python ;;
  Linux)
    os=linux program=passbundle helper=passbundle-update python=python3
    case "$(uname -m)" in
      x86_64) platform=linux-x64 ;;
      aarch64) platform=linux-arm64 ;;
      *) echo "No releases for $(uname -m)." >&2; exit 1 ;;
    esac ;;
  *) echo "This tests the Windows and Linux updater." >&2; exit 1 ;;
esac
case "$(openssl version)" in
  "OpenSSL 3"*) ;;
  *) echo "Signing needs OpenSSL 3 ($(openssl version))." >&2; exit 1 ;;
esac

native() { if [ "$os" = windows ]; then cygpath -w "$1"; else printf '%s' "$1"; fi; }
temp=${RUNNER_TEMP:-${TMPDIR:-/tmp}}
[ "$os" = windows ] && temp=$(cygpath -u "$temp")
work=$(mktemp -d "$temp/desktop-update-test.XXXXXX")
install="$work/install/PassBundle"
updating="$work/install/.PassBundle-update"
serve="$work/serve"
mkdir -p "$serve" "$work/install" "$work/data"
# The folder the app is installed in is the person's alone, as their own
# folders are (the updater won't replace itself in one others can change).
# On Windows, its access list as in a profile's folders: them, SYSTEM, and
# Administrators, and nothing inherited from the runner's own folders. CI's
# Windows machines run everything elevated, which the updater refuses (it
# only says there's an update when it runs as an administrator): builds with
# test hooks let PB_UPDATE_ELEVATED_OK=1 have it go on, as only here.
elevated_ok=
if [ "$os" = windows ]; then
  MSYS_NO_PATHCONV=1 icacls "$(native "$work/install")" /inheritance:r \
    /grant:r "$USERNAME:(OI)(CI)F" /grant:r "*S-1-5-18:(OI)(CI)F" /grant:r "*S-1-5-32-544:(OI)(CI)F" > /dev/null
  elevated_ok=1
fi

# This run's key: the throwaway one the app trusts here, in place of the
# release key (only builds with test hooks take PB_UPDATE_KEY).
openssl genpkey -algorithm ed25519 -out "$work/key.pem" 2> /dev/null
public=$(openssl pkey -in "$work/key.pem" -pubout -outform DER | tail -c 32 | od -An -tx1 | tr -d ' \n')

# The stand-in for GitHub.
"$python" "$root/tools/github-stand-in.py" "$(native "$serve")" "$(native "$work/port")" &
server=$!
for _ in $(seq 50); do [ -s "$work/port" ] && break; sleep 0.1; done
port=$(cat "$work/port")
base="http://127.0.0.1:$port"
echo "The stand-in for GitHub is at $base."

# The PIDs of the processes running a program from the app's folder, as the
# OS says (on Windows, Windows' own PIDs). A process that ends while it's
# asked about is just left out.
running() {
  if [ "$os" = windows ]; then
    { powershell -NoProfile -Command "\$f = '$(native "$install")\\'; Get-Process -ErrorAction SilentlyContinue | Where-Object { \$_.Path -and \$_.Path.StartsWith(\$f, [StringComparison]::OrdinalIgnoreCase) } | ForEach-Object { \$_.Id }; exit 0" 2> /dev/null || true; } | tr -d '\r' | grep -E '^[0-9]+$' | sort || true
  else
    for exe in /proc/[0-9]*/exe; do
      target=$(readlink "$exe" 2> /dev/null) || continue
      case "$target" in "$install"/*) pid=${exe#/proc/}; echo "${pid%/exe}" ;; esac
    done | sort
  fi
}
stop_all() {
  local pids
  pids=$(running)
  [ -n "$pids" ] || return 0
  if [ "$os" = windows ]; then
    for pid in $pids; do taskkill //F //PID "$pid" > /dev/null 2>&1 || true; done
  else
    kill $pids 2> /dev/null || true
  fi
  for _ in $(seq 50); do [ -z "$(running)" ] && return 0; sleep 0.2; done
}
finish() {
  stop_all || true
  kill "$server" 2> /dev/null || true
  if [ "${1:-}" != ok ]; then
    echo "--- requests"; cat "$serve/requests.log" 2> /dev/null || true
    echo "--- the work folder"; find "$updating" -maxdepth 2 2> /dev/null || true
    cat "$updating/failed.json" 2> /dev/null || true
    echo "--- what the app heard"; cat "$work/data/update-test.log" 2> /dev/null || true
  fi
}
trap 'finish' EXIT

# A release, as the release workflows make it: its folder, packed, its
# manifest, and the manifest's signature (the sign jobs' own commands).
release() {
  local version=$1 broken=${2:-}
  local tag="$os-v$version" stage="$work/stage-$version"
  "$root/tools/release-folder.sh" "$bundle" "$core" - "$stage" "$version" "$platform" > /dev/null
  if [ -n "$broken" ]; then
    # A program that ends at once, with an error: a new version that doesn't start.
    cp "$core/$helper" "$stage/PassBundle/$program"
  fi
  mkdir -p "$serve/$tag"
  local name
  if [ "$os" = windows ]; then
    name="PassBundle-$version-$platform.zip"
    "$python" "$root/tools/zip-release.py" "$(native "$stage")" "$(native "$serve/$tag/$name")"
  else
    name="PassBundle-$version-$platform.tar.gz"
    tar -czf "$serve/$tag/$name" -C "$stage" PassBundle
  fi
  local file="$serve/$tag/$name" size sha
  size=$(wc -c < "$file" | tr -d ' ')
  sha=$(openssl dgst -sha256 -r "$file" | cut -d' ' -f1)
  jq -n -c --arg tag "$tag" --arg version "$version" --arg commit "$(git -C "$root" rev-parse HEAD)" \
    --arg notes "What's new since 0.1.1:"$'\n\n'"- Version $version" \
    --arg name "$name" --arg platform "$platform" --argjson size "$size" --arg sha "$sha" \
    '{format: 1, app: "com.passbundle.app", tag: $tag, version: $version, commit: $commit, notes: $notes,
      files: [{name: $name, platform: $platform, size: $size, sha256: $sha}]}' > "$serve/$tag/manifest.json"
  { printf 'PassBundle desktop release v1\n'; cat "$serve/$tag/manifest.json"; } > "$work/signed"
  openssl pkeyutl -sign -inkey "$work/key.pem" -rawin -in "$work/signed" -out "$work/signature"
  openssl base64 -A -in "$work/signature" > "$serve/$tag/manifest.json.sig"
  rm -rf "$stage"
  # GitHub's list, newest first, with an Android release too.
  local url="$base/PimpinPumpkin/passbundle-apps/releases/download/$tag"
  jq -c --arg tag "$tag" --arg url "$url" --arg name "$name" --argjson size "$size" --arg sha "$sha" \
    '[{tag_name: $tag, draft: false, prerelease: false, body: "Not what the app shows.",
       assets: [
         {name: $name, size: $size, digest: ("sha256:" + $sha), browser_download_url: ($url + "/" + $name)},
         {name: "manifest.json", browser_download_url: ($url + "/manifest.json")},
         {name: "manifest.json.sig", browser_download_url: ($url + "/manifest.json.sig")}
       ]}] + .' "$serve/releases.json" > "$serve/releases.next"
  mv "$serve/releases.next" "$serve/releases.json"
}
echo '[{"tag_name": "android-v0.1.40", "draft": false, "prerelease": false, "assets": []}]' > "$serve/releases.json"

# The older version, installed: a release's folder at 0.1.1.
install_old() {
  stop_all
  rm -rf "$install" "$updating"
  "$root/tools/release-folder.sh" "$bundle" "$core" - "$work/install" 0.1.1 "$platform" > /dev/null
}

# Opens the installed version, which updates as it opens; `opened` is its
# PID, as the OS says.
open_app() {
  PB_DATA_DIR=$(native "$work/data") PB_SECRETS=memory \
    PB_UPDATE_SOURCE="$base" PB_UPDATE_KEY="$public" PB_UPDATE_NOW=1 \
    PB_UPDATE_ELEVATED_OK="$elevated_ok" \
    "$install/$program" > "$work/app.log" 2>&1 &
  opened=$!
  if [ "$os" = windows ]; then
    for _ in $(seq 50); do [ -r "/proc/$opened/winpid" ] && break; sleep 0.1; done
    opened=$(cat "/proc/$opened/winpid")
  fi
}

version_installed() { jq -r .version "$install/release.json" 2> /dev/null | tr -d '\r' || echo none; }

# Waits for the app's folder to be at `version`, with nothing left beside it,
# and a PassBundle other than `before` running from it.
wait_for() {
  local version=$1 before=$2
  for _ in $(seq 360); do
    local now
    now=$(running)
    if [ "$(version_installed)" = "$version" ] && [ ! -e "$updating" ] \
      && [ -n "$now" ] && [ -z "$(comm -12 <(echo "$before") <(echo "$now"))" ]; then
      return 0
    fi
    sleep 0.5
  done
  echo "Waited 3 minutes for PassBundle $version to open in place of $before; it's $(version_installed), running $(running | tr '\n' ' ')." >&2
  return 1
}

# 1. 0.1.1 updates itself to 0.1.2 and opens again.
release 0.1.2
install_old
open_app
wait_for 0.1.2 "$opened"
grep -q "GET /assets/$os-v0.1.2/PassBundle-0.1.2-$platform" "$serve/requests.log"
grep -q "GET /assets/$os-v0.1.2/manifest.json.sig" "$serve/requests.log"
# What the new version heard as it opened (builds with test hooks note it).
grep -q '"updated":"0.1.2"' "$work/data/update-test.log"
# Nothing left beside it: no old version, no download, no updater.
leftovers=$(ls -A "$work/install")
[ "$leftovers" = PassBundle ] || { echo "Left beside it: $leftovers" >&2; exit 1; }
echo "PassBundle 0.1.1 updated itself to 0.1.2, and opened again."

# 2. A newer version that doesn't start: the older one comes back.
release 0.1.3 broken
install_old
: > "$serve/requests.log"
: > "$work/data/update-test.log"
open_app
wait_for 0.1.1 "$opened"
grep -q "GET /assets/$os-v0.1.3/PassBundle-0.1.3-$platform" "$serve/requests.log"
grep -q "PassBundle 0.1.3 didn't start, so PassBundle went back to 0.1.1." "$work/data/update-test.log"
leftovers=$(ls -A "$work/install")
[ "$leftovers" = PassBundle ] || { echo "Left beside it: $leftovers" >&2; exit 1; }
echo "PassBundle 0.1.3 didn't start, so PassBundle went back to 0.1.1, which opened again."

finish ok
trap - EXIT
rm -rf "$work"
