# Hardware day

The first real claim of a Meta Neural Band by air-gestures. Go top to bottom;
stop at any ✋ and decide together. Nothing irreversible happens before §3.

## 0. Before touching the band
- Phone: close the Meta AI app and turn the phone's Bluetooth off. Glasses off.
- `bluetoothctl show` → `Powered: yes`, `Pairable: yes`.
- `systemctl --user is-active air-gesturesd` → active; `air-gestures status` → no band paired.
- In a second terminal keep `journalctl --user -fu air-gesturesd` running, and optionally `sudo btmon` (the user runs it).

## 1. Sign in to Meta first (the band is untouched)
Sign-in is the step most likely to need retries, so do it before the band is reset.
- `air-gestures signin` → Meta's page opens in a window titled with its real host (auth.meta.com). Sign in with email + password there (avoid "Continue with Facebook/Instagram" pop-ups). The password goes only to Meta's page.
- A blank/white window on this hybrid-GPU setup → close it and retry with `WEBKIT_DISABLE_DMABUF_RENDERER=1 air-gestures signin` (or `WEBKIT_DISABLE_COMPOSITING_MODE=1`).
- If Meta answers with an error, note the code/subcode. Meta may email a "new login" notice or ask for a checkpoint inside the page — finish it there.
- A note "ignored a sign-in redirect that didn't match" is harmless; keep going in the window.
- Check: `stat -c '%a %n' ~/.local/state/air-gestures ~/.local/state/air-gestures/meta-session.json` → 700 / 600; `jq 'del(.access_token)' ~/.local/state/air-gestures/meta-session.json` → universe "ar", numeric user_id (never print the token).

## 2. See the band (no state change)
- Put the band in pairing mode (hold the button 3 s until it flashes).
- `systemctl --user stop air-gesturesd; air-gesturesd --scan; systemctl --user start air-gesturesd` → a line `<address>  <name>  rssi N` with a name starting "Meta Band".
- `bluetoothctl info <address>` → note `AddressType`. random with a first octet 0x40–0x7F = resolvable private address → check in §5 that band.json ends up with the identity address.

## 3. ✋ Factory reset (the band leaves the glasses)
- Hold the button ~16 s. The band forgets its owner (the glasses/phone lose it until re-setup in the Meta AI app).
- Put it back in pairing mode (3 s, flashing).

## 4. ✋ Claim
- `air-gestures pair` (signed in already, so no window should open; if it asks to sign in again, do it, then put the band back in pairing mode and press Enter when prompted).
- Watch the stages, roughly: looking for a band in pairing mode → preparing Bluetooth pairing → connecting to <name> → reading the band identity → claiming the band → asking Meta to approve the claim → confirming ownership → confirming the claim with Meta → the band accepted this computer as its owner → establishing trust → connected as the owner → connecting as the new owner → "Paired with <name>".
- "claimed <name>; it hasn't connected yet" is a success; air-gesturesd keeps retrying.
- If it fails: read `air-gestures status` (`pairing failed: …`) and the journal before retrying. Never delete `~/.local/state/air-gestures/owner*.key` by hand.

## 5. Check the claim
- `ls -l ~/.local/state/air-gestures` → `owner.key` (0600, 97 bytes), `band.json`; no `owner.pending.key`.
- `cat ~/.local/state/air-gestures/band.json` → the address matches `bluetoothctl info`'s Address (identity address).
- `bluetoothctl info <address>` → Paired: yes, Bonded: yes, Trusted: yes.
- waybar: green hand + battery.

## 6. Gestures
- Thumb swipe left/right → workspace changes. Up → wofi. Down → Escape.
- Index double tap → play/pause. Middle double tap → mute toggle (OSD).
- Pinch thumb+index, hold ~0.2 s, turn the wrist → volume steps with OSD. `air-gestures dial brightness` → brightness. Left wrist: `air-gestures hand left`, confirm the dial direction.
- `air-gestures semg view` → 8 channels move when you tense the forearm; `q` quits.

## 7. Robustness
- Suspend (lid / Fn+F11) and wake → reconnects on its own (`air-gestures status`).
- Walk out of range and back → reconnects.
- `sudo systemctl restart bluetooth` (the user runs it), or reboot → reconnects by the stored address.
- Log out of Hyprland and back in → the exec-once restarts air-gesturesd with the new session; gestures still act.
- If the L2CAP channel is refused anywhere: note the journal line; the fallback is a Medium security level on the socket (air-gesturesd bluez.rs note).

## 8. Afterwards
- Record findings (address type, pairing method in btmon, anything odd) below under "Findings".
- Once everything works: push air-gestures master and the arch_rice branch (the user decides).

## Findings

2026-09-25, Meta Band 00DF: claimed and connected; gestures drive Hyprland.

- **Address:** advertises a resolvable private address (a new one after each bond removal); a fixed identity address (public type) once bonded. Removing a stale bond also drops the band's IRK, so `prepare_pairing` finds the band again by name (only a device heard in that scan: BlueZ's cached entry for an old private address matches by name but can't be connected).
- **Encryption scheme 26:** the band offers 27 and names 26 in EnableEncryption whatever we offer (older firmware used 3; the glasses use 31). Scheme 26 = 31's key derivation (HKDF over the raw ECDH secret, salt SHA256(C‖R)) with the encryption key reused as MAC key and 31's `02 02 00 00` MAC prefix — `SCHEME_26_GUESSES[0]`, stored in band.json as `scheme_guess`.
- **Padding:** records are padded by 1..=16 bytes; a block-aligned frame gets a whole block of 0xd0 (the 1888-byte identity reply arrived as 1904 bytes). We pad the same way under scheme 26.
- **Channels:** flagged channels the band answers or opens carry extra bits that change per connection (0x9c01, 0x9801, 0x9501 for 0x8001; same for 0x8002). Frames are routed on `channel & 0x80ff`.
- **Meta:** the final pair reply has no `device_ec_public_key`, so owner.key is 32 bytes and the band's trust proof isn't verified against a key.
- **Stream:** gyro (0200020f), orientation (02000212) and gestures (0200020d) on channel 5; battery on 8; gesture field layout unchanged.

## 2026-10-22: firmware 1054928927 — handwriting / text pipeline

Firmware **1054928927** (Meta AI app: "latest"). Pairing, encryption (scheme 26,
`scheme_guess: 0`), padding, channel bits and the gesture/dial/battery/motion
streams are all unchanged from September. Everything below is new.

### What the probe was doing wrong (two bugs, both fixed on `firmware-explore`)

1. **Inference frames never reached the decoder.** `RAW_INFERENCE`
   (`0x0200020c`) was handled *after* the "ignore unrelated services" allowlist,
   which returns early for any unlisted kind — so every inference frame was
   dropped before decode and the handler was dead code. The probe logged the
   frame headers (`frame ch 0x5 words [0200020c] 48B`) but not one sample.
   Fix: add it to the allowlist.
2. **The band was never put into data collection.** The probe wrote
   `ConfigReq.deviceState = 4` (WakeState HANDWRITING_CC) and
   `emgSensorState = 4` only. The config reply showed `mode = 4`
   (`MODE_DATA_COLLECTION_NONE`) and `dataCollectionModeEnabled = 0`, and the
   band declined stream 43 (`enableInferenceText`). Fix: also write
   `mode = 5` (`MODE_DATA_COLLECTION_HANDWRITING_CC`) and
   `dataCollectionModeEnabled = 1`. The band then enabled stream 43 and started
   sending the text pipeline.

### Observed protocol facts

- **`RawInferenceSample` = `0x0200020c`**, fields `1` seq, `2` timestamp,
  `3` channels, `10` `pipeline_type`. Arrives ~50 Hz on channel 5.
- `PipelineType` (`config_common.proto`): `1` wake, `2` active, `3` text,
  `10` cascadeCcClassifier, `13` cascadePcClassifier.
- **`active` (2)** carries **4 logits** (16 B of f32 in a ~36–50 B frame), not
  the 9 the startup `inferenceConfig.numLogits` reports.
- **`text` (3)** carries **400 B = 100 little-endian f32**, and it is a
  **log-softmax**: `sum(exp(x)) = 1.000` over all 100 values, verified on every
  captured frame. One probability distribution per frame.
- **Class 99 = "no ink"** (`p = 1.000` whenever nothing is happening).
- **Class 87 = separator** between strokes/letters (`p` 0.6–0.95).
- Landed on ~70–75 % of all frames being class 99 in an idle session.
- `inferenceConfig` in the config reply changes with state:
  before the write `pipeline=wake downsampleWindow=0 modelStride=72 numLogits=3`,
  after `pipeline=active modelStride=32 numLogits=9`. So `HANDWRITING_CC` does
  move the band onto a different pipeline.
- The band sends **no** other stream types in this mode: with the probe naming
  unhandled frames, zero appeared. In particular no `DetectorUpdate`
  (on-body / band-tightness) is sent, so the model's input quality can't be
  read that way.
- `StreamControlResp` flags seen enabled: `[3, 4, 6, 8, 29]` — gestures, raw
  inference, gyro, quat, band-orientation-result.

### Turning classes into text — NOT solved

Repeated labelled captures were taken (writing on a surface with the index
finger). Results, honestly:

- Writing **A, B, C** in one run gave a **reproducible signature three times**:
  `73, 1, 8, 2` with 87 separators between, and class `53` before it. Peaks:
  73≈0.55–0.91, 1≈0.75–0.90, 8≈0.94–0.98, 2≈0.98–0.99.
- Writing **only "A"** three times produced **class 0** (p 0.83, 0.59), *not*
  class 8.

**So the classes are context-dependent and are not a per-letter alphabet.** The
tempting mapping (8=A, 2=B, 1=C) is **wrong** and was retracted. The likeliest
reading is that these are sequence-position / pooled stroke tokens, i.e. a
**CTC-style sequence decode over the band's real vocabulary is required**, not a
lookup table. That vocabulary is **not in the decoded protos** — it lives in
Meta's model asset.

Also unresolved: **acquisition is erratic**. Several capture windows returned
`p = 1.000` blank for every frame (hundreds of frames) while gyro streamed
normally at ~128 Hz and the config still read `mode=5, enabled=1`. Not
root-caused. The hand-readback/writing position at the time correlates with
success but wasn't isolated.

### Reproducing

Opt-in env vars on the user service (systemd drop-in in
`~/.config/systemd/user/air-gesturesd.service.d/`):

```
AIR_GESTURES_LOG=air_gesturesd=debug,band_core=debug,info
AIR_GESTURES_HANDWRITING=1
AIR_GESTURES_EXTRA_STREAMS=29,4,43
```

Text-pipeline frames log all 400 channel bytes (16 B head for other pipelines).
To decode offline, parse the hex, `<100f` unpack, `exp()` each value; drop 99,
treat 87 as a separator.

### Code state

- `firmware-explore` (local, **not pushed**): the probe. 5 commits.
- `daemon-reconnects-after-adapter-loss` (local, **not pushed**): a real bug fix
  independent of all this — `BluezTransport` cached the `Adapter` from startup,
  so when BlueZ dropped the adapter object (suspend/adapter reset) every call
  failed `NotFound` and was misreported as "Bluetooth is unavailable"; the
  daemon then retried the dead handle until restarted. Now re-resolves once.
  Verified surviving a suspend/resume with the band asleep (same PID reconnected
  8 s after resume). The adapter-object-gone branch itself is unit-tested only.

### The band persists ConfigReq writes (battery trap)

`deviceState`, `emgSensorState`, `mode` and `dataCollectionModeEnabled` are
**persisted on the band**. Reverting the daemon does not undo them:
`BandSession::stop()` disables the streams but does not restore the config, so
after the handwriting probe the band stayed in `HANDWRITING_CC` with data
collection on and kept sending the inference pipeline (~50 Hz, 400 B frames).

`BandSession::write_config` (added for this) restores it:
`emgSensorState=2, deviceState=2` (ACTIVE), `mode=0` (MODE_DEFAULT),
`dataCollectionModeEnabled=0`. Verified: afterwards the journal shows no
`0200020c` frames at all, only gyro `0200020f` and orientation `02000212`.
Helper: `cargo run --release -p air-gestures --example band-reset-state`, run
with the daemon stopped.
