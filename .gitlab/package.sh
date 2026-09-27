#!/bin/sh
# Builds the release binaries for $1 (a Rust target) and uploads
# air-gestures-$CI_COMMIT_TAG-$2-linux.tar.gz to the generic package registry.
set -eu
target="$1"
arch="$2"
cargo build --workspace --release --locked --target "$target"
dir="air-gestures-$CI_COMMIT_TAG"
tarball="air-gestures-$CI_COMMIT_TAG-$arch-linux.tar.gz"
mkdir -p "$dir"
install -m755 "target/$target/release/air-gestures" "target/$target/release/air-gesturesd" "$dir/"
install -m644 packaging/air-gesturesd.service contrib/config.toml README.md LICENSE LICENSE-kinesis "$dir/"
tar -czf "$tarball" --owner=0 --group=0 "$dir"
tar -tzvf "$tarball"
curl --fail --header "JOB-TOKEN: $CI_JOB_TOKEN" --upload-file "$tarball" \
	"$CI_API_V4_URL/projects/$CI_PROJECT_ID/packages/generic/air-gestures/$CI_COMMIT_TAG/$tarball"
