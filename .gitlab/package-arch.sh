#!/bin/sh
# Builds the Arch Linux packages for $CI_COMMIT_TAG from packaging/aur's PKGBUILD and
# the tarballs `package` and `package-aarch64` just made, and uploads them to the
# generic package registry as air-gestures-$CI_COMMIT_TAG-<arch>-linux.pkg.tar.zst —
# what the AUR package will build, for `pacman -U` until it is published there.
# Runs in archlinux:base-devel as root; makepkg itself runs as an unprivileged user.
set -eu
version="${CI_COMMIT_TAG#v}"
pkgver=$(sed -n 's/^pkgver=//p' packaging/aur/air-gestures-bin/PKGBUILD)
if [ "$pkgver" != "$version" ]; then
	echo "PKGBUILD's pkgver is $pkgver, the tag is $CI_COMMIT_TAG" >&2
	exit 1
fi

useradd -m builder
for arch in x86_64 aarch64; do
	dir="/home/builder/$arch"
	mkdir -p "$dir"
	cp packaging/aur/air-gestures-bin/PKGBUILD "$dir/"
	# The source makepkg would download, already here under the name it looks for.
	cp "air-gestures-$CI_COMMIT_TAG-$arch-linux.tar.gz" "$dir/air-gestures-$version-$arch-linux.tar.gz"
	# This runner is x86_64; for aarch64 only CARCH changes, since nothing is compiled,
	# and its strip cannot read aarch64 binaries.
	sed "s/^CARCH=.*/CARCH=\"$arch\"/; s/^CHOST=.*/CHOST=\"$arch-pc-linux-gnu\"/" \
		/etc/makepkg.conf > "$dir/makepkg.conf"
	if [ "$arch" != x86_64 ]; then
		sed -i '/^OPTIONS=/s/\([( ]\)strip\([ )]\)/\1!strip\2/' "$dir/makepkg.conf"
	fi
	echo 'PACKAGER="896kb <raducugarjoaba@gmail.com>"' >> "$dir/makepkg.conf"
	chown -R builder: "$dir"
	# The checksums are filled in after the release (see packaging/aur/README.md); these
	# tarballs come from this pipeline. --nodeps: package() only copies files.
	su builder -c "cd $dir && makepkg --config makepkg.conf --skipchecksums --nodeps --noconfirm"
	cp "$dir"/air-gestures-bin-"$version"-*-"$arch".pkg.tar.zst "air-gestures-$CI_COMMIT_TAG-$arch-linux.pkg.tar.zst"
	tar -tvf "air-gestures-$CI_COMMIT_TAG-$arch-linux.pkg.tar.zst"
	tar -xOf "air-gestures-$CI_COMMIT_TAG-$arch-linux.pkg.tar.zst" .PKGINFO
done

for arch in x86_64 aarch64; do
	file="air-gestures-$CI_COMMIT_TAG-$arch-linux.pkg.tar.zst"
	curl --fail --header "JOB-TOKEN: $CI_JOB_TOKEN" --upload-file "$file" \
		"$CI_API_V4_URL/projects/$CI_PROJECT_ID/packages/generic/air-gestures/$CI_COMMIT_TAG/$file"
done
