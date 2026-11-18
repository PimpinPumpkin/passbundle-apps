#!/usr/bin/env bash
# Makes the Linux app's .deb or .rpm from a release's PassBundle folder
# (README, "Linux packages"; the meta repo's DESIGN.md, "Linux packages").
#
#   tools/linux-packages.sh deb|rpm <PassBundle folder> <version> <linux-x64|linux-arm64> <out folder>
#
# Each installs what the meta repo's install script
# (installer/linux/install-passbundle.sh) installs for the app, at the same
# paths, with the same owner and modes: the folder in /opt/passbundle,
# root's, that no one else can change; its desktop entry
# (app/linux/com.passbundle.app.desktop, the script's own, which also takes
# passbundle:// links); and its icon. Its scripts update the menus' icon
# cache and the cache of what each desktop entry opens after it's installed
# and removed, as the install script does. It writes no browser policies
# and not the app's policy file (/etc/passbundle/policy.json): those are the
# install script's, which needs the client's server and token, and removing
# the package leaves them. The .deb also has its copyright file, as every
# Debian package does.
#
# A first install refuses to go over a PassBundle in /opt/passbundle that
# no package put there (the install script's, or one unpacked by hand), and
# says how to switch.
#
# Dependencies: the .deb's are what dpkg-shlibdeps finds the programs and
# libraries in it link to, from the packages that have those libraries on
# the computer it's made on. The .rpm's are what rpmbuild's own generator
# finds the same way, so make it on an RPM system (the release workflow uses
# a Fedora container). The libraries the folder has itself (its lib folder)
# are neither depended on nor offered to other packages. Make each on the
# chip it's for.
set -euo pipefail
umask 022

fail() {
  echo "$1" >&2
  exit 1
}

[ $# -eq 5 ] || fail "usage: tools/linux-packages.sh deb|rpm <PassBundle folder> <version> <linux-x64|linux-arm64> <out folder>"
kind=$1 folder=$2 version=$3 platform=$4 out=$5
here=$(cd "$(dirname "$0")" && pwd)
entry="$here/../app/linux/com.passbundle.app.desktop"
app=/opt/passbundle

case "$kind" in
  deb | rpm) ;;
  *) fail "Packages are deb or rpm, not $kind." ;;
esac
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "The version is a release's, such as 0.1.5, not $version."
case "$platform" in
  linux-x64) deb_arch=amd64 rpm_arch=x86_64 ;;
  linux-arm64) deb_arch=arm64 rpm_arch=aarch64 ;;
  *) fail "No packages for $platform." ;;
esac
[ "$(uname -m)" = "$rpm_arch" ] || fail "Make the $platform packages on an $rpm_arch computer, not $(uname -m)."
# What the install script checks the folder has.
for file in passbundle lib/libpassbundle_core.so lib/libapp.so data/com.passbundle.app.png; do
  [ -f "$folder/$file" ] || fail "$folder is missing $file."
done
[ -z "$(find "$folder" -type l)" ] || fail "$folder has links in it; a release has none."
mkdir -p "$out"
out=$(cd "$out" && pwd)

work=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/linux-packages.XXXXXX")
trap 'rm -rf "$work"' EXIT
root="$work/root"

# What the install script puts on the computer: the folder copied, then made
# root's (the package tool's part) and readable by all, with no one but its
# owner able to change it (chmod -R go-w,a+rX); the desktop entry and the
# icon, 0644.
mkdir -p "$root/opt" "$root/usr/share/applications" "$root/usr/share/icons/hicolor/256x256/apps"
cp -R "$folder" "$root$app"
chmod -R go-w,a+rX "$root$app"
install -m 644 "$entry" "$root/usr/share/applications/com.passbundle.app.desktop"
install -m 644 "$root$app/data/com.passbundle.app.png" "$root/usr/share/icons/hicolor/256x256/apps/com.passbundle.app.png"

summary="PassBundle's password manager app"
description="The PassBundle app for Linux: the vault on your PassBundle server, on this
computer. It installs to /opt/passbundle with a menu entry, and opens
passbundle:// links, which bring a Duo sign-in back to the app."

# The scripts' parts, in POSIX shell.
refresh_menus='# The menus'"'"' icons, and the cache of what each desktop entry opens
# (mimeinfo.cache), which lists the app for passbundle:// links.
if command -v gtk-update-icon-cache > /dev/null 2>&1; then
  gtk-update-icon-cache -q -t /usr/share/icons/hicolor || true
fi
if command -v update-desktop-database > /dev/null 2>&1; then
  update-desktop-database -q /usr/share/applications || true
fi'
refuse_others="# A PassBundle no package put here (the install script's, or one unpacked by
# hand) stays as it is: say how to switch.
if [ -L $app ] || { [ -e $app ] && [ -n \"\$(ls -A $app 2> /dev/null)\" ]; }; then
  echo \"PassBundle is already in $app, and not from this package: PassBundle's install script (install-passbundle.sh) or someone by hand put it there. To switch to this package, run the install script as root with --remove, which takes that copy and the browser policies away, then install this package, then run the install script again to set the policies: it leaves the package's app alone. Nothing was changed.\" >&2
  exit 1
fi"

# Every program and library in the folder.
elves=()
while IFS= read -r -d '' file; do
  if [ "$(od -An -tx1 -N4 "$file" | tr -d ' \n')" = 7f454c46 ]; then
    elves+=("$file")
  fi
done < <(find "$root$app" -type f -print0)
[ ${#elves[@]} -gt 0 ] || fail "No programs in $folder."

# The maintainer scripts' $1 is theirs, not this script's.
# shellcheck disable=SC2016
make_deb() {
  command -v dpkg-shlibdeps > /dev/null || fail "Making the .deb takes dpkg-dev (dpkg-shlibdeps)."
  mkdir -p "$root/usr/share/doc/passbundle"
  cat > "$root/usr/share/doc/passbundle/copyright" << 'EOF'
PassBundle for Linux
Source: https://github.com/PimpinPumpkin/passbundle-apps

Licensed under the GNU General Public License, version 3 (GPL-3.0-only).
On Debian and Ubuntu, its text is /usr/share/common-licenses/GPL-3.
EOF
  chmod 644 "$root/usr/share/doc/passbundle/copyright"
  mkdir -p "$root/DEBIAN"

  # dpkg-shlibdeps runs in a source package's folder, so it gets one with
  # nothing but a control file. It finds the folder's own libraries (lib/,
  # where the app's RUNPATH points) in the package itself, which has no
  # dependency information for them: those are ignored, not depended on.
  mkdir -p "$work/source/debian"
  printf 'Source: passbundle\n\nPackage: passbundle\nArchitecture: any\n' > "$work/source/debian/control"
  substvars=$(cd "$work/source" && dpkg-shlibdeps -O --ignore-missing-info -l"$root$app/lib" "${elves[@]}")
  depends=$(printf '%s\n' "$substvars" | sed -n 's/^shlibs:Depends=//p')
  [ -n "$depends" ] || fail "dpkg-shlibdeps found nothing the app depends on: $substvars"
  size=$(du -sk "$root/opt" "$root/usr" | awk '{ total += $1 } END { print total }')

  {
    echo "Package: passbundle"
    echo "Version: $version"
    echo "Architecture: $deb_arch"
    echo "Maintainer: PassBundle <hello@passbundle.com>"
    echo "Installed-Size: $size"
    echo "Depends: $depends"
    echo "Section: utils"
    echo "Priority: optional"
    echo "Homepage: https://passbundle.com"
    echo "Description: $summary"
    printf '%s\n' "$description" | sed 's/^/ /'
  } > "$root/DEBIAN/control"
  {
    printf '#!/bin/sh\nset -e\n'
    printf '# A first install, not an upgrade.\nif [ "$1" = install ]; then\n'
    printf '%s\n' "$refuse_others" | sed 's/^/  /'
    printf 'fi\nexit 0\n'
  } > "$root/DEBIAN/preinst"
  {
    printf '#!/bin/sh\nset -e\nif [ "$1" = configure ]; then\n'
    printf '%s\n' "$refresh_menus" | sed 's/^/  /'
    printf 'fi\nexit 0\n'
  } > "$root/DEBIAN/postinst"
  {
    printf '#!/bin/sh\nset -e\ncase "$1" in\n  remove | purge)\n'
    printf '%s\n' "$refresh_menus" | sed 's/^/    /'
    printf '    ;;\nesac\nexit 0\n'
  } > "$root/DEBIAN/postrm"
  chmod 644 "$root/DEBIAN/control"
  chmod 755 "$root/DEBIAN/preinst" "$root/DEBIAN/postinst" "$root/DEBIAN/postrm"
  for script in preinst postinst postrm; do
    sh -n "$root/DEBIAN/$script"
  done

  deb="$out/passbundle_${version}_${deb_arch}.deb"
  # xz, which every dpkg reads (Ubuntu's own default, zstd, isn't).
  dpkg-deb --root-owner-group -Zxz --build "$root" "$deb"
  dpkg-deb --info "$deb"
  dpkg-deb --contents "$deb"
}

make_rpm() {
  command -v rpmbuild > /dev/null || fail "Making the .rpm takes rpmbuild (rpm-build)."
  # The folder's own libraries, by name up to .so: whatever links to them
  # finds them there (RUNPATH), so no other package is asked for them.
  private=""
  for file in "$root$app"/lib/*.so*; do
    name=$(basename "$file")
    name="${name%%.so*}.so"
    [[ $name =~ ^[A-Za-z0-9_.-]+$ ]] || fail "A library's name the .rpm can't take: $name"
    private="${private:+$private|}$name"
  done
  [ -n "$private" ] || fail "No libraries in $folder/lib."
  spec="$work/passbundle.spec"
  pre=$(printf '%s\n' "$refuse_others" | sed 's/^/  /')
  # The package's files exactly as they are in the folder: none of
  # rpmbuild's steps that strip, compress, or add to them (debug packages,
  # build-id links in /usr/lib/.build-id). The scripts' text goes in as it
  # is: rpm expands only % in them, and they have none.
  case "$refuse_others$refresh_menus$description" in
    *%*) fail "A script or the description has a %, which rpm would expand." ;;
  esac
  cat > "$spec" << EOF
Name: passbundle
Version: $version
Release: 1
Summary: $summary
License: GPL-3.0-only
URL: https://passbundle.com
Vendor: PassBundle
Packager: PassBundle <hello@passbundle.com>
ExclusiveArch: $rpm_arch

%global __requires_exclude ^($private)
%global __provides_exclude_from ^$app/
%global debug_package %{nil}
%global _build_id_links none
%global __os_install_post %{nil}
%global __arch_install_post %{nil}

%description
$description

%install
cp -a %{pb_root}/. %{buildroot}/

%pre
# A first install, not an upgrade.
if [ "\$1" -eq 1 ]; then
$pre
fi

%post
$refresh_menus

%postun
$refresh_menus

%files
%defattr(-,root,root,-)
$app
/usr/share/applications/com.passbundle.app.desktop
/usr/share/icons/hicolor/256x256/apps/com.passbundle.app.png
EOF
  rpmbuild -bb "$spec" \
    --target "$rpm_arch" \
    --define "_topdir $work/rpmbuild" \
    --define "_rpmdir $out" \
    --define "_build_name_fmt %%{NAME}-%%{VERSION}-%%{RELEASE}.%%{ARCH}.rpm" \
    --define "pb_root $root"
  rpm="$out/passbundle-$version-1.$rpm_arch.rpm"
  rpm -qpi "$rpm"
  echo "Requires:"
  rpm -qpR "$rpm"
  rpm -qp --scripts "$rpm"
  rpm -qplv "$rpm"
}

if [ "$kind" = deb ]; then
  make_deb
else
  make_rpm
fi
