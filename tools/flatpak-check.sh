#!/usr/bin/env bash
# Builds the Flatpak (app/linux/flatpak/com.passbundle.app.yml) from a Linux
# release's tarball, as a check only (README, "Linux packages"), installs it
# for this person from the build, and checks it: it holds the release's
# folder as it is, its permissions are no wider than the manifest's, its
# desktop entry takes passbundle:// links and opens them for the person,
# the app opens in its sandbox on a virtual display and stays open, and a
# link opened with xdg-open reaches it there. A check that fails shows what
# its command said.
#
#   tools/flatpak-check.sh <PassBundle-<version>-linux-x64.tar.gz>
#
# Needs flatpak and flatpak-builder (CI takes the runner's own packages),
# Xvfb, xdg-utils, desktop-file-utils, and dbus-run-session. The runtime
# and its SDK come from Flathub, whose signing key is in
# app/linux/flatpak/flathub.flatpakrepo (the copy GNOME Builder and others
# ship, checked here by its SHA-256): Flatpak takes nothing from Flathub
# that key didn't sign. Everything goes in this person's own Flatpak
# installation (--user), on a runner that's thrown away after.
set -euo pipefail

[ $# -eq 1 ] || { echo "usage: tools/flatpak-check.sh <PassBundle-<version>-linux-x64.tar.gz>" >&2; exit 2; }
tarball=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
root=$(cd "$(dirname "$0")/.." && pwd)
manifests="$root/app/linux/flatpak"
app_id=com.passbundle.app
flathub_sha256=3371dd250e61d9e1633630073fefda153cd4426f72f4afa0c3373ae2e8fea03a
failures=0

# A check passes when its command does; when it fails, what the command
# said is shown under it.
check() {
  name=$1
  shift
  if said=$("$@" 2>&1); then
    echo "ok    $name"
  else
    echo "FAIL  $name"
    [ -z "$said" ] || printf '%s\n' "$said" | sed 's/^/      /'
    failures=$((failures + 1))
  fi
}

[ "$(sha256sum "$manifests/flathub.flatpakrepo" | cut -d' ' -f1)" = "$flathub_sha256" ] || {
  echo "app/linux/flatpak/flathub.flatpakrepo isn't the copy this script pins." >&2
  exit 1
}
work=$(mktemp -d "${RUNNER_TEMP:-/tmp}/flatpak-check.XXXXXX")
# The manifest's source: the release's tarball, next to it.
cp "$tarball" "$manifests/PassBundle.tar.gz"
trap 'rm -f "$manifests/PassBundle.tar.gz"' EXIT

echo "Building"
flatpak remote-add --user --if-not-exists --from flathub "$manifests/flathub.flatpakrepo"
# Without rofiles-fuse, which a runner can't mount.
flatpak-builder --user --install-deps-from=flathub --disable-rofiles-fuse --force-clean \
  --state-dir="$work/state" --repo="$work/repo" "$work/build" "$manifests/$app_id.yml"
flatpak remote-add --user --no-gpg-verify passbundle-check "$work/repo"
flatpak install --user -y --noninteractive passbundle-check "$app_id"
flatpak info --user "$app_id"

echo "Its files"
# The release's folder as the tarball has it, nothing stripped or changed on
# the way in (the manifest's no-debuginfo).
mkdir "$work/release"
tar -xzf "$tarball" -C "$work/release"
installed="$(flatpak info --user --show-location "$app_id")/files/passbundle"
check "it holds the release's folder as it is" diff -r "$work/release/PassBundle" "$installed"

echo "Its permissions"
permissions=$(flatpak info --user --show-permissions "$app_id")
echo "$permissions"
has() { grep -qE "$1" <<< "$permissions"; }
lacks() { ! has "$1"; }
check "it reaches the network" has '^shared=.*network'
# Nothing that opens the sandbox wide: not the home folder or the whole
# computer (or its /etc), not every device, and not Flatpak's own service,
# which can run programs outside the sandbox.
check "not the home folder, the whole computer, or its /etc" lacks '^filesystems=.*(home|host)'
check "not every device" lacks '^devices=.*all'
check "not Flatpak's own service" lacks '^org\.freedesktop\.Flatpak='

echo "Its desktop entry"
exports="$HOME/.local/share/flatpak/exports/share"
entry="$exports/applications/$app_id.desktop"
cat "$entry" || true
check "Flatpak exports the desktop entry" test -f "$entry"
check "it takes passbundle:// links" grep -qx "MimeType=x-scheme-handler/passbundle;" "$entry"
check "it runs the app in its sandbox, with the link" grep -q "^Exec=.*flatpak run .*$app_id.*%u" "$entry"
check "with its icon" test -f "$exports/icons/hicolor/256x256/apps/$app_id.png"
# The folders a desktop session adds for Flatpak's exports (flatpak's own
# profile script), as at a desktop.
export XDG_DATA_DIRS="$exports:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
check "passbundle:// links open it for the person" sh -c "[ \"\$(xdg-mime query default x-scheme-handler/passbundle)\" = $app_id.desktop ]"

echo "The app, in its sandbox"
if [ -z "${XDG_RUNTIME_DIR:-}" ] || [ ! -d "$XDG_RUNTIME_DIR" ]; then
  export XDG_RUNTIME_DIR="$work/runtime"
  mkdir -m 700 "$XDG_RUNTIME_DIR"
fi
Xvfb :95 -screen 0 1280x800x24 > /dev/null 2>&1 &
display=$!
sleep 2
export DISPLAY=:95
# Its data folder, where a second copy leaves a link for the one signing in.
handed="$HOME/.var/app/$app_id/data/PassBundle/duo-callback.txt"
link="passbundle://duo-callback?code=c0de&state=FlatpakCheck"
rm -f "$handed"
# shellcheck disable=SC2016 # the session's own script, with its arguments
dbus-run-session -- bash -c '
  flatpak run "$4" > "$1/app.log" 2>&1 &
  for _ in $(seq 30); do pgrep -x passbundle > /dev/null && break; sleep 1; done
  sleep 5
  if pgrep -x passbundle > /dev/null; then echo opened > "$1/opened"; fi
  flatpak kill "$4" || true
  sleep 2
  # Opened as a browser hands one on.
  xdg-open "$2" &
  for _ in $(seq 60); do [ -s "$3" ] && break; sleep 1; done
  flatpak kill "$4" 2> /dev/null || true
' _ "$work" "$link" "$handed" "$app_id"
cat "$work/app.log" || true
check "opens and stays open" test -f "$work/opened"
check "a passbundle:// link reaches the app in its sandbox" sh -c "[ \"\$(cat '$handed')\" = '$link' ]"
kill "$display" 2> /dev/null || true

if [ "$failures" -gt 0 ]; then
  echo "$failures failed"
  exit 1
fi
echo "All passed"
