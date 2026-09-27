#!/bin/sh
# Sets up a Debian bookworm container to cross-build for aarch64: the cross
# compiler, the arm64 BlueZ/GTK/WebKit headers and libraries, and Rust.
set -eu
dpkg --add-architecture arm64
apt-get update -qq
apt-get install -y -qq --no-install-recommends \
	equivs ca-certificates curl gcc libc6-dev gcc-aarch64-linux-gnu libc6-dev-arm64-cross pkg-config
# The arm64 GTK headers depend on gobject-introspection:arm64, which drags in
# arm64 Python and can't sit next to the host's. Linking never runs it, so an
# empty stand-in of the same version satisfies the dependency.
version=$(apt-cache show gobject-introspection | sed -n 's/^Version: //p' | head -n1)
(
	cd /tmp
	printf 'Package: gobject-introspection\nVersion: %s\nArchitecture: arm64\nMulti-Arch: same\nDescription: empty stand-in for cross-linking\n' \
		"$version" > gi.control
	equivs-build --arch arm64 gi.control > /dev/null
	dpkg -i gobject-introspection_*_arm64.deb
)
apt-get install -y -qq --no-install-recommends \
	libdbus-1-dev:arm64 libgtk-3-dev:arm64 libwebkit2gtk-4.1-dev:arm64 \
	libsoup-3.0-dev:arm64 libjavascriptcoregtk-4.1-dev:arm64
curl -sSf https://sh.rustup.rs | sh -s -- -y -q --profile minimal --target aarch64-unknown-linux-gnu
