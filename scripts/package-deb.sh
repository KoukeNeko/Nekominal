#!/usr/bin/env bash
# Builds the release executable and packs it into dist/nekominal_<version>_<arch>.deb, with a checksum file beside it. The
# architecture is the one of the machine (amd64, arm64): the build is native. Run from any directory; cargo and dpkg-deb
# have to be on PATH.
#
#   scripts/package-deb.sh             # build, then pack
#   scripts/package-deb.sh --no-build  # pack the executable that is already built (in $CARGO_TARGET_DIR or target/)
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

version="$(sed -n '/^\[workspace\.package\]/,/^\[/{s/^version *= *"\(.*\)"/\1/p}' Cargo.toml | head -n1)"
[ -n "$version" ] || { echo "No workspace version in Cargo.toml." >&2; exit 1; }
# Before the long build, not after it.
if [ "${GITHUB_REF_TYPE:-}" = tag ] && [ "${GITHUB_REF_NAME:-}" != "v$version" ]; then
  echo "The tag $GITHUB_REF_NAME does not match the version $version of Cargo.toml." >&2
  exit 1
fi
arch="$(dpkg --print-architecture)"

if [ "${1:-}" != --no-build ]; then
  cargo build --release --locked -p nekominal-shell
fi
exe="${CARGO_TARGET_DIR:-target}/release/nekominal-next"
[ -x "$exe" ] || { echo "Missing $exe. Build first, without --no-build." >&2; exit 1; }

name="nekominal_${version}_${arch}"
stage="dist/$name"
rm -rf "$stage"
install -Dm755 "$exe" "$stage/usr/bin/nekominal-next"
# `nekominal cli ...` is how the documentation spells the automation commands.
ln -s nekominal-next "$stage/usr/bin/nekominal"
install -Dm644 distribution/nekominal.desktop "$stage/usr/share/applications/nekominal.desktop"
install -Dm644 assets/icon.svg "$stage/usr/share/icons/hicolor/scalable/apps/nekominal.svg"
install -Dm644 LICENSE "$stage/usr/share/doc/nekominal/copyright"
install -Dm644 THIRD-PARTY-NOTICES.md "$stage/usr/share/doc/nekominal/THIRD-PARTY-NOTICES.md"
install -Dm644 README.md "$stage/usr/share/doc/nekominal/README.md"

size="$(du -sk "$stage" | cut -f1)"
mkdir -p "$stage/DEBIAN"
# The windowing, font and graphics libraries are opened when the program runs, so the packager cannot see them: they are
# named here. Either windowing system will do; the drivers and the helpers are recommended, not required.
cat > "$stage/DEBIAN/control" <<CONTROL
Package: nekominal
Version: $version
Section: x11
Priority: optional
Architecture: $arch
Installed-Size: $size
Maintainer: KoukeNeko <111033412+KoukeNeko@users.noreply.github.com>
Homepage: https://github.com/KoukeNeko/Nekominal
Depends: libc6 (>= 2.35), libfontconfig1, libvulkan1, libxkbcommon0
Recommends: mesa-vulkan-drivers | nvidia-vulkan-icd, libwayland-client0, libx11-6, libxkbcommon-x11-0, libxcursor1, fonts-dejavu-core, xdg-utils, libnotify-bin, zenity | kdialog
Suggests: wl-clipboard, fonts-noto-cjk, fonts-noto-color-emoji
Description: Terminal that stays light, and knows what your agents are doing
 Nekominal draws its own window with wgpu on Wayland and X11, runs shells on a pty, and shows for every tab whether an
 agent (Claude Code, Codex, Gemini CLI, Aider) is working, waiting for you, done or failed.
CONTROL

mkdir -p dist
dpkg-deb --root-owner-group -Zxz --build "$stage" "dist/$name.deb" >/dev/null
rm -rf "$stage"
(cd dist && sha256sum "$name.deb" > "$name.deb.sha256")
echo "dist/$name.deb"
