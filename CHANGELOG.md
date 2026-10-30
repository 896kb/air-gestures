# Changelog

Each release's section becomes its GitLab release notes. Add a section for the version
before tagging it; the release job fails without one.

## 0.9.31

- **Skip a drag you can't make** (thanks to David Ljung Madison Stellar):
  `air-gestures teach-drags --skip up,down` teaches the turn and the other drags; a skipped
  drag is never read, and a move like it does nothing. Calibrations saved before read as all
  six drags taught.

## 0.9.18

- **New installs start with band controls:** the sample config (`/usr/share/air-gestures/config.toml`,
  `contrib/config.toml`) puts `toggle` on the middle hold, `next_profile` and
  `previous_profile` on the index and middle triple taps, `draw` on the index hold, and the
  middle dial on brightness, as in the Android app; with no config file the daemon runs it.
  A config that leaves these keys out works as before (no band controls, no middle dial).
- **Slider commands:** a slider set to anything but `"volume"`, `"brightness"` or `"off"` runs
  that command as you slide, at most every 250 ms, with `AIR_GESTURES_SLIDER`
  (`index_left_right` …), `AIR_GESTURES_STEPS` (the net steps since, `+3` or `-2`) and
  `AIR_GESTURES_PROFILE`, and once more when you let go: one script can drive smart lights
  from every slider (README has a Home Assistant example).
- **Several actions on one gesture:** the README shows a command that runs a few in a row
  (`cmd1; cmd2`), the way the Android app's sequences do.
- **Band controls:** `[band] toggle`, `next_profile`, `previous_profile` and `draw` each sit on
  one gesture you pick (any gesture, or `"none"`), above every profile, optionally only
  in some palm positions (`toggle_palm = ["up"]`). A profile's command on that gesture is kept
  and paused; the daemon logs each one. `toggle = "middle_hold"` from older configs keeps working.
- **Profiles:** `[[profile]]` tables in config.toml, each with its own gestures. Switch with
  `next_profile` / `previous_profile`, or `air-gestures profile next|previous|<name>`;
  `on_profile` runs on every switch with `AIR_GESTURES_PROFILE` set.
- **The pinch:** index and middle each have a hold, a dial (`index_dial`, `middle_dial`) and six
  drags, as gestures (`index_drag_left` …) or sliders (`index_drags = "sliders"`). New gestures
  `index_hold`, `index_triple`, `middle_triple` and the twelve drags.
  `air-gestures teach-drags` teaches your own drags.
- **Palm position:** `[gestures.palm_up]`, `.sideways` and `.palm_down` override gestures in
  that position; `air-gestures calibrate` fits palm down and up to your wrist (needed for the
  left hand).
- **Drawings:** Draw a shape with a pinch to run its action (`[drawings]`,
  `[profile.drawings]`); teach shapes with `air-gestures draw teach <name>`; `on_draw` shows
  what was drawn.
- Every config load logs warnings (no way to switch profiles, Draw on a tap) and each paused
  command.
- Palm down and up the right way round (band-core's right-hand defaults had them swapped), and
  drawings the right way up. Drawings taught before are turned over when the daemon loads them
  (exact for those taught without a palm calibration; the originals are kept as
  `<name>.json.bak`): teach one again if it still doesn't match well.
- The log says what each gesture did, with its palm: `index tap · palm up → <command>`, a
  band control, or why nothing ran (`nothing bound here`, `the controls are off`, …); a dial or
  slider pinch as `index dial · palm up → volume up ×2`; `air-gestures status` shows the last
  gesture's palm.
- A slow triple tap no longer runs its finger's single tap first.
- Gestures, the dial and the controls now come from band-core's shared controls layer, the
  same code as the Android app.
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
