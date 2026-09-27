# Changelog

Each release's section becomes its GitLab release notes. Add a section for the version
before tagging it; the release job fails without one.

## 0.3.8

- Builds for 64-bit ARM Linux (aarch64: Raspberry Pi 4/5, Asahi, ARM laptops), next to
  x86_64. It's cross-compiled and tested under emulation only; reports from real
  hardware are welcome.
- `air-gestures disconnect` now also drops the band's Bluetooth link, so another
  device (like the Android app) can find and connect to the band right away.
