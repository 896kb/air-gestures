## Install

Linux daemon and CLI for the Meta Neural Band.

`air-gestures-{tag}-x86_64-linux.tar.gz` (x86_64) and
`air-gestures-{tag}-aarch64-linux.tar.gz` (64-bit ARM, tested under emulation only)
each contain the `air-gestures` and `air-gesturesd` binaries (glibc >= 2.36), a systemd user unit
(`air-gesturesd.service`, ExecStart=/usr/bin/air-gesturesd), the example
`config.toml`, README.md, LICENSE and LICENSE-kinesis.

Runtime needs: BlueZ, D-Bus and WebKitGTK 4.1 (`webkit2gtk-4.1` on Arch).

Extract the tarball, put both binaries on your PATH, install the unit to
~/.config/systemd/user/ (adjust ExecStart if the binaries are not in /usr/bin) and
copy config.toml to ~/.config/air-gestures/. See the README for claiming a band.
The AUR package (`air-gestures-bin`) follows once AUR account registration reopens.

Claiming a band unlinks it from your Meta glasses / phone app until you
factory-reset it and set it up there again.
