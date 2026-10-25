# Changelog

Each release's section becomes its GitLab release notes. Add a section for the version
before tagging it; the release job fails without one.

## Unreleased

- Profiles: `[[profile]]` tables in config.toml, each with its own gestures. Switch them
  from the band with `@next-profile` / `@previous-profile` (by default on the index and
  middle triple taps), or with `air-gestures profile next|previous|<name>`. `on_profile`
  runs on every switch with `AIR_GESTURES_PROFILE` set.
- Turning the controls off and on is a built-in action, `@controls`, that can go on any
  gesture (by default the middle hold). The old `[band] toggle` keeps working.
- New defaults where your config doesn't say: middle hold turns the controls off/on, and
  the triple taps switch profiles (they do nothing with one profile).
- A slow triple tap no longer runs its finger's single tap first.
- Gestures, the dial and the controls now come from band-core's shared controls layer,
  the same code as the Android app.
- A band busy with another device says so instead of "Missing protobuf integer".

## 0.3.10

- **Arch Linux packages** on every release, for x86_64 and aarch64: the AUR package
  `air-gestures-bin`, built by makepkg from its PKGBUILD in CI, ahead of its publication on
  the AUR (paused registration). `sudo pacman -U ./air-gestures-v0.3.10-x86_64-linux.pkg.tar.zst`
  pulls in BlueZ, WebKitGTK and the rest; checked by installing and running it on Arch.

## 0.3.9

- **.deb and .rpm packages** on every release, for x86_64 and aarch64, next to the
  tarballs: `sudo apt install ./air-gestures-v0.3.9-x86_64-linux.deb` on Debian 12+ and
  Ubuntu 24.04+, `sudo dnf install ./air-gestures-v0.3.9-x86_64-linux.rpm` on Fedora (or
  `zypper install` on openSUSE). They pull in BlueZ and the libraries, and install the
  binaries to /usr/bin and the user unit to /usr/lib/systemd/user/. Checked by installing
  and running them on Debian 12 and 13, Ubuntu 24.04, Fedora and openSUSE Tumbleweed.
- The example config is installed at /usr/share/air-gestures/config.toml (it was under
  /usr/share/doc, which minimal installs strip).

## 0.3.8

- Builds for 64-bit ARM Linux (aarch64: Raspberry Pi 4/5, Asahi, ARM laptops), next to
  x86_64. It's cross-compiled and tested under emulation only; reports from real
  hardware are welcome.
- `air-gestures disconnect` now also drops the band's Bluetooth link, so another
  device (like the Android app) can find and connect to the band right away.
