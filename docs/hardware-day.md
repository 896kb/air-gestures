# Hardware day

The first real claim of a Meta Neural Band by kinesis-linux. Go top to bottom;
stop at any ✋ and decide together. Nothing irreversible happens before §3.

## 0. Before touching the band
- Phone: close the Meta AI app and turn the phone's Bluetooth off. Glasses off.
- `bluetoothctl show` → `Powered: yes`, `Pairable: yes`.
- `systemctl --user is-active kinesisd` → active; `kinesisctl status` → no band paired.
- In a second terminal keep `journalctl --user -fu kinesisd` running, and optionally `sudo btmon` (the user runs it).

## 1. Sign in to Meta first (the band is untouched)
Sign-in is the step most likely to need retries, so do it before the band is reset.
- `kinesisctl signin` → Meta's page opens in a window titled with its real host (auth.meta.com). Sign in with email + password there (avoid "Continue with Facebook/Instagram" pop-ups). The password goes only to Meta's page.
- A blank/white window on this hybrid-GPU setup → close it and retry with `WEBKIT_DISABLE_DMABUF_RENDERER=1 kinesisctl signin` (or `WEBKIT_DISABLE_COMPOSITING_MODE=1`).
- If Meta answers with an error, note the code/subcode. Meta may email a "new login" notice or ask for a checkpoint inside the page — finish it there.
- A note "ignored a sign-in redirect that didn't match" is harmless; keep going in the window.
- Check: `stat -c '%a %n' ~/.local/state/kinesis ~/.local/state/kinesis/meta-session.json` → 700 / 600; `jq 'del(.access_token)' ~/.local/state/kinesis/meta-session.json` → universe "ar", numeric user_id (never print the token).

## 2. See the band (no state change)
- Put the band in pairing mode (hold the button 3 s until it flashes).
- `systemctl --user stop kinesisd; kinesisd --scan; systemctl --user start kinesisd` → a line `<address>  <name>  rssi N` with a name starting "Meta Band".
- `bluetoothctl info <address>` → note `AddressType`. random with a first octet 0x40–0x7F = resolvable private address → check in §5 that band.json ends up with the identity address.

## 3. ✋ Factory reset (the band leaves the glasses)
- Hold the button ~16 s. The band forgets its owner (the glasses/phone lose it until re-setup in the Meta AI app).
- Put it back in pairing mode (3 s, flashing).

## 4. ✋ Claim
- `kinesisctl pair` (signed in already, so no window should open; if it asks to sign in again, do it, then put the band back in pairing mode and press Enter when prompted).
- Watch the stages, roughly: looking for a band in pairing mode → preparing Bluetooth pairing → connecting to <name> → reading the band identity → claiming the band → asking Meta to approve the claim → confirming ownership → confirming the claim with Meta → the band accepted this computer as its owner → establishing trust → connected as the owner → connecting as the new owner → "Paired with <name>".
- "claimed <name>; it hasn't connected yet" is a success; kinesisd keeps retrying.
- If it fails: read `kinesisctl status` (`pairing failed: …`) and the journal before retrying. Never delete `~/.local/state/kinesis/owner*.key` by hand.

## 5. Check the claim
- `ls -l ~/.local/state/kinesis` → `owner.key` (0600, 97 bytes), `band.json`; no `owner.pending.key`.
- `cat ~/.local/state/kinesis/band.json` → the address matches `bluetoothctl info`'s Address (identity address).
- `bluetoothctl info <address>` → Paired: yes, Bonded: yes, Trusted: yes.
- waybar: green hand + battery.

## 6. Gestures
- Thumb swipe left/right → workspace changes. Up → wofi. Down → Escape.
- Index double tap → play/pause. Middle double tap → mute toggle (OSD).
- Pinch thumb+index, hold ~0.2 s, turn the wrist → volume steps with OSD. `kinesisctl dial brightness` → brightness. Left wrist: `kinesisctl hand left`, confirm the dial direction.
- `kinesisctl semg view` → 8 channels move when you tense the forearm; `q` quits.

## 7. Robustness
- Suspend (lid / Fn+F11) and wake → reconnects on its own (`kinesisctl status`).
- Walk out of range and back → reconnects.
- `sudo systemctl restart bluetooth` (the user runs it), or reboot → reconnects by the stored address.
- Log out of Hyprland and back in → the exec-once restarts kinesisd with the new session; gestures still act.
- If the L2CAP channel is refused anywhere: note the journal line; the fallback is a Medium security level on the socket (kinesisd bluez.rs note).

## 8. Afterwards
- Record findings (address type, pairing method in btmon, anything odd) below under "Findings".
- Once everything works: push kinesis-linux master and the arch_rice branch (the user decides).

## Findings
_(fill in on the day)_
