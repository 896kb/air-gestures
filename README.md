# air-gestures

Use a **Meta Neural Band** (the sEMG wristband) to control a Linux desktop: thumb swipes, index
and middle finger taps, a middle-finger hold and pinch + turn your wrist each run a shell command
you choose (workspaces, media keys, volume, brightness, …). Includes a waybar module and a raw
sEMG viewer/recorder.

A Rust port of [callbacked/kinesis](https://github.com/callbacked/kinesis) (macOS, MIT) for
Linux/BlueZ.

> **Status: experimental, working.** Tested on one band (September 2026 firmware) with Arch
> Linux, BlueZ and Hyprland: claim, reconnect, gestures, pinch dial, battery. That firmware
> differs from what kinesis was built against (encryption scheme 26, 16-byte padding, tagged
> channels); see [`docs/hardware-day.md`](docs/hardware-day.md#findings).

## Read this before you use a band with it

- The band has **one owner**. Claiming it for your computer (the same enrollment the Meta AI
  app does) unlinks it from your Meta glasses / phone app until you factory-reset it (hold its
  button ~16 s) and set it up there again.
- Claiming needs a one-time Meta sign-in on Meta's own page; after that, connecting uses only a
  locally stored key.
- The protocol is reverse-engineered and experimental. Meta can change it at any time.

## Layout

| crate | what it is |
|---|---|
| `band-core` | sans-IO protocol core: AirShield encryption, DataX framing, session state machine (handshake, owner trust, ownership ceremony, input subscription), gestures, pinch dial, handedness, battery, raw sEMG, and `SimBand` — a scripted simulated band for tests and dry runs |
| `air-gesturesd` | BlueZ daemon: gestures → configurable shell actions, pinch dial, ownership claim, control socket (`air-gesturesd --dry-run` runs it against a simulated band) |
| `air-gestures` | Meta sign-in window, claiming (`pair`), status (plain or waybar JSON), pause/resume, dial target, hand, sEMG viewer/recorder |

Design (written under the working name kinesis-linux): [`docs/superpowers/specs/2026-09-24-kinesis-linux-design.md`](docs/superpowers/specs/2026-09-24-kinesis-linux-design.md).

## Install

Runtime needs BlueZ, D-Bus and WebKitGTK 4.1 (`webkit2gtk-4.1` on Arch) for the sign-in window.

### Arch Linux

> **Not on the AUR yet.** The AUR has temporarily closed new account registration (after a
> wave of automated sign-ups), so `air-gestures-bin` can't be published. It will be as soon
> as registration reopens. Until then each release carries the package it builds:
> [air-gestures-x86_64-linux.pkg.tar.zst](https://gitlab.com/896kb/air-gestures/-/releases/permalink/latest/downloads/air-gestures-x86_64-linux.pkg.tar.zst)
> ([aarch64](https://gitlab.com/896kb/air-gestures/-/releases/permalink/latest/downloads/air-gestures-aarch64-linux.pkg.tar.zst)),
> installed with `sudo pacman -U ./air-gestures-x86_64-linux.pkg.tar.zst` (it updates only
> when you install a newer one).

~~`paru -S air-gestures-bin`~~ once it's on the AUR, or the package above, then:

```sh
mkdir -p ~/.config/air-gestures
cp /usr/share/air-gestures/config.toml ~/.config/air-gestures/   # every key, commented
systemctl --user daemon-reload && systemctl --user enable --now air-gesturesd
```

### Debian, Ubuntu, Fedora, openSUSE (.deb / .rpm)

Debian 12+, Ubuntu 24.04+ and derivatives:
[air-gestures-x86_64-linux.deb](https://gitlab.com/896kb/air-gestures/-/releases/permalink/latest/downloads/air-gestures-x86_64-linux.deb)
([aarch64](https://gitlab.com/896kb/air-gestures/-/releases/permalink/latest/downloads/air-gestures-aarch64-linux.deb)).
Fedora, openSUSE and other RPM distributions:
[air-gestures-x86_64-linux.rpm](https://gitlab.com/896kb/air-gestures/-/releases/permalink/latest/downloads/air-gestures-x86_64-linux.rpm)
([aarch64](https://gitlab.com/896kb/air-gestures/-/releases/permalink/latest/downloads/air-gestures-aarch64-linux.rpm)).
Both pull in BlueZ and the runtime libraries.

```sh
sudo apt install ./air-gestures-x86_64-linux.deb     # or: sudo dnf install ./air-gestures-x86_64-linux.rpm
mkdir -p ~/.config/air-gestures
cp /usr/share/air-gestures/config.toml ~/.config/air-gestures/   # every key, commented
systemctl --user daemon-reload && systemctl --user enable --now air-gesturesd
```

### Release tarball (x86_64 or aarch64, glibc ≥ 2.36)

[air-gestures-x86_64-linux.tar.gz](https://gitlab.com/896kb/air-gestures/-/releases/permalink/latest/downloads/air-gestures-x86_64-linux.tar.gz)
or, for 64-bit ARM (Raspberry Pi 4/5, Asahi, ARM laptops),
[air-gestures-aarch64-linux.tar.gz](https://gitlab.com/896kb/air-gestures/-/releases/permalink/latest/downloads/air-gestures-aarch64-linux.tar.gz)
(latest release). The aarch64 build is cross-compiled and tested under emulation only; if
you run it with a real band, please report how it goes. Each has both binaries, the user unit, the example config and the licenses. The
unit runs `/usr/bin/air-gesturesd`; to install into your home instead:

```sh
tar xzf air-gestures-*-linux.tar.gz && cd air-gestures-v*/
install -Dm755 -t ~/.local/bin air-gestures air-gesturesd
mkdir -p ~/.config/systemd/user ~/.config/air-gestures
sed 's|^ExecStart=.*|ExecStart=%h/.local/bin/air-gesturesd|' air-gesturesd.service \
    > ~/.config/systemd/user/air-gesturesd.service
cp config.toml ~/.config/air-gestures/
systemctl --user daemon-reload && systemctl --user enable --now air-gesturesd
```

### From source

Needs Rust ≥ 1.88 and the development files for D-Bus and WebKitGTK 4.1.

```sh
cargo install --locked --path air-gesturesd
cargo install --locked --path air-gestures
mkdir -p ~/.config/systemd/user ~/.config/air-gestures
cp contrib/air-gesturesd.service ~/.config/systemd/user/
cp contrib/config.toml ~/.config/air-gestures/        # every key, commented
systemctl --user daemon-reload && systemctl --user enable --now air-gesturesd
```

## Claim your band

1. `air-gestures signin` — Meta's sign-in page opens in a window; the password only goes to Meta.
2. Factory reset the band (hold its button ~16 s), then put it in pairing mode (hold 3 s until
   it flashes). Make sure `bluetoothctl show` says `Pairable: yes`.
3. `air-gestures pair` — claims it; from then on air-gesturesd connects on its own.

`air-gestures status` shows the link, battery, hand and dial target. For waybar:

```jsonc
"custom/air-gestures": {
    "exec": "air-gestures status --waybar --follow",
    "return-type": "json",
    "on-click": "air-gestures toggle-connection",
    "on-click-right": "air-gestures toggle",
    "on-click-middle": "air-gestures dial cycle"
}
```

Gestures and the dial are configured in `~/.config/air-gestures/config.toml` (reloaded on save).
`AIR_GESTURES_LOG=air-gesturesd=debug` in the service logs every gesture and frame.

## Moving the band between devices

The band trusts one owner key; to use it from another computer or the
[Android app](https://gitlab.com/896kb/air-gestures-android), move that key instead of claiming again:

1. On the device that has it: `air-gestures export` writes `air-gestures-band.json` (the key
   and the band's address); in the Android app, **Setup → Export**.
2. On the other: `air-gestures import air-gestures-band.json`, or **Setup → Owner key →
   Import** in the app. The daemon reconnects with the new key.

The band talks to one device at a time: `air-gestures disconnect` on the one you're leaving.
Keep the file private and delete it once imported: anyone holding it can control the band.

After a band is **claimed** on another device, it drops its Bluetooth pairing with this
computer, so the imported key alone doesn't connect: remove the old pairing
(`bluetoothctl remove <address>`), put the band in pairing mode (hold 3 s) and pair it
again (`bluetoothctl`, then `pair`); the daemon then connects with the imported key.
Moving a key between devices that already paired with the band needs none of this.

## Build and test

Rust ≥ 1.88.

```sh
cargo test --workspace
cargo run -p air-gesturesd -- --dry-run      # the whole daemon against a simulated band
```

All tests run offline against the simulated band.

## License

MIT — see [`LICENSE`](LICENSE). Derived from kinesis; its notice is kept in
[`LICENSE-kinesis`](LICENSE-kinesis).
