# Shared band-core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One `band-core` repository, used by the Linux daemon and the Android app as a git submodule at the paths they use today.

**Architecture:**
- The new repo `~/Projects/band-core` is a split of Android's `rust/band-core`, history included, so Dave's commits keep their authors.
- On top of that split come:
  - Linux's `write_config` and its test;
  - the two clippy fixes;
  - the files that make it standalone.
- Each app replaces its copy with a submodule at the same path, so neither app needs Cargo or Gradle path changes.
- Until the owner approves creating `gitlab.com/896kb/band-core`, the submodules point at the local repo.

**Tech Stack:** Rust 2024 (MSRV 1.88), Cargo workspaces, git submodules, `git subtree`, GitLab CI (homelab runner, tag `docker`).

**Spec:** `docs/superpowers/specs/2026-10-25-shared-band-core-design.md` (in the Linux repo).

**Already verified (2026-10-25):** a throwaway clone of the Linux repo, with Android's `band-core` dropped in, builds and passes `cargo test --workspace` with no source changes. The `Event` enum is identical in both copies.

**House rules for whoever runs this:**
- No `git push`, no tags, and no GitLab project creation until the owner says so (Task 9 is gated).
- No sudo.
- Never run `iwctl`.
- Android dev builds are signed with `~/.android/debug.keystore.linux-backup` (`local.properties` already points there).
- Never uninstall the phone's dev build.
- Commits end with:
  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01MLDqGPYx8bTGKJcVLgoyzo
  ```

`S` below is the scratch directory: `S=/tmp/claude-1000/-home-user888/0b28f152-ab6d-44d1-b86a-97f9f03c7086/scratchpad/band-core-work`.

## Files

**New repo `~/Projects/band-core/`:**
- Everything currently in `air-gestures-android/rust/band-core/`, history included: `src/`, `tests/`, `Cargo.toml`.
- Removed: `ORIGIN.md`.
- Changed:
  - `Cargo.toml`: its own version 0.4.0, and explicit `edition`, `license` and `rust-version` instead of `.workspace = true`, so it builds alone.
  - `src/session.rs`: `write_config` added; `is_multiple_of`.
  - `src/ota.rs`: a `while let` loop.
  - `tests/session_hand.rs`: the `write_config` test added.
- New:
  - `LICENSE` and `LICENSE-kinesis`, copied from the Android repo.
  - `README.md`, `CHANGELOG.md`, `.gitignore`, `.gitlab-ci.yml`.
  - `Cargo.lock`, generated.

**Android `~/Projects/air-gestures-android/` (branch `shared-band-core`):**
- `rust/band-core/`: the tracked copy is replaced by a submodule.
- `.gitmodules`: new.
- `rust/Cargo.lock`: the band-core version changes.
- `.gitlab-ci.yml`: `GIT_SUBMODULE_STRATEGY`.
- `README.md`: the clone line.

**Linux `~/Projects/air-gestures/` (branch `shared-band-core`):** the same changes, with `band-core/` and `Cargo.lock` at the root.

---

### Task 1: Split Android's band-core, with its history, into a new repo

**Files:**
- Create: `~/Projects/band-core/`, a git repository on branch `main`.

- [ ] **Step 1: Split in a throwaway clone, so the Android repo gets no extra branch**

```bash
S=/tmp/claude-1000/-home-user888/0b28f152-ab6d-44d1-b86a-97f9f03c7086/scratchpad/band-core-work
rm -rf "$S" && mkdir -p "$S"
git clone -q ~/Projects/air-gestures-android "$S/android"
git -C "$S/android" subtree split --prefix=rust/band-core -b split
```

Expected: the last line printed is a commit hash.

- [ ] **Step 2: Create the repo from the split branch**

```bash
test ! -e ~/Projects/band-core
git init -q -b main ~/Projects/band-core
git -C ~/Projects/band-core pull -q "$S/android" split
git -C ~/Projects/band-core log --oneline | wc -l
```

Expected: `29`, the same as `git -C ~/Projects/air-gestures-android log --oneline -- rust/band-core | wc -l`.

- [ ] **Step 3: Check that the tree matches Android's copy exactly**

```bash
diff -r --exclude=.git ~/Projects/band-core ~/Projects/air-gestures-android/rust/band-core && echo identical
```

Expected: `identical`.

### Task 2: Make it build on its own

**Files:**
- Modify: `~/Projects/band-core/Cargo.toml`.
- Delete: `~/Projects/band-core/ORIGIN.md`.
- Create: `LICENSE`, `LICENSE-kinesis`, `.gitignore`, `README.md`, `CHANGELOG.md`, `Cargo.lock`.

- [ ] **Step 1: Check that it fails alone today**

Run: `cd ~/Projects/band-core && cargo test 2>&1 | head -3`

Expected: an error that mentions `workspace` (the `edition.workspace = true` inheritance has no workspace to inherit from).

- [ ] **Step 2: Write the standalone `Cargo.toml` header**

Replace the `[package]` section of `~/Projects/band-core/Cargo.toml` with the following. Leave `[dependencies]` and `[dev-dependencies]` unchanged.

```toml
[package]
name = "band-core"
version = "0.4.0"
edition = "2024"
license = "MIT"
rust-version = "1.88"
description = "Sans-IO Meta Neural Band protocol core"
repository = "https://gitlab.com/896kb/band-core"
```

Explicit values, not `.workspace = true`: inside an app's workspace they still work, and alone they don't need one. **Do not add a `[workspace]` table**: a member with its own `[workspace]` makes Cargo fail with "multiple workspace roots" inside the apps.

- [ ] **Step 3: Licences, ignore file and README; drop ORIGIN.md**

```bash
cd ~/Projects/band-core
git rm -q ORIGIN.md
cp ~/Projects/air-gestures-android/LICENSE ~/Projects/air-gestures-android/LICENSE-kinesis .
printf '/target/\n' > .gitignore
```

Write `~/Projects/band-core/README.md`:

```markdown
# band-core

The Meta Neural Band protocol core of Air Gestures, in Rust and sans-IO. It covers:
- the AirShield encryption handshake;
- DataX framing;
- the protobuf fields the band speaks;
- gestures and the pinch-and-turn dial;
- battery and raw sEMG;
- firmware updates;
- the band's handwriting stream.

It never touches Bluetooth itself. A caller feeds it the bytes the band sent, and sends the
bytes it returns.

It is shared by both Air Gestures apps, which include it as a git submodule:
- [the Linux daemon](https://gitlab.com/896kb/air-gestures), at `band-core/`;
- [the Android app](https://gitlab.com/896kb/air-gestures-android), at `rust/band-core/`.

A change lands here once, then both apps move their submodule to it.

    cargo test
    cargo clippy --all-targets -- -D warnings

`tests/` runs every session against a simulated band (`responder.rs`, `sim.rs`), so no band
is needed.

## License

MIT, see [`LICENSE`](LICENSE). Parts of it started from
[callbacked/kinesis](https://github.com/callbacked/kinesis) (MIT), whose notice is in
[`LICENSE-kinesis`](LICENSE-kinesis).
```

Write `~/Projects/band-core/CHANGELOG.md`:

```markdown
# Changelog

## 0.4.0

- One band-core for both apps. Until now Linux and Android each kept a copy. This one
  starts from the Android copy, which already had:
  - firmware updates and the band's handwriting stream (by David Ljung Madison Stellar);
  - the newer-firmware record padding;
  - the stream probe;
  - the ACTIVE state on every connect.
- `BandSession::write_config` from the Linux copy: it writes a `ConfigReq`, to put back
  settings the band keeps across connections.
```

- [ ] **Step 4: Build and test alone**

Run: `cd ~/Projects/band-core && cargo test 2>&1 | grep -E "^error|test result" | sort | uniq -c`

Expected: no `error` lines, and every `test result: ok.` with `0 failed`. `Cargo.lock` now exists.

- [ ] **Step 5: Commit**

```bash
cd ~/Projects/band-core
git add -A
git commit -q -m "A repository of its own: version 0.4.0, licences, README, changelog

band-core was copied into each app; from here both include this one as a submodule.
Builds alone now (explicit edition, licence and MSRV instead of a parent workspace's).

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01MLDqGPYx8bTGKJcVLgoyzo"
```

### Task 3: Bring over `write_config` from Linux

**Files:**
- Modify: `~/Projects/band-core/src/session.rs`, inserted just before `    fn request_hand(&mut self, hand: Option<Hand>, reading: bool, time: f64) -> Result<Vec<u8>> {`.
- Test: `~/Projects/band-core/tests/session_hand.rs`.

- [ ] **Step 1: Write the failing test**

In `tests/session_hand.rs`, change the imports at the top to:

```rust
mod common;

use band_core::events::Hand;
use band_core::proto::{field_bytes as fb, field_int as fi};
use common::*;
```

and append at the end of the file:

```rust

/// `write_config` opens the config service on first use, then reuses the channel.
#[test]
fn write_config_opens_the_service_then_carries_only_the_config() {
    let mut peer = Peer::legacy(true, false);
    let config = [fi(9, 0), fi(23, 0)].concat();
    let bytes = peer.session.write_config(&config).unwrap();
    let frame = peer.requests(&bytes).unwrap().remove(0);
    assert_eq!(frame.channel, 0x8006);
    assert_eq!(frame.payload, [fi(1, 2), fb(5, &config)].concat());
}
```

- [ ] **Step 2: Run it to see it fail**

Run: `cd ~/Projects/band-core && cargo test --test session_hand 2>&1 | grep -E "error\[|no method"`

Expected: `error[E0599]: no method named \`write_config\` found for struct \`BandSession\``.

- [ ] **Step 3: Add `write_config`**

Insert into `src/session.rs`, immediately before the line `    fn request_hand(&mut self, hand: Option<Hand>, reading: bool, time: f64) -> Result<Vec<u8>> {`:

```rust
    /// Write an arbitrary `ConfigReq` and return the bytes to send.
    ///
    /// The band persists several settings in `Config` — `deviceState`,
    /// `emgSensorState`, `mode`, `dataCollectionModeEnabled` — so anything that
    /// changes them must change them back. This is the escape hatch for that;
    /// `stop()` only disables the streams, it does not restore the config.
    pub fn write_config(&mut self, config: &[u8]) -> Result<Vec<u8>> {
        if self.stopping {
            return Err(BandError::Busy("The session is stopping.".into()));
        }
        let id = self.configuration_id + 1;
        let words: &[u32] = if self.configuration_id == 0 {
            &SERVICE_OPEN
        } else {
            &[]
        };
        let frame = encode_frame(
            CONFIGURATION_CHANNEL,
            words,
            &[field_int(1, id), field_bytes(5, config)].concat(),
        )?;
        let bytes = self.encrypt(&frame)?;
        self.configuration_id = id;
        Ok(bytes)
    }

```

The symbols it uses are already in this file:
- `stopping` (field);
- `configuration_id` (field);
- `SERVICE_OPEN`, `CONFIGURATION_CHANNEL`;
- `encode_frame`, `field_int`, `field_bytes` (imported);
- `BandError::Busy`;
- `encrypt`.

- [ ] **Step 4: Run it to see it pass, with nothing else broken**

Run: `cargo test 2>&1 | grep -E "^error|FAILED|test result" | sort | uniq -c`

Expected: no `error` and no `FAILED`. Every `test result: ok.`.

- [ ] **Step 5: Commit**

```bash
git add src/session.rs tests/session_hand.rs
git commit -q -m "Add BandSession::write_config, from the Linux copy

The band persists deviceState, emgSensorState, mode and dataCollectionModeEnabled;
stop() only disables the streams. write_config sends an arbitrary ConfigReq, so a
caller can put the band back (Linux's band-reset example uses it).

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01MLDqGPYx8bTGKJcVLgoyzo"
```

### Task 4: Clippy clean, and CI

**Files:**
- Modify: `~/Projects/band-core/src/session.rs` (`probe_note`), `~/Projects/band-core/src/ota.rs` (`fill`).
- Create: `~/Projects/band-core/.gitlab-ci.yml`.

- [ ] **Step 1: See the two warnings**

Run: `cd ~/Projects/band-core && cargo clippy --all-targets -- -D warnings 2>&1 | grep -E "^error" | sort -u`

Expected: two errors, `manual implementation of \`.is_multiple_of()\`` (in `src/session.rs`, `probe_note`) and `this loop could be written as a \`while let\` loop` (in `src/ota.rs`, `fill`).

- [ ] **Step 2: Fix `probe_note` in `src/session.rs`**

Replace

```rust
        if count <= PROBE_SHOWN || count % PROBE_EVERY == 0 {
```

with

```rust
        if count <= PROBE_SHOWN || count.is_multiple_of(PROBE_EVERY) {
```

- [ ] **Step 3: Fix `fill` in `src/ota.rs`**

Replace

```rust
        loop {
            let Some(transfer) = self.transfer.as_ref() else { break };
            if transfer.pending.len() >= transfer.window || transfer.next >= size {
                break;
            }
```

with

```rust
        while let Some(transfer) = self.transfer.as_ref() {
            if transfer.pending.len() >= transfer.window || transfer.next >= size {
                break;
            }
```

The rest of the loop body (`let offset = transfer.next;` through `out.push(body);`) and the closing `}` stay as they are.

- [ ] **Step 4: Clean, and still green**

Run:

```bash
cargo clippy --all-targets -- -D warnings 2>&1 | tail -1
cargo test 2>&1 | grep -E "^error|FAILED" | wc -l
```

Expected: `Finished …`, then `0`.

- [ ] **Step 5: Write `.gitlab-ci.yml`**

```yaml
# CI for band-core: the tests (against the simulated band) and clippy, on every push.
# It releases nothing; the apps pin a commit of this repository as a git submodule.

# Every job runs on the homelab's docker runner (brigadahq-docker), not GitLab's
# metered shared runners.
default:
  tags: [docker]
  image: rust:1-bookworm

variables:
  CARGO_HOME: "$CI_PROJECT_DIR/.cargo-home"
  CARGO_TERM_COLOR: "always"

test:
  cache:
    key:
      files: [Cargo.lock]
    paths:
      - .cargo-home/registry/index/
      - .cargo-home/registry/cache/
      - .cargo-home/git/db/
      - target/
  script:
    - rustup component add clippy
    - cargo test --locked
    - cargo clippy --locked --all-targets -- -D warnings
```

- [ ] **Step 6: Commit**

```bash
git add src/session.rs src/ota.rs .gitlab-ci.yml
git commit -q -m "Clippy clean (is_multiple_of, while let), and CI: tests and clippy

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01MLDqGPYx8bTGKJcVLgoyzo"
```

### Task 5: Android uses the submodule

**Files:**
- Replace: `~/Projects/air-gestures-android/rust/band-core/` (the tracked files become a submodule).
- Create: `~/Projects/air-gestures-android/.gitmodules`.
- Modify: `rust/Cargo.lock`, `.gitlab-ci.yml`, `README.md`.

- [ ] **Step 1: Branch, and keep the old tree for the byte check**

```bash
cd ~/Projects/air-gestures-android
git status -s    # expected: empty
git switch -c shared-band-core
mkdir -p "$S/android-old" && git archive HEAD rust/band-core | tar -x -C "$S/android-old"
```

- [ ] **Step 2: Swap the copy for the submodule**

```bash
git rm -r -q rust/band-core
rm -rf rust/band-core
git -c protocol.file.allow=always submodule add /home/user888/Projects/band-core rust/band-core
cat .gitmodules
```

Expected: `.gitmodules` has `path = rust/band-core` and `url = /home/user888/Projects/band-core`. Task 9 changes the URL to GitLab.

- [ ] **Step 3: Byte check: only the planned changes**

```bash
diff -r --exclude=.git "$S/android-old/rust/band-core" rust/band-core | grep -E "^(Only in|diff -r)" 
```

Expected, and nothing else:
- `Only in …android-old/rust/band-core: ORIGIN.md`
- `Only in rust/band-core:` followed by each of `.gitignore`, `.gitlab-ci.yml`, `CHANGELOG.md`, `Cargo.lock`, `LICENSE`, `LICENSE-kinesis`, `README.md`
- `diff -r` lines for `Cargo.toml`, `src/ota.rs`, `src/session.rs`, `tests/session_hand.rs`

- [ ] **Step 4: Rust tests (this refreshes `rust/Cargo.lock`: band-core goes from 0.3.8 to 0.4.0)**

Run: `cd rust && cargo test 2>&1 | grep -E "^error|FAILED|test result" | sort | uniq -c; cd ..`

Expected: no `error` and no `FAILED`.

Then: `git diff --stat rust/Cargo.lock` shows a small change (the band-core version).

- [ ] **Step 5: CI clones the submodule**

In `.gitlab-ci.yml`, in the top-level `variables:` block, add after `RUSTUP_HOME: "$CI_PROJECT_DIR/.rustup"`:

```yaml
  # rust/band-core is a submodule (gitlab.com/896kb/band-core, public).
  GIT_SUBMODULE_STRATEGY: recursive
```

- [ ] **Step 6: README: how to clone**

Find the build or development section with `grep -n "build-rust\|git clone" README.md`. Add this line before the first build command there:

```markdown
`rust/band-core` is a git submodule ([band-core](https://gitlab.com/896kb/band-core), shared with the Linux daemon): clone with `git clone --recursive`, or run `git submodule update --init` after a plain clone.
```

- [ ] **Step 7: Build the app and its unit tests**

```bash
./build-rust.sh
./gradlew -q assembleDebug testDebugUnitTest
```

Expected: no output from Gradle, and exit status 0.

- [ ] **Step 8: Install the dev build on the phone and try it**

```bash
adb devices -l     # the phone, model 25098PN5AC
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

Expected: `Success`. If it fails with `INSTALL_FAILED_USER_RESTRICTED`, the phone's "Install via USB" prompt needs a tap; ask the owner, then retry.

The owner then checks, with the band connected to the phone:
- a swipe;
- an index tap;
- pinch and turn;
- handwriting in a text field.

- [ ] **Step 9: Commit**

```bash
git add .gitmodules rust/band-core rust/Cargo.lock .gitlab-ci.yml README.md
git commit -q -m "band-core is now a submodule, shared with the Linux daemon

The same sources as the copy (plus write_config and two clippy fixes), now from the
band-core repository.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01MLDqGPYx8bTGKJcVLgoyzo"
```

### Task 6: Linux uses the submodule

**Files:**
- Replace: `~/Projects/air-gestures/band-core/`.
- Create: `~/Projects/air-gestures/.gitmodules`.
- Modify: `Cargo.lock`, `.gitlab-ci.yml`, `README.md`.

- [ ] **Step 1: Branch from master, which has several unpushed commits (the kinesis wording, the findings docs, this spec and plan); that's expected**

```bash
cd ~/Projects/air-gestures
git status -s    # expected: only ?? air-gestures/examples/ (left alone on purpose)
git switch -c shared-band-core
```

- [ ] **Step 2: Swap the copy for the submodule**

```bash
git rm -r -q band-core
rm -rf band-core
git -c protocol.file.allow=always submodule add /home/user888/Projects/band-core band-core
```

- [ ] **Step 3: Tests and clippy for the whole workspace (this refreshes `Cargo.lock`: band-core goes from 0.3.10 to 0.4.0)**

```bash
cargo test --workspace 2>&1 | grep -E "^error|FAILED|test result" | sort | uniq -c
cargo clippy --workspace --all-targets 2>&1 | grep -E "^(warning|error)" | sort | uniq -c
```

Expected:
- **Tests:** no `error` and no `FAILED`.
- **Clippy:** none from band-core (Task 4 made it clean). Any warnings in the daemon or CLI must be the same as on `master`. To compare, run the same clippy command in a checkout of master: `git worktree add "$S/linux-master" master`, run it there, then `git worktree remove "$S/linux-master"`.

- [ ] **Step 4: The untracked band-reset example still builds against `write_config`**

Run: `cargo build -p air-gestures --example band-reset-state 2>&1 | tail -1`

Expected: `Finished …`.

- [ ] **Step 5: CI clones the submodule**

In `.gitlab-ci.yml`, in the top-level `variables:` block, add after `TARGET: "x86_64-unknown-linux-gnu"`:

```yaml
  # band-core is a submodule (gitlab.com/896kb/band-core, public).
  GIT_SUBMODULE_STRATEGY: recursive
```

- [ ] **Step 6: README: how to clone**

Find the build-from-source section with `grep -n "cargo build\|git clone" README.md`. Add before its first command:

```markdown
`band-core` is a git submodule ([band-core](https://gitlab.com/896kb/band-core), shared with the Android app): clone with `git clone --recursive`, or run `git submodule update --init` after a plain clone.
```

- [ ] **Step 7: Commit**

```bash
git add .gitmodules band-core Cargo.lock .gitlab-ci.yml README.md
git commit -q -m "band-core is now a submodule, shared with the Android app

Linux picks up what the Android copy had gained: the newer-firmware record padding,
the ACTIVE state on every connect, the stream probe, and the firmware update and
handwriting APIs (not used here yet).

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01MLDqGPYx8bTGKJcVLgoyzo"
```

### Task 7: Linux hardware check on the laptop's band

This needs the owner at the laptop with the band on. The band talks to one host at a time, so the phone must not hold it.

- [ ] **Step 1: Build the daemon**

Run: `cd ~/Projects/air-gestures && cargo build --release -p air-gesturesd 2>&1 | tail -1`

Expected: `Finished …`.

- [ ] **Step 2: Swap the running daemon for this build, in the foreground**

```bash
systemctl --user stop air-gesturesd
bluetoothctl connect A4:0E:2B:90:A6:64    # wakes the band
AIR_GESTURES_LOG=air_gesturesd=debug,band_core=debug,info \
  timeout 180 target/release/air-gesturesd > "$S/linux-hw.log" 2>&1
```

During those 3 minutes the owner does:
- a swipe each way;
- an index tap and a double tap;
- a middle tap, a double tap and a hold (lock off, then on);
- pinch and turn;
- then leaves the band idle for about 60 s, so it sleeps, and wakes it with a tap.

- [ ] **Step 3: Read the log**

```bash
grep -ciE "gesture|swipe|tap" "$S/linux-hw.log"
grep -iE "dial|battery" "$S/linux-hw.log" | head -5
grep -c "0200020c" "$S/linux-hw.log"
grep -iE "error|unavailable|claim" "$S/linux-hw.log" | head
```

Expected:
- gesture lines are present, and dial and battery lines appear;
- `0` frames of `0200020c` (the handwriting pipeline is not running);
- no claim errors.

If the band had dropped and reconnected, the log shows a reconnect followed by gestures again.

- [ ] **Step 4: Put the released daemon back**

Run: `systemctl --user start air-gesturesd && systemctl --user is-active air-gesturesd`

Expected: `active`.

Nothing is installed: the service still runs the released 0.3.10 binary from `~/.cargo/bin`.

### Task 8: Note it for later sessions

**Files:**
- Modify: `~/.claude/projects/-home-user888/memory/air-gestures-android.md` and `~/.claude/projects/-home-user888/memory/flow-z13-kinesis-neural-band.md`.

- [ ] **Step 1: Update both memory notes**

Both files currently describe band-core as copied. Replace that with: band-core lives in `~/Projects/band-core` and both apps include it as a git submodule, Android at `rust/band-core` and Linux at `band-core`. Until Task 9 the submodule URL is the local path, so cloning needs `-c protocol.file.allow=always`. A protocol change lands in band-core first; each app then moves its submodule. Remove the "fix upstream first, then copy" / ORIGIN.md instruction.

### Task 9 (only after the owner says "push"): GitLab, and the apps

- [ ] **Step 1: Create the project and push band-core**

```bash
glab api user > /dev/null    # refreshes the token
glab repo create 896kb/band-core --public --description "Meta Neural Band protocol core shared by Air Gestures for Linux and Android"
cd ~/Projects/band-core
git remote add origin https://gitlab.com/896kb/band-core.git
AUTH=$(printf 'oauth2:%s' "$(glab config get token --host gitlab.com)" | base64 -w0)
git -c credential.helper= -c http.extraHeader="Authorization: Basic $AUTH" push -u origin main
```

Never run `glab auth git-credential get`: it prints the refresh token.

- [ ] **Step 2: Point both submodules at GitLab**

```bash
cd ~/Projects/air-gestures-android && git submodule set-url rust/band-core https://gitlab.com/896kb/band-core.git && git add .gitmodules && git commit -q -m "band-core submodule: from GitLab

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01MLDqGPYx8bTGKJcVLgoyzo"
cd ~/Projects/air-gestures && git submodule set-url band-core https://gitlab.com/896kb/band-core.git && git add .gitmodules && git commit -q -m "band-core submodule: from GitLab

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01MLDqGPYx8bTGKJcVLgoyzo"
```

- [ ] **Step 3: Merge and push each app, as the owner directs**

For each app, run `git switch master && git merge --no-ff shared-band-core`, then push with the same `AUTH` header. Linux `master` also carries its earlier unpushed commits. Push only what the owner OKs.

- [ ] **Step 4: Watch the three pipelines**

```bash
for p in band-core air-gestures air-gestures-android; do glab ci list -R 896kb/$p -P 1; done
```

Expected: all `success`. A failing clone step means `GIT_SUBMODULE_STRATEGY` or the URL is wrong.
