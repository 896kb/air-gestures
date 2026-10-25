# Shared band-core — design

Date: 2026-10-25 · Approved by the owner in conversation the same day.

## Goal

One `band-core` for Linux and Android, so the coming band features are built once and act the same on both platforms:
- index hold
- the middle pinch-and-turn dial
- palm-position overrides
- firmware profiles

Today each project has its own copy. Android's was copied from Linux at `91e7217` (2026-09-25) and the copies have drifted by about 740 diff lines.

## What the two copies hold today

**Android only** (`air-gestures-android/rust/band-core`):
- `ota.rs`: firmware updates over RPC/bulk transfer (Dave's merge request !1).
- `handwriting.rs`: the band's handwriting stream and its decoding (Dave's merge request !1).
- **Newer-firmware framing:**
  - `datax.rs`: `always_padded`, `without_padding`, and the `recent` record log.
  - `airshield.rs`: `pads_aligned`.
- `proto.rs`: `Value::Fixed(Vec<u8>)`, `byte_fields`, `text`, `describe` and `describe_nested`.
- `session.rs`:
  - the stream probe (`set_probe`, `probe_config`);
  - `device_info`;
  - `request_active`: the ACTIVE state on every connect;
  - the OTA calls;
  - the handwriting calls;
  - `RAW_INFERENCE` and `BAND_LOG` handling.
- Tests changed to match: `proto`, `session_ceremony`, `session_enrolled`, `session_hand`, `session_startup`.

**Linux only** (`air-gestures/band-core`, branch `master`):
- `BandSession::write_config` (commit `47fb615`). It writes a `ConfigReq`; the band-reset example uses it to put the band back to ACTIVE / `MODE_DEFAULT`.

**Shared:**
- The `Event` enum is identical in both copies, so the Linux daemon's match arms stay valid.

## Decisions

1. **New repository: `gitlab.com/896kb/band-core`.**
   - Public, MIT.
   - Carries `LICENSE` and `LICENSE-kinesis`: the core began from callbacked/kinesis, and the MIT notice must travel with it.
   - It starts from **the Android copy with its history** (a `git subtree split` of `rust/band-core` from the Android repo, so Dave's commits keep their authorship).
   - Then **one commit adds `write_config`** from Linux, along with its test in `tests/session_hand.rs`. The result is a superset of both copies.
2. **Version.** `band-core` gets its own version, starting at **0.4.0**, and a `CHANGELOG.md`.
   - A tag there releases nothing.
   - The apps pin a commit through the submodule.
   - The apps' own versions stay as they are.
3. **Included as a git submodule at the paths each project already uses:**
   - Linux: `band-core/` (a workspace member).
   - Android: `rust/band-core/` (a member of the `rust/` workspace).

   With the paths unchanged, these need no edits:
   - every `Cargo.toml` path dependency;
   - `build-rust.sh`;
   - the Gradle build.

   Linux's `band-core/Cargo.toml` currently says `version.workspace = true`; the shared one gets its own `version = "0.4.0"`. Cargo allows a member with its own version, so neither workspace needs to change.
4. **CI.**
   - Both apps' `.gitlab-ci.yml` get `GIT_SUBMODULE_STRATEGY: recursive`. The submodule URL is the public HTTPS one, so runners need no credentials.
   - `band-core` has its own `.gitlab-ci.yml` on the homelab runner (tag `docker`): `cargo test` and `cargo clippy -- -D warnings`, on `rust:1-bookworm` like the Linux project.
5. **Clean-up in the apps:**
   - Android's `rust/band-core/ORIGIN.md` goes away.
   - Both READMEs say to clone with `--recursive`, or to run `git submodule update --init` after a plain clone.
   - Both app repos keep their own root `LICENSE-kinesis`, because the apps include the core. Linux's packaging (the PKGBUILD and the release tarball) keeps installing that root copy, unchanged.
6. **Linux picks up the Android behaviour** (agreed). That means:
   - the newer-firmware padding fixes;
   - ACTIVE-on-connect;
   - the probe, OTA and handwriting APIs, which exist in the core but Linux doesn't call them.
7. **Not shared:**
   - Linux's local `firmware-explore` branch stays local and outside the core.
   - The uncommitted `air-gestures/examples/band-reset-state.rs` stays where it is.
   - The `daemon-reconnects-after-adapter-loss` fix only touches the daemon (`bluez.rs`) and is unaffected.

## Verification

- **`band-core` on its own:** `cargo test` and `cargo clippy -- -D warnings` are clean.
- **Linux (`~/Projects/air-gestures`):**
  - `cargo test --workspace --locked` and `cargo clippy --workspace` are clean.
  - **Hardware check before any release**, on the laptop's band (firmware 1054928927, build 70), running the built daemon: gestures, the dial, the battery reading, a reconnect after the band sleeps, and the claim staying valid.
- **Android (`~/Projects/air-gestures-android`):**
  - `cargo test` in `rust/`, then `./build-rust.sh` and `./gradlew assembleDebug testDebugUnitTest`.
  - The dev build (key `debug.keystore.linux-backup`, dev icon) goes on the phone, where gestures, the dial and handwriting must still work.
- **Byte-for-byte check:** Android's `rust/band-core` sources after the switch are identical to before, except for the `write_config` addition. `git diff` against the pre-switch tree proves it.

## Order of work

1. Create the local `band-core` repo from the Android subtree, then add `write_config`, `LICENSE-kinesis`, CI, the changelog and the version.
2. Switch Android to the submodule (pointing at the local repo until GitLab exists) and verify.
3. Switch Linux to the submodule and verify, including the hardware check.
4. **With the owner's go-ahead:** create `gitlab.com/896kb/band-core`, push it, point both submodules at its HTTPS URL, and push both apps.

## Out of scope

- The findings from the review of Dave's merge request. They're fixed separately, later, in the shared core.
- Index hold, the middle dial, palm-position overrides and firmware profiles. They're the next projects, built on this one.
- Tags or releases of any app. The owner decides the versions.
