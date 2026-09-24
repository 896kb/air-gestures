# kinesisctl + arch_rice Implementation Plan (plan 3 of 3)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The user-facing half: `kinesisctl` (control, waybar output, Meta sign-in window, pairing, sEMG record/view) and the arch_rice integration on the Z13 profile, ending in a hardware-day runbook the user and Claude execute together at the band.

**Architecture:** A new `kinesisctl` crate in the `~/Projects/kinesis-linux` workspace. It talks to `kinesisd` only through the JSON control socket (types reused from the `kinesisd` library: `ipc::{Request, Response, socket_path}`, `status::Status`, `store::Store`, `meta::{MetaSession, url_form}`). The Meta sign-in is a port of kinesis `MetaAuth.swift` + `MetaLoginView.swift`: three HTTPS calls around an incognito WebKitGTK window (`wry` + `tao`) that intercepts the `fb-viewapp://…frl_login` redirect. Everything except the window and the network calls is tested offline, most of it against an in-process `kinesisd` daemon on a `SimTransport`.

**Tech Stack:** Rust 1.88+ (edition 2024), clap (derive), tokio, reqwest (rustls), wry + tao (WebKitGTK 4.1, already installed), ratatui (+ its crossterm), serde_json, base64, sha2, uuid, url; dev: tempfile.

**Spec:** `docs/superpowers/specs/2026-09-24-kinesis-linux-design.md`. **Contract:** the "What plan 3 picks up" section at the end of `docs/superpowers/plans/2026-09-24-kinesisd.md` (IPC requests/responses, Status v1, service-start and hardware-day notes). Read it.

**Ground rules**
- Work in `~/Projects/kinesis-linux` on a new branch `kinesisctl` (Task 1 creates it from `master`); arch_rice work (Task 7) happens in `~/arch_rice` on its current branch `multi-host-profiles`, **committing only the files this plan touches by explicit path** — that tree has the user's uncommitted work-in-progress, which must stay untouched and unstaged. **Never push** either repo.
- arch_rice files are live (symlinked into `$HOME`; Hyprland reloads on write). Create new files before editing anything that references them.
- The only real-hardware actions allowed before hardware day: `kinesisd --scan`, and running `kinesisd` with **no band paired** (it then never touches Bluetooth). No pairing, no Meta sign-in against the real service except in Task 8 with the user present.
- Meta access tokens never reach logs, stdout, or `Debug`.
- `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check` green after every task. Crate versions below are intended majors; adapt minimally if they don't resolve, and report.
- Every commit message ends with:
  ```
  Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01VsiceY7FhkoM7aRuA9DEfz
  ```

---

## File structure

```
kinesisctl/
├── Cargo.toml
├── src/
│   ├── main.rs        clap CLI → commands (sync main; small tokio runtimes around async steps,
│   │                  because the sign-in window's event loop must own the main thread)
│   ├── lib.rs
│   ├── client.rs      call() with timeout, Subscription (status lines / sEMG lines)
│   ├── text.rs        human-readable status
│   ├── waybar.rs      Status → waybar JSON line; follow loop
│   ├── metaauth.rs    MetaAuth port: lsd/jazoest, multipart, callback check, tokens/decrypt/login
│   ├── signin.rs      the WebKitGTK sign-in window
│   ├── pair.rs        pairing orchestration (sign-in on demand, --replace, progress)
│   └── semg.rs        record (JSONL) + view (ratatui) + batch decoding helpers
└── tests/
    ├── support/mod.rs in-process kinesisd on SimTransport + temp socket
    ├── commands.rs  waybar.rs  metaauth.rs  pair.rs  semg.rs
docs/hardware-day.md   the runbook (Task 8)

~/arch_rice (branch multi-host-profiles)
├── .config/kinesis/config.toml          Z13 gesture/dial commands
├── .local/bin/kinesis-dial              OSD-aware volume/brightness steps for the dial
├── .config/waybar/profile.d/z13.jsonc   + custom/kinesis module
├── .config/waybar/style.css             + #custom-kinesis classes
├── .config/hypr/profile.d/z13.conf      + exec-once: import env, start kinesisd
├── profiles/z13.sh                      + neural_band feature
├── install.sh                           + rice_install_kinesis (cargo install, unit link)
├── .gitignore                           + whitelist .config/kinesis
└── kinesis-linux                        submodule pointer → local master
```

---

### Task 1: kinesisctl scaffold, socket client, simple commands

**Files:**
- Create: `kinesisctl/Cargo.toml`, `kinesisctl/src/{main,lib,client,text}.rs`, `kinesisctl/tests/support/mod.rs`, `kinesisctl/tests/commands.rs`
- Modify: root `Cargo.toml` (members), `kinesisd/src/ipc.rs` (stale doc on `Forget`)

- [ ] **Step 0: Branch**

```bash
cd ~/Projects/kinesis-linux && git switch -c kinesisctl
```

- [ ] **Step 1: Manifests**

Root `Cargo.toml`: `members = ["band-core", "kinesisd", "kinesisctl"]`.

`kinesisctl/Cargo.toml`:
```toml
[package]
name = "kinesisctl"
version = "0.1.0"
edition.workspace = true
license.workspace = true
rust-version.workspace = true
description = "Control kinesisd: status, waybar, Meta sign-in, pairing, sEMG"

[dependencies]
band-core = { path = "../band-core" }
kinesisd = { path = "../kinesisd" }
base64 = "0.22"
clap = { version = "4", features = ["derive"] }
rand_core = { version = "0.6", features = ["getrandom"] }
ratatui = "0.29"
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"
tao = "0.34"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "io-util", "net", "time", "signal", "fs"] }
url = "2"
uuid = { version = "1", features = ["v4"] }
wry = "0.53"

[dev-dependencies]
tempfile = "3"
tokio = { version = "1", features = ["test-util"] }
```

Fix the stale doc comment on `Request::Forget` in `kinesisd/src/ipc.rs`: "Forget the band: keys, band record and the Bluetooth bond (the Meta sign-in is kept; `kinesisctl signout` removes it)."

- [ ] **Step 2: Test support — an in-process daemon**

`kinesisctl/tests/support/mod.rs`:
```rust
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use band_core::identity::{EnrollmentIdentity, verifying_key_from_point64};
use band_core::responder::SimMode;
use kinesisd::actions::RecordingRunner;
use kinesisd::daemon::Daemon;
use kinesisd::ipc::{self, Handles};
use kinesisd::meta::FakeFactory;
use kinesisd::store::{BandRecord, Store};
use kinesisd::transport::{SIM_ADDRESS, SIM_NAME, SimTransport};
use p256::ecdsa::SigningKey;
use rand_core::OsRng;
use tokio::sync::mpsc;

pub struct TestDaemon {
    pub socket: PathBuf,
    pub transport: SimTransport,
    pub runner: RecordingRunner,
    pub store: Store,
    pub dir: tempfile::TempDir,
}

/// A kinesisd on a simulated band, serving a socket in a temp dir.
/// `paired` pre-claims the band (store + sim owner).
pub async fn start(mode: SimMode, paired: bool) -> TestDaemon {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("kinesis.sock");
    let store = Store::open(dir.path().join("state")).unwrap();
    let transport = SimTransport::new(mode);
    if paired {
        store.save_band(&BandRecord { address: SIM_ADDRESS.into(), name: SIM_NAME.into() }).unwrap();
        let band_key = verifying_key_from_point64(&transport.band_point()).unwrap();
        let identity = EnrollmentIdentity::new(SigningKey::random(&mut OsRng), Some(band_key));
        store.save_identity(&identity).unwrap();
        transport.set_owner(&identity);
    }
    let runner = RecordingRunner::default();
    let factory = FakeFactory { device_point: Some(transport.band_point()), fail: None };
    let daemon = Daemon::new(Arc::new(transport.clone()), runner.clone(), factory, store.clone(), dir.path().join("config.toml"));
    let listener = ipc::bind(&socket).unwrap();
    let (calls, call_rx) = mpsc::channel(16);
    tokio::spawn(ipc::serve(listener, Handles { calls, status: daemon.status(), semg: daemon.semg() }));
    tokio::spawn(daemon.run(call_rx, std::future::pending()));
    TestDaemon { socket, transport, runner, store, dir }
}

impl TestDaemon {
    /// Poll `status` until `until` holds (real time, 20 s cap).
    pub async fn wait(&self, what: &str, until: impl Fn(&kinesisd::status::Status) -> bool) -> kinesisd::status::Status {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        loop {
            if let Ok(status) = kinesisctl::client::status(&self.socket).await
                && until(&status)
            {
                return status;
            }
            assert!(tokio::time::Instant::now() < deadline, "timed out waiting for {what}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}
```
(Add `p256 = { version = "0.13", features = ["ecdsa"] }` to `[dev-dependencies]` if the test crate can't see it through `kinesisd`.)

- [ ] **Step 3: Write the failing tests**

`kinesisctl/tests/commands.rs`:
```rust
mod support;

use band_core::responder::SimMode;
use kinesisctl::client;
use kinesisctl::text::describe;
use kinesisd::config::DialTarget;
use kinesisd::ipc::{HandChoice, Request};
use kinesisd::status::{LinkState, Status};

#[tokio::test]
async fn status_reports_a_connected_band() {
    let daemon = support::start(SimMode::Enrolled, true).await;
    let status = daemon.wait("connected", |s| s.state == LinkState::Connected).await;
    let text = describe(&status);
    assert!(text.contains("connected"), "{text}");
    assert!(text.contains("Meta Band (simulated)"), "{text}");
}

#[tokio::test]
async fn without_a_band_the_status_says_how_to_pair() {
    let daemon = support::start(SimMode::Enrolled, false).await;
    let status = daemon.wait("the error", |s| s.error.is_some()).await;
    assert!(describe(&status).contains("kinesisctl pair"));
}

#[tokio::test]
async fn simple_requests_change_the_status() {
    let daemon = support::start(SimMode::Enrolled, true).await;
    daemon.wait("connected", |s| s.state == LinkState::Connected).await;
    let paused = client::request(&daemon.socket, &Request::Pause).await.unwrap();
    assert!(paused.status.unwrap().paused);
    let dial = client::request(&daemon.socket, &Request::Dial { target: DialTarget::Brightness }).await.unwrap();
    assert_eq!(dial.status.unwrap().dial, DialTarget::Brightness);
    let hand = client::request(&daemon.socket, &Request::Hand { hand: HandChoice::Left }).await.unwrap();
    assert!(hand.ok);
    daemon.wait("the left hand", |s| s.hand.as_deref() == Some("left")).await;
}

#[tokio::test]
async fn a_daemon_error_is_returned_as_err() {
    let daemon = support::start(SimMode::Enrolled, false).await;
    let error = client::request(&daemon.socket, &Request::Connect).await.unwrap_err();
    assert!(error.contains("kinesisctl pair"), "{error}");
}

#[tokio::test]
async fn a_missing_daemon_is_a_clear_error() {
    let dir = tempfile::tempdir().unwrap();
    let error = client::status(&dir.path().join("nope.sock")).await.unwrap_err();
    assert!(error.contains("kinesisd isn't running"), "{error}");
}

#[tokio::test]
async fn subscriptions_deliver_changes() {
    let daemon = support::start(SimMode::Enrolled, true).await;
    let mut subscription = client::Subscription::status(&daemon.socket).await.unwrap();
    let first: Status = subscription.next_status().await.unwrap();
    assert_eq!(first.version, 1);
    loop {
        let next = tokio::time::timeout(std::time::Duration::from_secs(20), subscription.next_status())
            .await
            .unwrap()
            .unwrap();
        if next.state == LinkState::Connected {
            break;
        }
    }
}

#[test]
fn the_text_status_shows_battery_hand_dial_and_pause() {
    let status = Status {
        state: LinkState::Connected,
        battery: Some(64),
        charging: Some(true),
        hand: Some("right".into()),
        paused: true,
        band: Some("Meta Band 1234".into()),
        ..Status::default()
    };
    let text = describe(&status);
    for part in ["Meta Band 1234", "connected", "64%", "charging", "right", "volume", "paused"] {
        assert!(text.contains(part), "missing {part} in {text}");
    }
}
```

- [ ] **Step 4: Run to verify failure**

Run: `cargo test -p kinesisctl --test commands`
Expected: FAIL — unresolved `kinesisctl::client` / `text`.

- [ ] **Step 5: Implement**

`kinesisctl/src/lib.rs`:
```rust
//! kinesisctl: the command-line and waybar front end for kinesisd.

pub mod client;
pub mod text;
```

`kinesisctl/src/client.rs`:
```rust
//! Talking to kinesisd over its control socket (newline-delimited JSON).

use std::path::Path;
use std::time::Duration;

use kinesisd::ipc::{Request, Response};
use kinesisd::status::Status;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

const CALL_TIMEOUT: Duration = Duration::from_secs(10);

fn not_running(path: &Path, error: std::io::Error) -> String {
    format!("kinesisd isn't running ({}: {error}). start it with `systemctl --user start kinesisd`", path.display())
}

async fn connect(path: &Path) -> Result<UnixStream, String> {
    UnixStream::connect(path).await.map_err(|error| not_running(path, error))
}

async fn send(write: &mut OwnedWriteHalf, request: &Request) -> Result<(), String> {
    let mut line = serde_json::to_vec(request).map_err(|error| error.to_string())?;
    line.push(b'\n');
    write.write_all(&line).await.map_err(|error| format!("writing to kinesisd failed: {error}"))
}

/// One request, one response. `Err` carries the daemon's error text (or a transport problem).
pub async fn request(path: &Path, request: &Request) -> Result<Response, String> {
    let stream = connect(path).await?;
    let (read, mut write) = stream.into_split();
    send(&mut write, request).await?;
    let mut lines = BufReader::new(read).lines();
    let line = tokio::time::timeout(CALL_TIMEOUT, lines.next_line())
        .await
        .map_err(|_| "kinesisd didn't answer in time".to_string())?
        .map_err(|error| format!("reading from kinesisd failed: {error}"))?
        .ok_or_else(|| "kinesisd closed the connection".to_string())?;
    let response: Response = serde_json::from_str(&line).map_err(|error| format!("unexpected answer from kinesisd: {error}"))?;
    if response.ok { Ok(response) } else { Err(response.error.unwrap_or_else(|| "kinesisd refused the request".into())) }
}

pub async fn status(path: &Path) -> Result<Status, String> {
    request(path, &Request::Status)
        .await?
        .status
        .ok_or_else(|| "kinesisd sent no status".to_string())
}

/// A long-lived `subscribe` / `subscribe-semg` connection. The write half is
/// kept open: the daemon treats EOF from the client as a hang-up.
pub struct Subscription {
    lines: Lines<BufReader<OwnedReadHalf>>,
    _write: OwnedWriteHalf,
}

impl Subscription {
    async fn open(path: &Path, request: &Request) -> Result<Self, String> {
        let stream = connect(path).await?;
        let (read, mut write) = stream.into_split();
        send(&mut write, request).await?;
        Ok(Self { lines: BufReader::new(read).lines(), _write: write })
    }

    pub async fn status(path: &Path) -> Result<Self, String> {
        Self::open(path, &Request::Subscribe).await
    }

    pub async fn semg(path: &Path) -> Result<Self, String> {
        Self::open(path, &Request::SubscribeSemg).await
    }

    async fn next_response(&mut self) -> Option<Response> {
        loop {
            let line = self.lines.next_line().await.ok()??;
            if let Ok(response) = serde_json::from_str::<Response>(&line) {
                return Some(response);
            }
        }
    }

    /// The next status line; `None` when the daemon goes away.
    pub async fn next_status(&mut self) -> Option<Status> {
        loop {
            let response = self.next_response().await?;
            if let Some(status) = response.status {
                return Some(status);
            }
        }
    }

    /// The next raw sEMG payload (base64-decoded); `None` when the daemon goes away.
    pub async fn next_semg(&mut self) -> Option<Vec<u8>> {
        use base64::Engine;
        loop {
            let response = self.next_response().await?;
            if let Some(frame) = response.semg
                && let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(frame)
            {
                return Some(bytes);
            }
        }
    }
}
```

`kinesisctl/src/text.rs`:
```rust
//! Human-readable status for `kinesisctl status`.

use kinesisd::config::DialTarget;
use kinesisd::status::{LinkState, Status};

fn state(status: &Status) -> &'static str {
    match status.state {
        LinkState::Idle => "not connected",
        LinkState::Connecting => "connecting",
        LinkState::Connected => "connected",
        LinkState::Pairing => "pairing",
    }
}

pub fn describe(status: &Status) -> String {
    let mut lines = Vec::new();
    let band = status.band.as_deref().unwrap_or("no band paired");
    lines.push(format!("{band}: {}", state(status)));
    if let Some(level) = status.battery {
        let charging = if status.charging == Some(true) { ", charging" } else { "" };
        lines.push(format!("battery {level}%{charging}"));
    }
    if let Some(hand) = &status.hand {
        lines.push(format!("hand: {hand}"));
    }
    let dial = match status.dial {
        DialTarget::Volume => "volume",
        DialTarget::Brightness => "brightness",
        DialTarget::Off => "off",
    };
    lines.push(format!("dial: {dial}{}", if status.paused { " · controls paused" } else { "" }));
    if let Some(gesture) = &status.last_gesture {
        lines.push(format!("last gesture: {gesture} ({} this run)", status.gestures));
    }
    if let Some(stage) = &status.pairing {
        lines.push(format!("pairing: {stage}"));
    }
    if let Some(seconds) = status.reconnect_in {
        lines.push(format!("reconnecting in {seconds} s"));
    }
    for (label, message) in [("error", &status.error), ("pairing failed", &status.pairing_error), ("config", &status.config_error)] {
        if let Some(message) = message {
            lines.push(format!("{label}: {message}"));
        }
    }
    lines.join("\n")
}
```

`kinesisctl/src/main.rs` (commands grow in later tasks):
```rust
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use kinesisd::config::DialTarget;
use kinesisd::ipc::{HandChoice, Request};
use kinesisd::status::LinkState;

#[derive(Parser)]
#[command(name = "kinesisctl", version, about = "Control the Meta Neural Band daemon (kinesisd)")]
struct Cli {
    /// Talk to the `kinesisd --dry-run` daemon instead of the real one.
    #[arg(long, global = true)]
    dry_run: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
enum DialArg {
    Volume,
    Brightness,
    Off,
    Cycle,
}

#[derive(Clone, Copy, ValueEnum)]
enum HandArg {
    Left,
    Right,
}

#[derive(Subcommand)]
enum Command {
    /// Show the band's state.
    Status,
    /// Connect to the paired band.
    Connect,
    /// Disconnect from the band.
    Disconnect,
    /// Connect if idle, else disconnect.
    ToggleConnection,
    /// Stop running gesture actions (the band stays connected).
    Pause,
    /// Run gesture actions again.
    Resume,
    /// Pause or resume.
    Toggle,
    /// Choose what the pinch-and-turn dial controls.
    Dial { target: DialArg },
    /// Tell the band which wrist it's on.
    Hand { hand: HandArg },
    /// Forget the band (keys, record, Bluetooth bond). Factory reset it afterwards.
    Forget,
}

fn socket(dry_run: bool) -> PathBuf {
    kinesisd::ipc::socket_path(dry_run)
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio runtime")
}

fn print_status_of(response: kinesisd::ipc::Response) {
    if let Some(message) = &response.message {
        println!("{message}");
    }
    if let Some(status) = &response.status {
        println!("{}", kinesisctl::text::describe(status));
    }
}

fn simple(socket: &std::path::Path, request: Request) -> ExitCode {
    match runtime().block_on(kinesisctl::client::request(socket, &request)) {
        Ok(response) => {
            print_status_of(response);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let socket = socket(cli.dry_run);
    match cli.command {
        Command::Status => simple(&socket, Request::Status),
        Command::Connect => simple(&socket, Request::Connect),
        Command::Disconnect => simple(&socket, Request::Disconnect),
        Command::ToggleConnection => {
            let state = runtime().block_on(kinesisctl::client::status(&socket)).map(|s| s.state);
            match state {
                Ok(LinkState::Idle) => simple(&socket, Request::Connect),
                Ok(_) => simple(&socket, Request::Disconnect),
                Err(error) => {
                    eprintln!("{error}");
                    ExitCode::FAILURE
                }
            }
        }
        Command::Pause => simple(&socket, Request::Pause),
        Command::Resume => simple(&socket, Request::Resume),
        Command::Toggle => simple(&socket, Request::TogglePause),
        Command::Dial { target } => simple(
            &socket,
            match target {
                DialArg::Volume => Request::Dial { target: DialTarget::Volume },
                DialArg::Brightness => Request::Dial { target: DialTarget::Brightness },
                DialArg::Off => Request::Dial { target: DialTarget::Off },
                DialArg::Cycle => Request::CycleDial,
            },
        ),
        Command::Hand { hand } => simple(
            &socket,
            Request::Hand { hand: match hand { HandArg::Left => HandChoice::Left, HandArg::Right => HandChoice::Right } },
        ),
        Command::Forget => simple(&socket, Request::Forget),
    }
}
```

- [ ] **Step 6: Run the tests**

Run: `cargo test -p kinesisctl --test commands && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`
Expected: `7 passed`; clean. Also `cargo run -p kinesisctl -- --help` lists the commands.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock kinesisctl kinesisd/src/ipc.rs
git commit -F - <<'EOF'
feat(kinesisctl): socket client and simple control commands

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01VsiceY7FhkoM7aRuA9DEfz
EOF
```

---

### Task 2: waybar output

`kinesisctl status --waybar [--follow]` prints waybar `return-type: json` lines: `{"text","tooltip","class","alt"}`. With `--follow` it keeps a `subscribe` connection, prints a line on every change (deduplicated), and while kinesisd is down prints an "off" line and retries every 2 s. Classes: `connected`, `paused`, `pairing`, `connecting`, `off`, plus `low` when battery < 20 %.

**Files:**
- Create: `kinesisctl/src/waybar.rs`, `kinesisctl/tests/waybar.rs`
- Modify: `kinesisctl/src/lib.rs`, `kinesisctl/src/main.rs`

- [ ] **Step 1: Write the failing tests**

`kinesisctl/tests/waybar.rs`:
```rust
mod support;

use band_core::responder::SimMode;
use kinesisctl::waybar::{line, next_lines};
use kinesisd::status::{LinkState, PairResult, Status};

const HAND: &str = "\u{f256}";
const BOLT: &str = "\u{f0e7}";

#[test]
fn a_connected_band_shows_its_battery() {
    let status = Status { state: LinkState::Connected, battery: Some(64), band: Some("Meta Band".into()), ..Status::default() };
    let json = line(Some(&status));
    assert_eq!(json["text"], format!("{HAND} 64%"));
    assert_eq!(json["class"], "connected");
    assert!(json["tooltip"].as_str().unwrap().contains("Meta Band"));
}

#[test]
fn charging_low_paused_and_pairing_have_their_own_look() {
    let charging = Status { state: LinkState::Connected, battery: Some(12), charging: Some(true), ..Status::default() };
    let json = line(Some(&charging));
    assert_eq!(json["text"], format!("{HAND} 12% {BOLT}"));
    assert_eq!(json["class"], serde_json::json!(["connected", "low"]));
    let paused = Status { state: LinkState::Connected, paused: true, battery: Some(80), ..Status::default() };
    assert_eq!(line(Some(&paused))["class"], "paused");
    let pairing = Status { state: LinkState::Pairing, pairing: Some("confirming the claim with Meta".into()), ..Status::default() };
    let json = line(Some(&pairing));
    assert_eq!(json["class"], "pairing");
    assert!(json["tooltip"].as_str().unwrap().contains("confirming the claim"));
}

#[test]
fn idle_or_missing_daemon_is_off_and_errors_go_in_the_tooltip() {
    let idle = Status { error: Some("no band paired yet — run `kinesisctl pair`".into()), ..Status::default() };
    let json = line(Some(&idle));
    assert_eq!(json["text"], HAND);
    assert_eq!(json["class"], "off");
    assert!(json["tooltip"].as_str().unwrap().contains("kinesisctl pair"));
    let gone = line(None);
    assert_eq!(gone["class"], "off");
    assert!(gone["tooltip"].as_str().unwrap().contains("kinesisd isn't running"));
    let failed = Status {
        state: LinkState::Connected,
        battery: Some(50),
        last_pair: Some(PairResult { ok: false, message: "no band found".into() }),
        pairing_error: Some("no band found".into()),
        ..Status::default()
    };
    let json = line(Some(&failed));
    assert_eq!(json["class"], "connected", "a note doesn't make a working band look broken");
    assert!(json["tooltip"].as_str().unwrap().contains("no band found"));
}

#[tokio::test]
async fn follow_emits_a_line_per_change() {
    let daemon = support::start(SimMode::Enrolled, true).await;
    let mut lines = next_lines(daemon.socket.clone());
    let mut classes = Vec::new();
    while !classes.iter().any(|class: &String| class.contains("connected")) {
        let line = tokio::time::timeout(std::time::Duration::from_secs(20), lines.recv()).await.unwrap().unwrap();
        let json: serde_json::Value = serde_json::from_str(&line).unwrap();
        classes.push(json["class"].to_string());
    }
    assert_ne!(classes.first().map(String::as_str), Some("\"connected\""), "starts from the pre-connect state");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p kinesisctl --test waybar` — FAIL (unresolved `kinesisctl::waybar`).

- [ ] **Step 3: Implement**

`kinesisctl/src/waybar.rs`:
```rust
//! The waybar `custom/kinesis` module: one JSON line per state.

use std::path::PathBuf;
use std::time::Duration;

use kinesisd::config::DialTarget;
use kinesisd::status::{LinkState, Status};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::client::Subscription;

/// Font Awesome hand (in Nerd Fonts) and bolt.
const HAND: &str = "\u{f256}";
const BOLT: &str = "\u{f0e7}";

pub fn line(status: Option<&Status>) -> Value {
    let Some(status) = status else {
        return json!({
            "text": HAND,
            "tooltip": "kinesisd isn't running (systemctl --user start kinesisd)",
            "class": "off",
            "alt": "off",
        });
    };
    let mut text = HAND.to_string();
    if status.state == LinkState::Connected
        && let Some(level) = status.battery
    {
        text.push_str(&format!(" {level}%"));
        if status.charging == Some(true) {
            text.push(' ');
            text.push_str(BOLT);
        }
    }
    let base = match status.state {
        LinkState::Connected if status.paused => "paused",
        LinkState::Connected => "connected",
        LinkState::Connecting => "connecting",
        LinkState::Pairing => "pairing",
        LinkState::Idle => "off",
    };
    let low = status.state == LinkState::Connected && status.battery.is_some_and(|level| level < 20);
    let class = if low { json!([base, "low"]) } else { json!(base) };
    json!({ "text": text, "tooltip": tooltip(status), "class": class, "alt": base })
}

fn tooltip(status: &Status) -> String {
    let mut lines = vec![status.band.clone().unwrap_or_else(|| "Neural Band".into())];
    lines.push(match status.state {
        LinkState::Idle => "not connected".into(),
        LinkState::Connecting => "connecting…".into(),
        LinkState::Connected if status.paused => "connected · controls paused".into(),
        LinkState::Connected => "connected".into(),
        LinkState::Pairing => format!("pairing: {}", status.pairing.as_deref().unwrap_or("…")),
    });
    if let Some(hand) = &status.hand {
        lines.push(format!("hand: {hand}"));
    }
    let dial = match status.dial {
        DialTarget::Volume => "volume",
        DialTarget::Brightness => "brightness",
        DialTarget::Off => "off",
    };
    lines.push(format!("dial: {dial}"));
    if let Some(gesture) = &status.last_gesture {
        lines.push(format!("last: {gesture} ({})", status.gestures));
    }
    if let Some(seconds) = status.reconnect_in {
        lines.push(format!("reconnecting in {seconds} s"));
    }
    for message in [&status.error, &status.pairing_error, &status.config_error].into_iter().flatten() {
        lines.push(format!("⚠ {message}"));
    }
    lines.push("click: connect/disconnect · right: pause · middle: dial".into());
    lines.join("\n")
}

/// Status lines for waybar, forever: re-subscribes every 2 s while kinesisd is down.
pub fn next_lines(socket: PathBuf) -> mpsc::Receiver<String> {
    let (sender, receiver) = mpsc::channel(16);
    tokio::spawn(async move {
        let mut last = String::new();
        loop {
            match Subscription::status(&socket).await {
                Ok(mut subscription) => {
                    while let Some(status) = subscription.next_status().await {
                        let text = line(Some(&status)).to_string();
                        if text != last {
                            last = text.clone();
                            if sender.send(text).await.is_err() {
                                return;
                            }
                        }
                    }
                }
                Err(_) => {}
            }
            let text = line(None).to_string();
            if text != last {
                last = text.clone();
                if sender.send(text).await.is_err() {
                    return;
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
    receiver
}
```

Add `pub mod waybar;` to `lib.rs`. In `main.rs` replace `Status` with:
```rust
    /// Show the band's state.
    Status {
        /// Print a waybar JSON line instead of text.
        #[arg(long)]
        waybar: bool,
        /// Keep printing a line on every change (waybar `exec` without `interval`).
        #[arg(long, requires = "waybar")]
        follow: bool,
    },
```
and handle it:
```rust
        Command::Status { waybar: false, .. } => simple(&socket, Request::Status),
        Command::Status { waybar: true, follow: false } => {
            let status = runtime().block_on(kinesisctl::client::status(&socket)).ok();
            println!("{}", kinesisctl::waybar::line(status.as_ref()));
            ExitCode::SUCCESS
        }
        Command::Status { waybar: true, follow: true } => runtime().block_on(async {
            use std::io::Write;
            let mut lines = kinesisctl::waybar::next_lines(socket.clone());
            while let Some(line) = lines.recv().await {
                let mut out = std::io::stdout().lock();
                if writeln!(out, "{line}").and_then(|_| out.flush()).is_err() {
                    break; // waybar went away
                }
            }
            ExitCode::SUCCESS
        }),
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p kinesisctl --test waybar` — `4 passed`.

- [ ] **Step 5: Commit**

```bash
git add kinesisctl
git commit -F - <<'EOF'
feat(kinesisctl): waybar output with follow mode

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01VsiceY7FhkoM7aRuA9DEfz
EOF
```

---

### Task 3: Meta sign-in HTTP steps (`metaauth.rs`)

Port of kinesis `MetaAuth.swift`: (1) anonymous `webview_tokens_query` (multipart, FRL client) → `native_sso_token` + `native_sso_etoken`; the user signs in at `auth.meta.com/?native_app_id=…&native_sso_etoken=<etoken>`; the page redirects to `fb-viewapp://…frl_login?token=<16 hex>&blob=<…>`; (2) check `token` == first 16 hex of SHA-256(native_sso_token) in constant time; (3) `webview_blobs_decrypt` → FRL access token; (4) `ar-genai.graph.meta.com/login` (form, `Authorization: OAuth <AR client>`) → `access_token` + `user_id` = the session saved as `meta-session.json`.

**Files:**
- Create: `kinesisctl/src/metaauth.rs`, `kinesisctl/tests/metaauth.rs`
- Modify: `kinesisctl/src/lib.rs`

- [ ] **Step 1: Write the failing tests** (port of kinesis `MetaPairClientTests.swift` sign-in parts)

`kinesisctl/tests/metaauth.rs`:
```rust
use kinesisctl::metaauth::{
    auth_entry_url, callback_matches, expected_callback_token, is_callback, jazoest, make_lsd, multipart_body,
    parse_callback, parse_login, parse_tokens,
};
use serde_json::json;

#[test]
fn lsd_follows_the_reference_shape() {
    let lsd = make_lsd();
    assert_eq!(lsd.len(), 9);
    assert!(lsd.starts_with("S0."));
    assert!(lsd[3..].chars().all(|c| c.is_ascii_digit()));
    let sum: u32 = lsd.chars().map(|c| c as u32).sum();
    assert_eq!(jazoest(&lsd), format!("2{sum}"));
}

#[test]
fn multipart_bodies_wrap_every_field_once() {
    let body = String::from_utf8(multipart_body(&[("blob", "b"), ("lsd", "S0.1")], "boundary")).unwrap();
    let expected = "--boundary\r\nContent-Disposition: form-data; name=\"blob\"\r\n\r\nb\r\n\
                    --boundary\r\nContent-Disposition: form-data; name=\"lsd\"\r\n\r\nS0.1\r\n--boundary--\r\n";
    assert_eq!(body, expected);
}

#[test]
fn callback_validation_compares_the_truncated_token_digest() {
    let native = "epfEORXEWkN2obB5";
    let expected = expected_callback_token(native);
    assert_eq!(expected.len(), 16);
    assert!(callback_matches(Some(&expected), native));
    let mut wrong = expected.clone();
    wrong.pop();
    wrong.push(if expected.ends_with('0') { '1' } else { '0' });
    assert!(!callback_matches(Some(&wrong), native));
    assert!(!callback_matches(None, native));
    assert!(!callback_matches(Some("short"), native));
}

#[test]
fn the_callback_is_recognized_and_parsed() {
    let url = "fb-viewapp://frl_login?token=0123456789abcdef&blob=AbC%2B%2F%3D";
    assert!(is_callback(url));
    assert!(is_callback("https://m.meta.com/frl_login/?token=x&blob=y"));
    assert!(!is_callback("https://auth.meta.com/login"));
    let (token, blob) = parse_callback(url).unwrap();
    assert_eq!(token.as_deref(), Some("0123456789abcdef"));
    assert_eq!(blob.as_deref(), Some("AbC+/="));
}

#[test]
fn responses_parse_or_fail_clearly() {
    let tokens = parse_tokens(&json!({ "native_sso_token": "n", "native_sso_etoken": "e" })).unwrap();
    assert_eq!((tokens.native_sso_token.as_str(), tokens.etoken.as_str()), ("n", "e"));
    assert!(auth_entry_url(&tokens).ends_with("native_sso_etoken=e"));
    assert!(auth_entry_url(&tokens).starts_with("https://auth.meta.com/?native_app_id=388177446008673"));
    assert!(parse_tokens(&json!({ "native_sso_token": "" })).is_err());
    let session = parse_login(&json!({ "access_token": "t", "user_id": 42 }), "device").unwrap();
    assert_eq!((session.access_token.as_str(), session.user_id.as_str()), ("t", "42"));
    assert!(parse_login(&json!({ "access_token": "t" }), "device").is_err());
}
```

- [ ] **Step 2: Run to verify failure** — `cargo test -p kinesisctl --test metaauth` FAILS (unresolved module).

- [ ] **Step 3: Implement**

`kinesisctl/src/metaauth.rs`:
```rust
//! Meta sign-in HTTP steps (kinesis `MetaAuth.swift`). The client ids below
//! are Meta's own public app identifiers (the same in every copy of Meta's
//! apps), not secrets. The resulting access token never reaches logs.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use kinesisd::meta::{MetaSession, url_form};
use rand_core::{OsRng, RngCore};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const FRL_CLIENT: &str = "FRL|388177446008673|083800dd7efbbd42eab18c9886d79c18";
pub const AR_CLIENT: &str = "AR|306760944872162|a919421a55a8ea18080ab2f10f57f1be";
const TOKENS_QUERY_URL: &str = "https://meta.graph.meta.com/webview_tokens_query";
const BLOBS_DECRYPT_URL: &str = "https://meta.graph.meta.com/webview_blobs_decrypt";
const LOGIN_URL: &str = "https://ar-genai.graph.meta.com/login";
const AUTH_ENTRY_BASE: &str =
    "https://auth.meta.com/?native_app_id=388177446008673&source_app_id=388177446008673&native_sso_etoken=";

pub struct SsoTokens {
    pub native_sso_token: String,
    pub etoken: String,
}

pub fn auth_entry_url(tokens: &SsoTokens) -> String {
    format!("{AUTH_ENTRY_BASE}{}", tokens.etoken)
}

pub fn make_lsd() -> String {
    let mut digits = [0u8; 6];
    OsRng.fill_bytes(&mut digits);
    format!("S0.{}", digits.iter().map(|d| char::from(b'0' + d % 10)).collect::<String>())
}

pub fn jazoest(lsd: &str) -> String {
    format!("2{}", lsd.chars().map(|c| c as u32).sum::<u32>())
}

pub fn multipart_body(fields: &[(&str, &str)], boundary: &str) -> Vec<u8> {
    let mut body = Vec::new();
    for (name, value) in fields {
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes());
        body.extend_from_slice(value.as_bytes());
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

/// The token the login callback must present: the first 16 hex digits of SHA-256(native token).
pub fn expected_callback_token(native_sso_token: &str) -> String {
    Sha256::digest(native_sso_token.as_bytes()).iter().map(|b| format!("{b:02x}")).collect::<String>()[..16].to_string()
}

pub fn callback_matches(token: Option<&str>, native_sso_token: &str) -> bool {
    let Some(token) = token.filter(|t| t.len() == 16) else { return false };
    let expected = expected_callback_token(native_sso_token);
    token.bytes().zip(expected.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

/// The login finishes with a custom-scheme redirect carrying the token and the encrypted blob.
pub fn is_callback(url: &str) -> bool {
    url.to_ascii_lowercase().starts_with("fb-viewapp:") || url.contains("frl_login")
}

/// `(token, blob)` from the callback URL's query.
pub fn parse_callback(url: &str) -> Option<(Option<String>, Option<String>)> {
    let parsed = url::Url::parse(url).ok()?;
    let mut token = None;
    let mut blob = None;
    for (name, value) in parsed.query_pairs() {
        match name.as_ref() {
            "token" => token = Some(value.into_owned()),
            "blob" => blob = Some(value.into_owned()),
            _ => {}
        }
    }
    Some((token, blob))
}

fn text(json: &Value, key: &str) -> Option<String> {
    json.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned)
}

pub fn parse_tokens(json: &Value) -> Result<SsoTokens, String> {
    match (text(json, "native_sso_token"), text(json, "native_sso_etoken")) {
        (Some(native_sso_token), Some(etoken)) => Ok(SsoTokens { native_sso_token, etoken }),
        _ => Err("meta didn't return sign-in tokens. try again.".into()),
    }
}

pub fn parse_login(json: &Value, device_id: &str) -> Result<MetaSession, String> {
    let token = text(json, "access_token");
    let user = text(json, "user_id").or_else(|| json.get("user_id").and_then(Value::as_u64).map(|id| id.to_string()));
    let (Some(access_token), Some(user_id)) = (token, user) else {
        return Err("meta didn't return an account session. try again.".into());
    };
    let mut session = MetaSession::new(access_token, user_id);
    session.device_id = device_id.to_owned();
    session.obtained_at = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    Ok(session)
}

fn http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("could not start an HTTP client: {error}"))
}

async fn finish(response: reqwest::Result<reqwest::Response>) -> Result<Value, String> {
    let response = response.map_err(|error| format!("couldn't reach meta: {error}"))?;
    let status = response.status().as_u16();
    let json: Option<Value> = response.json().await.ok();
    let json = match json {
        Some(json) if status == 200 => json,
        _ => return Err(format!("meta sign-in failed (http {status}). try again.")),
    };
    if let Some(error) = json.get("error") {
        let code = error.get("code").and_then(Value::as_i64).map_or("-".into(), |c| c.to_string());
        let sub = error.get("error_subcode").and_then(Value::as_i64).map_or("-".into(), |c| c.to_string());
        return Err(format!("meta sign-in failed (code {code}, subcode {sub}). try again."));
    }
    Ok(json)
}

async fn post_multipart(url: &str, fields: &[(&str, &str)]) -> Result<Value, String> {
    let boundary = format!("kinesis.form.{}", uuid::Uuid::new_v4());
    finish(
        http()?
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, format!("multipart/form-data; boundary={boundary}"))
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::ORIGIN, "https://auth.meta.com")
            .body(multipart_body(fields, &boundary))
            .send()
            .await,
    )
    .await
}

/// Step 1: the anonymous tokens query.
pub async fn tokens_query() -> Result<SsoTokens, String> {
    let lsd = make_lsd();
    let jaz = jazoest(&lsd);
    parse_tokens(&post_multipart(TOKENS_QUERY_URL, &[("access_token", FRL_CLIENT), ("lsd", &lsd), ("jazoest", &jaz)]).await?)
}

/// Step 3: decrypt the login blob into the FRL access token.
pub async fn decrypt_blob(blob: &str, request_token: &str) -> Result<String, String> {
    let lsd = make_lsd();
    let jaz = jazoest(&lsd);
    let fields = [("blob", blob), ("request_token", request_token), ("access_token", FRL_CLIENT), ("lsd", &lsd), ("jazoest", &jaz)];
    text(&post_multipart(BLOBS_DECRYPT_URL, &fields).await?, "access_token")
        .ok_or_else(|| "meta didn't confirm the sign-in. try again.".into())
}

/// Step 4: exchange the FRL token for the `ar` user session.
pub async fn login(frl_access_token: &str) -> Result<MetaSession, String> {
    let device_id = uuid::Uuid::new_v4().to_string();
    let fields = vec![
        ("frl_access_token", frl_access_token.to_owned()),
        ("logging_session_id", uuid::Uuid::new_v4().to_string()),
        ("format", "json".to_owned()),
        ("device_id", device_id.clone()),
        ("generate_session_cookies", "1".to_owned()),
        ("generate_analytics_claim", "1".to_owned()),
        ("method", "POST".to_owned()),
    ];
    let json = finish(
        http()?
            .post(LOGIN_URL)
            .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::AUTHORIZATION, format!("OAuth {AR_CLIENT}"))
            .body(url_form(&fields))
            .send()
            .await,
    )
    .await?;
    parse_login(&json, &device_id)
}
```

Check `MetaSession`'s field names in `kinesisd/src/meta.rs` (`device_id`, `obtained_at` are `pub`); adapt if they differ. Add `pub mod metaauth;` to `lib.rs`.

- [ ] **Step 4: Run the tests** — `cargo test -p kinesisctl --test metaauth`: `5 passed`.

- [ ] **Step 5: Commit**

```bash
git add kinesisctl
git commit -F - <<'EOF'
feat(kinesisctl): Meta sign-in HTTP steps (port of kinesis MetaAuth)

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01VsiceY7FhkoM7aRuA9DEfz
EOF
```

---

### Task 4: The sign-in window and `kinesisctl signin` / `signout`

A `tao` window with a `wry` WebKitGTK view: incognito, loads the auth entry URL, allows only `https:` and `about:blank` navigations, cancels the callback navigation and hands its URL back, and quits on close. Runs on the main thread (so `main` is synchronous and uses short-lived tokio runtimes around the HTTP steps).

**Files:**
- Create: `kinesisctl/src/signin.rs`
- Modify: `kinesisctl/src/lib.rs`, `kinesisctl/src/main.rs`

This window can't be tested headlessly here; the unit-testable parts (callback parsing/validation) are Task 3's. Verification is: it compiles, `kinesisctl signin --help` works, and on hardware day (Task 8) the user signs in for real.

- [ ] **Step 1: Implement**

`kinesisctl/src/signin.rs`:
```rust
//! The Meta sign-in window (kinesis `MetaLoginView.swift`): an incognito
//! WebKitGTK view on Meta's own page. It never sees the password's value;
//! it only intercepts the final redirect.

use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::platform::unix::WindowExtUnix;
use tao::window::WindowBuilder;
use wry::WebViewBuilderExtUnix;

use crate::metaauth::is_callback;

enum UserEvent {
    Callback(String),
}

/// Show Meta's sign-in page at `url`. Returns the callback URL, or `None` if
/// the window was closed first.
pub fn open(url: &str) -> Result<Option<String>, String> {
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let window = WindowBuilder::new()
        .with_title("Sign in with Meta — auth.meta.com (kinesis-linux)")
        .with_inner_size(tao::dpi::LogicalSize::new(640.0, 760.0))
        .build(&event_loop)
        .map_err(|error| format!("could not open the sign-in window: {error}"))?;
    let _webview = wry::WebViewBuilder::new()
        .with_url(url)
        .with_incognito(true)
        .with_navigation_handler(move |target: String| {
            if is_callback(&target) {
                let _ = proxy.send_event(UserEvent::Callback(target));
                return false;
            }
            target.starts_with("https://") || target == "about:blank"
        })
        .build_gtk(window.gtk_window())
        .map_err(|error| format!("could not start the web view: {error}"))?;
    let mut result = None;
    event_loop.run_return(|event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(UserEvent::Callback(target)) => {
                result = Some(target);
                *control_flow = ControlFlow::Exit;
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => *control_flow = ControlFlow::Exit,
            _ => {}
        }
    });
    Ok(result)
}
```

Adaptation notes (wry/tao drift is likely): keep the rules — incognito; only https/about:blank; the callback navigation is cancelled and its URL returned; closing the window returns `None`. If `with_navigation_handler` doesn't see the custom-scheme redirect on WebKitGTK, register `with_custom_protocol("fb-viewapp", …)` and capture the request URL there instead. Pop-ups (`window.open`) may be ignored.

Add `pub mod signin;` to `lib.rs`. In `main.rs`, add:
```rust
    /// Sign in to Meta (needed once to claim a band). Opens Meta's own page.
    Signin,
    /// Delete the saved Meta sign-in.
    Signout,
```
and a function used by both `signin` and `pair`:
```rust
/// The whole sign-in: tokens query → window → callback check → decrypt → login → save.
fn sign_in() -> Result<(), String> {
    println!("Opening Meta's sign-in page. You type your password there; kinesis only keeps the resulting session token.");
    let tokens = runtime().block_on(kinesisctl::metaauth::tokens_query())?;
    let url = kinesisctl::metaauth::auth_entry_url(&tokens);
    let Some(callback) = kinesisctl::signin::open(&url)? else {
        return Err("sign-in cancelled".into());
    };
    let (token, blob) = kinesisctl::metaauth::parse_callback(&callback).unwrap_or((None, None));
    let blob = blob.filter(|b| !b.is_empty());
    let Some(blob) = blob.filter(|_| kinesisctl::metaauth::callback_matches(token.as_deref(), &tokens.native_sso_token)) else {
        return Err("meta didn't confirm this sign-in. try again.".into());
    };
    let session = runtime().block_on(async {
        let frl = kinesisctl::metaauth::decrypt_blob(&blob, &tokens.native_sso_token).await?;
        kinesisctl::metaauth::login(&frl).await
    })?;
    let store = kinesisd::store::Store::open(kinesisd::store::Store::default_dir()).map_err(|e| e.to_string())?;
    store.save_meta_session(&session).map_err(|e| format!("could not save the sign-in: {e}"))?;
    println!("Signed in to Meta.");
    Ok(())
}
```
with the handlers:
```rust
        Command::Signin => match sign_in() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        },
        Command::Signout => {
            let result = kinesisd::store::Store::open(kinesisd::store::Store::default_dir()).and_then(|s| s.delete_meta_session());
            match result {
                Ok(()) => {
                    println!("Signed out of Meta.");
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("{error}");
                    ExitCode::FAILURE
                }
            }
        }
```
For `--dry-run`, the store dir must match the dry-run daemon's (`Store::default_dir().with_file_name("kinesis-dry-run")`); add a helper `fn store_dir(dry_run: bool) -> PathBuf` and use it everywhere kinesisctl opens the store.

- [ ] **Step 2: Build and smoke-check**

Run: `cargo build -p kinesisctl && cargo run -p kinesisctl -- signin --help && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`
Expected: builds, help prints. Do **not** run `signin` itself (that contacts Meta) — it's exercised on hardware day.

- [ ] **Step 3: Commit**

```bash
git add kinesisctl
git commit -F - <<'EOF'
feat(kinesisctl): Meta sign-in window, signin and signout commands

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01VsiceY7FhkoM7aRuA9DEfz
EOF
```

---

### Task 5: `kinesisctl pair`

Flow: if a band is paired → refuse unless `--replace`, which runs `forget`, prints the factory-reset instructions and waits for Enter. Then send `pair` (optionally `--address`). If the daemon answers "sign in to Meta first", run the sign-in and send `pair` again once. Then follow the status: print each new pairing stage; when the pairing ends print the result from `last_pair`; on success wait (≤ 60 s) for `connected`.

**Files:**
- Create: `kinesisctl/src/pair.rs`, `kinesisctl/tests/pair.rs`
- Modify: `kinesisctl/src/lib.rs`, `kinesisctl/src/main.rs`

- [ ] **Step 1: Write the failing tests** (in-process daemon with `FakeFactory`, so no sign-in is ever asked for)

`kinesisctl/tests/pair.rs`:
```rust
mod support;

use band_core::responder::SimMode;
use kinesisctl::pair::{Outcome, follow_pairing, start_pairing};
use kinesisd::status::LinkState;

#[tokio::test]
async fn a_pairing_is_followed_to_a_connected_owner() {
    let daemon = support::start(SimMode::Legacy, false).await;
    daemon.wait("the unpaired error", |s| s.error.is_some()).await;
    start_pairing(&daemon.socket, None).await.unwrap();
    let mut stages = Vec::new();
    let outcome = follow_pairing(&daemon.socket, |stage| stages.push(stage.to_owned())).await.unwrap();
    assert!(matches!(outcome, Outcome::Paired { ref band } if band.contains("simulated")), "{outcome:?}");
    assert!(stages.iter().any(|s| s.contains("Meta")), "{stages:?}");
    assert_eq!(kinesisctl::client::status(&daemon.socket).await.unwrap().state, LinkState::Connected);
}

#[tokio::test]
async fn pairing_is_refused_while_a_band_is_paired() {
    let daemon = support::start(SimMode::Enrolled, true).await;
    let error = start_pairing(&daemon.socket, None).await.unwrap_err();
    assert!(error.contains("forget"), "{error}");
}

#[tokio::test]
async fn a_failed_pairing_is_reported() {
    let daemon = support::start(SimMode::Legacy, false).await;
    daemon.transport.set_reject_claim(Some(0x03001042));
    start_pairing(&daemon.socket, None).await.unwrap();
    let outcome = follow_pairing(&daemon.socket, |_| {}).await.unwrap();
    assert!(matches!(outcome, Outcome::Failed { ref message } if message.contains("different meta account")), "{outcome:?}");
}

#[test]
fn a_sign_in_request_is_recognized() {
    assert!(kinesisctl::pair::needs_sign_in("sign in to Meta first (`kinesisctl pair` opens the sign-in window)"));
    assert!(!kinesisctl::pair::needs_sign_in("a pairing is already running"));
}
```

- [ ] **Step 2: Run to verify failure** — FAILS (unresolved `kinesisctl::pair`).

- [ ] **Step 3: Implement**

`kinesisctl/src/pair.rs`:
```rust
//! Driving a pairing through kinesisd and reporting its progress.

use std::path::Path;
use std::time::Duration;

use kinesisd::ipc::Request;
use kinesisd::status::LinkState;

use crate::client::{self, Subscription};

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Paired { band: String },
    Failed { message: String },
}

/// The daemon asks for a Meta sign-in with this wording (kinesisd `start_pairing`).
pub fn needs_sign_in(error: &str) -> bool {
    error.to_ascii_lowercase().contains("sign in to meta")
}

pub async fn start_pairing(socket: &Path, address: Option<String>) -> Result<(), String> {
    client::request(socket, &Request::Pair { address }).await.map(|_| ())
}

/// Follow the status until the pairing ends; on success also wait for the owner connection.
pub async fn follow_pairing(socket: &Path, mut on_stage: impl FnMut(&str)) -> Result<Outcome, String> {
    let mut subscription = Subscription::status(socket).await?;
    let mut last_stage = String::new();
    let mut seen_pairing = false;
    let outcome = loop {
        let status = tokio::time::timeout(Duration::from_secs(240), subscription.next_status())
            .await
            .map_err(|_| "the pairing took too long".to_string())?
            .ok_or_else(|| "kinesisd went away during the pairing".to_string())?;
        if status.state == LinkState::Pairing {
            seen_pairing = true;
            if let Some(stage) = &status.pairing
                && *stage != last_stage
            {
                last_stage = stage.clone();
                on_stage(stage);
            }
            continue;
        }
        if let Some(result) = &status.last_pair {
            break if result.ok {
                Outcome::Paired { band: status.band.clone().unwrap_or_else(|| "the band".into()) }
            } else {
                Outcome::Failed { message: result.message.clone() }
            };
        }
    };
    if let Outcome::Paired { .. } = &outcome {
        on_stage("connecting as the new owner");
        let connected = tokio::time::timeout(Duration::from_secs(60), async {
            while let Some(status) = subscription.next_status().await {
                if status.state == LinkState::Connected {
                    return true;
                }
            }
            false
        })
        .await
        .unwrap_or(false);
        if !connected {
            return Ok(Outcome::Failed {
                message: "the band was claimed but didn't connect yet; kinesisd keeps retrying (see `kinesisctl status`)".into(),
            });
        }
    }
    Ok(outcome)
}
```

Add `pub mod pair;` to `lib.rs`. In `main.rs` add the command:
```rust
    /// Claim a band in pairing mode for this computer (signs in to Meta if needed).
    Pair {
        /// A specific band's Bluetooth address (default: the first one found).
        #[arg(long)]
        address: Option<String>,
        /// Forget the currently paired band first.
        #[arg(long)]
        replace: bool,
    },
```
and its handler:
```rust
fn pair(socket: &std::path::Path, address: Option<String>, replace: bool) -> ExitCode {
    let rt = runtime();
    let paired = rt.block_on(kinesisctl::client::status(socket)).map(|s| s.band.is_some());
    match paired {
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
        Ok(true) if !replace => {
            eprintln!("a band is already paired. to claim a different (or reset) band: `kinesisctl pair --replace`");
            return ExitCode::FAILURE;
        }
        Ok(true) => {
            if let Err(error) = rt.block_on(kinesisctl::client::request(socket, &Request::Forget)) {
                eprintln!("{error}");
                return ExitCode::FAILURE;
            }
            println!("Forgot the old band. Now factory reset the band ({}),", band_core::ceremony::FACTORY_RESET_HINT);
            println!("then hold its button for 3 seconds until it flashes (pairing mode), and press Enter.");
            let _ = std::io::stdin().read_line(&mut String::new());
        }
        Ok(false) => {
            println!("Put the band in pairing mode: hold its button for 3 seconds until it flashes.");
        }
    }
    let mut started = rt.block_on(kinesisctl::pair::start_pairing(socket, address.clone()));
    if let Err(error) = &started
        && kinesisctl::pair::needs_sign_in(error)
    {
        drop(rt);
        if let Err(error) = sign_in() {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
        started = runtime().block_on(kinesisctl::pair::start_pairing(socket, address));
    }
    if let Err(error) = started {
        eprintln!("{error}");
        return ExitCode::FAILURE;
    }
    match runtime().block_on(kinesisctl::pair::follow_pairing(socket, |stage| println!("… {stage}"))) {
        Ok(kinesisctl::pair::Outcome::Paired { band }) => {
            println!("Paired with {band}. It's yours now — try a swipe.");
            ExitCode::SUCCESS
        }
        Ok(kinesisctl::pair::Outcome::Failed { message }) => {
            eprintln!("pairing failed: {message}");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
```
(`sign_in()` runs `runtime().block_on` internally and the window on the main thread — drop any runtime you hold first, as above, so no runtime is running while GTK owns the thread.)

Once the state leaves `pairing` and `last_pair` is set, the pairing is over; the daemon clears `last_pair` when a pairing starts, so an old result can't be mistaken for the new one. (`seen_pairing` is kept only for readability of the stage handling; drop it if clippy flags it unused.)

- [ ] **Step 4: Run the tests** — `cargo test -p kinesisctl --test pair`: `4 passed`.

- [ ] **Step 5: Commit**

```bash
git add kinesisctl
git commit -F - <<'EOF'
feat(kinesisctl): pair command with on-demand sign-in and progress

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01VsiceY7FhkoM7aRuA9DEfz
EOF
```

---

### Task 6: `kinesisctl semg record` and `semg view`

`record FILE [--seconds N]`: turns sEMG on, subscribes, writes one JSON line per frame `{"t_ms": <unix ms>, "payload": "<base64 raw payload>"}` until Ctrl-C or N seconds, then turns sEMG off. `view`: a ratatui screen with 8 channel sparklines (last ~2 s at 2048 Hz, decimated to the terminal width), frames/s and dropped-batch count from sequence gaps; `q`/`Esc` quits (sEMG turned off on exit; the daemon also turns it off 3 s after the last viewer leaves).

**Files:**
- Create: `kinesisctl/src/semg.rs`, `kinesisctl/tests/semg.rs`
- Modify: `kinesisctl/src/lib.rs`, `kinesisctl/src/main.rs`

- [ ] **Step 1: Write the failing tests**

`kinesisctl/tests/semg.rs`:
```rust
mod support;

use std::time::Duration;

use band_core::responder::SimMode;
use kinesisctl::semg::{Channels, record};
use kinesisd::status::LinkState;
use kinesisd::transport::SimInject;

#[test]
fn batches_decode_into_eight_channels_and_gaps_are_counted() {
    let mut channels = Channels::new(64);
    let samples: Vec<u8> = (0..128u16).flat_map(|i| (1000 + i).to_le_bytes()).collect();
    let payload = [
        band_core::proto::field_int(1, 7),
        band_core::proto::field_int(2, 1_000),
        band_core::proto::field_bytes(3, &samples),
    ]
    .concat();
    channels.push(&payload).unwrap();
    assert_eq!(channels.channel(0)[..2], [1000, 1008]);
    assert_eq!(channels.channel(7)[0], 1007);
    let next = [band_core::proto::field_int(1, 10), band_core::proto::field_int(2, 2_000), band_core::proto::field_bytes(3, &samples)].concat();
    channels.push(&next).unwrap();
    assert_eq!(channels.dropped(), 2, "sequence 8 and 9 never arrived");
    assert!(channels.push(&[1, 2, 3]).is_err());
}

#[tokio::test]
async fn record_writes_one_line_per_frame() {
    let daemon = support::start(SimMode::Enrolled, true).await;
    daemon.wait("connected", |s| s.state == LinkState::Connected).await;
    let file = daemon.dir.path().join("rec.jsonl");
    let socket = daemon.socket.clone();
    let path = file.clone();
    let recording = tokio::spawn(async move { record(&socket, &path, Some(Duration::from_secs(3))).await });
    daemon.wait("sEMG on", |s| s.semg).await;
    for _ in 0..5 {
        daemon.transport.inject(SimInject::RawEmg);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let frames = recording.await.unwrap().unwrap();
    assert_eq!(frames, 5);
    let text = std::fs::read_to_string(&file).unwrap();
    assert_eq!(text.lines().count(), 5);
    let first: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert!(first["t_ms"].as_u64().unwrap() > 0 && first["payload"].as_str().unwrap().len() > 300);
    daemon.wait("sEMG off after recording", |s| !s.semg).await;
}
```

- [ ] **Step 2: Run to verify failure** — FAILS (unresolved `kinesisctl::semg`).

- [ ] **Step 3: Implement**

`kinesisctl/src/semg.rs`:
```rust
//! Raw sEMG: recording to JSONL and a live terminal view. Values are unscaled
//! ADC counts (8 channels × 16 samples per batch, 2048 Hz).

use std::collections::VecDeque;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use band_core::events::{EmgBatch, EmgConfig};
use base64::Engine;
use kinesisd::ipc::Request;

use crate::client::{self, Subscription};

const CONFIG: EmgConfig = EmgConfig { sample_rate: 2048, channels: 8, adc_bits: 16, samples_per_batch: 16, encoding: 0 };

/// Rolling per-channel sample buffers plus dropped-batch accounting.
pub struct Channels {
    capacity: usize,
    data: Vec<VecDeque<u16>>,
    last_sequence: Option<u64>,
    dropped: u64,
    frames: u64,
}

impl Channels {
    pub fn new(capacity: usize) -> Self {
        Self { capacity, data: vec![VecDeque::with_capacity(capacity); 8], last_sequence: None, dropped: 0, frames: 0 }
    }

    pub fn push(&mut self, payload: &[u8]) -> Result<(), String> {
        let batch = EmgBatch::parse(payload, &CONFIG).map_err(|error| error.to_string())?;
        if let Some(last) = self.last_sequence
            && batch.sequence > last + 1
        {
            self.dropped += batch.sequence - last - 1;
        }
        self.last_sequence = Some(batch.sequence);
        self.frames += 1;
        for (index, value) in batch.values.iter().enumerate() {
            let channel = &mut self.data[index % 8];
            if channel.len() == self.capacity {
                channel.pop_front();
            }
            channel.push_back(*value);
        }
        Ok(())
    }

    pub fn channel(&self, index: usize) -> Vec<u16> {
        self.data[index].iter().copied().collect()
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// Record frames to `path` (JSONL) until Ctrl-C or `limit`; returns the frame count.
pub async fn record(socket: &Path, path: &Path, limit: Option<Duration>) -> Result<u64, String> {
    let mut file = std::io::BufWriter::new(std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?);
    let mut subscription = Subscription::semg(socket).await?;
    client::request(socket, &Request::Semg { on: true }).await?;
    let deadline = limit.map(|limit| tokio::time::Instant::now() + limit);
    let mut frames = 0u64;
    let result = loop {
        let stop = async {
            match deadline {
                Some(deadline) => tokio::time::sleep_until(deadline).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = stop => break Ok(()),
            _ = tokio::signal::ctrl_c() => break Ok(()),
            frame = subscription.next_semg() => match frame {
                Some(payload) => {
                    let line = serde_json::json!({
                        "t_ms": now_ms(),
                        "payload": base64::engine::general_purpose::STANDARD.encode(&payload),
                    });
                    if let Err(error) = writeln!(file, "{line}") {
                        break Err(format!("{}: {error}", path.display()));
                    }
                    frames += 1;
                }
                None => break Err("kinesisd went away".to_string()),
            },
        }
    };
    let _ = client::request(socket, &Request::Semg { on: false }).await;
    file.flush().map_err(|e| format!("{}: {e}", path.display()))?;
    result.map(|()| frames)
}

/// The live view (runs until `q`/`Esc`/Ctrl-C).
pub async fn view(socket: &Path) -> Result<(), String> {
    use ratatui::crossterm::event::{self, Event, KeyCode};
    use ratatui::layout::{Constraint, Layout};
    use ratatui::widgets::{Block, Paragraph, Sparkline};

    let mut subscription = Subscription::semg(socket).await?;
    client::request(socket, &Request::Semg { on: true }).await?;
    let mut channels = Channels::new(4096);
    let mut terminal = ratatui::init();
    let started = tokio::time::Instant::now();
    let mut redraw = tokio::time::interval(Duration::from_millis(50));
    let result: Result<(), String> = loop {
        tokio::select! {
            frame = subscription.next_semg() => match frame {
                Some(payload) => { let _ = channels.push(&payload); }
                None => break Err("kinesisd went away".into()),
            },
            _ = redraw.tick() => {
                if event::poll(Duration::ZERO).unwrap_or(false)
                    && let Ok(Event::Key(key)) = event::read()
                    && matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
                {
                    break Ok(());
                }
                let elapsed = started.elapsed().as_secs_f64().max(0.001);
                let draw = terminal.draw(|frame| {
                    let rows = Layout::vertical([Constraint::Length(1); 9]).split(frame.area());
                    let header = format!(
                        "raw sEMG · {:.0} frames/s · {} dropped · q to quit",
                        channels.frames() as f64 / elapsed,
                        channels.dropped()
                    );
                    frame.render_widget(Paragraph::new(header), rows[0]);
                    let width = usize::from(frame.area().width.saturating_sub(6)).max(1);
                    for index in 0..8 {
                        let data: Vec<u64> = channels.channel(index).iter().rev().take(width).rev().map(|v| u64::from(*v)).collect();
                        frame.render_widget(Sparkline::default().block(Block::new().title(format!("{index} "))).data(&data), rows[index + 1]);
                    }
                });
                if let Err(error) = draw {
                    break Err(error.to_string());
                }
            }
        }
    };
    ratatui::restore();
    let _ = client::request(socket, &Request::Semg { on: false }).await;
    result
}
```

If `EmgConfig` isn't constructible as a `const` (private fields or no `const` literal), build it in a `fn config()` instead. Sparkline row layout: if the 1-line rows are too cramped, use `Constraint::Ratio(1, 8)` for channels — cosmetic. Add `pub mod semg;` to `lib.rs`. `main.rs`:
```rust
    /// Raw sEMG readings.
    Semg {
        #[command(subcommand)]
        what: SemgCommand,
    },
```
```rust
#[derive(Subcommand)]
enum SemgCommand {
    /// Live 8-channel view in the terminal.
    View,
    /// Record raw frames to a JSONL file until Ctrl-C (or --seconds).
    Record {
        file: PathBuf,
        #[arg(long)]
        seconds: Option<u64>,
    },
}
```
Handlers call `runtime().block_on(kinesisctl::semg::view(&socket))` / `record(&socket, &file, seconds.map(Duration::from_secs))` and print `recorded N frames to FILE`.

- [ ] **Step 4: Run the tests** — `cargo test -p kinesisctl --test semg`: `2 passed`. Then `cargo test --workspace`, clippy, fmt.

- [ ] **Step 5: Commit**

```bash
git add kinesisctl
git commit -F - <<'EOF'
feat(kinesisctl): sEMG record (JSONL) and live terminal view

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01VsiceY7FhkoM7aRuA9DEfz
EOF
```

---

### Task 7: arch_rice integration (Z13 profile)

**Repos:** `~/Projects/kinesis-linux` (merge `kinesisctl` → `master` first) and `~/arch_rice` (branch `multi-host-profiles`, which has the user's uncommitted WIP — commit **only** the files listed here, by explicit path).

- [ ] **Step 1: Land the code in kinesis-linux master**

```bash
cd ~/Projects/kinesis-linux && cargo test --workspace && git switch master && git merge --ff-only kinesisctl && git branch -d kinesisctl
```

- [ ] **Step 2: Point the arch_rice submodule at it (local, not pushed)**

```bash
cd ~/arch_rice/kinesis-linux && git fetch ~/Projects/kinesis-linux master && git checkout --detach FETCH_HEAD
cd ~/arch_rice && git -C kinesis-linux log --oneline -1
```

- [ ] **Step 3: Install the binaries** (user-level, no sudo)

```bash
cargo install --locked --path ~/arch_rice/kinesis-linux/kinesisd
cargo install --locked --path ~/arch_rice/kinesis-linux/kinesisctl
~/.cargo/bin/kinesisctl --version
```

- [ ] **Step 4: Create the new files first** (arch_rice is live)

`~/arch_rice/.local/bin/kinesis-dial` (mode 755):
```bash
#!/usr/bin/env bash
# One pinch-dial step for kinesisd, with the rice's OSD.
#   kinesis-dial volume up|down
#   kinesis-dial brightness up|down
OSD="$HOME/.local/bin/osd"
case "$1 $2" in
    "volume up")   wpctl set-volume -l 1.0 @DEFAULT_AUDIO_SINK@ 2%+ ;;
    "volume down") wpctl set-volume @DEFAULT_AUDIO_SINK@ 2%- ;;
    "brightness up")   exec "$HOME/.local/bin/rocker" bright-up ;;
    "brightness down") exec "$HOME/.local/bin/rocker" bright-down ;;
    *) echo "usage: kinesis-dial {volume|brightness} {up|down}" >&2; exit 2 ;;
esac
v=$(wpctl get-volume @DEFAULT_AUDIO_SINK@ | awk '{print int($2*100)}')
"$OSD" vol "󰕾  ${v}%" "$v"
```
Then `ln -sf ~/arch_rice/.local/bin/kinesis-dial ~/.local/bin/kinesis-dial`.

`~/arch_rice/.config/kinesis/config.toml`:
```toml
# kinesisd — Meta Neural Band gestures on the Z13 (see kinesis-linux/contrib/config.toml for every key).
# Reloaded on save.

[band]
hand = "band"
autostart = true

[gestures]
swipe_left    = "hyprctl dispatch workspace e-1"
swipe_right   = "hyprctl dispatch workspace e+1"
swipe_up      = "hyprctl dispatch exec 'wofi --show drun'"
swipe_down    = "hyprctl dispatch sendshortcut , Escape, activewindow"
index_double  = "playerctl play-pause"
middle_double = "~/.local/bin/audio-mute sink"
index_tap     = ""
middle_tap    = ""
middle_hold   = ""

[dial]
target = "volume"
sensitivity = 1.0
volume_up       = "~/.local/bin/kinesis-dial volume up"
volume_down     = "~/.local/bin/kinesis-dial volume down"
brightness_up   = "~/.local/bin/kinesis-dial brightness up"
brightness_down = "~/.local/bin/kinesis-dial brightness down"
```
(`hyprctl dispatch exec` launches wofi as Hyprland's child, so it doesn't live in kinesisd's cgroup.) Link it: `mkdir -p ~/.config/kinesis && ln -sf ~/arch_rice/.config/kinesis/config.toml ~/.config/kinesis/config.toml`. Validate it parses: `python3 -c 'import tomllib,os;tomllib.load(open(os.path.expanduser("~/arch_rice/.config/kinesis/config.toml"),"rb"))'` now, and after Step 6 the daemon itself must report no `config_error`.

Systemd unit: `mkdir -p ~/.config/systemd/user && ln -sf ~/arch_rice/kinesis-linux/contrib/kinesisd.service ~/.config/systemd/user/kinesisd.service && systemctl --user daemon-reload`.

- [ ] **Step 5: Wire references** (after Step 4)

`.gitignore` (append, keeping the whitelist style):
```
# kinesis-linux (Neural Band) config
!.config/kinesis
!.config/kinesis/**/*
```

`.config/waybar/profile.d/z13.jsonc`: add `"custom/kinesis"` to `modules-right` right before `"custom/earbuds"`, and the module:
```jsonc
    // Meta Neural Band (kinesisd) — status/battery, click connect/disconnect,
    // right-click pause, middle-click dial volume<->brightness
    "custom/kinesis": {
        "exec": "~/.cargo/bin/kinesisctl status --waybar --follow",
        "return-type": "json",
        "format": "{}",
        "on-click": "~/.cargo/bin/kinesisctl toggle-connection",
        "on-click-right": "~/.cargo/bin/kinesisctl toggle",
        "on-click-middle": "~/.cargo/bin/kinesisctl dial cycle",
        "tooltip": true
    },
```

`.config/waybar/style.css` (next to the earbuds rules):
```css
#custom-kinesis { padding: 0 10px; margin: 0 5px; }
#custom-kinesis.connected  { color: #b5bd68; }  /* green = connected */
#custom-kinesis.paused     { color: #ffff78; }  /* accent = controls paused */
#custom-kinesis.pairing,
#custom-kinesis.connecting { color: #81a2be; }  /* blue = busy */
#custom-kinesis.low        { color: #f0c674; }  /* amber = battery < 20% */
#custom-kinesis.off        { color: #555555; }  /* gray = not connected */
```

`.config/hypr/profile.d/z13.conf` (plain Hyprland doesn't start graphical-session.target, so export the session to systemd and start the daemon ourselves):
```
# Meta Neural Band daemon (kinesis-linux): give user services the Wayland/Hyprland env, then start it.
exec-once = dbus-update-activation-environment --systemd WAYLAND_DISPLAY HYPRLAND_INSTANCE_SIGNATURE XDG_CURRENT_DESKTOP && systemctl --user start kinesisd
```

`profiles/z13.sh`: add `neural_band       # Meta Neural Band via kinesis-linux (kinesisd + kinesisctl)` to `PROFILE_FEATURES`.

`install.sh`: after the host-profile fragment linking block, add:
```bash
# --- Meta Neural Band (kinesis-linux submodule) -------------------------------
rice_install_kinesis() {
    local src="$RICE_ROOT/kinesis-linux"
    [ -f "$src/Cargo.toml" ] || { print_warning "kinesis-linux submodule missing (git submodule update --init)"; return; }
    command -v cargo >/dev/null || { print_warning "cargo not found — skipping kinesis-linux"; return; }
    cargo install --locked --path "$src/kinesisd" && cargo install --locked --path "$src/kinesisctl" \
        && print_success "kinesisd + kinesisctl installed"
    mkdir -p "$HOME/.config/systemd/user" "$HOME/.config/kinesis"
    ln -sf "$src/contrib/kinesisd.service" "$HOME/.config/systemd/user/kinesisd.service"
    ln -sf "$DEPLOY_ROOT/.config/kinesis/config.toml" "$HOME/.config/kinesis/config.toml"
    systemctl --user daemon-reload 2>/dev/null
    print_success "kinesisd unit linked (started by Hyprland exec-once; pair with: kinesisctl pair)"
}
if rice_has_feature neural_band; then
    rice_install_kinesis
fi
```
(Use the names install.sh already defines for the repo root / deploy root — read the file; `$RICE_ROOT` comes from lib/profile.sh, `$DEPLOY_ROOT` is used by the fragment linking. Adjust to what exists. `.config/kinesis` is a single file, so linking the file rather than the dir is consistent with the `config.toml` symlink above.)

- [ ] **Step 6: Verify without a band** (no band is paired, so kinesisd never touches Bluetooth)

```bash
bash -n ~/arch_rice/install.sh
systemctl --user start kinesisd && sleep 2 && systemctl --user is-active kinesisd
~/.cargo/bin/kinesisctl status            # → "no band paired: not connected" + "error: no band paired yet — run `kinesisctl pair`"
~/.cargo/bin/kinesisctl status --waybar   # → {"text":"","class":"off",…}
pkill -SIGUSR2 waybar                     # reload waybar; the hand icon appears, gray
journalctl --user -u kinesisd -n 20 --no-pager
```
Expected: service active, status as above, `config_error` null (check with `kinesisctl status` — no "config:" line), waybar shows the gray module. Report the outputs. Leave the service running.

- [ ] **Step 7: Commit arch_rice (explicit paths only)**

```bash
cd ~/arch_rice
git add .gitignore .config/kinesis/config.toml .local/bin/kinesis-dial .config/waybar/profile.d/z13.jsonc .config/waybar/style.css .config/hypr/profile.d/z13.conf profiles/z13.sh install.sh kinesis-linux
git status --short | head -30   # confirm nothing else is staged
git commit -F - <<'EOF'
Neural Band: kinesisd config, waybar module, service start, install step

Z13 profile gains the neural_band feature: install.sh builds kinesisd and
kinesisctl from the kinesis-linux submodule and links the user unit;
Hyprland exports the session env and starts it; waybar gets custom/kinesis.
The submodule points at the local (unpushed) kinesis-linux master.

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01VsiceY7FhkoM7aRuA9DEfz
EOF
```
**Careful:** `.config/hypr/profile.d/z13.conf`, `install.sh` and `.config/waybar/style.css` may already have uncommitted user edits (the WIP listed in `git status`). If a file you need to change already has unrelated uncommitted changes, stage only your hunks with `git add -p <file>` answering by hunk — or stop and report so the user can decide. Never stage the user's WIP.

---

### Task 8: Hardware-day runbook (executed with the user)

**Files:** Create `docs/hardware-day.md` in kinesis-linux (commit on master).

This task writes the runbook only. Executing it is interactive: Claude and the user go through it together, step by step, and nothing irreversible (factory reset, claim) happens without the user's explicit go-ahead at that step.

- [ ] **Step 1: Write `docs/hardware-day.md`**

```markdown
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
```

- [ ] **Step 2: Commit**

```bash
cd ~/Projects/kinesis-linux && git add docs/hardware-day.md && git commit -F - <<'EOF'
docs: hardware-day runbook

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01VsiceY7FhkoM7aRuA9DEfz
EOF
```
