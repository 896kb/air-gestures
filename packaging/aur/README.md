# AUR packaging

`air-gestures-bin/` holds the PKGBUILD and .SRCINFO for the
[`air-gestures-bin`](https://aur.archlinux.org/packages/air-gestures-bin) AUR package. It
repackages the x86_64 or aarch64 tarball that CI attaches to each GitLab release:

- `/usr/bin/air-gestures`, `/usr/bin/air-gesturesd`
- `/usr/lib/systemd/user/air-gesturesd.service` (from `packaging/air-gesturesd.service`,
  `ExecStart=/usr/bin/air-gesturesd`)
- `/usr/share/air-gestures/config.toml` (example config) and `/usr/share/doc/air-gestures/README.md`
- `/usr/share/licenses/air-gestures-bin/LICENSE`, `LICENSE-kinesis`

The sources are `https://gitlab.com/896kb/air-gestures/-/releases/v$pkgver/downloads/air-gestures-<arch>-linux.tar.gz`
(`source_x86_64`, `source_aarch64`), the release's direct asset links, which only exists once the `v$pkgver` tag pipeline has
finished. Until then the `sha256sums_*` are placeholders of zeros; fill it with `updpkgsums`.

## Publishing a new version

1. Bump the version in the workspace `Cargo.toml`, commit, then tag and push:
   `git tag vX.Y.Z && git push origin vX.Y.Z`. Wait for the `release` job to finish.
2. In `air-gestures-bin/PKGBUILD` set `pkgver=X.Y.Z` and `pkgrel=1`.
3. Update the checksum and metadata, and test-build:

   ```sh
   cd packaging/aur/air-gestures-bin
   updpkgsums                          # downloads both release tarballs, writes the sha256sums
   makepkg --printsrcinfo > .SRCINFO
   makepkg -f && namcap air-gestures-bin-*.pkg.tar.zst   # optional check
   ```

   Commit the updated PKGBUILD and .SRCINFO in this repo as well.
4. Push to the AUR (first time: the clone is empty, which is fine):

   ```sh
   git clone ssh://aur@aur.archlinux.org/air-gestures-bin.git ~/aur/air-gestures-bin
   cp packaging/aur/air-gestures-bin/{PKGBUILD,.SRCINFO} ~/aur/air-gestures-bin/
   cd ~/aur/air-gestures-bin
   git add PKGBUILD .SRCINFO
   git commit -m "Update to X.Y.Z"
   git push
   ```

The AUR repo must contain only `PKGBUILD` and `.SRCINFO` (no tarballs or built packages), and
its branch must be `master`.

## After installing

```sh
mkdir -p ~/.config/air-gestures
cp /usr/share/air-gestures/config.toml ~/.config/air-gestures/
systemctl --user daemon-reload && systemctl --user enable --now air-gesturesd
```
