# air-gestures

Use a **Meta Neural Band** (the sEMG wristband) to control a Linux desktop: thumb swipes, index
and middle finger taps, a middle-finger hold and pinch + turn your wrist each run a shell command
you choose (workspaces, media keys, volume, brightness, …). Includes a waybar module and a raw
sEMG viewer/recorder. Written in Rust, for Linux and BlueZ.

> **Status: experimental, working.** Tested on one band (September 2026 firmware) with Arch
> Linux, BlueZ and Hyprland: claim, reconnect, gestures, pinch dial, battery. That firmware
> differs from earlier ones (encryption scheme 26, 16-byte padding, tagged
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

Design: [the original spec](docs/superpowers/specs/2026-09-24-kinesis-linux-design.md).

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

`band-core` is a git submodule ([band-core](https://gitlab.com/896kb/band-core), shared with the Android app): clone with `git clone --recursive`, or run `git submodule update --init` after a plain clone.

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

## Configure

Everything is in `~/.config/air-gestures/config.toml` (reloaded on save); every key is in
[`contrib/config.toml`](contrib/config.toml). A gesture runs a shell command; `""` does nothing.
The daemon logs config problems as errors, and every command a band control pauses as a warning.

### Gestures and the pinch

24 gestures: `swipe_left|right|up|down`, and per finger (`index_`, `middle_`) `tap`, `double`,
`triple`, `hold` and six drags, `drag_left|right|toward|away|up|down`.

A pinch (thumb + finger) is one of three things, decided once you've moved 15°:
- let go without moving: the finger's **hold**;
- turn your wrist: the finger's **dial**, `index_dial` / `middle_dial` = `"volume"`,
  `"brightness"` or `"off"` (index: `[dial] target`; middle: off);
- move your hand: a **drag**. With `index_drags = "gestures"` (the default) each drag runs its
  `index_drag_*` command once; with `"sliders"`, the pair you start along (left/right,
  toward/away, up/down) moves `index_slider_left_right|toward_away|up_down` (`"volume"`,
  `"brightness"`, `"off"`, or a command: see below) for as long as you hold the pinch.

When a finger's dial and drags do nothing, its hold acts as soon as the band reports it
(the middle hold, by default); otherwise when you let go. `air-gestures dial
volume|brightness|off|cycle` changes the index dial until the profile changes.

**Teach your drags:** drags and sliders work best taught to your own hand.
`air-gestures teach-drags` takes one pinch per move (the turn, then the six drags) and keeps
them for this band and hand; `--reset` forgets them (with the band connected: it's for the hand
the band is on).

**Slider commands:** any slider value other than `"volume"`, `"brightness"`, `"off"` and `""`
is a shell command. While you slide, its steps are gathered: the first runs it at once, then it
runs at most every 250 ms with the steps since, and once more when you let go. It gets
`AIR_GESTURES_SLIDER` (`index_left_right`, `middle_up_down`, …), `AIR_GESTURES_STEPS` (the
net steps, `+3` or `-2`) and `AIR_GESTURES_PROFILE`. One script can serve every slider, e.g.
Home Assistant lights through its REST API:

```toml
[[profile]]
name = "Living room"
index_drags = "sliders"
index_slider_left_right = "~/bin/lights"
index_slider_up_down = "~/bin/lights"
```

```sh
#!/bin/sh
# ~/bin/lights: one script for every slider
light() {  # light <entity> <brightness change in %>
  curl -s -X POST http://homeassistant.local:8123/api/services/light/turn_on \
    -H "Authorization: Bearer $(cat ~/.config/hass-token)" \
    -H "Content-Type: application/json" \
    -d "{\"entity_id\": \"$1\", \"brightness_step_pct\": $2}"
}
case "$AIR_GESTURES_PROFILE:$AIR_GESTURES_SLIDER" in
  "Living room:index_left_right") light light.sofa_lamp $((AIR_GESTURES_STEPS * 5)) ;;
  "Living room:index_up_down")    light light.ceiling $((AIR_GESTURES_STEPS * 5)) ;;
esac
```

### Band controls

Four controls sit above every profile, each on one gesture you pick in `[band]`:
`toggle` (controls off/on; works while they're off), `next_profile`, `previous_profile` and
`draw`. Each is any gesture key or `"none"` (the default). `<key>_palm = ["up"]` limits one to
one or two palm positions; in the others the gesture does what the profile says.

A profile's command on a band-control gesture is kept but doesn't run ("paused"); the daemon
logs each one, e.g. `middle_double in profile "media" is overridden by [band] next_profile`,
and it runs again once the band control moves. Two band controls on one gesture and position
is a config error. `toggle = "middle_hold"` from older configs keeps working.

### Profiles

`[gestures]` is the first profile (named `default`, or `[band] profile`); more go in
`[[profile]]` tables with a `name` and the same keys. A gesture a profile leaves out does
nothing there:

```toml
[band]
next_profile = "index_triple"

[[profile]]
name = "media"
index_tap = "playerctl play-pause"
index_dial = "volume"
```

Switch with the band controls or `air-gestures profile next|previous|<name>`; the current one
shows in `air-gestures status` and the waybar tooltip. `on_profile` runs on every switch with
`AIR_GESTURES_PROFILE` set.

### Palm position

Any gesture can do something else with the palm up, sideways or down: write it in
`[gestures.palm_up]`, `[gestures.sideways]` or `[gestures.palm_down]` (in a profile:
`[profile.palm_up]` …), at most two positions per gesture; the normal command covers the rest.
`""` there means nothing in that position.

```toml
[gestures.palm_up]
index_tap = "playerctl next"
```

The right hand works out of the box; `air-gestures calibrate` (palm down, then palm up, 2 s
each) fits it to your wrist, and is needed for the left hand (until then every gesture counts
as palm down). It's kept per band and hand; `--reset` forgets it (with the band connected: it's
for the hand the band is on).

### Drawings

Put Draw on a gesture (`[band] draw = "swipe_up"`), make it, and draw with a pinch as the pen,
in the air or pen grip on a table; the drawing ends 1 s after your last stroke. Teach drawings
first, three tries each: `air-gestures draw teach lightbulb` (`draw list`, `draw delete
<name>`). Then give them actions:

```toml
[drawings]                 # every profile
lightbulb = "hass-cli light toggle"

[[profile]]
name = "media"
[profile.drawings]         # this profile's own, wins over [drawings]
lightbulb = "playerctl play-pause"
```

`on_draw` runs after each drawing with `AIR_GESTURES_DRAWING` set to its name (empty when
nothing matched); by default it shows a notification. Templates live in
`~/.local/share/air-gestures/drawings/`.

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

MIT — see [`LICENSE`](LICENSE). Parts of the protocol core started from
[callbacked/kinesis](https://github.com/callbacked/kinesis) (MIT); its notice is kept in
[`LICENSE-kinesis`](LICENSE-kinesis).
