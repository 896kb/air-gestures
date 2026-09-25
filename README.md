# kinesis-linux

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
| `kinesisd` | BlueZ daemon: gestures → configurable shell actions, pinch dial, ownership claim, control socket (`kinesisd --dry-run` runs it against a simulated band) |
| `kinesisctl` | Meta sign-in window, claiming (`pair`), status (plain or waybar JSON), pause/resume, dial target, hand, sEMG viewer/recorder |

Design: [`docs/superpowers/specs/2026-09-24-kinesis-linux-design.md`](docs/superpowers/specs/2026-09-24-kinesis-linux-design.md).

## Install

Needs Rust ≥ 1.88, BlueZ, and WebKitGTK 4.1 (`webkit2gtk-4.1` on Arch) for the sign-in window.

```sh
cargo install --locked --path kinesisd
cargo install --locked --path kinesisctl
mkdir -p ~/.config/systemd/user ~/.config/kinesis
cp contrib/kinesisd.service ~/.config/systemd/user/
cp contrib/config.toml ~/.config/kinesis/        # every key, commented
systemctl --user daemon-reload && systemctl --user enable --now kinesisd
```

## Claim your band

1. `kinesisctl signin` — Meta's sign-in page opens in a window; the password only goes to Meta.
2. Factory reset the band (hold its button ~16 s), then put it in pairing mode (hold 3 s until
   it flashes). Make sure `bluetoothctl show` says `Pairable: yes`.
3. `kinesisctl pair` — claims it; from then on kinesisd connects on its own.

`kinesisctl status` shows the link, battery, hand and dial target. For waybar:

```jsonc
"custom/kinesis": {
    "exec": "kinesisctl status --waybar --follow",
    "return-type": "json",
    "on-click": "kinesisctl toggle-connection",
    "on-click-right": "kinesisctl toggle",
    "on-click-middle": "kinesisctl dial cycle"
}
```

Gestures and the dial are configured in `~/.config/kinesis/config.toml` (reloaded on save).
`KINESIS_LOG=kinesisd=debug` in the service logs every gesture and frame.

## Build and test

Rust ≥ 1.88.

```sh
cargo test --workspace
cargo run -p kinesisd -- --dry-run      # the whole daemon against a simulated band
```

All tests run offline against the simulated band.

## License

MIT — see [`LICENSE`](LICENSE). Derived from kinesis; its notice is kept in
[`LICENSE-kinesis`](LICENSE-kinesis).
