# kinesis-linux

Use a **Meta Neural Band** (the sEMG wristband) to control a Linux desktop — swipe between
Hyprland workspaces, play/pause, mute, and pinch + turn your wrist for volume or brightness.

A Rust port of [callbacked/kinesis](https://github.com/callbacked/kinesis) (macOS, MIT) for
Linux/BlueZ.

> **Status: work in progress.** Only the protocol library (`band-core`) exists so far. The
> daemon (`kinesisd`) and CLI (`kinesisctl`) are next. Nothing here talks to a real band yet.

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
| `kinesisctl` | *(planned)* pairing/sign-in, status for waybar, hand setting, sEMG viewer/recorder |

Design: [`docs/superpowers/specs/2026-09-24-kinesis-linux-design.md`](docs/superpowers/specs/2026-09-24-kinesis-linux-design.md).

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
