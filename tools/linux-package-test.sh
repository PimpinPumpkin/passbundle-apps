#!/usr/bin/env bash
# Checks a Linux package from tools/linux-packages.sh on a clean system
# (README, "Linux packages"): the .deb in an Ubuntu container, the .rpm in a
# Fedora one, as the release workflow's package tests run it.
#
#   tools/linux-package-test.sh <passbundle .deb or .rpm>
#
# It installs the package with apt or dnf, which brings in only what the
# package depends on, and checks every program and library in it finds the
# libraries it links to. Then, with the tools for the rest, it installs it
# again (so its scripts run with the menus' tools there) and checks what the
# meta repo's tests check the install script installs: every file, owner,
# and mode, the desktop entry, the icon, that passbundle:// links open the
# app (the system's cache, and xdg-mime's answer for a person), and that
# the package manager says the app is its own, as the install script and
# the app ask it. The app opens on a virtual display and stays open, and a
# link opened with xdg-open reaches it. Removing it leaves nothing of ours
# but the policy file the install script would have written. Last, it
# refuses to install over a PassBundle no package put in /opt/passbundle,
# saying how to switch, and leaves that one as it was.
#
# It installs and removes packages and adds a user: it runs only as root
# in a container, never on a computer of yours.
set -uo pipefail

[ $# -eq 1 ] || { echo "usage: tools/linux-package-test.sh <passbundle .deb or .rpm>" >&2; exit 2; }
[ "$(id -u)" -eq 0 ] || { echo "Run it as root, in a container." >&2; exit 2; }
if [ ! -f /.dockerenv ] && [ ! -f /run/.containerenv ]; then
  echo "It changes the computer it runs on: run it only in a throwaway container." >&2
  exit 2
fi

package=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
case "$package" in
  *.deb) kind=deb ;;
  *.rpm) kind=rpm ;;
  *) echo "$package isn't a .deb or an .rpm." >&2; exit 2 ;;
esac
app=/opt/passbundle
entry=/usr/share/applications/com.passbundle.app.desktop
icon=/usr/share/icons/hicolor/256x256/apps/com.passbundle.app.png
copyright=/usr/share/doc/passbundle/copyright
mime_cache=/usr/share/applications/mimeinfo.cache
policy=/etc/passbundle/policy.json
person=person
person_home=/home/$person
failures=0
export DEBIAN_FRONTEND=noninteractive

check() {
  name=$1
  shift
  if "$@" > /dev/null 2>&1; then
    echo "ok    $name"
  else
    echo "FAIL  $name"
    failures=$((failures + 1))
  fi
}

# Installs this package, or the named ones, with only what they depend on.
add() {
  if [ "$kind" = deb ]; then
    apt-get install -y -q --no-install-recommends "$@"
  else
    dnf install -y -q --setopt=install_weak_deps=False "$@"
  fi
}

as_person() {
  runuser -u "$person" -- env HOME="$person_home" DISPLAY=:99 "$@"
}

# The package that owns a path, as the install script and the app ask.
owner() {
  if [ "$kind" = deb ]; then
    dpkg-query -S "$1" 2> /dev/null | sed -n '1s/:.*//p'
  else
    # rpm says "not owned" on its standard output, so only its answer counts.
    name=$(rpm -qf --queryformat '%{NAME}' "$1" 2> /dev/null) && printf '%s' "$name"
  fi
}

# Every path the package has, one per line.
package_paths() {
  if [ "$kind" = deb ]; then
    dpkg-query -L passbundle
  else
    rpm -ql passbundle
  fi
}

# Paths the package may have: the app's folder and all in it, the desktop
# entry, the icon, and in the .deb its copyright file, and the folders
# they're in (which a .deb lists too).
ours() {
  case "$1" in
    "$app" | "$app"/* | "$entry" | "$icon") return 0 ;;
    "$copyright" | /usr/share/doc/passbundle) [ "$kind" = deb ] ;;
    /. | /opt | /usr | /usr/share | /usr/share/applications | /usr/share/icons | /usr/share/icons/hicolor | \
      /usr/share/icons/hicolor/256x256 | /usr/share/icons/hicolor/256x256/apps | /usr/share/doc) [ -d "$1" ] ;;
    *) return 1 ;;
  esac
}

only_ours() {
  while IFS= read -r path; do
    ours "$path" || { echo "not ours: $path" >&2; return 1; }
  done < <(package_paths)
}

# What the app's folder holds is exactly what its release.json lists.
as_released() {
  [ -f "$app/release.json" ] || return 0
  diff <(jq -r '.files[]' "$app/release.json" | LC_ALL=C sort) \
    <(cd "$app" && find . -mindepth 1 | sed 's|^\./||' | LC_ALL=C sort)
}

# shellcheck disable=SC1091 # the system's own
. /etc/os-release
echo "Installing $(basename "$package") on a clean $PRETTY_NAME"
if [ "$kind" = deb ]; then
  apt-get update -q
else
  # What this script needs for the first checks (find), if the image left it
  # out: no library the app could link to.
  dnf install -y -q findutils || exit 1
fi
for tool in find ldd od; do
  command -v "$tool" > /dev/null || { echo "This needs $tool." >&2; exit 1; }
done
if ! add "$package"; then
  echo "FAIL  installs, with what it depends on"
  exit 1
fi
echo "ok    installs, with what it depends on"
missing=""
while IFS= read -r -d '' file; do
  if [ "$(od -An -tx1 -N4 "$file" | tr -d ' \n')" = 7f454c46 ]; then
    found=$(ldd "$file" 2>&1 | grep 'not found')
    [ -z "$found" ] || missing="$missing$file: $found"$'\n'
  fi
done < <(find "$app" -type f -print0)
printf '%s' "$missing"
check "every program and library in it finds what it links to" test -z "$missing"

# A desktop has these; a container doesn't. Mesa is the display's OpenGL,
# which GTK loads when it draws (not something the app links to).
echo "The tools for the rest: a virtual display with Mesa, xdg-utils, and the menus' tools"
if [ "$kind" = deb ]; then
  add xvfb libgl1-mesa-dri libegl1 libgl1 xdg-utils desktop-file-utils procps jq || exit 1
else
  add xorg-x11-server-Xvfb mesa-dri-drivers mesa-libEGL mesa-libGL xdg-utils desktop-file-utils procps-ng \
    util-linux shadow-utils jq || exit 1
fi
# Installing the tools made the menus' cache; installing the package again
# has to make it anew.
rm -f "$mime_cache"
if [ "$kind" = deb ]; then
  apt-get install -y -q --reinstall "$package" > /dev/null
else
  dnf reinstall -y -q "$package" > /dev/null
fi
check "installs again over itself, as an update does" test $? -eq 0
useradd -m "$person"

echo "What it installs"
check "the app is in /opt" test -x "$app/passbundle"
check "with its core" test -f "$app/lib/libpassbundle_core.so"
check "only root can change it" sh -c "[ \"\$(stat -c %U '$app')\" = root ] && [ -z \"\$(find '$app' -perm -o+w -print -quit)\" ]"
check "everything in it is root's, and no group or anyone else can change it" \
  sh -c "[ -z \"\$(find '$app' \\( ! -user root -o ! -group root -o -perm /022 \\) -print -quit)\" ]"
check "its folders are 0755, its files 0644 or 0755, as the install script makes them" \
  sh -c "[ -z \"\$(find '$app' \\( \\( -type d ! -perm 755 \\) -o \\( -type f ! -perm 644 ! -perm 755 \\) \\) -print -quit)\" ]"
check "it holds exactly what its release.json lists" as_released
check "it's in the applications menu" grep -qx "Exec=$app/passbundle %u" "$entry"
check "under its name" grep -qx "Name=PassBundle" "$entry"
check "the desktop entry is valid" desktop-file-validate "$entry"
check "the desktop entry is root's and read-only" sh -c "[ \"\$(stat -c '%U:%G %a' '$entry')\" = 'root:root 644' ]"
check "with its icon" cmp -s "$icon" "$app/data/com.passbundle.app.png"
check "the icon is root's and read-only" sh -c "[ \"\$(stat -c '%U:%G %a' '$icon')\" = 'root:root 644' ]"
if [ "$kind" = deb ]; then
  check "its copyright file says GPL-3.0" grep -q 'GPL-3.0-only' "$copyright"
fi
check "it takes passbundle:// links" grep -qx "MimeType=x-scheme-handler/passbundle;" "$entry"
check "the system's cache lists it for passbundle:// links" grep -qx "x-scheme-handler/passbundle=com.passbundle.app.desktop;" "$mime_cache"
check "passbundle:// links open it for the person" sh -c "[ \"\$(runuser -u '$person' -- env HOME='$person_home' xdg-mime query default x-scheme-handler/passbundle)\" = com.passbundle.app.desktop ]"
check "the package manager says the app is the passbundle package's" sh -c "[ \"\$1\" = passbundle ]" _ "$(owner "$app/passbundle")"
check "it has nothing but those (no policies, nothing in /etc)" only_ours

echo "The app"
Xvfb :99 -screen 0 1280x800x24 > /dev/null 2>&1 &
sleep 2
as_person "$app/passbundle" > /tmp/passbundle.log 2>&1 &
sleep 5
check "opens and stays open" pgrep -x passbundle
pkill -x passbundle
sleep 1

echo "A passbundle:// link"
# Opened as a browser hands one on, the link reaches the installed app,
# which leaves it in the person's data folder for the copy signing in, and
# ends (core/src/two_step.rs, hand_over).
link="passbundle://duo-callback?code=c0de&state=PackageTest"
handed="$person_home/.local/share/PassBundle/duo-callback.txt"
as_person xdg-open "$link" > /dev/null 2>&1 &
waited=0
while [ ! -s "$handed" ] && [ "$waited" -lt 30 ]; do
  sleep 1
  waited=$((waited + 1))
done
check "a passbundle:// link reaches the installed app" sh -c "[ \"\$(cat '$handed')\" = '$link' ]"
pkill -x passbundle

echo "Removing"
# What install-passbundle.sh --server-url writes: the package leaves it.
mkdir -p "$(dirname "$policy")"
printf '{\n  "ServerUrl": "https://vault.example.test"\n}\n' > "$policy"
chmod 644 "$policy"
policy_before=$(cat "$policy")
if [ "$kind" = deb ]; then
  apt-get remove -y -q passbundle > /dev/null
  check "removes" test $? -eq 0
  apt-get purge -y -q passbundle > /dev/null
  check "and purges" test $? -eq 0
else
  dnf remove -y -q passbundle > /dev/null
  check "removes" test $? -eq 0
fi
check "the app is gone" test ! -e "$app"
check "the menu entry is gone" test ! -e "$entry"
check "the icon is gone" test ! -e "$icon"
check "the copyright file is gone" test ! -e "$copyright"
check "no package has the app" test -z "$(owner "$app/passbundle")"
check "nothing in the system's cache takes passbundle:// links" sh -c "! grep -q '^x-scheme-handler/passbundle=' '$mime_cache'"
check "nothing opens them for the person" sh -c "[ -z \"\$(runuser -u '$person' -- env HOME='$person_home' xdg-mime query default x-scheme-handler/passbundle)\" ]"
check "the install script's policy file is still there, as it was" sh -c "[ \"\$(cat '$policy')\" = \"\$1\" ]" _ "$policy_before"
# Anywhere but the person's own data (the vault's folder stays, as with any
# app), the package managers' caches, and this test's own folders.
leftovers=$(find / -xdev \( -path /proc -o -path /sys -o -path /dev -o -path /home -o -path /root -o -path /tmp \
  -o -path /var/cache -o -path /var/log -o -path /__w -o -path /__t -o -path /__e -o -path /github \) -prune \
  -o -iname '*passbundle*' -print | LC_ALL=C sort)
printf '%s\n' "$leftovers"
check "nothing of ours is left but the policy file" sh -c "[ \"\$1\" = \"\$(printf '/etc/passbundle\n/etc/passbundle/policy.json')\" ]" _ "$leftovers"
rm -rf /etc/passbundle

echo "Over a PassBundle no package put there"
# As the install script leaves it: the app's folder, root's.
mkdir -p "$app/lib" "$app/data"
printf '#!/bin/sh\n' > "$app/passbundle"
chmod 755 "$app/passbundle"
before=$(find "$app" -exec stat -c '%n %U %a %s' {} + | LC_ALL=C sort)
out=$(add "$package" 2>&1)
status=$?
printf '%s\n' "$out" | tail -n 5
check "refuses to install over it" test "$status" -ne 0
check "says how to switch: the install script's --remove, then the package" \
  sh -c "printf '%s' \"\$1\" | grep -q 'install-passbundle.sh' && printf '%s' \"\$1\" | grep -q -- '--remove'" _ "$out"
check "and leaves it as it was" sh -c "[ \"\$(find '$app' -exec stat -c '%n %U %a %s' {} + | LC_ALL=C sort)\" = \"\$1\" ]" _ "$before"
check "the package isn't installed" test -z "$(owner "$app/passbundle")"
check "nor its desktop entry" test ! -e "$entry"
rm -rf "$app"

if [ "$failures" -gt 0 ]; then
  echo "$failures failed"
  exit 1
fi
echo "All passed"
