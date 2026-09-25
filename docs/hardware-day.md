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
- **Encryption scheme 26:** the band offers 27 and names 26 in EnableEncryption whatever we offer (kinesis only knew 3; the glasses use 31). Scheme 26 = 31's key derivation (HKDF over the raw ECDH secret, salt SHA256(C‖R)) with the encryption key reused as MAC key and 31's `02 02 00 00` MAC prefix — `SCHEME_26_GUESSES[0]`, stored in band.json as `scheme_guess`.
- **Padding:** records are padded by 1..=16 bytes; a block-aligned frame gets a whole block of 0xd0 (the 1888-byte identity reply arrived as 1904 bytes). We pad the same way under scheme 26.
- **Channels:** flagged channels the band answers or opens carry extra bits that change per connection (0x9c01, 0x9801, 0x9501 for 0x8001; same for 0x8002). Frames are routed on `channel & 0x80ff`.
- **Meta:** the final pair reply has no `device_ec_public_key` (kinesis notes the same), so owner.key is 32 bytes and the band's trust proof isn't verified against a key.
- **Stream:** gyro (0200020f), orientation (02000212) and gestures (0200020d) on channel 5; battery on 8; gesture field layout unchanged.
