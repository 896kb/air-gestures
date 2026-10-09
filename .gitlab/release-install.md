## Install

Linux daemon and CLI for the Meta Neural Band.

`air-gestures-{tag}-x86_64-linux.tar.gz` (x86_64) and
`air-gestures-{tag}-aarch64-linux.tar.gz` (64-bit ARM, tested under emulation only)
each contain the `air-gestures` and `air-gesturesd` binaries (glibc >= 2.36), a systemd user unit
(`air-gesturesd.service`, ExecStart=/usr/bin/air-gesturesd), the example
`config.toml`, README.md, LICENSE and LICENSE-kinesis.

Runtime needs: BlueZ, D-Bus and WebKitGTK 4.1 (`webkit2gtk-4.1` on Arch).

**Debian, Ubuntu and derivatives** (Debian 12+, Ubuntu 24.04+):
`sudo apt install ./air-gestures-{tag}-x86_64-linux.deb` (or the `aarch64` one).
**Fedora, openSUSE and other RPM distributions**:
`sudo dnf install ./air-gestures-{tag}-x86_64-linux.rpm` (or the `aarch64` one).
Both install the binaries to /usr/bin, the user unit to /usr/lib/systemd/user/ and the
example config to /usr/share/air-gestures/, and pull in the runtime libraries. Then:

    mkdir -p ~/.config/air-gestures
    cp /usr/share/air-gestures/config.toml ~/.config/air-gestures/
    systemctl --user daemon-reload && systemctl --user enable --now air-gesturesd

**Arch Linux**: `sudo pacman -U ./air-gestures-{tag}-x86_64-linux.pkg.tar.zst` (or the
`aarch64` one), the AUR package `air-gestures-bin` built ahead of its publication there.
The same steps as above follow it.

**Any other distribution**:

extract the tarball, put both binaries on your PATH, install the unit to
~/.config/systemd/user/ (adjust ExecStart if the binaries are not in /usr/bin) and
copy config.toml to ~/.config/air-gestures/. See the README for claiming a band.
On the AUR itself, `air-gestures-bin` follows once AUR account registration reopens.

Claiming a band unlinks it from your Meta glasses / phone app until you
factory-reset it and set it up there again.
