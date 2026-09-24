# Hardware day

The first real claim of a Meta Neural Band by kinesis-linux. Go top to bottom;
stop at any ✋ and decide together.

## 0. Before touching the band
- Phone: close the Meta AI app and turn the phone's Bluetooth off. Glasses off.
- `bluetoothctl show` → `Powered: yes`, `Pairable: yes`.
- `systemctl --user is-active kinesisd` → active; `kinesisctl status` → no band paired.
- In a second terminal keep `journalctl --user -fu kinesisd` running, and optionally `sudo btmon` (the user runs it).

## 1. See the band (no state change)
- Put the band in pairing mode (hold the button 3 s until it flashes).
- `kinesisd --scan` (stop the service first if the adapter is busy: `systemctl --user stop kinesisd`) → the band appears as `Meta Band …` with an RSSI.
- `bluetoothctl info <address>` → note `AddressType`. random with a first octet 0x40–0x7F = resolvable private address → watch that band.json ends up with the identity address after the claim.

## 2. ✋ Factory reset (the band leaves the glasses)
- Hold the button ~16 s. The band forgets its owner (the glasses/phone lose it until re-setup in the Meta AI app).
- Put it back in pairing mode (3 s, flashing).

## 3. ✋ Claim
- `systemctl --user start kinesisd` (if stopped) then `kinesisctl pair`.
- The Meta sign-in window opens on auth.meta.com — sign in there (the password goes only to Meta's page).
- Watch the stages: looking for a band → preparing Bluetooth pairing → connecting → reading the band identity → asking Meta to approve the claim → confirming the claim with Meta → the band accepted this computer as its owner → connected as the new owner.
- If it fails: read `kinesisctl status` (`pairing_error`) and the journal before retrying. Never delete `~/.local/state/kinesis/owner*.key` by hand.
- Sign-in window tips: sign in with email + password on Meta's page (avoid "Continue with Facebook/Instagram" pop-ups). A blank/white window on this hybrid-GPU setup → close it and retry with `WEBKIT_DISABLE_DMABUF_RENDERER=1 kinesisctl pair` (or `WEBKIT_DISABLE_COMPOSITING_MODE=1`). If Meta answers with an error, note the code/subcode; Meta may email a "new login" notice or ask for a checkpoint in the page.
- After sign-in: `stat -c '%a %n' ~/.local/state/kinesis ~/.local/state/kinesis/meta-session.json` → 700 / 600; `jq 'del(.access_token)' ~/.local/state/kinesis/meta-session.json` → universe "ar", numeric user_id (never print the token).

## 4. Check the claim
- `ls -l ~/.local/state/kinesis` → `owner.key` (0600, 97 bytes), `band.json`; no `owner.pending.key`.
- `cat ~/.local/state/kinesis/band.json` → the address matches `bluetoothctl info`'s Address (identity address).
- `bluetoothctl info <address>` → Paired: yes, Bonded: yes, Trusted: yes.
- waybar: green hand + battery.

## 5. Gestures
- Thumb swipe left/right → workspace changes. Up → wofi. Down → Escape.
- Index double tap → play/pause. Middle double tap → mute toggle (OSD).
- Pinch thumb+index, hold ~0.2 s, turn the wrist → volume steps with OSD. `kinesisctl dial brightness` → brightness. Left wrist: `kinesisctl hand left`, confirm the dial direction.
- `kinesisctl semg view` → 8 channels move when you tense the forearm; `q` quits.

## 6. Robustness
- Suspend (lid / Fn+F11) and wake → reconnects on its own (`kinesisctl status`).
- Walk out of range and back → reconnects.
- `systemctl --user restart bluetooth` (or reboot) → reconnects by the stored address.
- If the L2CAP channel is refused anywhere: note the journal line; the fallback is a Medium security level on the socket (kinesisd bluez.rs note).

## 7. Afterwards
- Record findings (address type, pairing method in btmon, anything odd) in `docs/hardware-day.md` under "Findings".
- Once everything works: push kinesis-linux master and the arch_rice branch (the user decides).
