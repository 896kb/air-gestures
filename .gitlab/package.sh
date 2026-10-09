#!/bin/sh
# Builds the release binaries for $1 (a Rust target), packs them as
# air-gestures-$CI_COMMIT_TAG-$2-linux.tar.gz, .deb and .rpm, and uploads all three
# to the generic package registry. The .deb and .rpm are described in
# air-gesturesd/Cargo.toml.
set -eu
target="$1"
arch="$2"
cargo build --workspace --release --locked --target "$target"
# The packagers run on the build machine: under the aarch64 cross-build, pkg-config
# must not hand them the arm64 libraries.
env -u PKG_CONFIG_LIBDIR -u PKG_CONFIG_ALLOW_CROSS \
	cargo install --locked cargo-deb@3.8.0 cargo-generate-rpm@0.21.0

dir="air-gestures-$CI_COMMIT_TAG"
name="air-gestures-$CI_COMMIT_TAG-$arch-linux"
mkdir -p "$dir"
install -m755 "target/$target/release/air-gestures" "target/$target/release/air-gesturesd" "$dir/"
install -m644 packaging/air-gesturesd.service contrib/config.toml README.md LICENSE LICENSE-kinesis "$dir/"
tar -czf "$name.tar.gz" --owner=0 --group=0 "$dir"
tar -tzvf "$name.tar.gz"

# Dependencies are read off the binaries: dpkg-shlibdeps for the .deb, the ELF's
# own sonames for the .rpm.
cargo deb -p air-gesturesd --no-build --target "$target" -o "$name.deb"
dpkg-deb --info "$name.deb"
dpkg-deb --contents "$name.deb"
cargo generate-rpm -p air-gesturesd --target "$target" -o "$name.rpm"

for file in "$name.tar.gz" "$name.deb" "$name.rpm"; do
	curl --fail --header "JOB-TOKEN: $CI_JOB_TOKEN" --upload-file "$file" \
		"$CI_API_V4_URL/projects/$CI_PROJECT_ID/packages/generic/air-gestures/$CI_COMMIT_TAG/$file"
done
