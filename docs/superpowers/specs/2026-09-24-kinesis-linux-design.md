# kinesis-linux — design

Date: 2026-09-24
Status: approved in brainstorming, awaiting spec review

## Goal

Use a Meta Neural Band to control Hyprland on the ASUS ROG Flow Z13 (ACRNM,
Arch Linux), with the features of [callbacked/kinesis](https://github.com/callbacked/kinesis)
(macOS, MIT, reference commit `bb4b2c9`): recognized swipes/taps trigger actions,
pinch + wrist-turn drives a volume/brightness dial, the band is claimed by this
laptop, battery/charging is shown, and raw sEMG can be viewed and recorded.

## Decisions

- **Ownership: the laptop claims the band** (kinesis's ownership ceremony). The
  band stops working with the user's Meta Ray-Ban Display glasses until it is
  factory reset and re-set-up in the Meta AI app. Accepted by the user.
- **Language: Rust.** Ported from kinesis's Swift (MIT). `neural-band-poc` has
  no license and is used as reading reference only — no code copied from it.
- **Features in scope:** gestures → Hyprland actions, pinch dial, waybar widget,
  remappable TOML config, raw sEMG view/record, CLI pairing.
- **Out of scope:** GUI / 3D hand / practice screen, Xiaomi Book Pro host,
  switching the band back to glasses, multiple bands, importing a phone-extracted
  identity key.

## Architecture

Standalone repo `~/Projects/kinesis-linux`, Cargo workspace with three crates.
`~/arch_rice` holds only config, waybar module, systemd unit and an install step.

| crate | kind | responsibility | main deps |
|---|---|---|---|
| `band-core` | lib, no I/O | sans-IO protocol: AirShield crypto (P-256 ECDH, HKDF, HMAC, AES-CBC, MAC-before-plaintext), datax framing + protobuf fields, session state machine (handshake, EnableTrust, subscription, handedness, battery, shutdown), gesture decoding + dedup, pinch dial, ownership-ceremony state machine, sEMG batch parsing | `p256`, `ecdsa`, `hkdf`, `hmac`, `sha2`, `aes`, `cbc` |
| `kinesisd` | bin, tokio | BlueZ transport via `bluer` (discovery on service `0xFD5F`, GATT battery + PSM characteristic, LE L2CAP CoC on PSM 255), reconnect/heartbeat, action runner, config load + hot reload, control socket, scoped pairing agent, ceremony HTTP calls | `band-core`, `bluer`, `tokio`, `toml`, `serde`, `reqwest` |
| `kinesisctl` | bin | CLI over the control socket; Meta sign-in window (writes `meta-session.json`); sEMG TUI | `wry`/`tao` (WebKitGTK 4.1), `ratatui`, `reqwest` |

`band-core` exposes an interface of the form: feed inbound bytes / timer ticks →
receive outbound bytes and typed events. It never touches sockets, clocks, or files
(time is passed in as a monotonic instant).

### Data flow

```
band ──L2CAP──▶ kinesisd transport ──bytes──▶ band-core session
                                                 │ events: Swipe(dir), Tap(finger),
                                                 │ DoubleTap(finger), Hold(finger),
                                                 │ Dial(steps), Battery{pct,charging},
                                                 │ Hand(l|r), Emg(batch), Link(state)
                                                 ▼
                    config lookup ──▶ action runner (sh -c, detached, 5 s timeout)
                                                 │
                    control socket ◀─────────────┘ (status + event stream, JSON lines)
                        ▲            ▲
             waybar (kinesisctl   kinesisctl CLI / semg view
             status --follow)
```

Control socket: `$XDG_RUNTIME_DIR/kinesis.sock`, newline-delimited JSON
requests/responses plus a subscribe mode that streams status and events.

## Pairing, sign-in, secrets

`kinesisctl pair` (kinesisctl runs only the sign-in window and writes the session file; kinesisd does all Bluetooth work and the ceremony's Meta HTTP calls using that session):

1. Remove any stale BlueZ bond for the band (a factory reset rotates the IRK and a
   stale bond breaks pairing).
2. Scan for a band in pairing mode; pick it (prompt if several).
3. Register a BlueZ agent that auto-accepts Just Works pairing **only for that
   address and only for the duration of `pair`**.
4. If no stored Meta session: open a WebKitGTK window running kinesis's
   `MetaAuth` sequence — anonymous tokens query → `auth.meta.com` sign-in →
   callback token check (first 16 hex of SHA-256, constant-time compare) →
   blob decrypt → `ar` session login. The password is only typed into Meta's page.
5. Ownership ceremony (kinesis `OwnershipCeremony` + `MetaPairClient`): generate
   a P-256 owner key; identity read `0x3000→0x3001`; nonce `0x2000→0x2001`;
   `pair_request` to `graph.facebook-hardware.com`; `StartChangeOwner`
   `0x2002→0x2003`; `pair`; `FinishChangeOwner` `0x2004→0x2005`; store the band
   public key from `additional_data.device_ec_public_key`. Receipt strings are
   kept byte-for-byte.
6. Reconnect in enrolled mode (EnableTrust both directions, band signature
   verified against the stored band key), read handedness. Done.

Subsequent connects use only the stored owner key; no Meta traffic.

**Secrets** live in `~/.local/state/kinesis/` (dir `0700`, files `0600`), never in
`~/arch_rice` and never in logs: `owner.key` (raw 32-byte P-256 scalar),
`band.pub` (65-byte X9.63), `meta-session.json`, `band.json` (address, name).
No keyring daemon is running on this machine, so libsecret is not used.
`kinesisctl forget` deletes these, removes the BlueZ bond, and prints the
factory-reset reminder.

**Errors** (codes as in kinesis `enrollment.md`): `0x1043` wrong key and
`0x1042` wrong account get explicit messages and offer re-sign-in with another
account; HTTP 401/403 or graph error 190 drops the session and re-prompts
sign-in; any ceremony failure restarts from the identity read (no mid-ceremony
resume; band must stay in pairing mode). `pair --dry-run` runs the whole flow
against the simulated band (Meta HTTP calls stubbed).

## Gestures, actions, config

Config: `~/arch_rice/.config/kinesis/config.toml` (symlinked to
`~/.config/kinesis/`), hot-reloaded on change. Invalid config → keep the previous
one, log the error, show it in the waybar tooltip.

```toml
[band]
hand = "band"            # "band" (read from band) | "left" | "right"
autostart = true         # connect + enable controls when the service starts

[gestures]               # shell commands run with sh -c; "" = unassigned
swipe_left    = "hyprctl dispatch workspace e-1"
swipe_right   = "hyprctl dispatch workspace e+1"
swipe_up      = "wofi --show drun"
swipe_down    = "hyprctl dispatch sendshortcut , Escape, activewindow"
index_double  = "playerctl play-pause"
middle_double = "~/.local/bin/audio-mute sink"
index_tap     = ""
middle_tap    = ""
middle_hold   = ""

[dial]                   # pinch thumb+index, then turn the wrist
target = "volume"        # "volume" | "brightness" | "off"
sensitivity = 1.0        # 1.0 = 2° relative wrist rotation per step
```

- Actions run detached with a 5 s timeout; they never block the transport.
- Built-in dial targets: volume = `wpctl set-volume -l 1.0 @DEFAULT_AUDIO_SINK@ 2%±`
  then `~/.local/bin/osd vol "󰕾  N%" N`; brightness = `brightnessctl set 2%±` then
  `osd bri …`.
- Dial behaviour (kinesis `PinchDial`): arms after ~200 ms of held index pinch;
  ≤ 1 step per 80 ms, excess flick steps discarded; release / reversal / stale
  motion clears pending movement; direction reversed for a *confirmed* left hand;
  waits for a confirmed hand setting.
- Input hygiene: duplicate and stale gesture messages dropped; reconnects never
  replay; all gates use the monotonic clock.
- Handedness: `kinesisctl hand left|right` pauses controls, writes `ConfigReq`
  field 10 on channel `0x8006`, then re-reads; confirmed only on match.
- Pause: keeps the link, suppresses all actions.

## Waybar, service, sEMG

**Waybar** `custom/kinesis`, added to `~/arch_rice/.config/waybar/profile.d/z13.jsonc`
only. `exec: kinesisctl status --waybar --follow` (one JSON line per change, no
polling). Glyph + battery %, bolt when charging. Colours: green `#b5bd68`
connected & active, `#ffff78` paused, `#555555` disconnected / daemon down, amber
`#f0c674` battery < 20%. Tooltip: hand, dial target, last gesture → action,
gesture count, config error if any. Click = connect/disconnect, right-click =
pause/resume, middle-click = cycle dial volume ↔ brightness. `osd kinesis …`
popups only on connect, disconnect, low battery.

**Service** `~/arch_rice/.config/systemd/user/kinesisd.service`:
`ExecStart=%h/.cargo/bin/kinesisd`, `Restart=on-failure`,
`WantedBy=graphical-session.target`. Reconnect backoff 1 s → 30 s. Heartbeat:
after 2 s without authenticated traffic, read subscription status; only an
authenticated reply refreshes it; otherwise time out and reconnect (covers s2idle
resume and out-of-range). Lock file `$XDG_RUNTIME_DIR/kinesisd.lock`. Logs to
journald. `install.sh` (Z13 profile) runs
`cargo install --path ~/Projects/kinesis-linux/{kinesisd,kinesisctl}` and enables
the unit.

**sEMG**: `kinesisctl semg view` (ratatui: 8 channels, received vs configured
2048 Hz rate, dropped batches) and `kinesisctl semg record FILE.jsonl` (raw
payloads, kinesis/POC-compatible JSONL). The daemon enables raw flag 2 only while
a viewer/recorder is attached and disables it on detach, waiting ≤ 3 s for the
ack. Config read on `0x8007`; supported config: 2048 Hz, 8 ch, 16-bit, 16
samples/batch, encoding 0; payload `0x0200020a`. Values are unscaled ADC counts.

**Battery**: GATT battery characteristic (read + notify) and `BatteryInfoReq` on
`0x8008` every 5 s while streaming; charging unknown unless reported.

## Testing

1. `band-core` unit tests ported from kinesis `KinesisCoreTests` (crypto vectors,
   fragmented frames, subscription flags, gesture decoding, dial, ownership
   ceremony, enrolled identity / EnableTrust).
2. Simulated encrypted band driving the real session over an in-memory transport
   (handshake, subscription, shutdown acks, heartbeat timeout, reconnect).
   Also backs `pair --dry-run`.
3. Optional replay: `KINESIS_CAPTURE=path cargo test` checks a recorded session.
4. Hardware day, only after 1–3 pass: factory reset → `kinesisctl pair` with logs
   watched → every gesture + dial → suspend/resume → out of range and back.

## Risks

- Meta's sign-in routes and app credentials are undocumented and may change;
  an already-claimed band keeps working without them.
- Protocol is reverse-engineered against specific firmware; newer firmware may
  differ.
- Ownership ceremony failures can require another factory reset — hence tests
  1–2 and the dry run before touching the real band.
