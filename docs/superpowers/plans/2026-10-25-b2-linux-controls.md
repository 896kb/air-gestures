# Plan B2: the Linux daemon on band controls, the pinch, palm and drawings

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `air-gesturesd` and `air-gestures` adopt band-core's current `controls` API:
- band controls in `[band]` (`toggle`, `next_profile`, `previous_profile`, `draw`, each with an optional `<key>_palm`), above every profile; `@…` values become a config error;
- the 24 gestures, per-finger dials, drag modes and sliders;
- palm-position overrides (`[gestures.palm_up]` …) with a stored palm calibration (`air-gestures calibrate`);
- taught drags (`air-gestures teach-drags`);
- drawings (`[drawings]`, `[profile.drawings]`, templates on disk, `on_draw`, `air-gestures draw teach|list|delete`);
- every config load logs the core's warnings, what's invalid, and each paused entry.

**Architecture:**
- **`config.rs`** keeps reading `[gestures]` and `[[profile]]` as raw tables (so it knows what was written), now `toml::Table` because of the palm and drawings sub-tables. It validates everything at parse time and builds `Profiles<String>` (band controls, `Binding`s, `FingerSetup`s, drawings) and `Options`. A pure `config::notes()` turns `Profiles::warnings()`, `invalid()`, `paused()` and `paused_sliders()` into log lines, so they're unit-tested.
- **`store.rs`** keeps, per band address and hand, a `HandCalibration { palm, drags }` in `calibration.json`; the daemon hands the one for the hand in use to `Controls::set_calibration(palm, drags)`, again whenever the band reports another hand (that keeps the runtime hand and writes nothing to the band).
- **`drawings.rs`** (new) keeps the templates as `$XDG_DATA_HOME/air-gestures/drawings/<name>.json`; the daemon loads them into a `draw::Library` at start and whenever the directory's mtime changes (polled on the 250 ms tick, like the config file).
- **`teach.rs`** (new, daemon side) is the state of one teaching step that waits for the band (rolls for 2 s, one `DragSample`, one `DrawFinished`). The daemon keeps the step and its reply `oneshot` and answers when it's done or at its deadline, so the IPC call blocks without blocking the daemon loop.
- **The CLI** drives the guided flows (`air-gestures/src/teach.rs`) over those IPC steps. It checks the results with band-core (`PalmCalibration::from_rolls`, `Prototypes::taught`, `draw::check_tries`, `Library::looks_like`) so it can say what to redo, then stores them through the daemon (`set-palm`, `set-drags`) or the drawings directory.

**Tech Stack:** Rust 2024, tokio, serde/serde_json/toml 0.8, clap 4. band-core at its local `controls` branch tip **at execution time** (0.5.0, unreleased; at least `5ff7ca7`, which adds `Controls::set_calibration`, `Controls::palm` and the responder's `gyro3` / `orientation`).

**Specs (band-core repo, branch `controls`):**
- `docs/superpowers/specs/2026-10-25-shared-controls-design.md` (revised: band controls; its "Linux" section and "Changes from the first build");
- `docs/superpowers/specs/2026-10-25-pinch-palm-drawings-design.md` (parts 2–4, and "Platform side → Linux": its config sketch is the target).

**House rules:**
- Work in `~/Projects/air-gestures` on the local branch `controls`. Never push (neither this repo nor the `band-core` submodule) and never tag.
- No `cargo fmt` (the repo isn't formatted with it; it would rewrite unrelated code).
- Stage files by name (`git add <paths>`), never `git add -A` / `.`; `air-gestures/examples/` stays untracked.
- Never touch the owner's live config `~/arch_rice/.config/air-gestures/config.toml` (Hyprland and the daemon reload it on write).
- Never stop, start or restart the `air-gesturesd` user service. Smoke tests use a `--dry-run` daemon with every XDG directory in a temp dir (Task 13).
- No sudo. Never run `iwctl`.
- Before every commit: `cargo test --workspace` passes and `cargo clippy --workspace --all-targets` adds no warnings in `air-gesturesd` / `air-gestures`.
- Every commit message ends with these trailers (use `git commit -q -F - <<'EOF' … EOF`):
  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01MLDqGPYx8bTGKJcVLgoyzo
  ```

## band-core's API used here (`band-core/src/controls/`; read it for details)

- `Gesture::{ALL (24), key(), label(), from_key(), drag_of()}`; `Finger::{Index, Middle}`; `Palm::{Down, Side, Up}` with `key()` = `"down" | "sideways" | "up"` and `from_key()`; `Palms` (a set: `iter()`, `len()`, `contains()`, `From<[Palm; N]>`, `FromIterator`).
- `Builtin::{Controls, NextProfile, PreviousProfile, Draw}` (`ALL`); `BandControl { gesture, palm: PalmSet }`, `BandControl::on(gesture)`; `PalmSet::{All, Only(Palms)}` (`MAX_ONLY = 2`, `contains`); `BandControls { controls, next, previous, draw }` (`Default` = none, `control(builtin)`).
- `Binding<A> { normal: Option<A>, overrides: BTreeMap<Palm, Option<A>> }` (`From<A>`, `action(palm)`, `valid()`, `MAX_OVERRIDES = 2`).
- `Dial<A> { up, down }`; `FingerSetup<A> { dial: Option<Dial<A>>, drags: DragMode<A> }`; `DragMode::{Gestures, Sliders([Option<Dial<A>>; 3])}` (pairs left/right, toward/away, up/down).
- `Profile<A> { name, gestures: BTreeMap<Gesture, Binding<A>>, index, middle, drawings: BTreeMap<String, A> }`, `Profile::new(name)`.
- `Profiles<A> { band, list, picked, apps, calls, drawings }`; `warnings() -> Vec<Warning::{NoControls, NoWayOut, DrawOnTap}>`; `invalid() -> Vec<Invalid::{Binding{place, gesture}, BandControl(builtin), DrawCantCancel}>`; `paused() -> Vec<Paused { builtin, gesture, used: Use { place, palm, action }, positions }>`; `paused_sliders() -> Vec<PausedSlider { builtin, gesture, profile, finger, pair, positions }>`; `drawing_action(profile, name)`; `Place::{Profile(i), Calls}`.
- `Options { hand, autostart, dial_sensitivity, drags: Option<Prototypes>, palm: Option<PalmCalibration> }` (`Default`).
- `Controls<String>`: as before (`new`, `on_event`, `poll`, `next_deadline`, `pause`/`resume`/`toggle`, `pick`, `request_hand`, `set_options`, `set_profiles`, `status`, `active`, `profiles`, `options`, `enabled`), plus `set_dial(Option<Dial>)` / `clear_dial()` / `dial()` / `dial_overridden()`; `set_library(draw::Library)`; `set_teaching(bool)`; `start_drawing(now) -> Vec<Output>`; `cancel_drawing() -> Vec<Output>`; `drawing()`; `set_drag_teaching(bool) -> Vec<Output>`; `set_calibration(palm: Option<PalmCalibration>, drags: Option<Prototypes>)` (no outputs; keeps the hand, a runtime one from `request_hand` too, and writes nothing to the band; re-reads the palm; loses a live pinch only when the drags change); `palm(now) -> Palm` (down without an orientation in the last 0.5 s or without a calibration).
- `Output<String>`: `Gesture`, `Run`, `Controls{enabled}`, `Profile{index, step}`, `ProfileHeld{app}`, `SetHand`, `Practiced`, `DrawStarted`, `DrawPoint{stroke, x, y}`, `DrawFinished{strokes}`, `Drawn{name, distance}`, `NotDrawn`, `DragSample{finger, direction}`, `DrawCancelled`.
- `controls::palm::{PalmCalibration { down_roll, up_roll }, PalmCalibration::from_rolls(&[f64], &[f64]) -> Result<_, CalibrationError::{Empty(Palm), NotStill(Palm), TooClose}>, RIGHT_PALM_DOWN_ROLL, MIN_APART}`.
- `controls::pinch::{Prototypes { palm_down_roll, turn, drags: Option<[[f64; 3]; 6]> }, Prototypes::right_hand(), Prototypes::taught(turn, drags, palm_down_roll) -> Result<_, TaughtError::{NoTurn, NoDrag(Drag), TooAlike(Drag, Drag), LikeTheTurn(Drag)}>, Drag::{ALL, index()}, up_in_band(q), roll_or(up, fallback)}`.
- `controls::draw::{Stroke = Vec<[f64; 2]>, Drawing { name, tries: Vec<Vec<Stroke>> }, Library::new(&[Drawing]), Library::names(), Library::looks_like(&[Vec<Stroke>]) -> Option<String>, check_tries(&[Vec<Stroke>]) -> Result<(), TeachError::{TooSmall(i), NotAlike{first, second, distance}, NoTries}>}`.

The sim band (`band_core::responder::Responder`, behind `SimTransport`) sends gestures (any action number: press = 1, release = 2), one-axis gyro (`gyro(x)`), three-axis gyro (`gyro3(axes)`), orientation (`orientation(q)`) and raw sEMG. Task 7 adds `SimInject::Gyro3([i16; 3])` and `SimInject::Orientation([f32; 4])` to `SimTransport`, so daemon and CLI tests run palm positions, calibration and teaching end to end. Two facts the tests rely on:
- an orientation `q = [cos(−r/2), 0, sin(−r/2), 0]` has roll `r` about the forearm (`roll_or(up_in_band(q), …)`); the right hand's measured palm down is −142.3°, palm up −11.3° (sideways ≈ −76.8°);
- with no fresh orientation the pinch frame is palm down, so an x-axis pinch (`Gyro(+x)`) is a drag **away** (forearm direction ≈ `[-0.791, 0, -0.612]`) outside a drawing and a **vertical stroke** (`[0, +y]`) inside one.

## File structure

| File | Change |
|---|---|
| `band-core` (submodule) | moved to band-core `controls` tip (local commit, not pushed) |
| `air-gesturesd/src/config.rs` | band controls, gesture/finger/palm/drawings keys, `notes()`, `missing_drawings()` |
| `air-gesturesd/src/store.rs` | `StoredPalm`, `StoredDrags`, `HandCalibration`, `calibration.json` |
| `air-gesturesd/src/drawings.rs` | new: the drawings directory |
| `air-gesturesd/src/teach.rs` | new: error texts; the waiting teaching step (`Wait`) |
| `air-gesturesd/src/ipc.rs` | `palm-rolls`, `teach-drag`, `draw-try`, `set-palm`, `set-drags`; `Response::teach` |
| `air-gesturesd/src/daemon.rs` | notes on load, new outputs, `on_draw`, dial override via `set_dial`, per-hand calibration via `set_calibration`, library reload, teaching steps, the `palm` request |
| `air-gesturesd/src/transport.rs` | `SimInject::Gyro3`, `SimInject::Orientation` |
| `air-gesturesd/src/main.rs`, `src/lib.rs` | drawings dir into `Daemon::new`; new modules |
| `air-gesturesd/tests/{config,store,drawings,teach,ipc,daemon}.rs` | tests |
| `air-gestures/src/client.rs` | `request_within` |
| `air-gestures/src/teach.rs`, `src/lib.rs` | new: the guided flows |
| `air-gestures/src/main.rs` | `calibrate`, `teach-drags`, `draw teach|list|delete`; clap tests |
| `air-gestures/tests/{support/mod.rs,teach.rs}` | drawings dir in the test daemon; flow tests |
| `contrib/config.toml`, `README.md`, `CHANGELOG.md` | docs |

---

### Task 1: band-core's `controls` tip, and band controls in `[band]`

The submodule move breaks the build (`Action` is gone), so this task also ports the code far enough to build: band controls, plain-command gestures, the notes on load. Commit both together.

**Files:**
- Modify: `band-core` (submodule pointer), `air-gesturesd/src/config.rs`, `air-gesturesd/src/daemon.rs`, `air-gesturesd/tests/config.rs`, `air-gesturesd/tests/daemon.rs`, `contrib/config.toml`

- [ ] **Step 1: Move the submodule (local only; do not push band-core)**

```bash
cd ~/Projects/air-gestures
git status -s                        # only "?? air-gestures/examples/"
git branch --show-current            # controls
git -C band-core fetch -q /home/user888/Projects/band-core controls
git -C band-core checkout -q FETCH_HEAD
git -C band-core log --oneline -1    # the band-core controls tip right now: 5ff7ca7 "controls: set_calibration and palm(); motion in the responder" or later
git -C band-core grep -q 'pub fn set_calibration' src/controls/mod.rs && echo ok   # ok: the API this plan uses
cargo build --workspace 2>&1 | grep -c '^error'   # > 0: Action is gone; the steps below fix it
```

The submodule's remote is GitLab; this commit exists only in the local band-core repo until the owner pushes it. Never `git -C band-core push`.

- [ ] **Step 2: Write the failing tests**

Replace `air-gesturesd/tests/config.rs` with this (the old tests' intent, on the new API; later tasks append to it):

```rust
use air_gesturesd::config::{Config, DialTarget, HandSetting, Level, notes};
use band_core::controls::{BandControl, BandControls, Binding, Gesture, Palm, PalmSet, Palms};

fn run(command: &str) -> Option<Binding<String>> {
    Some(Binding::from(command.to_owned()))
}

fn texts(config: &Config) -> Vec<(Level, String)> {
    notes(&config.profiles(DialTarget::Volume)).into_iter().map(|n| (n.level, n.text)).collect()
}

#[test]
fn defaults_match_the_design() {
    let config = Config::default();
    assert_eq!(config.band.hand, HandSetting::Band);
    assert!(config.band.autostart);
    let profiles = config.profiles(DialTarget::Volume);
    assert_eq!(profiles.band, BandControls::default(), "Linux: no band controls unless asked");
    assert_eq!(profiles.list.len(), 1);
    let first = &profiles.list[0];
    assert_eq!(first.name, "default");
    let get = |g| first.gestures.get(&g).cloned();
    assert_eq!(get(Gesture::SwipeLeft), run("hyprctl dispatch workspace e-1"));
    assert_eq!(get(Gesture::SwipeRight), run("hyprctl dispatch workspace e+1"));
    assert_eq!(get(Gesture::SwipeUp), run("wofi --show drun"));
    assert_eq!(get(Gesture::SwipeDown), run("hyprctl dispatch sendshortcut , Escape, activewindow"));
    assert_eq!(get(Gesture::IndexDouble), run("playerctl play-pause"));
    assert_eq!(get(Gesture::MiddleDouble), run("wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle"));
    for nothing in [Gesture::IndexTap, Gesture::MiddleTap, Gesture::MiddleHold, Gesture::IndexHold, Gesture::IndexTriple, Gesture::MiddleTriple, Gesture::IndexDragLeft] {
        assert_eq!(get(nothing), None, "{nothing:?}: 0.3.10's defaults");
    }
    let dial = first.index.dial.clone().unwrap();
    assert_eq!(dial.up, "wpctl set-volume -l 1.0 @DEFAULT_AUDIO_SINK@ 2%+");
    assert_eq!(config.dial.target, DialTarget::Volume);
    assert_eq!(config.dial.sensitivity, 1.0);
    assert!(config.profiles(DialTarget::Off).list[0].index.dial.is_none());
    assert_eq!(config.profiles(DialTarget::Brightness).list[0].index.dial.clone().unwrap().down, "brightnessctl -q set 2%-");
}

#[test]
fn partial_files_override_only_what_they_name() {
    let config = Config::parse(
        r#"
        [band]
        hand = "left"
        [gestures]
        swipe_left = "echo left"
        [dial]
        target = "brightness"
        sensitivity = 2.0
        brightness_up = ""
        "#,
    )
    .unwrap();
    assert_eq!(config.band.hand, HandSetting::Left);
    assert!(config.band.autostart, "unnamed keys keep their defaults");
    let first = &config.profiles(DialTarget::Brightness).list[0];
    assert_eq!(first.gestures.get(&Gesture::SwipeLeft).cloned(), run("echo left"));
    assert_eq!(first.gestures.get(&Gesture::SwipeRight).cloned(), run("hyprctl dispatch workspace e+1"));
    assert_eq!(config.dial.target, DialTarget::Brightness);
    assert_eq!(config.options().dial_sensitivity, 2.0);
    assert!(first.index.dial.is_none(), "brightness_up unassigned: no dial for that target");
}

#[test]
fn whitespace_or_empty_values_are_unassigned() {
    let config = Config::parse("[gestures]\nindex_double = \"   \"\nmiddle_hold = \"\"\n").unwrap();
    let first = &config.profiles(DialTarget::Volume).list[0];
    assert_eq!(first.gestures.get(&Gesture::IndexDouble), None);
    assert_eq!(first.gestures.get(&Gesture::MiddleHold), None);
}

#[test]
fn a_whitespace_only_dial_command_is_unassigned() {
    let config = Config::parse("[dial]\nvolume_up = \"\\t\\n\"\n").unwrap();
    assert_eq!(config.dial.command(DialTarget::Volume, true), None);
    assert!(config.profiles(DialTarget::Volume).list[0].index.dial.is_none());
}

#[test]
fn profiles_come_in_order_and_start_empty() {
    let config = Config::parse(
        r#"
        [band]
        profile = "desk"
        [[profile]]
        name = "games"
        swipe_left = "echo games"
        index_hold = "echo held"
        [[profile]]
        name = "media"
        "#,
    )
    .unwrap();
    let profiles = config.profiles(DialTarget::Volume);
    let names: Vec<&str> = profiles.list.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["desk", "games", "media"]);
    let games = &profiles.list[1];
    assert_eq!(games.gestures.get(&Gesture::SwipeLeft).cloned(), run("echo games"));
    assert_eq!(games.gestures.get(&Gesture::IndexHold).cloned(), run("echo held"));
    assert_eq!(games.gestures.get(&Gesture::SwipeRight), None, "extra profiles don't inherit");
    assert!(games.index.dial.is_some(), "every profile turns the dial target");
}

#[test]
fn band_controls_take_any_gesture_and_palm_positions() {
    let config = Config::parse(
        r#"
        [band]
        toggle = "middle_hold"
        next_profile = "index_triple"
        next_profile_palm = ["up"]
        previous_profile = "swipe_down"
        draw = "index_drag_up"
        draw_palm = ["down", "sideways"]
        "#,
    )
    .unwrap();
    let band = config.profiles(DialTarget::Volume).band;
    assert_eq!(band.controls, Some(BandControl::on(Gesture::MiddleHold)));
    assert_eq!(band.next, Some(BandControl { gesture: Gesture::IndexTriple, palm: PalmSet::Only(Palms::from([Palm::Up])) }));
    assert_eq!(band.previous, Some(BandControl::on(Gesture::SwipeDown)));
    assert_eq!(band.draw, Some(BandControl { gesture: Gesture::IndexDragUp, palm: PalmSet::Only(Palms::from([Palm::Down, Palm::Side])) }));
}

#[test]
fn two_band_controls_may_share_a_gesture_only_in_different_positions() {
    let error = Config::parse("[band]\ntoggle = \"middle_hold\"\ndraw = \"middle_hold\"\n").unwrap_err();
    assert!(error.contains("band.toggle and band.draw") && error.contains("middle_hold"), "{error}");
    let error = Config::parse("[band]\ntoggle = \"middle_hold\"\ndraw = \"middle_hold\"\ndraw_palm = [\"up\"]\n").unwrap_err();
    assert!(error.contains("with the palm up"), "{error}");
    let fine = "[band]\ntoggle = \"middle_hold\"\ntoggle_palm = [\"down\"]\ndraw = \"middle_hold\"\ndraw_palm = [\"up\", \"sideways\"]\n";
    assert!(Config::parse(fine).is_ok());
}

/// Configs from before band controls: `toggle = "middle_hold"` keeps the lock there, and a
/// command the old documented config wrote on that gesture is kept but paused (logged).
#[test]
fn the_old_toggle_key_is_a_band_control_and_its_gestures_value_is_paused() {
    for (written, paused) in [("\"\"", 0), ("\"echo never\"", 1)] {
        let config = Config::parse(&format!("[band]\ntoggle = \"middle_hold\"\n[gestures]\nmiddle_hold = {written}\n")).unwrap();
        assert_eq!(config.profiles(DialTarget::Volume).band.controls, Some(BandControl::on(Gesture::MiddleHold)));
        let count = texts(&config).iter().filter(|(_, t)| t.contains("overridden")).count();
        assert_eq!(count, paused, "middle_hold = {written}");
    }
}

#[test]
fn every_config_load_notes_warnings_and_paused_entries() {
    let config = Config::parse(
        r#"
        [band]
        next_profile = "middle_double"
        [[profile]]
        name = "media"
        middle_double = "echo paused"
        "#,
    )
    .unwrap();
    let texts = texts(&config);
    let warn = |text: &str| (Level::Warn, text.to_owned());
    assert!(texts.contains(&warn("middle_double in profile \"default\" is overridden by [band] next_profile")), "{texts:?}");
    assert!(texts.contains(&warn("middle_double in profile \"media\" is overridden by [band] next_profile")), "{texts:?}");
    assert!(texts.iter().any(|(l, t)| *l == Level::Info && t.contains("[band] toggle")), "no lock: {texts:?}");
    let two = texts_of("[[profile]]\nname = \"b\"\n");
    assert!(two.iter().any(|(l, t)| *l == Level::Warn && t.contains("air-gestures profile next") && t.contains("[band] next_profile")), "{two:?}");
    let tap = texts_of("[band]\ndraw = \"index_tap\"\n");
    assert!(tap.iter().any(|(l, t)| *l == Level::Warn && t.contains("single tap")), "{tap:?}");
    let hold = texts_of("[band]\ndraw = \"index_hold\"\n");
    assert!(hold.iter().any(|(l, t)| *l == Level::Error && t.contains("index_hold") && t.contains("can't cancel")), "{hold:?}");
}

fn texts_of(text: &str) -> Vec<(Level, String)> {
    texts(&Config::parse(text).unwrap())
}

#[test]
fn mistakes_are_reported() {
    for (text, needle) in [
        ("[band]\ncolour = \"red\"\n", "colour"),
        ("[band]\ntoggle = \"pinky_hold\"\n", "pinky_hold"),
        ("[band]\ntoggle_palm = [\"up\"]\n", "set band.toggle"),
        ("[band]\ntoggle = \"middle_hold\"\ntoggle_palm = [\"left\"]\n", "\"left\""),
        ("[band]\ntoggle = \"middle_hold\"\ntoggle_palm = [\"down\", \"sideways\", \"up\"]\n", "one or two"),
        ("[band]\ntoggle = \"middle_hold\"\ntoggle_palm = []\n", "one or two"),
        ("[gestures]\npinky_tap = \"x\"\n", "pinky_tap"),
        ("[gestures]\nmiddle_hold = \"@controls\"\n", "[band]"),
        ("[gestures]\nswipe_up = \"@lock\"\n", "@lock"),
        ("[dial]\nsensitivity = 9.0\n", "sensitivity"),
        ("[[profile]]\nswipe_up = \"x\"\n", "name"),
        ("[[profile]]\nname = \"default\"\n", "default"),
        ("[[profile]]\nname = \"a\"\n[[profile]]\nname = \"a\"\n", "\"a\""),
        ("[[profile]]\nname = \"it's\"\n", "it's"),
        ("[[profile]]\nname = \".hidden\"\n", ".hidden"),
        ("[[profile]]\nname = \"x\"\nnope = \"y\"\n", "nope"),
        ("[[profile]]\nname = \"x\"\nindex_tap = \"@next-profile\"\n", "next_profile"),
    ] {
        let error = Config::parse(text).unwrap_err();
        assert!(error.contains(needle), "{text:?} → {error}");
    }
}

#[test]
fn mistakes_carry_the_bad_value() {
    let error = Config::parse("[dial]\nsensitivity = 9.0").unwrap_err();
    assert!(error.contains("(got 9"), "should report the bad value: {error}");
    assert!(Config::parse("[band]\nhand = \"both\"").is_err());
}

#[test]
fn a_missing_file_means_the_defaults() {
    let config = Config::load(std::path::Path::new("/nonexistent/air-gestures/config.toml")).unwrap();
    assert_eq!(config, Config::default());
}

#[test]
fn an_existing_file_is_loaded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[dial]\ntarget = \"off\"\n").unwrap();
    assert_eq!(Config::load(&path).unwrap().dial.target, DialTarget::Off);
}

#[test]
fn the_documented_config_is_the_default() {
    let documented = Config::parse(include_str!("../../contrib/config.toml")).unwrap();
    let default = Config::default();
    assert_eq!(documented.band, default.band);
    assert_eq!(documented.dial, default.dial);
    assert_eq!(documented.options(), default.options());
    for target in [DialTarget::Volume, DialTarget::Brightness, DialTarget::Off] {
        assert_eq!(documented.profiles(target), default.profiles(target));
    }
}

#[test]
fn the_dial_target_cycles() {
    assert_eq!(DialTarget::Volume.next(), DialTarget::Brightness);
    assert_eq!(DialTarget::Brightness.next(), DialTarget::Volume);
    assert_eq!(DialTarget::Off.next(), DialTarget::Volume);
}

#[test]
fn options_carry_hand_autostart_and_sensitivity() {
    let config = Config::parse("[band]\nhand = \"right\"\nautostart = false\n[dial]\nsensitivity = 0.5\n").unwrap();
    let options = config.options();
    assert_eq!(options.hand, band_core::controls::HandSetting::Right);
    assert!(!options.autostart);
    assert_eq!(options.dial_sensitivity, 0.5);
    assert!(options.palm.is_none() && options.drags.is_none(), "the daemon adds what's taught");
}
```

In `air-gesturesd/tests/daemon.rs`:
- `a_band_toggle_runs_on_toggle_but_ipc_pause_does_not`: the comment becomes `// The lock is a band control now; none by default.` and the config `"[band]\ntoggle = \"middle_hold\"\non_toggle = \"osd\"\n"`.
- `profiles_switch_from_the_band_and_run_on_profile`: the config becomes `"[band]\non_profile = \"osd\"\nnext_profile = \"index_triple\"\n[[profile]]\nname = \"games\"\n"`.
- Add:

```rust
#[tokio::test(start_paused = true)]
async fn a_band_control_wins_over_the_profiles_command() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    // swipe_left's default command is paused; with one profile, next does nothing.
    daemon.configure("[band]\nnext_profile = \"swipe_left\"\n").await;
    daemon.transport.inject(SWIPE_LEFT);
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(daemon.runner.commands().is_empty(), "{:?}", daemon.runner.commands());
}
```

- [ ] **Step 3: Implement `config.rs`**

Replace the module doc, the imports, `ToggleGesture`, `BandConfig` (+ its `Default`), `default_gesture`, `action`, `check_name` and `Config::build` as follows; `HandSetting`, `DialTarget`, `DialConfig`, `load`, `default_path`, `home`, `xdg` and the unit test stay.

```rust
//! `~/.config/air-gestures/config.toml`: the band controls (`[band]`), profiles of
//! gesture → shell command (`""` = nothing), and the dial.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use band_core::controls::{
    BandControl, BandControls, Binding, Builtin, Dial, Gesture, Invalid, Options, Palm, PalmSet, Palms, Place, Profile,
    Profiles, Warning,
};
use serde::{Deserialize, Serialize};
```

```rust
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BandConfig {
    pub hand: HandSetting,
    /// Connect and enable controls when the daemon starts.
    pub autostart: bool,
    /// Band controls, above every profile: each a gesture key (any of the 24) or "none".
    /// `toggle` turns the controls off and on (it works while they're off).
    pub toggle: String,
    pub next_profile: String,
    pub previous_profile: String,
    /// Opens a drawing; made again, cancels it.
    pub draw: String,
    /// Where each band control acts: one or two of "down", "sideways", "up"; unset = all.
    pub toggle_palm: Option<Vec<String>>,
    pub next_profile_palm: Option<Vec<String>>,
    pub previous_profile_palm: Option<Vec<String>>,
    pub draw_palm: Option<Vec<String>>,
    /// Run after a gesture toggled the controls, with `AIR_GESTURES_ENABLED=1` or `0`
    /// exported (e.g. an OSD).
    pub on_toggle: String,
    /// Run after the active profile changed, with `AIR_GESTURES_PROFILE=<name>` exported.
    pub on_profile: String,
    /// The name of the `[gestures]` profile (the first one).
    pub profile: String,
}

impl Default for BandConfig {
    fn default() -> Self {
        Self {
            hand: HandSetting::Band,
            autostart: true,
            toggle: "none".into(),
            next_profile: "none".into(),
            previous_profile: "none".into(),
            draw: "none".into(),
            toggle_palm: None,
            next_profile_palm: None,
            previous_profile_palm: None,
            draw_palm: None,
            on_toggle: String::new(),
            on_profile: String::new(),
            profile: "default".into(),
        }
    }
}

/// `[band]`'s key for a band control.
pub fn band_key(builtin: Builtin) -> &'static str {
    match builtin {
        Builtin::Controls => "toggle",
        Builtin::NextProfile => "next_profile",
        Builtin::PreviousProfile => "previous_profile",
        Builtin::Draw => "draw",
    }
}

/// " with the palm down or up", or "" for every position.
fn palm_suffix(palms: Palms) -> String {
    if palms.len() == Palm::ALL.len() {
        return String::new();
    }
    let names: Vec<&str> = palms.iter().map(Palm::key).collect();
    format!(" with the palm {}", names.join(" or "))
}

fn palm_set(key: &str, palm: Option<&Vec<String>>) -> Result<PalmSet, String> {
    let Some(names) = palm else { return Ok(PalmSet::All) };
    let mut set = Vec::new();
    for name in names {
        set.push(Palm::from_key(name).ok_or_else(|| {
            format!("band.{key}_palm: unknown position \"{name}\" (\"down\", \"sideways\" or \"up\")")
        })?);
    }
    let set: Palms = set.into_iter().collect();
    if !(1..=PalmSet::MAX_ONLY).contains(&set.len()) {
        return Err(format!("band.{key}_palm: list one or two positions (leave it out for all three)"));
    }
    Ok(PalmSet::Only(set))
}

impl BandConfig {
    fn control(&self, builtin: Builtin) -> (&str, Option<&Vec<String>>) {
        match builtin {
            Builtin::Controls => (&self.toggle, self.toggle_palm.as_ref()),
            Builtin::NextProfile => (&self.next_profile, self.next_profile_palm.as_ref()),
            Builtin::PreviousProfile => (&self.previous_profile, self.previous_profile_palm.as_ref()),
            Builtin::Draw => (&self.draw, self.draw_palm.as_ref()),
        }
    }

    /// The band controls; two on one gesture in one position is an error.
    pub fn band_controls(&self) -> Result<BandControls, String> {
        let mut band = BandControls::default();
        for builtin in Builtin::ALL {
            let key = band_key(builtin);
            let (value, palm) = self.control(builtin);
            let control = match value.trim() {
                "none" | "" => {
                    if palm.is_some() {
                        return Err(format!("band.{key}_palm: set band.{key} to a gesture first"));
                    }
                    None
                }
                name => Some(BandControl {
                    gesture: Gesture::from_key(name).ok_or_else(|| {
                        format!("band.{key}: unknown gesture \"{name}\" (a gesture key like \"middle_hold\", or \"none\")")
                    })?,
                    palm: palm_set(key, palm)?,
                }),
            };
            match builtin {
                Builtin::Controls => band.controls = control,
                Builtin::NextProfile => band.next = control,
                Builtin::PreviousProfile => band.previous = control,
                Builtin::Draw => band.draw = control,
            }
        }
        for (i, &a) in Builtin::ALL.iter().enumerate() {
            for &b in &Builtin::ALL[i + 1..] {
                let (Some(x), Some(y)) = (band.control(a), band.control(b)) else { continue };
                let shared: Palms = Palm::ALL.into_iter().filter(|&p| x.palm.contains(p) && y.palm.contains(p)).collect();
                if x.gesture == y.gesture && !shared.is_empty() {
                    return Err(format!(
                        "band.{} and band.{} are both on {}{}: move one to another gesture or palm position",
                        band_key(a),
                        band_key(b),
                        x.gesture.key(),
                        palm_suffix(shared)
                    ));
                }
            }
        }
        Ok(band)
    }
}

/// `[gestures]` keys nobody wrote: the first profile's defaults (0.3.10's).
fn default_gesture(gesture: Gesture) -> Option<&'static str> {
    match gesture {
        Gesture::SwipeLeft => Some("hyprctl dispatch workspace e-1"),
        Gesture::SwipeRight => Some("hyprctl dispatch workspace e+1"),
        Gesture::SwipeUp => Some("wofi --show drun"),
        Gesture::SwipeDown => Some("hyprctl dispatch sendshortcut , Escape, activewindow"),
        Gesture::IndexDouble => Some("playerctl play-pause"),
        Gesture::MiddleDouble => Some("wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle"),
        _ => None,
    }
}

/// A command value at `place.key`: blank = nothing. `@…` was the first build's built-in syntax.
fn command(place: &str, key: &str, value: &str) -> Result<Option<String>, String> {
    let trimmed = value.trim();
    if trimmed.starts_with('@') {
        return Err(format!(
            "{place}.{key} = \"{trimmed}\": band controls are set in [band] now (toggle, next_profile, previous_profile, draw = \"{key}\")"
        ));
    }
    Ok((!trimmed.is_empty()).then(|| value.to_owned()))
}

/// Profile and drawing names go into `AIR_GESTURES_PROFILE='…'` / `AIR_GESTURES_DRAWING='…'`
/// (and drawings into file names): letters, digits, space, `.`, `_`, `-`, not starting
/// with `.`, at most 64 characters.
pub fn check_name(what: &str, name: &str) -> Result<(), String> {
    let fine = !name.trim().is_empty()
        && !name.starts_with('.')
        && name.chars().count() <= 64
        && name.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-'));
    if fine {
        Ok(())
    } else {
        Err(format!("{what} name \"{name}\": use up to 64 letters, digits, spaces, '.', '_' or '-' (not starting with '.')"))
    }
}
```

`Config::options` fills the new fields from the default (the daemon adds what's taught, Task 7):

```rust
    pub fn options(&self) -> Options {
        Options {
            hand: match self.band.hand {
                HandSetting::Band => band_core::controls::HandSetting::Band,
                HandSetting::Left => band_core::controls::HandSetting::Left,
                HandSetting::Right => band_core::controls::HandSetting::Right,
            },
            autostart: self.band.autostart,
            dial_sensitivity: self.dial.sensitivity,
            ..Options::default()
        }
    }
```

`Config::build` (still `profiles(target)` in this task; Task 2 drops the argument):

```rust
    fn build(&self, target: DialTarget) -> Result<Profiles<String>, String> {
        let band = self.band.band_controls()?;
        let dial = self.dial.dial(target);
        check_name("profile", &self.band.profile).map_err(|e| format!("band.profile: {e}"))?;
        let mut first = Profile::new(&self.band.profile);
        for gesture in Gesture::ALL {
            if let Some(command) = default_gesture(gesture) {
                first.gestures.insert(gesture, Binding::from(command.to_owned()));
            }
        }
        for (key, value) in &self.gestures {
            let gesture = Gesture::from_key(key).ok_or_else(|| format!("gestures: unknown gesture \"{key}\""))?;
            match command("gestures", key, value)? {
                Some(command) => first.gestures.insert(gesture, Binding::from(command)),
                None => first.gestures.remove(&gesture),
            };
        }
        first.index.dial = dial.clone();
        let mut list = vec![first];
        for table in &self.profile {
            let name = table.get("name").ok_or("every [[profile]] needs a name")?;
            check_name("profile", name)?;
            if list.iter().any(|p: &Profile<String>| p.name == *name) {
                return Err(format!("two profiles are called \"{name}\""));
            }
            let place = format!("profile \"{name}\"");
            let mut profile = Profile::new(name);
            for (key, value) in table {
                if key == "name" {
                    continue;
                }
                let gesture = Gesture::from_key(key).ok_or_else(|| format!("{place}: unknown key \"{key}\""))?;
                if let Some(command) = command(&place, key, value)? {
                    profile.gestures.insert(gesture, Binding::from(command));
                }
            }
            profile.index.dial = dial.clone();
            list.push(profile);
        }
        Ok(Profiles { band, list, picked: 0, apps: BTreeMap::new(), calls: BTreeMap::new(), drawings: BTreeMap::new() })
    }
```

Add the notes after `impl Config` (before `home`):

```rust
/// How loud a config note is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

/// Something the daemon logs about the loaded config (never blocks loading it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub level: Level,
    pub text: String,
}

impl Note {
    fn new(level: Level, text: impl Into<String>) -> Self {
        Self { level, text: text.into() }
    }
}

/// The sub-table a palm override is written in.
pub fn palm_section(palm: Palm) -> &'static str {
    match palm {
        Palm::Down => "palm_down",
        Palm::Side => "sideways",
        Palm::Up => "palm_up",
    }
}

fn place_text(profiles: &Profiles<String>, place: Place) -> String {
    match place {
        Place::Profile(index) => format!("profile \"{}\"", profiles.list[index].name),
        Place::Calls => "the call layer".into(),
    }
}

/// What to log on every config load: the core's warnings (no lock gesture is Linux's
/// default, so that one is info), what's invalid (errors), and every entry a band control
/// pauses (warnings).
pub fn notes(profiles: &Profiles<String>) -> Vec<Note> {
    let mut notes = Vec::new();
    for warning in profiles.warnings() {
        notes.push(match warning {
            Warning::NoControls => Note::new(Level::Info, "no band gesture turns the controls off/on: set [band] toggle to one (or use `air-gestures toggle`)"),
            Warning::NoWayOut => Note::new(
                Level::Warn,
                format!("{} profiles, but no band gesture switches them: set [band] next_profile (or previous_profile), or use `air-gestures profile next`", profiles.list.len()),
            ),
            Warning::DrawOnTap => Note::new(Level::Warn, "[band] draw is on a single tap: in a drawing a short pinch is that tap, and cancels the drawing"),
        });
    }
    for invalid in profiles.invalid() {
        notes.push(Note::new(Level::Error, match invalid {
            Invalid::DrawCantCancel => format!(
                "[band] draw is on {}, which can't cancel a drawing: it ends by itself 1 s after the last stroke (or after 10 s with none)",
                profiles.band.draw.map_or("?", |c| c.gesture.key())
            ),
            Invalid::Binding { place, gesture } => format!("{} in {}: more than two palm overrides", gesture.key(), place_text(profiles, place)),
            Invalid::BandControl(builtin) => format!("[band] {}_palm: list one or two positions", band_key(builtin)),
        }));
    }
    for paused in profiles.paused() {
        let what = match paused.used.palm {
            None => paused.gesture.key().to_owned(),
            Some(palm) => format!("{} ({})", paused.gesture.key(), palm_section(palm)),
        };
        notes.push(Note::new(Level::Warn, format!(
            "{what} in {} is overridden by [band] {}{}",
            place_text(profiles, paused.used.place),
            band_key(paused.builtin),
            palm_suffix(paused.positions)
        )));
    }
    for slider in profiles.paused_sliders() {
        let finger = match slider.finger {
            band_core::controls::Finger::Index => "index",
            band_core::controls::Finger::Middle => "middle",
        };
        let pair = ["left_right", "toward_away", "up_down"][slider.pair];
        notes.push(Note::new(Level::Warn, format!(
            "{finger}_slider_{pair} in profile \"{}\" is overridden by [band] {} on {}{}",
            profiles.list[slider.profile].name,
            band_key(slider.builtin),
            slider.gesture.key(),
            palm_suffix(slider.positions)
        )));
    }
    notes
}
```

(`paused_sliders` can only fire once sliders are configurable, Task 2, which tests it.)

- [ ] **Step 4: Implement `daemon.rs`**

- `use crate::config::{Config, DialTarget, Level};`.
- `Daemon::new` and `reload_config`: replace both `warn_profiles(&controls)` / `warn_profiles(&self.controls)` calls with `log_notes(controls.profiles())` / `log_notes(self.controls.profiles())`.
- Replace `fn warn_profiles` with:

```rust
fn log_notes(profiles: &band_core::controls::Profiles<String>) {
    for note in crate::config::notes(profiles) {
        match note.level {
            Level::Info => tracing::info!("config: {}", note.text),
            Level::Warn => tracing::warn!("config: {}", note.text),
            Level::Error => tracing::error!("config: {}", note.text),
        }
    }
}
```

- In `apply`, after the `Output::Practiced(_) => {}` arm, add (Task 7 and 8 fill these in):

```rust
                Output::DrawPoint { stroke, x, y } => tracing::trace!(stroke, x, y, "draw point"),
                other @ (Output::DrawStarted
                | Output::DrawFinished { .. }
                | Output::Drawn { .. }
                | Output::NotDrawn
                | Output::DrawCancelled
                | Output::DragSample { .. }) => tracing::debug!(?other, "controls"),
```

- [ ] **Step 5: `contrib/config.toml` without built-in values**

Replace the header's "Built-in actions …" comment block with:

```toml
# Band controls ([band] toggle, next_profile, previous_profile, draw) sit on one gesture each,
# above every profile; a profile's command on that gesture is kept but paused.
```

In `[band]`, replace the `toggle = …` line with:

```toml
toggle = "none"            # controls off/on: any gesture key, or "none"
next_profile = "none"      # next profile (needs 2+ profiles)
previous_profile = "none"  # previous profile
draw = "none"              # open a drawing (see [drawings])
```

In `[gestures]`, set `index_triple = ""`, `middle_triple = ""`, `middle_hold = ""`, and in the `[[profile]]` example drop the two `@` lines. (Task 12 rewrites the whole file.)

- [ ] **Step 6: Run the tests**

Run: `cargo test --workspace 2>&1 | grep -E "^test result|FAILED|panicked|^error" | sort | uniq -c`

Expected: only `test result: ok` lines. Then `cargo clippy --workspace --all-targets 2>&1 | grep -E "^(warning|error)" | sort | uniq -c` shows nothing new in air-gesturesd/air-gestures.

- [ ] **Step 7: Commit**

```bash
git add band-core Cargo.lock air-gesturesd/src/config.rs air-gesturesd/src/daemon.rs air-gesturesd/tests/config.rs air-gesturesd/tests/daemon.rs contrib/config.toml
git commit -q -F - <<'EOF'
band-core's band controls: [band] toggle, next_profile, previous_profile, draw

Band controls take any gesture and optional palm positions; @ values are a
config error; the first profile's defaults are 0.3.10's again; every config
load logs the warnings and each paused entry.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01MLDqGPYx8bTGKJcVLgoyzo
EOF
```

(`Cargo.lock` only if it changed; `git status -s` must show nothing else staged.)

---

### Task 2: Per-finger dials, drag modes and sliders; the IPC dial override through `set_dial`

**Files:**
- Modify: `air-gesturesd/src/config.rs`, `air-gesturesd/src/daemon.rs`, `air-gesturesd/tests/config.rs`, `air-gesturesd/tests/daemon.rs`

Decision: slider keys written while that finger's drags are `"gestures"` are accepted and unused (so `index_drags` can be flipped without deleting lines).

- [ ] **Step 1: Write the failing tests**

In `tests/config.rs`, replace every `config.profiles(DialTarget::X)` / `profiles(target)` with `config.profiles()` (the dial comes from `index_dial` / `[dial] target` now). In `defaults_match_the_design`, the last three assertions become:

```rust
    let first = &config.profiles().list[0];
    assert_eq!(first.index.dial.clone().unwrap().up, "wpctl set-volume -l 1.0 @DEFAULT_AUDIO_SINK@ 2%+");
    assert!(first.middle.dial.is_none());
    assert_eq!(first.index.drags, DragMode::Gestures);
    assert_eq!(config.index_dials(), [DialTarget::Volume]);
    let off = Config::parse("[dial]\ntarget = \"off\"\n").unwrap();
    assert!(off.profiles().list[0].index.dial.is_none());
```

`the_documented_config_is_the_default` compares `documented.profiles()` with `default.profiles()` once. `partial_files_override_only_what_they_name` reads `config.profiles().list[0]` (target brightness comes from `[dial] target`). Add `DragMode` to the `band_core::controls` import, and:

```rust
#[test]
fn fingers_have_dials_drag_modes_and_sliders() {
    let config = Config::parse(
        r#"
        [gestures]
        index_dial = "brightness"
        middle_dial = "volume"
        index_drag_left = "echo left"
        middle_drags = "sliders"
        middle_slider_up_down = "brightness"
        index_slider_left_right = "volume"
        [[profile]]
        name = "b"
        "#,
    )
    .unwrap();
    let profiles = config.profiles();
    let dial = |target| config.dial.dial(target);
    let first = &profiles.list[0];
    assert_eq!(first.index.dial, dial(DialTarget::Brightness));
    assert_eq!(first.middle.dial, dial(DialTarget::Volume));
    assert_eq!(first.index.drags, DragMode::Gestures, "sliders written, mode gestures: unused");
    assert_eq!(first.middle.drags, DragMode::Sliders([None, None, dial(DialTarget::Brightness)]));
    assert_eq!(first.gestures.get(&Gesture::IndexDragLeft).cloned(), run("echo left"));
    let b = &profiles.list[1];
    assert_eq!(b.index.dial, dial(DialTarget::Volume), "[dial] target is every profile's index default");
    assert!(b.middle.dial.is_none());
    assert_eq!(config.index_dials(), [DialTarget::Brightness, DialTarget::Volume]);
}

#[test]
fn a_band_control_on_a_drag_pauses_that_pairs_slider() {
    let config = Config::parse(
        "[band]\ndraw = \"middle_drag_up\"\n[gestures]\nmiddle_drags = \"sliders\"\nmiddle_slider_up_down = \"volume\"\n",
    )
    .unwrap();
    let notes = notes(&config.profiles());
    assert!(
        notes.iter().any(|n| n.level == Level::Warn
            && n.text == "middle_slider_up_down in profile \"default\" is overridden by [band] draw on middle_drag_up"),
        "{notes:?}"
    );
}
```

and add to `mistakes_are_reported`:

```rust
        ("[gestures]\nindex_dial = \"loud\"\n", "loud"),
        ("[gestures]\nmiddle_drags = \"both\"\n", "both"),
        ("[gestures]\nindex_slider_sideways = \"volume\"\n", "index_slider_sideways"),
        ("[gestures]\nring_dial = \"volume\"\n", "ring_dial"),
```

Update `texts()` in the test file to `notes(&config.profiles())`.

In `tests/daemon.rs` add:

```rust
#[tokio::test(start_paused = true)]
async fn the_dial_override_lasts_until_the_profile_changes() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    daemon.configure("[[profile]]\nname = \"quiet\"\nindex_dial = \"off\"\n").await;
    assert_eq!(daemon.status.borrow().dial, DialTarget::Volume);
    daemon.call(Request::Dial { target: DialTarget::Brightness }).await;
    daemon.wait("brightness", |s| s.dial == DialTarget::Brightness).await;
    daemon.call(Request::Profile { to: "quiet".into() }).await;
    daemon.wait("quiet's own dial", |s| s.dial == DialTarget::Off).await;
    daemon.call(Request::CycleDial).await;
    daemon.wait("cycled from off", |s| s.dial == DialTarget::Volume).await;
    daemon.call(Request::Profile { to: "default".into() }).await;
    daemon.wait("default's own dial", |s| s.dial == DialTarget::Volume && s.profile.as_deref() == Some("default")).await;
}
```

(`use air_gesturesd::config::DialTarget;` at the top of the file.)

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p air-gesturesd --test config 2>&1 | tail -3` — Expected: compile errors (`profiles()` takes an argument, no `index_dials`).

- [ ] **Step 3: Implement `config.rs`**

Import `DragMode` and `FingerSetup`. Add:

```rust
/// A profile's per-finger keys: `index_dial`, `middle_drags`, `index_slider_up_down`, ….
struct Fingers {
    dial: [Option<DialTarget>; 2],
    sliders_mode: [bool; 2],
    sliders: [[DialTarget; 3]; 2],
}

const SLIDER_PAIRS: [&str; 3] = ["left_right", "toward_away", "up_down"];

impl Default for Fingers {
    fn default() -> Self {
        Self { dial: [None; 2], sliders_mode: [false; 2], sliders: [[DialTarget::Off; 3]; 2] }
    }
}

fn parse_target(place: &str, key: &str, value: &str) -> Result<DialTarget, String> {
    match value.trim() {
        "volume" => Ok(DialTarget::Volume),
        "brightness" => Ok(DialTarget::Brightness),
        "off" => Ok(DialTarget::Off),
        other => Err(format!("{place}.{key}: \"{other}\" isn't a dial (\"volume\", \"brightness\" or \"off\")")),
    }
}

impl Fingers {
    /// Takes a per-finger key; `false` when `key` isn't one.
    fn set(&mut self, place: &str, key: &str, value: &str) -> Result<bool, String> {
        let (slot, rest) = match (key.strip_prefix("index_"), key.strip_prefix("middle_")) {
            (Some(rest), _) => (0, rest),
            (_, Some(rest)) => (1, rest),
            _ => return Ok(false),
        };
        match rest {
            "dial" => self.dial[slot] = Some(parse_target(place, key, value)?),
            "drags" => {
                self.sliders_mode[slot] = match value.trim() {
                    "gestures" => false,
                    "sliders" => true,
                    other => return Err(format!("{place}.{key}: \"{other}\" (\"gestures\" or \"sliders\")")),
                }
            }
            _ => match rest.strip_prefix("slider_").and_then(|pair| SLIDER_PAIRS.iter().position(|p| *p == pair)) {
                Some(pair) => self.sliders[slot][pair] = parse_target(place, key, value)?,
                None => return Ok(false),
            },
        }
        Ok(true)
    }
}
```

In `impl Config`:

```rust
    /// Both fingers' setups, and what the index dial turns.
    fn fingers(&self, fingers: &Fingers) -> (FingerSetup<String>, FingerSetup<String>, DialTarget) {
        let index = fingers.dial[0].unwrap_or(self.dial.target);
        let middle = fingers.dial[1].unwrap_or(DialTarget::Off);
        let setup = |slot: usize, target: DialTarget| FingerSetup {
            dial: self.dial.dial(target),
            drags: if fingers.sliders_mode[slot] {
                DragMode::Sliders(fingers.sliders[slot].map(|t| self.dial.dial(t)))
            } else {
                DragMode::Gestures
            },
        };
        (setup(0, index), setup(1, middle), index)
    }
```

`build` becomes `fn build(&self) -> Result<(Profiles<String>, Vec<DialTarget>), String>`: per table, a `let mut fingers = Fingers::default();`; in the key loop, before the gesture lookup, `if fingers.set(place, key, value)? { continue; }` (place = `"gestures"` or `profile "<name>"`); after the loop `let (index, middle, target) = self.fingers(&fingers); profile.index = index; profile.middle = middle; dials.push(target);` (drop the `first.index.dial = dial.clone()` lines). Then:

```rust
    /// Every profile.
    pub fn profiles(&self) -> Profiles<String> {
        self.build().expect("checked when the config was parsed").0
    }

    /// What each profile's index dial turns (for status), in profile order.
    pub fn index_dials(&self) -> Vec<DialTarget> {
        self.build().expect("checked when the config was parsed").1
    }
```

`parse` calls `config.build()?`.

- [ ] **Step 4: Implement `daemon.rs`**

The runtime dial target becomes an override through `Controls::set_dial` (cleared by the engine when the active profile changes); the status shows the override or the active profile's own.

- Fields: replace `dial_target: DialTarget` with

```rust
    /// The IPC dial choice, while `controls.dial_overridden()`.
    dial_override: DialTarget,
    /// Each profile's own index dial (from the config), for status.
    index_dials: Vec<DialTarget>,
```

  initialised `dial_override: DialTarget::Off, index_dials: config.index_dials()`; `Controls::new(config.options(), config.profiles())`.
- Add:

```rust
    /// What the index dial turns now: the IPC choice, else the active profile's own.
    fn dial_target(&self) -> DialTarget {
        if self.controls.dial_overridden() {
            self.dial_override
        } else {
            self.index_dials.get(self.controls.active()).copied().unwrap_or(DialTarget::Off)
        }
    }

    fn override_dial(&mut self, target: DialTarget) {
        self.dial_override = target;
        self.controls.set_dial(self.config.dial.dial(target));
    }
```

- `publish`: `dial: self.dial_target(),`.
- `handle`: `Request::Dial { target } => self.override_dial(target),` and `Request::CycleDial => self.override_dial(self.dial_target().next()),`. Delete `rebuild_profiles`.
- `reload_config`, in the `Ok(config)` arm, replacing the `config.dial.target` comparison:

```rust
                let active = self.controls.active();
                let before = self.index_dials.get(active).copied();
                self.config = config;
                self.index_dials = self.config.index_dials();
                let mut outputs = self.controls.set_options(self.config.options());
                outputs.extend(self.controls.set_profiles(self.config.profiles()));
                if self.controls.dial_overridden() {
                    // A changed dial in the file wins over a runtime choice, as before.
                    if self.index_dials.get(self.controls.active()).copied() != before {
                        self.controls.clear_dial();
                    } else {
                        self.override_dial(self.dial_override);
                    }
                }
                log_notes(self.controls.profiles());
                self.apply(outputs, false);
```

- [ ] **Step 5: Run the tests** — as Task 1 Step 6 (the CLI's `simple_requests_change_the_status` still sees `dial: brightness`).

- [ ] **Step 6: Commit** `air-gesturesd/src/config.rs air-gesturesd/src/daemon.rs air-gesturesd/tests/config.rs air-gesturesd/tests/daemon.rs` — message `config: per-finger dials, drag modes and sliders; the dial override is the engine's`.

---

### Task 3: Palm overrides (`[gestures.palm_up]`, `[gestures.sideways]`, `[gestures.palm_down]`)

**Files:**
- Modify: `air-gesturesd/src/config.rs`, `air-gesturesd/tests/config.rs`

- [ ] **Step 1: Write the failing tests** (append to `tests/config.rs`)

```rust
#[test]
fn palm_sub_tables_override_a_gesture_in_that_position() {
    let config = Config::parse(
        r#"
        [gestures]
        index_tap = "echo tap"
        [gestures.palm_up]
        index_tap = "echo up"
        swipe_left = ""
        [[profile]]
        name = "b"
        [profile.sideways]
        middle_tap = "echo side"
        "#,
    )
    .unwrap();
    let profiles = config.profiles();
    let tap = &profiles.list[0].gestures[&Gesture::IndexTap];
    assert_eq!(tap.action(Palm::Down).map(String::as_str), Some("echo tap"));
    assert_eq!(tap.action(Palm::Up).map(String::as_str), Some("echo up"));
    let left = &profiles.list[0].gestures[&Gesture::SwipeLeft];
    assert_eq!(left.action(Palm::Up), None, "\"\" = nothing with the palm up");
    assert_eq!(left.action(Palm::Side).map(String::as_str), Some("hyprctl dispatch workspace e-1"));
    let side = &profiles.list[1].gestures[&Gesture::MiddleTap];
    assert_eq!(side.normal, None);
    assert_eq!(side.action(Palm::Side).map(String::as_str), Some("echo side"));
}

#[test]
fn three_palm_overrides_on_one_gesture_are_an_error() {
    let error = Config::parse(
        "[gestures.palm_up]\nindex_tap = \"a\"\n[gestures.sideways]\nindex_tap = \"b\"\n[gestures.palm_down]\nindex_tap = \"c\"\n",
    )
    .unwrap_err();
    assert!(error.contains("index_tap") && error.contains("at most 2"), "{error}");
}

#[test]
fn an_override_a_band_control_pauses_is_named_with_its_position() {
    let config = Config::parse(
        r#"
        [band]
        draw = "index_tap"
        draw_palm = ["up"]
        [gestures]
        index_tap = "echo tap"
        [gestures.palm_up]
        index_tap = "echo up"
        "#,
    )
    .unwrap();
    let texts: Vec<String> = notes(&config.profiles()).into_iter().map(|n| n.text).collect();
    assert!(texts.contains(&"index_tap (palm_up) in profile \"default\" is overridden by [band] draw with the palm up".to_owned()), "{texts:?}");
    assert!(!texts.iter().any(|t| t.starts_with("index_tap in")), "the normal action isn't used with the palm up: {texts:?}");
}
```

and to `mistakes_are_reported`:

```rust
        ("[gestures.palm_up]\nnope = \"x\"\n", "nope"),
        ("[gestures.palm_up]\nindex_tap = \"@controls\"\n", "[band]"),
        ("[gestures]\nindex_tap = 3\n", "expected a string"),
        ("[gestures.palm_left]\nindex_tap = \"x\"\n", "palm_left"),
        ("[gestures]\npalm_up = \"x\"\n", "expected a table"),
```

- [ ] **Step 2: Run them to see them fail** — `cargo test -p air-gesturesd --test config` fails (sub-tables don't deserialize into `BTreeMap<String, String>`).

- [ ] **Step 3: Implement**

The raw tables become `toml::Table`:

```rust
    /// The first profile as written (unwritten gesture keys take `default_gesture`).
    gestures: toml::Table,
    /// More profiles, in order (`name` plus the same keys; unwritten gesture keys do nothing).
    profile: Vec<toml::Table>,
```

Add a string accessor and one function for every profile table, used by `build` for `[gestures]` (`place = "gestures"`, `first = true`) and each `[[profile]]` (`place = profile "<name>"`, `first = false`; `name` is read with `table.get("name").and_then(toml::Value::as_str)` and checked as before):

```rust
fn text<'a>(place: &str, key: &str, value: &'a toml::Value) -> Result<&'a str, String> {
    value.as_str().ok_or_else(|| format!("{place}.{key}: expected a string"))
}

impl Config {
    /// One profile from its table. `first` starts from the defaults. Also returns what its
    /// index dial turns.
    fn profile_from(&self, name: &str, table: &toml::Table, place: &str, first: bool) -> Result<(Profile<String>, DialTarget), String> {
        let mut profile = Profile::new(name);
        if first {
            for gesture in Gesture::ALL {
                if let Some(command) = default_gesture(gesture) {
                    profile.gestures.insert(gesture, Binding::from(command.to_owned()));
                }
            }
        }
        let mut fingers = Fingers::default();
        for (key, value) in table {
            let key = key.as_str();
            if (key == "name" && !first) || Palm::ALL.into_iter().any(|p| palm_section(p) == key) {
                continue;
            }
            let value = text(place, key, value)?;
            if fingers.set(place, key, value)? {
                continue;
            }
            let gesture = Gesture::from_key(key).ok_or_else(|| format!("{place}: unknown key \"{key}\""))?;
            match command(place, key, value)? {
                Some(command) => profile.gestures.insert(gesture, Binding::from(command)),
                None => profile.gestures.remove(&gesture),
            };
        }
        for palm in Palm::ALL {
            let section = palm_section(palm);
            let Some(value) = table.get(section) else { continue };
            let at = format!("{place}.{section}");
            let sub = value.as_table().ok_or_else(|| format!("{at}: expected a table of gesture = command"))?;
            for (key, value) in sub {
                let gesture = Gesture::from_key(key).ok_or_else(|| format!("{at}: unknown gesture \"{key}\""))?;
                let action = command(&at, key, text(&at, key, value)?)?;
                profile
                    .gestures
                    .entry(gesture)
                    .or_insert_with(|| Binding { normal: None, overrides: BTreeMap::new() })
                    .overrides
                    .insert(palm, action);
            }
        }
        if let Some((gesture, _)) = profile.gestures.iter().find(|(_, binding)| !binding.valid()) {
            return Err(format!(
                "{place}: {} has palm overrides in palm_down, sideways and palm_up; at most 2 (the normal action covers the third position)",
                gesture.key()
            ));
        }
        let (index, middle, target) = self.fingers(&fingers);
        profile.index = index;
        profile.middle = middle;
        Ok((profile, target))
    }
}
```

`[gestures] palm_left = …`-style keys fall through to "unknown key", and a string `palm_up = "x"` hits "expected a table". The duplicate-name and `name` checks stay in `build`.

- [ ] **Step 4: Run the tests** — as Task 1 Step 6.

- [ ] **Step 5: Commit** `air-gesturesd/src/config.rs air-gesturesd/tests/config.rs` — message `config: palm overrides in [gestures.palm_up], .sideways, .palm_down`.

---

### Task 4: Drawings' actions (`[drawings]`, `[profile.drawings]`)

**Files:**
- Modify: `air-gesturesd/src/config.rs`, `air-gesturesd/tests/config.rs`

Decision: the first profile's own drawings go in `[gestures.drawings]` (same code path as `[profile.drawings]`).

- [ ] **Step 1: Write the failing tests** (append)

```rust
#[test]
fn drawings_have_shared_and_profile_actions() {
    let config = Config::parse(
        r#"
        [drawings]
        lightbulb = "hass-cli light toggle"
        z = "echo z"
        nothing = ""
        [[profile]]
        name = "media"
        [profile.drawings]
        z = "playerctl next"
        "#,
    )
    .unwrap();
    let p = config.profiles();
    assert_eq!(p.drawing_action(0, "z").map(String::as_str), Some("echo z"));
    assert_eq!(p.drawing_action(1, "z").map(String::as_str), Some("playerctl next"), "a profile's own wins");
    assert_eq!(p.drawing_action(1, "lightbulb").map(String::as_str), Some("hass-cli light toggle"));
    assert_eq!(p.drawing_action(0, "nothing"), None);
}

#[test]
fn drawings_with_an_action_but_no_template_are_noted() {
    let config = Config::parse("[drawings]\nlightbulb = \"x\"\nz = \"y\"\n").unwrap();
    let notes = air_gesturesd::config::missing_drawings(&config.profiles(), &["z"]);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(notes[0].level, Level::Warn);
    assert!(notes[0].text.contains("air-gestures draw teach lightbulb"), "{notes:?}");
}
```

and to `mistakes_are_reported`:

```rust
        ("[drawings]\n\"it's\" = \"x\"\n", "it's"),
        ("[drawings]\nz = \"@draw\"\n", "[band]"),
        ("[drawings]\nz = 1\n", "expected a string"),
        ("[[profile]]\nname = \"x\"\ndrawings = \"z\"\n", "expected a table"),
```

- [ ] **Step 2: Run them to see them fail** (`missing_drawings` doesn't exist; `drawings` is an unknown field).

- [ ] **Step 3: Implement**

Field on `Config`: `/// Drawing name → command in every profile without its own. drawings: toml::Table,`. Add:

```rust
fn drawing_actions(at: &str, table: &toml::Table) -> Result<BTreeMap<String, String>, String> {
    let mut actions = BTreeMap::new();
    for (name, value) in table {
        check_name("drawing", name).map_err(|e| format!("{at}: {e}"))?;
        if let Some(command) = command(at, name, text(at, name, value)?)? {
            actions.insert(name.clone(), command);
        }
    }
    Ok(actions)
}

/// Drawings with an action whose template isn't taught (yet).
pub fn missing_drawings(profiles: &Profiles<String>, taught: &[&str]) -> Vec<Note> {
    let named: std::collections::BTreeSet<&str> = profiles
        .drawings
        .keys()
        .chain(profiles.list.iter().flat_map(|p| p.drawings.keys()))
        .map(String::as_str)
        .collect();
    named
        .into_iter()
        .filter(|name| !taught.contains(name))
        .map(|name| Note::new(Level::Warn, format!("drawing \"{name}\" has an action but isn't taught yet: `air-gestures draw teach {name}`")))
        .collect()
}
```

In `profile_from`: skip `"drawings"` in the key loop (with `name` and the palm sections), and before the fingers:

```rust
        if let Some(value) = table.get("drawings") {
            let at = format!("{place}.drawings");
            let sub = value.as_table().ok_or_else(|| format!("{at}: expected a table of drawing = command"))?;
            profile.drawings = drawing_actions(&at, sub)?;
        }
```

In `build`: `drawings: drawing_actions("drawings", &self.drawings)?` in the `Profiles` literal.

- [ ] **Step 4: Run the tests** — as Task 1 Step 6.

- [ ] **Step 5: Commit** `air-gesturesd/src/config.rs air-gesturesd/tests/config.rs` — message `config: drawings' actions, shared and per profile`.

---

### Task 5: Calibrations in the store, per band and hand

**Files:**
- Create: `air-gesturesd/src/teach.rs` (the error texts; Task 8 adds the waiting step)
- Modify: `air-gesturesd/src/lib.rs`, `air-gesturesd/src/store.rs`, `air-gesturesd/tests/store.rs`

Decisions: `calibration.json` sits in the private state dir like the other files (0600, atomic); `forget` keeps it, because it describes the wrist and the band's sensors, not the owner key (re-pairing the same band keeps its calibration).

- [ ] **Step 1: Write the failing tests** (append to `tests/store.rs`)

```rust
use air_gesturesd::store::{HandCalibration, StoredDrags, StoredPalm};
use band_core::controls::Options;
use band_core::controls::palm::PalmCalibration;
use band_core::controls::pinch::Prototypes;
use band_core::events::Hand;

fn taught() -> HandCalibration {
    HandCalibration {
        palm: Some(StoredPalm { down_roll: -140.0, up_roll: -10.0 }),
        drags: StoredDrags::from_prototypes(&Prototypes::right_hand()),
    }
}

#[test]
fn calibrations_round_trip_per_band_and_hand_and_survive_forget() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    assert_eq!(store.load_calibration("AA", Hand::Right).unwrap(), HandCalibration::default());
    store.save_calibration("AA", Hand::Right, &taught()).unwrap();
    let left = HandCalibration { palm: Some(StoredPalm { down_roll: 30.0, up_roll: 150.0 }), drags: None };
    store.save_calibration("BB", Hand::Left, &left).unwrap();
    assert_eq!(store.load_calibration("AA", Hand::Right).unwrap(), taught());
    assert_eq!(store.load_calibration("AA", Hand::Left).unwrap(), HandCalibration::default());
    assert_eq!(store.load_calibration("BB", Hand::Left).unwrap(), left);
    assert_eq!(mode(&root.path().join("calibration.json")), 0o600);
    store.forget().unwrap();
    assert_eq!(store.load_calibration("AA", Hand::Right).unwrap(), taught(), "it describes the wrist, not the key");
    store.save_calibration("AA", Hand::Right, &HandCalibration::default()).unwrap();
    assert_eq!(store.load_calibration("AA", Hand::Right).unwrap(), HandCalibration::default());
}

#[test]
fn a_corrupt_calibration_file_is_an_error() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    std::fs::write(root.path().join("calibration.json"), "{nope").unwrap();
    let error = store.load_calibration("AA", Hand::Right).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn a_calibration_goes_into_the_options_and_lookalike_drags_dont() {
    let mut options = Options::default();
    assert!(taught().apply(&mut options).is_empty());
    assert_eq!(options.palm, Some(PalmCalibration { down_roll: -140.0, up_roll: -10.0 }));
    let measured = Prototypes::right_hand().drags.unwrap();
    for (a, b) in options.drags.as_ref().unwrap().drags.unwrap().iter().zip(measured) {
        assert!((0..3).all(|k| (a[k] - b[k]).abs() < 1e-12));
    }
    let mut bad = taught();
    let stored = bad.drags.as_mut().unwrap();
    stored.drags[1] = stored.drags[0];
    let problems = bad.apply(&mut options);
    assert_eq!(problems.len(), 1);
    assert!(problems[0].contains("teach-drags"), "{problems:?}");
    assert!(options.drags.is_none());
    HandCalibration::default().apply(&mut options);
    assert!(options.palm.is_none(), "nothing taught: the core's defaults");
}
```

- [ ] **Step 2: Run them to see them fail** — `cargo test -p air-gesturesd --test store` fails to compile.

- [ ] **Step 3: Create `air-gesturesd/src/teach.rs`** (add `pub mod teach;` to `lib.rs`)

```rust
//! Teaching: the words for what went wrong (shared with `air-gestures`), and (Task 8) the
//! teaching steps that wait for the band.

use band_core::controls::Drag;
use band_core::controls::palm::{CalibrationError, Palm};
use band_core::controls::pinch::TaughtError;

pub fn drag_name(drag: Drag) -> &'static str {
    match drag {
        Drag::Left => "left",
        Drag::Right => "right",
        Drag::Toward => "toward",
        Drag::Away => "away",
        Drag::Up => "up",
        Drag::Down => "down",
    }
}

fn palm_name(palm: Palm) -> &'static str {
    match palm {
        Palm::Down => "palm down",
        Palm::Side => "palm sideways",
        Palm::Up => "palm up",
    }
}

pub fn taught_error_text(error: TaughtError) -> String {
    match error {
        TaughtError::NoTurn => "the turn didn't move: pinch and turn your wrist".into(),
        TaughtError::NoDrag(drag) => format!("the {} drag didn't move", drag_name(drag)),
        TaughtError::TooAlike(a, b) => format!("{} and {} came out alike: make them clearly different", drag_name(a), drag_name(b)),
        TaughtError::LikeTheTurn(drag) => format!("{} came out like the turn: move your hand without turning your wrist", drag_name(drag)),
    }
}

pub fn calibration_error_text(error: CalibrationError) -> String {
    match error {
        CalibrationError::NotStill(palm) => format!("you moved while holding {}: once more, and hold still", palm_name(palm)),
        CalibrationError::Empty(palm) => format!("couldn't read {}: keep your forearm level (not pointing up or down)", palm_name(palm)),
        CalibrationError::TooClose => "palm down and palm up came out less than 60° apart: turn your hand all the way over".into(),
    }
}
```

- [ ] **Step 4: Implement `store.rs`**

Add `use std::collections::BTreeMap;`, `use band_core::controls::{Options, PalmCalibration};`, `use band_core::controls::pinch::{Prototypes, TaughtError};`, `use band_core::events::Hand;`, and `const CALIBRATION: &str = "calibration.json";`. Extend the module doc: "…, the Meta session and, per band and hand, what was taught (`calibration.json`)".

```rust
/// Palm down and up, as the band's roll about the forearm (degrees).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StoredPalm {
    pub down_roll: f64,
    pub up_roll: f64,
}

impl StoredPalm {
    pub fn calibration(&self) -> PalmCalibration {
        PalmCalibration { down_roll: self.down_roll, up_roll: self.up_roll }
    }
}

impl From<PalmCalibration> for StoredPalm {
    fn from(c: PalmCalibration) -> Self {
        Self { down_roll: c.down_roll, up_roll: c.up_roll }
    }
}

/// Taught drag directions (forearm frame, unit): the turn, then the six drags in
/// `Drag::ALL` order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredDrags {
    pub palm_down_roll: f64,
    pub turn: [f64; 3],
    pub drags: [[f64; 3]; 6],
}

impl StoredDrags {
    /// `None` for turns-only prototypes (nothing taught).
    pub fn from_prototypes(p: &Prototypes) -> Option<Self> {
        Some(Self { palm_down_roll: p.palm_down_roll, turn: p.turn, drags: p.drags? })
    }

    /// Checked again on the way in: a hand-edited file can't make lookalike drags.
    pub fn prototypes(&self) -> Result<Prototypes, TaughtError> {
        Prototypes::taught(self.turn, self.drags, self.palm_down_roll)
    }
}

/// What was taught on one band for one hand.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct HandCalibration {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palm: Option<StoredPalm>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drags: Option<StoredDrags>,
}

impl HandCalibration {
    pub fn is_empty(&self) -> bool {
        self.palm.is_none() && self.drags.is_none()
    }

    /// Into `options` (meant for the hand in use; nothing taught = the core's defaults).
    /// Returns what couldn't be used.
    pub fn apply(&self, options: &mut Options) -> Vec<String> {
        let mut problems = Vec::new();
        options.palm = self.palm.map(|p| p.calibration());
        options.drags = match self.drags.as_ref().map(StoredDrags::prototypes) {
            None => None,
            Some(Ok(prototypes)) => Some(prototypes),
            Some(Err(error)) => {
                problems.push(format!(
                    "the taught drags can't be used ({}); teach them again with `air-gestures teach-drags`",
                    crate::teach::taught_error_text(error)
                ));
                None
            }
        };
        problems
    }
}

fn hand_key(hand: Hand) -> &'static str {
    match hand {
        Hand::Left => "left",
        Hand::Right => "right",
    }
}
```

In `impl Store`:

```rust
    /// What was taught on the band at `address` for `hand` (the default when nothing was).
    pub fn load_calibration(&self, address: &str, hand: Hand) -> io::Result<HandCalibration> {
        Ok(self.calibrations()?.get(address).and_then(|hands| hands.get(hand_key(hand))).cloned().unwrap_or_default())
    }

    /// Replace it (an empty one removes the entry).
    pub fn save_calibration(&self, address: &str, hand: Hand, calibration: &HandCalibration) -> io::Result<()> {
        let mut all = self.calibrations()?;
        let hands = all.entry(address.to_owned()).or_default();
        if calibration.is_empty() {
            hands.remove(hand_key(hand));
        } else {
            hands.insert(hand_key(hand).to_owned(), calibration.clone());
        }
        all.retain(|_, hands| !hands.is_empty());
        let json = serde_json::to_vec_pretty(&all).map_err(io::Error::other)?;
        self.write_secret(CALIBRATION, &json)
    }

    /// Band address → hand ("left"/"right") → what was taught.
    fn calibrations(&self) -> io::Result<BTreeMap<String, BTreeMap<String, HandCalibration>>> {
        match self.read(CALIBRATION)? {
            Some(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, format!("{CALIBRATION}: {error}"))),
            None => Ok(BTreeMap::new()),
        }
    }
```

Leave `forget` as it is (and say so in its doc: "… `calibration.json` survives too: it describes the wrist").

- [ ] **Step 5: Run the tests** — as Task 1 Step 6.

- [ ] **Step 6: Commit** `air-gesturesd/src/lib.rs air-gesturesd/src/teach.rs air-gesturesd/src/store.rs air-gesturesd/tests/store.rs` — message `store: palm calibration and taught drags per band and hand`.

---

### Task 6: The drawings directory

**Files:**
- Create: `air-gesturesd/src/drawings.rs`, `air-gesturesd/tests/drawings.rs`
- Modify: `air-gesturesd/src/lib.rs` (`pub mod drawings;`)

- [ ] **Step 1: Write the failing tests** (`air-gesturesd/tests/drawings.rs`)

```rust
use air_gesturesd::drawings;
use band_core::controls::draw::Drawing;

fn line(length: f64) -> Vec<Vec<[f64; 2]>> {
    vec![vec![[0.0, 0.0], [0.0, length]]]
}

fn drawing(name: &str) -> Drawing {
    Drawing { name: name.into(), tries: vec![line(10.0), line(12.0), line(11.0)] }
}

#[test]
fn drawings_round_trip_by_name_and_can_be_deleted() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("drawings");
    assert_eq!(drawings::load(&dir), (Vec::new(), Vec::new()), "no directory: no drawings");
    assert_eq!(drawings::stamp(&dir), None);
    drawings::save(&dir, &drawing("z")).unwrap();
    drawings::save(&dir, &drawing("light bulb")).unwrap();
    let (loaded, problems) = drawings::load(&dir);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(loaded, [drawing("light bulb"), drawing("z")]);
    assert!(dir.join("z.json").exists());
    let before = drawings::stamp(&dir);
    assert!(before.is_some());
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert!(drawings::delete(&dir, "z").unwrap());
    assert!(!drawings::delete(&dir, "z").unwrap(), "already gone");
    assert_ne!(drawings::stamp(&dir), before, "a delete changes the stamp");
    assert_eq!(drawings::load(&dir).0, [drawing("light bulb")]);
}

#[test]
fn unreadable_files_are_skipped_and_described() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path();
    drawings::save(dir, &drawing("ok")).unwrap();
    std::fs::write(dir.join("broken.json"), "{").unwrap();
    std::fs::write(dir.join("old.json"), r#"{"version":9,"tries":[]}"#).unwrap();
    std::fs::write(dir.join(".ok.json.tmp"), "{").unwrap();
    std::fs::write(dir.join("notes.txt"), "hi").unwrap();
    let (loaded, problems) = drawings::load(dir);
    assert_eq!(loaded, [drawing("ok")]);
    assert_eq!(problems.len(), 2, "{problems:?}");
    assert!(problems.iter().any(|p| p.contains("broken.json")));
    assert!(problems.iter().any(|p| p.contains("version 9")));
}

#[test]
fn names_are_checked() {
    for bad in ["", ".hidden", "../up", "it's", "a/b"] {
        assert!(drawings::check_name(bad).is_err(), "{bad:?}");
    }
    assert!(drawings::check_name("light bulb-2.v").is_ok());
    let root = tempfile::tempdir().unwrap();
    assert!(drawings::save(root.path(), &drawing("../x")).is_err());
}

#[test]
fn the_file_is_versioned_json_of_strokes() {
    let root = tempfile::tempdir().unwrap();
    drawings::save(root.path(), &drawing("z")).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(root.path().join("z.json")).unwrap()).unwrap();
    assert_eq!(json["version"], 1);
    assert_eq!(json["tries"][0][0][1], serde_json::json!([0.0, 10.0]));
}
```

(`air-gesturesd`'s dev-dependencies already have `tempfile`; `serde_json` is a normal dependency.)

- [ ] **Step 2: Run them to see them fail** — no `drawings` module.

- [ ] **Step 3: Implement `air-gesturesd/src/drawings.rs`**

```rust
//! Taught drawings on disk, one `<name>.json` per drawing in
//! `$XDG_DATA_HOME/air-gestures/drawings` (else `~/.local/share/air-gestures/drawings`):
//! `{"version": 1, "tries": [[[[x, y], …], …], …]}`, each try a list of strokes, each stroke
//! a list of [x, y] points in degrees (band-core's `draw::Stroke`).

use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use band_core::controls::draw::{Drawing, Stroke};
use serde::{Deserialize, Serialize};

const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    tries: Vec<Vec<Stroke>>,
}

/// The directory (the dry-run daemon's is separate, like its state).
pub fn default_dir(dry_run: bool) -> PathBuf {
    let base = crate::config::xdg("XDG_DATA_HOME").unwrap_or_else(|| crate::config::home().join(".local/share"));
    base.join(if dry_run { "air-gestures-dry-run" } else { "air-gestures" }).join("drawings")
}

/// Drawing names are file names and go into `AIR_GESTURES_DRAWING='…'`.
pub fn check_name(name: &str) -> Result<(), String> {
    crate::config::check_name("drawing", name)
}

fn path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.json"))
}

fn invalid(error: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, error)
}

/// Every drawing in `dir`, by name; what couldn't be read, described. No directory: none.
pub fn load(dir: &Path) -> (Vec<Drawing>, Vec<String>) {
    let (mut drawings, mut problems) = (Vec::new(), Vec::new());
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return (drawings, problems),
        Err(error) => {
            problems.push(format!("{}: {error}", dir.display()));
            return (drawings, problems);
        }
    };
    for entry in entries.flatten() {
        let file = entry.path();
        let Some(name) = file.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".json")) else { continue };
        if check_name(name).is_err() {
            continue;
        }
        let read = std::fs::read(&file)
            .map_err(|e| e.to_string())
            .and_then(|bytes| serde_json::from_slice::<File>(&bytes).map_err(|e| e.to_string()));
        match read {
            Ok(f) if f.version == VERSION => drawings.push(Drawing { name: name.to_owned(), tries: f.tries }),
            Ok(f) => problems.push(format!("{}: version {} (this air-gesturesd reads {VERSION})", file.display(), f.version)),
            Err(error) => problems.push(format!("{}: {error}", file.display())),
        }
    }
    drawings.sort_by(|a, b| a.name.cmp(&b.name));
    (drawings, problems)
}

/// Write it atomically (a temp file renamed into place), creating `dir` if needed.
pub fn save(dir: &Path, drawing: &Drawing) -> io::Result<()> {
    check_name(&drawing.name).map_err(invalid)?;
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_vec(&File { version: VERSION, tries: drawing.tries.clone() }).map_err(io::Error::other)?;
    let temp = dir.join(format!(".{}.json.tmp", drawing.name));
    std::fs::write(&temp, json)?;
    std::fs::rename(&temp, path(dir, &drawing.name))
}

/// Remove it; `false` when there was none.
pub fn delete(dir: &Path, name: &str) -> io::Result<bool> {
    check_name(name).map_err(invalid)?;
    match std::fs::remove_file(path(dir, name)) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// Changes whenever a drawing is saved or deleted: the directory's modification time.
pub fn stamp(dir: &Path) -> Option<SystemTime> {
    std::fs::metadata(dir).and_then(|m| m.modified()).ok()
}
```

`check_name` refuses `/` (not alphanumeric) and a leading `.`, so `../x` can't escape `dir`.

- [ ] **Step 4: Run the tests** — as Task 1 Step 6.

- [ ] **Step 5: Commit** `air-gesturesd/src/lib.rs air-gesturesd/src/drawings.rs air-gesturesd/tests/drawings.rs` — message `drawings: templates on disk, one file per drawing`.

---

### Task 7: The daemon: calibration per hand, the drawings library, `on_draw`

**Files:**
- Modify: `air-gesturesd/src/config.rs`, `air-gesturesd/src/daemon.rs`, `air-gesturesd/src/main.rs`, `air-gesturesd/src/transport.rs`, `air-gesturesd/tests/daemon.rs`, `air-gesturesd/tests/config.rs`, `air-gestures/tests/support/mod.rs`

Decisions: drawings are reloaded by polling the directory's mtime on the 250 ms tick (as the config is); `DrawPoint` logs at trace (50 per second); no "drawing" state in status (YAGNI).

- [ ] **Step 1: Write the failing tests**

`tests/config.rs`, in `defaults_match_the_design`:

```rust
    assert_eq!(config.band.on_draw, r#"notify-send -a air-gestures "Drawing" "${AIR_GESTURES_DRAWING:-not recognised}""#);
```

`tests/daemon.rs`: `start_seeded` passes `dir.path().join("drawings")` as the new `Daemon::new` argument (after `config_path`) and keeps it in a new `Harness` field `drawings: PathBuf`. Add:

```rust
const PRESS: u64 = 1;
const RELEASE: u64 = 2;
const INDEX: u64 = 2;
const SWIPE_UP: SimInject = SimInject::Gesture { action: 6, finger: 1, derived: 0 };

impl Harness {
    /// An index pinch turning the wrist about the band's x axis: `raw` gyro units for
    /// `samples` × 10 ms. With no fresh orientation that's a vertical stroke in a drawing,
    /// and a drag away from you outside one.
    async fn pinch_moving(&self, raw: i16, samples: usize) {
        self.transport.inject(SimInject::Gesture { action: PRESS, finger: INDEX, derived: 0 });
        for _ in 0..samples {
            self.transport.inject(SimInject::Gyro(raw));
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        self.transport.inject(SimInject::Gesture { action: RELEASE, finger: INDEX, derived: 0 });
    }

    /// Orientation samples with this roll about the forearm, every 50 ms for `seconds`.
    async fn hold_roll(&self, degrees: f64, seconds: f64) {
        for _ in 0..(seconds / 0.05).round() as usize {
            self.transport.inject(SimInject::Orientation(rolled(degrees)));
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// A taught drawing called `name`: three vertical lines.
    fn teach_line(&self, name: &str) {
        let line = vec![vec![[0.0, 0.0], [0.0, 15.0]]];
        let drawing = band_core::controls::draw::Drawing { name: name.into(), tries: vec![line.clone(), line.clone(), line] };
        air_gesturesd::drawings::save(&self.drawings, &drawing).unwrap();
    }
}

const DRAWING_CONFIG: &str = "[band]\ndraw = \"swipe_up\"\non_draw = \"osd\"\n[drawings]\nline = \"echo line\"\n";

/// An orientation (w, x, y, z) whose roll about the forearm is `degrees`.
fn rolled(degrees: f64) -> [f32; 4] {
    let half = (-degrees).to_radians() / 2.0;
    [half.cos() as f32, 0.0, half.sin() as f32, 0.0]
}

/// The right hand's measured palm down and palm up (band-core's `PalmCalibration::right_hand`).
const PALM_DOWN: f64 = -142.3;
const PALM_UP: f64 = -11.3;

#[tokio::test(start_paused = true)]
async fn a_palm_override_runs_with_the_palm_that_way() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    daemon.wait("the right hand", |s| s.hand.as_deref() == Some("right")).await;
    daemon.configure("[gestures.palm_up]\nswipe_left = \"echo up\"\n").await;
    daemon.hold_roll(PALM_UP, 0.3).await;
    daemon.transport.inject(SWIPE_LEFT);
    daemon.wait("the palm-up command", |s| s.last_action.as_deref() == Some("echo up")).await;
    daemon.hold_roll(PALM_DOWN, 0.6).await; // also past the 0.4 s action gate
    daemon.transport.inject(SWIPE_LEFT);
    daemon.wait("the normal command", |s| s.last_action.as_deref() == Some("hyprctl dispatch workspace e-1")).await;
}

#[tokio::test(start_paused = true)]
async fn a_stored_calibration_applies_to_the_hand_in_use() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start_seeded(transport.clone(), true, fake(&transport), |store| {
        // A wrist where -11.3° is palm down: the measured palm-up roll no longer reads as up.
        let palm = air_gesturesd::store::StoredPalm { down_roll: -11.3, up_roll: 120.0 };
        let calibration = air_gesturesd::store::HandCalibration { palm: Some(palm), drags: None };
        store.save_calibration(SIM_ADDRESS, band_core::events::Hand::Right, &calibration).unwrap();
    });
    daemon.connected().await;
    daemon.wait("the right hand", |s| s.hand.as_deref() == Some("right")).await;
    daemon.configure("[gestures.palm_up]\nswipe_left = \"echo up\"\n").await;
    daemon.hold_roll(PALM_UP, 0.3).await;
    daemon.transport.inject(SWIPE_LEFT);
    daemon.wait("the normal command", |s| s.last_action.as_deref() == Some("hyprctl dispatch workspace e-1")).await;
}

#[tokio::test(start_paused = true)]
async fn a_drawing_runs_on_draw_then_its_action() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    daemon.teach_line("line"); // after start: the library reloads on the tick
    daemon.configure(DRAWING_CONFIG).await;
    daemon.transport.inject(SWIPE_UP);
    tokio::time::sleep(Duration::from_millis(100)).await;
    daemon.pinch_moving(600, 40).await;
    daemon.wait("the drawing's action", |s| s.last_action.as_deref() == Some("echo line")).await;
    assert_eq!(
        daemon.runner.commands(),
        ["export AIR_GESTURES_DRAWING='line'; osd".to_string(), "echo line".to_string()],
        "swipe_up's own command is paused by [band] draw"
    );
}

#[tokio::test(start_paused = true)]
async fn an_unrecognised_drawing_runs_on_draw_with_nothing() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    daemon.teach_line("line");
    daemon.configure(DRAWING_CONFIG).await;
    daemon.transport.inject(SWIPE_UP);
    tokio::time::sleep(Duration::from_millis(100)).await;
    daemon.pinch_moving(50, 40).await; // 1.4°: too small to read
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(daemon.runner.commands(), ["export AIR_GESTURES_DRAWING=''; osd".to_string()]);
}

#[tokio::test(start_paused = true)]
async fn a_hand_chosen_over_ipc_survives_its_calibration_swap() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    daemon.configure("[band]\nhand = \"right\"\n").await;
    daemon.wait("the right hand", |s| s.hand.as_deref() == Some("right")).await;
    daemon.call(Request::Hand { hand: HandChoice::Left }).await;
    daemon.wait("the left hand", |s| s.hand.as_deref() == Some("left")).await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(daemon.status.borrow().hand.as_deref(), Some("left"), "the left hand's options didn't undo the choice");
}
```

`air-gestures/tests/support/mod.rs`: `spawn` passes `state_dir.join("drawings")` to `Daemon::new`; add

```rust
impl TestDaemon {
    /// Where this daemon reads its drawings.
    pub fn drawings(&self) -> PathBuf {
        self.dir.path().join("drawings")
    }
}
```

(`dir` is the daemon's state dir for both `start` and `start_at`.)

- [ ] **Step 2: Run them to see them fail** — `Daemon::new` takes one argument too many; no `on_draw`.

- [ ] **Step 3: `on_draw` in `config.rs`**

```rust
/// The default `on_draw`: the drawing's name (or "not recognised") as a notification.
pub const DEFAULT_ON_DRAW: &str = r#"notify-send -a air-gestures "Drawing" "${AIR_GESTURES_DRAWING:-not recognised}""#;
```

`BandConfig` gains `/// Run after a drawing, with AIR_GESTURES_DRAWING=<name> exported (empty: nothing matched). pub on_draw: String,` defaulting to `DEFAULT_ON_DRAW.into()`.

- [ ] **Step 4: The daemon**

Imports: `use std::time::SystemTime;` (already), `use band_core::controls::{Builtin, Options, PalmCalibration, PalmSet, Profiles};`, `use band_core::controls::draw::Library;`, `use band_core::controls::pinch::Prototypes;`, `use crate::config::HandSetting;`, `use crate::drawings;`.

New fields:

```rust
    drawings_dir: PathBuf,
    drawings_stamp: Option<SystemTime>,
    /// The taught drawings' names (for the config notes).
    drawing_names: Vec<String>,
    /// The hand whose calibration is in the controls.
    options_hand: Option<Hand>,
```

Free functions:

```rust
/// The configured hand, before the band says.
fn configured_hand(config: &Config) -> Option<Hand> {
    match config.band.hand {
        HandSetting::Left => Some(Hand::Left),
        HandSetting::Right => Some(Hand::Right),
        HandSetting::Band => None,
    }
}

/// What was taught on the paired band for `hand`, for `Controls::set_calibration`
/// (`None`s: the core's defaults for that hand).
fn calibration_for(store: &Store, hand: Option<Hand>) -> (Option<PalmCalibration>, Option<Prototypes>) {
    let mut options = Options::default();
    if let (Some(hand), Ok(Some(band))) = (hand, store.load_band()) {
        match store.load_calibration(&band.address, hand) {
            Ok(calibration) => {
                for problem in calibration.apply(&mut options) {
                    tracing::warn!("calibration: {problem}");
                }
            }
            Err(error) => tracing::warn!("calibration: {error}"),
        }
    }
    (options.palm, options.drags)
}

/// Some gesture or band control depends on the palm position.
fn uses_palm(profiles: &Profiles<String>) -> bool {
    profiles.list.iter().any(|p| p.gestures.values().any(|b| !b.overrides.is_empty()))
        || Builtin::ALL.into_iter().any(|b| profiles.band.control(b).is_some_and(|c| c.palm != PalmSet::All))
}
```

`log_notes` gains the drawings: `fn log_notes(profiles: &Profiles<String>, drawings: &[String])` logs `config::notes(profiles)` then `config::missing_drawings(profiles, &names)` with `let names: Vec<&str> = drawings.iter().map(String::as_str).collect();`.

`Daemon::new(transport, runner, factory, store, config_path, drawings_dir: PathBuf)`:

```rust
        let (drawings, problems) = drawings::load(&drawings_dir);
        for problem in problems {
            tracing::warn!("drawings: {problem}");
        }
        let drawings_stamp = drawings::stamp(&drawings_dir);
        let drawing_names: Vec<String> = drawings.iter().map(|d| d.name.clone()).collect();
        let options_hand = configured_hand(&config);
        let mut controls = Controls::new(config.options(), config.profiles());
        let (palm, drags) = calibration_for(&store, options_hand);
        controls.set_calibration(palm, drags);
        controls.set_library(Library::new(&drawings));
        log_notes(controls.profiles(), &drawing_names);
```

Methods:

```rust
    /// The hand whose calibration applies: the band's, else the configured one.
    fn hand_in_use(&self) -> Option<Hand> {
        self.controls.status().hand.or_else(|| configured_hand(&self.config))
    }

    /// The hand in use changed (or, `force`, what's taught for it): its calibration into the
    /// controls. `set_calibration` keeps the hand (an IPC choice too) and writes nothing.
    fn refresh_calibration(&mut self, force: bool) {
        let hand = self.hand_in_use();
        if !force && hand == self.options_hand {
            return;
        }
        self.options_hand = hand;
        let (palm, drags) = calibration_for(&self.store, hand);
        if hand == Some(Hand::Left) && palm.is_none() && uses_palm(self.controls.profiles()) {
            tracing::info!("the left hand has no palm calibration: every gesture counts as palm down until `air-gestures calibrate`");
        }
        self.controls.set_calibration(palm, drags);
    }

    fn reload_drawings(&mut self) {
        let stamp = drawings::stamp(&self.drawings_dir);
        if stamp == self.drawings_stamp {
            return;
        }
        self.drawings_stamp = stamp;
        let (drawings, problems) = drawings::load(&self.drawings_dir);
        for problem in problems {
            tracing::warn!("drawings: {problem}");
        }
        tracing::info!(count = drawings.len(), "drawings loaded");
        self.drawing_names = drawings.iter().map(|d| d.name.clone()).collect();
        self.controls.set_library(Library::new(&drawings));
        let names: Vec<&str> = self.drawing_names.iter().map(String::as_str).collect();
        for note in crate::config::missing_drawings(self.controls.profiles(), &names) {
            tracing::warn!("config: {}", note.text);
        }
    }

    fn run_draw_hook(&self, name: &str) {
        let hook = self.config.band.on_draw.trim();
        if !hook.is_empty() {
            // Names are checked (config and drawings): no quotes inside.
            self.runner.run(format!("export AIR_GESTURES_DRAWING='{name}'; {hook}"));
        }
    }
```

Wire them in:
- `on_link_event`: after `self.apply(outputs, true);` add `self.refresh_calibration(false);`.
- `on_tick`: after `self.reload_config();` add `self.reload_drawings();`.
- `reload_config`: after `set_options(self.config.options())` (whose options carry no calibration), `self.refresh_calibration(true);` puts it back; and `log_notes(self.controls.profiles(), &self.drawing_names)`.
- `transport.rs`: `SimInject` gains

  ```rust
      /// One gyro sample with all three axes (raw units).
      Gyro3([i16; 3]),
      /// One orientation sample (w, x, y, z).
      Orientation([f32; 4]),
  ```

  and `serve` maps them: `Some(SimInject::Gyro3(axes)) => band.gyro3(axes),` and `Some(SimInject::Orientation(q)) => band.orientation(q),`.
- `apply`: take `Drawn` and `NotDrawn` out of the debug arm:

```rust
                Output::Drawn { name, distance } => {
                    tracing::info!(drawing = %name, distance, "drawn");
                    self.run_draw_hook(&name);
                }
                Output::NotDrawn => {
                    tracing::info!("drawing not recognised");
                    self.run_draw_hook("");
                }
```

  (`Drawn` comes before the drawing's `Run`, so the hook runs first.)
- `main.rs`: both `Daemon::new` calls get `air_gesturesd::drawings::default_dir(dry_run)` as the last argument.

- [ ] **Step 5: Run the tests** — as Task 1 Step 6. If `a_drawing_runs_on_draw_then_its_action` sees no stroke, check with `AIR_GESTURES_LOG=air_gesturesd=debug cargo test -p air-gesturesd --test daemon a_drawing -- --nocapture` that `DrawStarted` and `DrawFinished` appear; the stroke must last ≥ 0.3 s (40 samples) to count.

- [ ] **Step 6: Commit** `air-gesturesd/src/config.rs air-gesturesd/src/daemon.rs air-gesturesd/src/main.rs air-gesturesd/src/transport.rs air-gesturesd/tests/config.rs air-gesturesd/tests/daemon.rs air-gestures/tests/support/mod.rs` — message `daemon: calibration per hand, the drawings library, on_draw`.

---

### Task 8: Teaching over IPC: palm rolls, drag samples, drawing tries, storing them

**Files:**
- Modify: `air-gesturesd/src/teach.rs`, `air-gesturesd/src/ipc.rs`, `air-gesturesd/src/daemon.rs`, `air-gesturesd/tests/ipc.rs`, `air-gesturesd/tests/daemon.rs`
- Create: `air-gesturesd/tests/teach.rs`

Design:
- `palm-rolls {seconds}`: collect the band's roll for `seconds` (0.5–5), answer `teach: {kind: "rolls", rolls, samples}` (rolls = the readable ones; `samples` = orientation samples seen).
- `teach-drag {seconds}`: `set_drag_teaching(true)` until the next `DragSample` or `seconds` (1–60); answer `teach: {kind: "drag", finger, direction}`. Decision: one request per sample (teaching is off between them), so a killed CLI leaves the band in teaching mode for at most `seconds`.
- `draw-try {seconds}`: `set_teaching(true)` and `start_drawing` now; answer the `DrawFinished` strokes (`teach: {kind: "strokes", strokes}`), or an error on `DrawCancelled` / `seconds` (1–120).
- One waiting step at a time; each needs a connected band; a disconnect fails it.
- `set-palm {palm: {down_roll, up_roll} | null}` / `set-drags {drags: {turn, drags} | null}`: store for the paired band and the hand in use (`null` forgets), and apply (`set_calibration`). The daemon checks them again (`from_rolls` / `Prototypes::taught`).
- `palm`: answers at once with the palm position the band reads now (`Controls::palm`), `teach: {kind: "palm", palm: "down" | "sideways" | "up"}`; `calibrate` uses it to confirm.

- [ ] **Step 1: Write the failing tests**

`air-gesturesd/tests/teach.rs`:

```rust
use air_gesturesd::ipc::Taught;
use air_gesturesd::teach::{Done, Step, Wait, calibration_error_text, taught_error_text};
use band_core::controls::palm::{CalibrationError, Palm};
use band_core::controls::pinch::TaughtError;
use band_core::controls::{Drag, Finger, Output};
use band_core::events::Event;

/// An orientation whose roll about the forearm is `degrees` (a turn about the band's y axis).
fn rolled(degrees: f64) -> Event {
    let half = (-degrees).to_radians() / 2.0;
    Event::Orientation([half.cos() as f32, 0.0, half.sin() as f32, 0.0])
}

#[test]
fn rolls_are_collected_until_the_deadline() {
    let mut wait = Wait::new(Step::Rolls, 10.0, 2.0);
    assert_eq!(wait.deadline(), 12.0);
    for _ in 0..5 {
        wait.on_event(&rolled(-140.0));
    }
    // Forearm pointing straight up: the roll can't be read.
    let half = std::f64::consts::FRAC_PI_4;
    wait.on_event(&Event::Orientation([half.cos() as f32, half.sin() as f32, 0.0, 0.0]));
    assert_eq!(wait.on_output(&Output::DrawCancelled), None, "not its step");
    let Done::Taught(Taught::Rolls { rolls, samples }) = wait.timed_out() else { panic!() };
    assert_eq!(samples, 6);
    assert_eq!(rolls.len(), 5);
    assert!(rolls.iter().all(|r| (r + 140.0).abs() < 0.01), "{rolls:?}");
}

#[test]
fn a_drag_step_takes_the_first_sample() {
    let mut wait = Wait::new(Step::Drag, 0.0, 30.0);
    wait.on_event(&rolled(0.0));
    assert_eq!(wait.on_output(&Output::Run("x".into())), None);
    let done = wait.on_output(&Output::DragSample { finger: Finger::Middle, direction: [0.0, 1.0, 0.0] });
    assert_eq!(done, Some(Done::Taught(Taught::Drag { finger: "middle".into(), direction: [0.0, 1.0, 0.0] })));
    let Done::Failed(why) = Wait::new(Step::Drag, 0.0, 30.0).timed_out() else { panic!() };
    assert!(why.contains("30 s"), "{why}");
}

#[test]
fn a_draw_step_takes_the_strokes_or_fails_on_nothing() {
    let strokes = vec![vec![[0.0, 0.0], [0.0, 10.0]]];
    let mut wait = Wait::new(Step::Draw, 0.0, 30.0);
    assert_eq!(wait.on_output(&Output::DrawStarted), None);
    assert_eq!(
        wait.on_output(&Output::DrawFinished { strokes: strokes.clone() }),
        Some(Done::Taught(Taught::Strokes { strokes }))
    );
    let mut cancelled = Wait::new(Step::Draw, 0.0, 30.0);
    assert!(matches!(cancelled.on_output(&Output::DrawCancelled), Some(Done::Failed(_))));
    assert!(matches!(cancelled.on_output(&Output::DrawFinished { strokes: Vec::new() }), Some(Done::Failed(_))));
}

#[test]
fn errors_say_what_to_do() {
    assert!(taught_error_text(TaughtError::TooAlike(Drag::Left, Drag::Right)).contains("left and right"));
    assert!(taught_error_text(TaughtError::LikeTheTurn(Drag::Up)).contains("without turning"));
    assert!(calibration_error_text(CalibrationError::NotStill(Palm::Up)).contains("hold still"));
    assert!(calibration_error_text(CalibrationError::TooClose).contains("60°"));
    assert!(calibration_error_text(CalibrationError::Empty(Palm::Down)).contains("level"));
}
```

`tests/ipc.rs`, in `requests_use_kebab_case_commands` (import `air_gesturesd::ipc::{Taught, TaughtDrags}` and `air_gesturesd::store::StoredPalm`):

```rust
    assert_eq!(parse(r#"{"cmd":"palm-rolls","seconds":2.0}"#), Request::PalmRolls { seconds: 2.0 });
    assert_eq!(parse(r#"{"cmd":"teach-drag","seconds":30.0}"#), Request::TeachDrag { seconds: 30.0 });
    assert_eq!(parse(r#"{"cmd":"draw-try","seconds":30.0}"#), Request::DrawTry { seconds: 30.0 });
    assert_eq!(parse(r#"{"cmd":"set-palm","palm":null}"#), Request::SetPalm { palm: None });
    assert_eq!(parse(r#"{"cmd":"set-palm"}"#), Request::SetPalm { palm: None });
    assert_eq!(
        parse(r#"{"cmd":"set-palm","palm":{"down_roll":-140.0,"up_roll":-10.0}}"#),
        Request::SetPalm { palm: Some(StoredPalm { down_roll: -140.0, up_roll: -10.0 }) }
    );
    let drags = TaughtDrags { turn: [0.0, 1.0, 0.0], drags: [[1.0, 0.0, 0.0]; 6] };
    let line = serde_json::to_string(&Request::SetDrags { drags: Some(drags.clone()) }).unwrap();
    assert!(line.starts_with(r#"{"cmd":"set-drags""#), "{line}");
    assert_eq!(parse(&line), Request::SetDrags { drags: Some(drags) });
    assert_eq!(parse(r#"{"cmd":"palm"}"#), Request::Palm);
    assert!(Request::DrawTry { seconds: 1.0 }.waits() && !Request::SetPalm { palm: None }.waits() && !Request::Palm.waits());
    let answer: Response = serde_json::from_str(r#"{"ok":true,"teach":{"kind":"rolls","rolls":[1.5],"samples":2}}"#).unwrap();
    assert_eq!(answer.teach, Some(Taught::Rolls { rolls: vec![1.5], samples: 2 }));
```

`tests/daemon.rs` (import `air_gesturesd::ipc::{Taught, TaughtDrags}`, `air_gesturesd::store::{HandCalibration, StoredPalm}`, `band_core::events::Hand`, `band_core::controls::pinch::Prototypes`):

```rust
impl Harness {
    /// Send a call and hand back where its answer will arrive.
    async fn call_later(&self, request: Request) -> oneshot::Receiver<Response> {
        let (reply, response) = oneshot::channel();
        self.calls.send((request, reply)).await.unwrap();
        response
    }

    async fn right_hand(&mut self) {
        self.connected().await;
        self.wait("the right hand", |s| s.hand.as_deref() == Some("right")).await;
    }
}

#[tokio::test(start_paused = true)]
async fn teaching_a_drag_answers_with_the_pinchs_direction_and_runs_nothing() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.right_hand().await;
    daemon.configure("[gestures]\nindex_drag_away = \"echo away\"\n").await;
    let answer = daemon.call_later(Request::TeachDrag { seconds: 30.0 }).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    daemon.pinch_moving(600, 50).await;
    let response = answer.await.unwrap();
    let Some(Taught::Drag { finger, direction }) = response.teach else { panic!("{response:?}") };
    assert_eq!(finger, "index");
    // Band x, palm down (roll −142.3°), in the forearm frame.
    assert!(direction[0] < -0.75 && direction[1].abs() < 0.05 && direction[2] < -0.55, "{direction:?}");
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(daemon.runner.commands().is_empty(), "teaching acts on nothing");
    // Teaching is off again: the same pinch is the drag gesture.
    daemon.pinch_moving(600, 50).await;
    daemon.wait("the drag", |s| s.last_action.as_deref() == Some("echo away")).await;
}

#[tokio::test(start_paused = true)]
async fn a_drag_step_with_no_pinch_times_out() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    let response = daemon.call(Request::TeachDrag { seconds: 2.0 }).await;
    assert!(response.error.unwrap().contains("no pinch"));
}

#[tokio::test(start_paused = true)]
async fn a_drawing_try_answers_with_its_strokes_and_matches_nothing() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    daemon.teach_line("line");
    daemon.configure(DRAWING_CONFIG).await;
    let answer = daemon.call_later(Request::DrawTry { seconds: 30.0 }).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    daemon.pinch_moving(600, 40).await;
    let response = answer.await.unwrap();
    let Some(Taught::Strokes { strokes }) = response.teach else { panic!("{response:?}") };
    assert_eq!(strokes.len(), 1);
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(daemon.runner.commands().is_empty(), "no on_draw, no action while teaching");
}

#[tokio::test(start_paused = true)]
async fn palm_rolls_without_orientation_come_back_empty() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    let response = daemon.call(Request::PalmRolls { seconds: 2.0 }).await;
    assert_eq!(response.teach, Some(Taught::Rolls { rolls: Vec::new(), samples: 0 }));
}

#[tokio::test(start_paused = true)]
async fn palm_rolls_and_the_palm_now_come_from_the_bands_orientation() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.right_hand().await;
    let answer = daemon.call_later(Request::PalmRolls { seconds: 2.0 }).await;
    daemon.hold_roll(-140.0, 2.5).await;
    let Some(Taught::Rolls { rolls, samples }) = answer.await.unwrap().teach else { panic!() };
    assert!(samples >= 30 && rolls.len() == samples, "{samples} samples, {} rolls", rolls.len());
    assert!(rolls.iter().all(|r| (r + 140.0).abs() < 0.01), "{rolls:?}");
    daemon.hold_roll(PALM_UP, 0.3).await;
    assert_eq!(daemon.call(Request::Palm).await.teach, Some(Taught::Palm { palm: "up".into() }));
    tokio::time::sleep(Duration::from_secs(1)).await;
    let stale = daemon.call(Request::Palm).await.teach;
    assert_eq!(stale, Some(Taught::Palm { palm: "down".into() }), "no orientation for 0.5 s reads as down");
}

#[tokio::test(start_paused = true)]
async fn teaching_needs_a_band_and_one_step_at_a_time() {
    let lonely = SimTransport::new(SimMode::Enrolled);
    let alone = start(lonely.clone(), false, fake(&lonely));
    assert!(alone.call(Request::PalmRolls { seconds: 2.0 }).await.error.unwrap().contains("connect the band first"));
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    let _first = daemon.call_later(Request::TeachDrag { seconds: 30.0 }).await;
    let second = daemon.call(Request::DrawTry { seconds: 30.0 }).await;
    assert!(second.error.unwrap().contains("another teaching step"));
}

#[tokio::test(start_paused = true)]
async fn set_palm_and_set_drags_store_for_the_band_and_hand() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.right_hand().await;
    let palm = StoredPalm { down_roll: -140.0, up_roll: -10.0 };
    assert!(daemon.call(Request::SetPalm { palm: Some(palm) }).await.ok);
    let p = Prototypes::right_hand();
    let drags = TaughtDrags { turn: p.turn, drags: p.drags.unwrap() };
    assert!(daemon.call(Request::SetDrags { drags: Some(drags.clone()) }).await.ok);
    let stored = daemon.store.load_calibration(SIM_ADDRESS, Hand::Right).unwrap();
    assert_eq!(stored.palm, Some(palm));
    assert_eq!(stored.drags.unwrap().palm_down_roll, -140.0, "taught under the calibration's palm down");
    let mut alike = drags;
    alike.drags[1] = alike.drags[0];
    assert!(daemon.call(Request::SetDrags { drags: Some(alike) }).await.error.unwrap().contains("alike"));
    let close = StoredPalm { down_roll: -140.0, up_roll: -120.0 };
    assert!(daemon.call(Request::SetPalm { palm: Some(close) }).await.error.unwrap().contains("60°"));
    daemon.call(Request::SetPalm { palm: None }).await;
    daemon.call(Request::SetDrags { drags: None }).await;
    assert_eq!(daemon.store.load_calibration(SIM_ADDRESS, Hand::Right).unwrap(), HandCalibration::default());
}
```

- [ ] **Step 2: Run them to see them fail** — the new types and requests don't exist.

- [ ] **Step 3: `ipc.rs`**

`use crate::store::StoredPalm;`. Add to `Request`:

```rust
    /// Teaching: the band's roll for `seconds`, for the palm calibration (answers `teach: rolls`).
    PalmRolls { seconds: f64 },
    /// Teaching drags: the next pinch that moves, within `seconds` (answers `teach: drag`).
    TeachDrag { seconds: f64 },
    /// Teaching a drawing: one drawing, opened now, within `seconds` (answers `teach: strokes`).
    DrawTry { seconds: f64 },
    /// Store (`null`: forget) the palm calibration for the paired band and the hand in use.
    SetPalm {
        #[serde(default)]
        palm: Option<StoredPalm>,
    },
    /// Store (`null`: forget) the taught drags for the paired band and the hand in use.
    SetDrags {
        #[serde(default)]
        drags: Option<TaughtDrags>,
    },
    /// The palm position the band reads now (answers `teach: palm` at once).
    Palm,
```

and:

```rust
impl Request {
    /// A teaching step: answered when the band has done it (or at its deadline).
    pub fn waits(&self) -> bool {
        matches!(self, Request::PalmRolls { .. } | Request::TeachDrag { .. } | Request::DrawTry { .. })
    }
}

/// Taught drag samples: the turn, then the six drags in `Drag::ALL` order (forearm frame).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaughtDrags {
    pub turn: [f64; 3],
    pub drags: [[f64; 3]; 6],
}

/// What a teaching step brought back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Taught {
    /// The readable rolls (degrees) of `samples` orientation samples.
    Rolls { rolls: Vec<f64>, samples: usize },
    /// One pinch's direction (forearm frame, unit); `finger` is "index" or "middle".
    Drag { finger: String, direction: [f64; 3] },
    /// One drawing's strokes of [x, y] points (degrees).
    Strokes { strokes: Vec<Vec<[f64; 2]>> },
    /// The palm position now: "down", "sideways" or "up" (`Palm::key`).
    Palm { palm: String },
}
```

`Response` gains `#[serde(default, skip_serializing_if = "Option::is_none")] pub teach: Option<Taught>,` and

```rust
    pub fn taught(status: Status, taught: Taught) -> Self {
        Self { ok: true, status: Some(status), teach: Some(taught), ..Self::default() }
    }
```

- [ ] **Step 4: `teach.rs`: the waiting step**

```rust
use band_core::controls::pinch::{roll_or, up_in_band};
use band_core::controls::{Finger, Output};
use band_core::events::Event;

use crate::ipc::Taught;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Rolls,
    Drag,
    Draw,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Done {
    Taught(Taught),
    Failed(String),
}

/// A teaching step waiting for the band.
#[derive(Debug)]
pub struct Wait {
    step: Step,
    until: f64,
    seconds: f64,
    rolls: Vec<f64>,
    samples: usize,
}

impl Wait {
    pub fn new(step: Step, now: f64, seconds: f64) -> Self {
        Self { step, until: now + seconds, seconds, rolls: Vec::new(), samples: 0 }
    }

    pub fn step(&self) -> Step {
        self.step
    }

    pub fn deadline(&self) -> f64 {
        self.until
    }

    /// A band event (rolls: each orientation's roll about the forearm, as the controls read it).
    pub fn on_event(&mut self, event: &Event) {
        if let (Step::Rolls, Event::Orientation(q)) = (self.step, event) {
            self.samples += 1;
            let roll = roll_or(up_in_band(*q), f64::NAN);
            if roll.is_finite() {
                self.rolls.push(roll);
            }
        }
    }

    /// An output of the controls: done when it's this step's.
    pub fn on_output(&mut self, output: &Output<String>) -> Option<Done> {
        match (self.step, output) {
            (Step::Drag, Output::DragSample { finger, direction }) => {
                let finger = match finger {
                    Finger::Index => "index",
                    Finger::Middle => "middle",
                };
                Some(Done::Taught(Taught::Drag { finger: finger.into(), direction: *direction }))
            }
            (Step::Draw, Output::DrawFinished { strokes }) if !strokes.is_empty() => {
                Some(Done::Taught(Taught::Strokes { strokes: strokes.clone() }))
            }
            (Step::Draw, Output::DrawFinished { .. } | Output::DrawCancelled) => {
                Some(Done::Failed("nothing was drawn (pinch to put the pen down; a quick tap isn't a stroke)".into()))
            }
            _ => None,
        }
    }

    /// The deadline passed: the rolls so far, else what didn't happen.
    pub fn timed_out(&mut self) -> Done {
        match self.step {
            Step::Rolls => Done::Taught(Taught::Rolls { rolls: std::mem::take(&mut self.rolls), samples: self.samples }),
            Step::Drag => Done::Failed(format!("no pinch that moved in {} s", self.seconds)),
            Step::Draw => Done::Failed(format!("the drawing didn't end within {} s", self.seconds)),
        }
    }
}
```

- [ ] **Step 5: The daemon**

Imports: `use tokio::sync::oneshot;`, `use band_core::controls::palm::RIGHT_PALM_DOWN_ROLL;`, `use crate::ipc::Taught;`, `use crate::store::{HandCalibration, StoredDrags};`, `use crate::teach::{Done, Step, Wait};` (`PalmCalibration` and `Prototypes` came in Task 7).

Field: `/// The teaching step waiting for the band, and where its answer goes. waiting: Option<(Wait, oneshot::Sender<Response>)>,` (initialised `None`).

`run`'s select:

```rust
                call = calls.recv() => match call {
                    Some((request, reply)) if request.waits() => self.start_wait(request, reply),
                    Some((request, reply)) => {
                        let response = self.handle(request).await;
                        let _ = reply.send(response);
                    }
                    None => break,
                },
```

and a new arm after the controls deadline:

```rust
                _ = until(self.waiting.as_ref().map(|(wait, _)| wait.deadline()), self.clock) => {
                    if let Some(done) = self.waiting.as_mut().map(|(wait, _)| wait.timed_out()) {
                        self.finish_wait(done);
                    }
                }
```

Methods:

```rust
    fn start_wait(&mut self, request: Request, reply: oneshot::Sender<Response>) {
        if self.state != LinkState::Connected {
            let _ = reply.send(Response::error("connect the band first (`air-gestures connect`)"));
            return;
        }
        if self.waiting.is_some() {
            let _ = reply.send(Response::error("another teaching step is running; wait for it to finish"));
            return;
        }
        let now = self.clock.now();
        let wait = match request {
            Request::PalmRolls { seconds } => Wait::new(Step::Rolls, now, seconds.clamp(0.5, 5.0)),
            Request::TeachDrag { seconds } => {
                let outputs = self.controls.set_drag_teaching(true);
                self.apply(outputs, false);
                Wait::new(Step::Drag, now, seconds.clamp(1.0, 60.0))
            }
            Request::DrawTry { seconds } => {
                let mut outputs = self.controls.cancel_drawing();
                self.controls.set_teaching(true);
                outputs.extend(self.controls.start_drawing(now));
                self.apply(outputs, false);
                Wait::new(Step::Draw, now, seconds.clamp(1.0, 120.0))
            }
            _ => unreachable!("only teaching steps wait"),
        };
        self.waiting = Some((wait, reply));
    }

    /// Teaching off again, and the answer.
    fn finish_wait(&mut self, done: Done) {
        let Some((wait, reply)) = self.waiting.take() else { return };
        let outputs = match wait.step() {
            Step::Rolls => Vec::new(),
            Step::Drag => self.controls.set_drag_teaching(false),
            Step::Draw => {
                self.controls.set_teaching(false);
                self.controls.cancel_drawing()
            }
        };
        self.apply(outputs, false);
        self.publish();
        let _ = reply.send(match done {
            Done::Taught(taught) => Response::taught(self.status.borrow().clone(), taught),
            Done::Failed(why) => Response::error(why),
        });
    }

    fn fail_wait(&mut self, why: &str) {
        if self.waiting.is_some() {
            self.finish_wait(Done::Failed(why.into()));
        }
    }

    /// The palm-down roll drags are taught under: the calibration's, else the right hand's
    /// measured one (an uncalibrated left hand: 0, like `Prototypes::turns_only`).
    fn palm_down_roll(&self) -> f64 {
        match (self.controls.options().palm, self.hand_in_use()) {
            (Some(palm), _) => palm.down_roll,
            (None, Some(Hand::Left)) => 0.0,
            (None, _) => RIGHT_PALM_DOWN_ROLL,
        }
    }

    /// Change what's taught for the paired band and the hand in use, and apply it.
    fn save_taught(&mut self, change: impl FnOnce(&mut HandCalibration)) -> Response {
        let (Some(hand), Ok(Some(band))) = (self.hand_in_use(), self.store.load_band()) else {
            return Response::error("connect the band first: it says which wrist it's on when it connects");
        };
        let mut calibration = self.store.load_calibration(&band.address, hand).unwrap_or_else(|error| {
            tracing::warn!("calibration: {error}; starting afresh");
            HandCalibration::default()
        });
        change(&mut calibration);
        if let Err(error) = self.store.save_calibration(&band.address, hand, &calibration) {
            return Response::error(format!("state: {error}"));
        }
        self.refresh_calibration(true);
        self.publish();
        Response::ok(self.status.borrow().clone())
    }
```

Wire-up:
- `apply`: first thing in the loop, `if let Some(done) = self.waiting.as_mut().and_then(|(wait, _)| wait.on_output(&output)) { self.finish_wait(done); }`.
- `on_link_event`: before `self.controls.on_event`, `if let Some((wait, _)) = &mut self.waiting { wait.on_event(&event); }`.
- `on_link_closed` (after `self.controls.on_disconnect();`) and `stop_link` (same place): `self.fail_wait("the band disconnected");`.
- `handle`:

```rust
            Request::PalmRolls { .. } | Request::TeachDrag { .. } | Request::DrawTry { .. } => {
                return Response::error("teaching steps are answered by the daemon loop");
            }
            Request::SetPalm { palm } => {
                if let Some(p) = palm
                    && let Err(error) = PalmCalibration::from_rolls(&[p.down_roll], &[p.up_roll])
                {
                    return Response::error(crate::teach::calibration_error_text(error));
                }
                return self.save_taught(|calibration| calibration.palm = palm);
            }
            Request::SetDrags { drags } => {
                let stored = match drags {
                    None => None,
                    Some(taught) => match Prototypes::taught(taught.turn, taught.drags, self.palm_down_roll()) {
                        Ok(prototypes) => StoredDrags::from_prototypes(&prototypes),
                        Err(error) => return Response::error(crate::teach::taught_error_text(error)),
                    },
                };
                return self.save_taught(|calibration| calibration.drags = stored);
            }
            Request::Palm => {
                if self.state != LinkState::Connected {
                    return Response::error("connect the band first (`air-gestures connect`)");
                }
                let palm = self.controls.palm(now).key().to_owned();
                self.publish();
                return Response::taught(self.status.borrow().clone(), Taught::Palm { palm });
            }
```

  (`set_palm_and_set_drags_store…` sends the palm first, so the drags are taught under −140.)

- [ ] **Step 6: Run the tests** — as Task 1 Step 6.

- [ ] **Step 7: Commit** `air-gesturesd/src/teach.rs air-gesturesd/src/ipc.rs air-gesturesd/src/daemon.rs air-gesturesd/tests/teach.rs air-gesturesd/tests/ipc.rs air-gesturesd/tests/daemon.rs` — message `daemon: teaching steps over IPC (palm rolls, drag samples, drawing tries) and storing them`.

---

### Task 9: `air-gestures calibrate`

**Files:**
- Modify: `air-gestures/src/client.rs`, `air-gestures/src/lib.rs` (`pub mod teach;`), `air-gestures/src/main.rs`, `air-gestures/tests/support/mod.rs`
- Create: `air-gestures/src/teach.rs`, `air-gestures/tests/teach.rs`

Decisions: each position starts on Enter (the user's other hand is on the keyboard), then a notification and 2 s of holding still; up to three rounds, redoing only what failed. After saving, `calibrate` asks for the palm up once more and reads `palm` (band-core's `Controls::palm`): "palm up now" confirms it; anything else is said, but the calibration stays saved (the user decides whether to run it again).

- [ ] **Step 1: Write the failing tests**

`air-gestures/tests/teach.rs`:

```rust
mod support;

use std::time::Duration;

use air_gestures::teach::{self, Guide};
use air_gesturesd::status::LinkState;
use std::sync::{Arc, Mutex};

use air_gesturesd::transport::SIM_ADDRESS;
use band_core::events::Hand;
use band_core::responder::SimMode;
use tokio::sync::mpsc;

/// Records what the flow says; each notification goes to the test (the cue to act). With
/// `roll`, the "Palm down" / "Palm up" notifications also turn the simulated wrist (before
/// the flow asks the daemon for rolls, so no sample of the other position gets in).
struct Recorded {
    said: Vec<String>,
    notified: mpsc::UnboundedSender<String>,
    roll: Option<Arc<Mutex<f64>>>,
}

impl Guide for Recorded {
    fn say(&mut self, text: &str) {
        self.said.push(text.to_owned());
    }
    fn notify(&mut self, title: &str, body: &str) {
        if let Some(roll) = &self.roll {
            match title {
                "Palm down" => *roll.lock().unwrap() = -140.0,
                "Palm up" => *roll.lock().unwrap() = -10.0,
                _ => {}
            }
        }
        let _ = self.notified.send(format!("{title}: {body}"));
    }
    fn ready(&mut self, _prompt: &str) -> bool {
        true
    }
}

fn guide() -> (Recorded, mpsc::UnboundedReceiver<String>) {
    let (notified, cues) = mpsc::unbounded_channel();
    (Recorded { said: Vec::new(), notified, roll: None }, cues)
}

#[tokio::test]
async fn calibrating_saves_palm_down_and_up_and_confirms_palm_up() {
    let daemon = support::start(SimMode::Enrolled, true).await;
    daemon.wait("the hand", |s| s.state == LinkState::Connected && s.hand.as_deref() == Some("right")).await;
    let roll = Arc::new(Mutex::new(-140.0));
    let feeder = tokio::spawn(support::feed_roll(daemon.transport.clone(), roll.clone()));
    let (mut guide, _cues) = guide();
    guide.roll = Some(roll);
    let socket = daemon.socket.clone();
    let flow = tokio::spawn(async move { teach::calibrate(&socket, &mut guide).await.map(|c| (c, guide.said)) });
    let (calibration, said) = flow.await.unwrap().unwrap();
    feeder.abort();
    assert!((calibration.down_roll + 140.0).abs() < 0.5 && (calibration.up_roll + 10.0).abs() < 0.5, "{calibration:?}");
    assert!(said.iter().any(|s| s.contains("palm up now")), "{said:?}");
    let stored = daemon.store.load_calibration(SIM_ADDRESS, Hand::Right).unwrap();
    assert_eq!(stored.palm.map(|p| p.calibration()), Some(calibration));
}

#[tokio::test]
async fn calibrating_without_orientation_says_so() {
    let daemon = support::start(SimMode::Enrolled, true).await;
    daemon.wait("the hand", |s| s.state == LinkState::Connected && s.hand.is_some()).await;
    let (mut guide, _cues) = guide();
    let error = teach::calibrate(&daemon.socket, &mut guide).await.unwrap_err();
    assert!(error.contains("no orientation"), "{error}");
}

#[tokio::test]
async fn calibrating_needs_a_connected_band() {
    let daemon = support::start(SimMode::Enrolled, false).await;
    let (mut guide, _cues) = guide();
    let error = teach::calibrate(&daemon.socket, &mut guide).await.unwrap_err();
    assert!(error.contains("connect the band first"), "{error}");
}
```

`air-gestures/tests/support/mod.rs` gains:

```rust
/// An orientation (w, x, y, z) whose roll about the forearm is `degrees`.
pub fn rolled(degrees: f64) -> [f32; 4] {
    let half = (-degrees).to_radians() / 2.0;
    [half.cos() as f32, 0.0, half.sin() as f32, 0.0]
}

/// The band's orientation every 50 ms, rolled as `roll` says now (until aborted).
pub async fn feed_roll(transport: SimTransport, roll: std::sync::Arc<std::sync::Mutex<f64>>) {
    use air_gesturesd::transport::SimInject;
    loop {
        let degrees = *roll.lock().unwrap();
        transport.inject(SimInject::Orientation(rolled(degrees)));
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
```

- [ ] **Step 2: Run them to see them fail** — no `teach` module.

- [ ] **Step 3: `client.rs`**

Rename the body of `request` into `request_within(path, request, timeout: Duration)` (using `timeout` where `CALL_TIMEOUT` was); `request` becomes `request_within(path, request, CALL_TIMEOUT).await`.

- [ ] **Step 4: `air-gestures/src/teach.rs`**

```rust
//! The guided teaching flows: palm calibration, teach your drags, teach a drawing. They talk
//! to air-gesturesd's teaching steps and check the results with band-core before storing them.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use air_gesturesd::ipc::{Request, Taught};
use air_gesturesd::status::{LinkState, Status};
use air_gesturesd::store::StoredPalm;
use air_gesturesd::teach::calibration_error_text;
use band_core::controls::palm::{CalibrationError, Palm, PalmCalibration};

use crate::client;

/// What a flow says and asks (the terminal, or a test).
pub trait Guide {
    fn say(&mut self, text: &str);
    /// A desktop notification (the user is watching their hand, not the terminal).
    fn notify(&mut self, title: &str, body: &str);
    /// Show `prompt`, wait for Enter; `false` on end of input.
    fn ready(&mut self, prompt: &str) -> bool;
}

pub struct Terminal;

impl Guide for Terminal {
    fn say(&mut self, text: &str) {
        println!("{text}");
    }
    fn notify(&mut self, title: &str, body: &str) {
        // Best effort: without a notification daemon the terminal still says it.
        let _ = std::process::Command::new("notify-send")
            .args(["-a", "air-gestures", title, body])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    fn ready(&mut self, prompt: &str) -> bool {
        use std::io::Write;
        print!("{prompt} ");
        let _ = std::io::stdout().flush();
        std::io::stdin().read_line(&mut String::new()).is_ok_and(|read| read > 0)
    }
}

const ROUNDS: usize = 3;
const HOLD: f64 = 2.0;

fn unexpected() -> String {
    "unexpected answer from air-gesturesd".into()
}

/// The band's hand, once it's connected and has said.
pub fn connected_hand(status: &Status) -> Result<&str, String> {
    if status.state != LinkState::Connected {
        return Err("connect the band first (`air-gestures connect`)".into());
    }
    status.hand.as_deref().ok_or_else(|| "the band hasn't said which wrist it's on yet; try again in a moment".into())
}

async fn hold(socket: &Path, guide: &mut impl Guide, palm: Palm) -> Result<Vec<f64>, String> {
    let (title, way) = match palm {
        Palm::Up => ("Palm up", "palm up"),
        _ => ("Palm down", "palm down"),
    };
    if !guide.ready(&format!("Forearm level in front of you, {way}. Press Enter, then hold still for 2 s.")) {
        return Err("cancelled; nothing was changed".into());
    }
    guide.notify(title, "Hold still for 2 seconds");
    let response = client::request_within(socket, &Request::PalmRolls { seconds: HOLD }, Duration::from_secs(10)).await?;
    match response.teach {
        Some(Taught::Rolls { samples: 0, .. }) => {
            Err("the band sent no orientation: is it on your wrist and connected? (`air-gestures status`)".into())
        }
        Some(Taught::Rolls { rolls, .. }) => Ok(rolls),
        _ => Err(unexpected()),
    }
}

/// The palm position the band reads now.
async fn palm_now(socket: &Path) -> Result<String, String> {
    match client::request(socket, &Request::Palm).await?.teach {
        Some(Taught::Palm { palm }) => Ok(palm),
        _ => Err(unexpected()),
    }
}

/// Palm up once more: does the band read it that way now? Only says so; the calibration
/// is saved either way.
async fn confirm(socket: &Path, guide: &mut impl Guide) -> Result<(), String> {
    if !guide.ready("Check: turn your palm up again and press Enter.") {
        return Ok(());
    }
    match palm_now(socket).await?.as_str() {
        "up" => guide.say("The band reads palm up now. Done."),
        other => guide.say(&format!(
            "The band reads palm {other} with your palm up: if that's wrong, run `air-gestures calibrate` again."
        )),
    }
    Ok(())
}

/// Palm down, then palm up, held still: the calibration for the hand in use, stored and
/// applied, then checked.
pub async fn calibrate(socket: &Path, guide: &mut impl Guide) -> Result<PalmCalibration, String> {
    let status = client::status(socket).await?;
    let hand = connected_hand(&status)?.to_owned();
    guide.say(&format!("Palm positions for your {hand} hand: palm down, then palm up."));
    let (mut down, mut up) = (None, None);
    let mut last = String::new();
    for _ in 0..ROUNDS {
        if down.is_none() {
            down = Some(hold(socket, guide, Palm::Down).await?);
        }
        if up.is_none() {
            up = Some(hold(socket, guide, Palm::Up).await?);
        }
        match PalmCalibration::from_rolls(down.as_deref().unwrap_or_default(), up.as_deref().unwrap_or_default()) {
            Ok(calibration) => {
                client::request(socket, &Request::SetPalm { palm: Some(StoredPalm::from(calibration)) }).await?;
                let text = format!(
                    "Saved: palm down at {:.0}°, palm up at {:.0}° (sideways is halfway).",
                    calibration.down_roll, calibration.up_roll
                );
                guide.say(&text);
                guide.notify("Calibrated", &text);
                confirm(socket, guide).await?;
                return Ok(calibration);
            }
            Err(error) => {
                last = calibration_error_text(error);
                guide.say(&format!("{last}."));
                match error {
                    CalibrationError::NotStill(Palm::Down) | CalibrationError::Empty(Palm::Down) => down = None,
                    CalibrationError::NotStill(_) | CalibrationError::Empty(_) => up = None,
                    CalibrationError::TooClose => (down, up) = (None, None),
                }
            }
        }
    }
    Err(format!("calibration didn't work in {ROUNDS} rounds ({last}); nothing was changed"))
}
```

- [ ] **Step 5: `main.rs`**

```rust
    /// Calibrate palm positions (down, up) for the band and the hand it's on.
    Calibrate {
        /// Forget the calibration (back to the measured right-hand one; none for the left hand).
        #[arg(long)]
        reset: bool,
    },
```

```rust
        Command::Calibrate { reset: true } => simple(&socket, Request::SetPalm { palm: None }),
        Command::Calibrate { reset: false } => report(
            runtime().block_on(air_gestures::teach::calibrate(&socket, &mut air_gestures::teach::Terminal)).map(|_| ()),
        ),
```

and a test module at the end of `main.rs` (extended by Tasks 10 and 11):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Command {
        Cli::try_parse_from(std::iter::once("air-gestures").chain(args.iter().copied())).unwrap().command
    }

    #[test]
    fn the_teaching_commands_parse() {
        assert!(matches!(parse(&["calibrate"]), Command::Calibrate { reset: false }));
        assert!(matches!(parse(&["calibrate", "--reset"]), Command::Calibrate { reset: true }));
        assert!(matches!(parse(&["--dry-run", "calibrate"]), Command::Calibrate { .. }));
    }
}
```

- [ ] **Step 6: Run the tests** — as Task 1 Step 6 (also runs `cargo test -p air-gestures --bin air-gestures`).

- [ ] **Step 7: Commit** `air-gestures/src/client.rs air-gestures/src/lib.rs air-gestures/src/teach.rs air-gestures/src/main.rs air-gestures/tests/teach.rs air-gestures/tests/support/mod.rs` — message `air-gestures calibrate: palm down and up, held still, then checked`.

---

### Task 10: `air-gestures teach-drags`

**Files:**
- Modify: `air-gestures/src/teach.rs`, `air-gestures/src/main.rs`, `air-gestures/tests/teach.rs`

- [ ] **Step 1: Write the failing tests** (append to `air-gestures/tests/teach.rs`)

```rust
use band_core::controls::Drag;
use band_core::controls::pinch::TaughtError;

#[test]
fn a_refused_set_of_drags_names_the_moves_to_teach_again() {
    assert_eq!(teach::redo(TaughtError::NoTurn), [0]);
    assert_eq!(teach::redo(TaughtError::NoDrag(Drag::Toward)), [3]);
    assert_eq!(teach::redo(TaughtError::LikeTheTurn(Drag::Up)), [5]);
    assert_eq!(teach::redo(TaughtError::TooAlike(Drag::Left, Drag::Right)), [1, 2]);
    assert_eq!(teach::MOVES[0].0, "turn");
    assert_eq!(teach::MOVES.map(|m| m.0)[1..], Drag::ALL.map(air_gesturesd::teach::drag_name));
}

#[tokio::test]
async fn one_drag_sample_comes_back_from_the_band() {
    let daemon = support::start(SimMode::Enrolled, true).await;
    daemon.wait("the hand", |s| s.state == LinkState::Connected && s.hand.is_some()).await;
    let (mut guide, mut cues) = guide();
    let socket = daemon.socket.clone();
    let sample = tokio::spawn(async move { teach::drag_sample(&socket, &mut guide, 1).await });
    cues.recv().await.expect("the cue");
    tokio::time::sleep(Duration::from_millis(300)).await;
    support::pinch_moving(&daemon.transport, 600, 50).await;
    let direction = sample.await.unwrap().unwrap();
    assert!(direction[0] < -0.75, "{direction:?}");
}
```

and in `support/mod.rs`:

```rust
/// An index pinch turning the wrist about the band's x axis (see the daemon tests).
pub async fn pinch_moving(transport: &SimTransport, raw: i16, samples: usize) {
    use air_gesturesd::transport::SimInject;
    transport.inject(SimInject::Gesture { action: 1, finger: 2, derived: 0 });
    for _ in 0..samples {
        transport.inject(SimInject::Gyro(raw));
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    transport.inject(SimInject::Gesture { action: 2, finger: 2, derived: 0 });
}
```

In `main.rs`'s tests: `assert!(matches!(parse(&["teach-drags"]), Command::TeachDrags { reset: false }));` and `--reset`.

- [ ] **Step 2: Run them to see them fail.**

- [ ] **Step 3: Implement** (in `air-gestures/src/teach.rs`; import `air_gesturesd::ipc::TaughtDrags`, `air_gesturesd::teach::taught_error_text`, `band_core::controls::pinch::{Prototypes, TaughtError}`)

```rust
/// The moves taught, in order: the turn, then `Drag::ALL`'s.
pub const MOVES: [(&str, &str); 7] = [
    ("turn", "turn your wrist the way that should turn the volume up"),
    ("left", "move your hand left"),
    ("right", "move your hand right"),
    ("toward", "pull your hand toward you"),
    ("away", "push your hand away from you"),
    ("up", "lift your hand"),
    ("down", "lower your hand"),
];

/// Which moves (indexes into `MOVES`) to teach again.
pub fn redo(error: TaughtError) -> Vec<usize> {
    match error {
        TaughtError::NoTurn => vec![0],
        TaughtError::NoDrag(drag) | TaughtError::LikeTheTurn(drag) => vec![1 + drag.index()],
        TaughtError::TooAlike(a, b) => vec![1 + a.index(), 1 + b.index()],
    }
}

/// One move's direction, from the next pinch that moves.
pub async fn drag_sample(socket: &Path, guide: &mut impl Guide, index: usize) -> Result<[f64; 3], String> {
    let (name, how) = MOVES[index];
    guide.say(&format!("{}/{} {name}: pinch (thumb + index), {how}, let go.", index + 1, MOVES.len()));
    guide.notify("Teach your drags", &format!("Pinch, {how}, let go"));
    let response = client::request_within(socket, &Request::TeachDrag { seconds: 30.0 }, Duration::from_secs(40)).await?;
    match response.teach {
        Some(Taught::Drag { direction, .. }) => Ok(direction),
        _ => Err(unexpected()),
    }
}

/// The turn and six drags, one pinch each: the hand's prototypes, stored and applied.
pub async fn teach_drags(socket: &Path, guide: &mut impl Guide) -> Result<(), String> {
    let status = client::status(socket).await?;
    let hand = connected_hand(&status)?.to_owned();
    guide.say(&format!("Teach your drags ({hand} hand): one pinch for each move, the way you'll make it."));
    let mut samples: [Option<[f64; 3]>; 7] = [None; 7];
    let mut last = String::new();
    for _ in 0..ROUNDS {
        for (index, sample) in samples.iter_mut().enumerate() {
            if sample.is_none() {
                *sample = Some(drag_sample(socket, guide, index).await?);
            }
        }
        let all = samples.map(|s| s.unwrap_or_default());
        let drags: [[f64; 3]; 6] = std::array::from_fn(|i| all[i + 1]);
        // The palm-down roll doesn't change the check; the daemon sets the real one.
        match Prototypes::taught(all[0], drags, 0.0) {
            Ok(_) => {
                client::request(socket, &Request::SetDrags { drags: Some(TaughtDrags { turn: all[0], drags }) }).await?;
                guide.say("Saved: drags now read the way you make them.");
                guide.notify("Drags taught", "Saved");
                return Ok(());
            }
            Err(error) => {
                last = taught_error_text(error);
                guide.say(&format!("{last}: once more."));
                for index in redo(error) {
                    samples[index] = None;
                }
            }
        }
    }
    Err(format!("teaching didn't work in {ROUNDS} rounds ({last}); nothing was changed"))
}
```

`main.rs`:

```rust
    /// Teach the turn and the six drags (one pinch each) for the band and the hand it's on.
    TeachDrags {
        /// Forget them (back to the measured right-hand ones; turns only for the left hand).
        #[arg(long)]
        reset: bool,
    },
```

```rust
        Command::TeachDrags { reset: true } => simple(&socket, Request::SetDrags { drags: None }),
        Command::TeachDrags { reset: false } => {
            report(runtime().block_on(air_gestures::teach::teach_drags(&socket, &mut air_gestures::teach::Terminal)))
        }
```

- [ ] **Step 4: Run the tests** — as Task 1 Step 6.

- [ ] **Step 5: Commit** `air-gestures/src/teach.rs air-gestures/src/main.rs air-gestures/tests/teach.rs air-gestures/tests/support/mod.rs` — message `air-gestures teach-drags: the turn and six drags, one pinch each`.

---

### Task 11: `air-gestures draw teach | list | delete`

**Files:**
- Modify: `air-gestures/src/teach.rs`, `air-gestures/src/main.rs`, `air-gestures/tests/teach.rs`

Decisions: teaching a name that exists is refused (delete it first; no silent overwrite); a "looks like" match aborts without saving; the daemon picks the new file up on its next tick (no IPC reload request).

- [ ] **Step 1: Write the failing tests** (append)

```rust
use air_gesturesd::drawings;
use band_core::controls::draw::Drawing;

#[tokio::test]
async fn teaching_a_drawing_saves_three_alike_tries() {
    let daemon = support::start(SimMode::Enrolled, true).await;
    daemon.wait("the hand", |s| s.state == LinkState::Connected && s.hand.is_some()).await;
    let (mut guide, mut cues) = guide();
    let (socket, dir) = (daemon.socket.clone(), daemon.drawings());
    let teach = tokio::spawn(async move { teach::teach_drawing(&socket, &dir, "line", &mut guide).await.map(|()| guide.said) });
    for _ in 0..3 {
        cues.recv().await.expect("a try");
        tokio::time::sleep(Duration::from_millis(300)).await;
        support::pinch_moving(&daemon.transport, 600, 40).await;
    }
    let said = teach.await.unwrap().unwrap();
    assert!(said.iter().any(|s| s.contains("[drawings]")), "{said:?}");
    let (saved, problems) = drawings::load(&daemon.drawings());
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].name, "line");
    assert_eq!(saved[0].tries.len(), 3);
}

#[tokio::test]
async fn a_taken_or_bad_name_is_refused_before_drawing() {
    let dir = tempfile::tempdir().unwrap();
    let line = vec![vec![[0.0, 0.0], [0.0, 10.0]]];
    drawings::save(dir.path(), &Drawing { name: "z".into(), tries: vec![line] }).unwrap();
    let nowhere = dir.path().join("no.sock");
    let (mut guide, _cues) = guide();
    let taken = teach::teach_drawing(&nowhere, dir.path(), "z", &mut guide).await.unwrap_err();
    assert!(taken.contains("already taught") && taken.contains("air-gestures draw delete z"), "{taken}");
    let bad = teach::teach_drawing(&nowhere, dir.path(), "it's", &mut guide).await.unwrap_err();
    assert!(bad.contains("it's"), "{bad}");
}

#[test]
fn drawings_are_listed_and_deleted() {
    let dir = tempfile::tempdir().unwrap();
    assert!(teach::list_drawings(dir.path()).unwrap().contains("no drawings yet"));
    let line = vec![vec![[0.0, 0.0], [0.0, 10.0]]];
    drawings::save(dir.path(), &Drawing { name: "z".into(), tries: vec![line.clone(), line] }).unwrap();
    assert_eq!(teach::list_drawings(dir.path()).unwrap(), "z (2 tries)");
    teach::delete_drawing(dir.path(), "z").unwrap();
    assert!(teach::delete_drawing(dir.path(), "z").unwrap_err().contains("no drawing \"z\""));
}
```

In `main.rs`'s tests:

```rust
        assert!(matches!(parse(&["draw", "teach", "lightbulb"]), Command::Draw { what: DrawCommand::Teach { name } } if name == "lightbulb"));
        assert!(matches!(parse(&["draw", "list"]), Command::Draw { what: DrawCommand::List }));
        assert!(matches!(parse(&["draw", "delete", "z"]), Command::Draw { what: DrawCommand::Delete { name } } if name == "z"));
        assert!(Cli::try_parse_from(["air-gestures", "draw", "teach"]).is_err(), "a name is needed");
```

- [ ] **Step 2: Run them to see them fail.**

- [ ] **Step 3: Implement** (in `air-gestures/src/teach.rs`; import `air_gesturesd::drawings`, `band_core::controls::draw::{self, Drawing, Library, Stroke, TeachError}`)

```rust
/// Tries a drawing is taught with.
pub const TRIES: usize = 3;

/// `name` as a TOML key.
fn toml_key(name: &str) -> String {
    if name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') { name.to_owned() } else { format!("\"{name}\"") }
}

async fn draw_try(socket: &Path, guide: &mut impl Guide, name: &str, index: usize) -> Result<Vec<Stroke>, String> {
    guide.say(&format!("Try {} of {TRIES}: draw it now.", index + 1));
    guide.notify("Draw", &format!("{name}: try {} of {TRIES}", index + 1));
    let response = client::request_within(socket, &Request::DrawTry { seconds: 30.0 }, Duration::from_secs(40)).await?;
    match response.teach {
        Some(Taught::Strokes { strokes }) => Ok(strokes),
        _ => Err(unexpected()),
    }
}

/// Three alike tries of a new drawing, saved in `dir` (where the daemon reads drawings).
pub async fn teach_drawing(socket: &Path, dir: &Path, name: &str, guide: &mut impl Guide) -> Result<(), String> {
    drawings::check_name(name)?;
    let (existing, _) = drawings::load(dir);
    if existing.iter().any(|d| d.name == name) {
        return Err(format!("\"{name}\" is already taught: `air-gestures draw delete {name}` first"));
    }
    connected_hand(&client::status(socket).await?)?;
    guide.say(&format!(
        "Draw \"{name}\" {TRIES} times, the way you'll draw it later (in the air, or pen grip on a table). \
         Pinch to put the pen down, let go to lift it; each try ends 1 s after your last stroke."
    ));
    let mut tries: Vec<Option<Vec<Stroke>>> = vec![None; TRIES];
    for _ in 0..ROUNDS {
        for (index, slot) in tries.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(draw_try(socket, guide, name, index).await?);
            }
        }
        let all: Vec<Vec<Stroke>> = tries.iter().map(|t| t.clone().unwrap_or_default()).collect();
        match draw::check_tries(&all) {
            Ok(()) => {
                if let Some(other) = Library::new(&existing).looks_like(&all) {
                    return Err(format!("that looks like \"{other}\": teach something more different (nothing was saved)"));
                }
                drawings::save(dir, &Drawing { name: name.to_owned(), tries: all })
                    .map_err(|error| format!("could not save \"{name}\": {error}"))?;
                guide.say(&format!("Saved \"{name}\". Give it an action in config.toml:\n  [drawings]\n  {} = \"…\"", toml_key(name)));
                guide.notify("Drawing taught", name);
                return Ok(());
            }
            Err(TeachError::TooSmall(index)) => {
                guide.say(&format!("Try {} was too small to read: draw bigger.", index + 1));
                tries[index] = None;
            }
            Err(TeachError::NotAlike { .. } | TeachError::NoTries) => {
                guide.say("These don't look alike: try again, or draw bigger.");
                tries.iter_mut().for_each(|t| *t = None);
            }
        }
    }
    Err(format!("\"{name}\" didn't come out alike in {ROUNDS} rounds; nothing was saved"))
}

/// One line per drawing, or how to make one.
pub fn list_drawings(dir: &Path) -> Result<String, String> {
    let (all, problems) = drawings::load(dir);
    for problem in problems {
        eprintln!("{problem}");
    }
    if all.is_empty() {
        return Ok("no drawings yet: `air-gestures draw teach <name>`".into());
    }
    Ok(all.iter().map(|d| format!("{} ({} tries)", d.name, d.tries.len())).collect::<Vec<_>>().join("\n"))
}

pub fn delete_drawing(dir: &Path, name: &str) -> Result<(), String> {
    match drawings::delete(dir, name) {
        Ok(true) => Ok(()),
        Ok(false) => Err(format!("no drawing \"{name}\" (`air-gestures draw list`)")),
        Err(error) => Err(format!("could not delete \"{name}\": {error}")),
    }
}
```

`main.rs`:

```rust
    /// Drawings: teach one (three tries), list them, delete one.
    Draw {
        #[command(subcommand)]
        what: DrawCommand,
    },
```

```rust
#[derive(Subcommand)]
enum DrawCommand {
    /// Teach a drawing by drawing it three times.
    Teach { name: String },
    /// The taught drawings.
    List,
    /// Forget a drawing.
    Delete { name: String },
}
```

```rust
        Command::Draw { what } => {
            let dir = air_gesturesd::drawings::default_dir(cli.dry_run);
            match what {
                DrawCommand::Teach { name } => report(runtime().block_on(air_gestures::teach::teach_drawing(
                    &socket,
                    &dir,
                    &name,
                    &mut air_gestures::teach::Terminal,
                ))),
                DrawCommand::List => report(air_gestures::teach::list_drawings(&dir).map(|list| println!("{list}"))),
                DrawCommand::Delete { name } => report(air_gestures::teach::delete_drawing(&dir, &name).map(|()| println!("deleted \"{name}\""))),
            }
        }
```

- [ ] **Step 4: Run the tests** — as Task 1 Step 6.

- [ ] **Step 5: Commit** `air-gestures/src/teach.rs air-gestures/src/main.rs air-gestures/tests/teach.rs` — message `air-gestures draw teach|list|delete`.

---

### Task 12: Docs: README, contrib/config.toml, CHANGELOG

**Files:**
- Modify: `README.md`, `contrib/config.toml`, `CHANGELOG.md`

- [ ] **Step 1: `contrib/config.toml`** — replace it with the following (it must still parse to exactly the defaults: `the_documented_config_is_the_default` checks it):

```toml
# air-gesturesd config: ~/.config/air-gestures/config.toml (reloaded on save).
# Every gesture runs a shell command through `sh -c`; "" does nothing.
# Gestures: swipe_left/right/up/down; per finger (index_, middle_): tap, double, triple, hold,
# drag_left/right/toward/away/up/down. Triple taps: taps < 0.45 s apart.
# A pinch is a hold (let go without moving), a turn (the finger's dial) or a drag.

[band]
hand = "band"              # "band" = whatever the band reports | "left" | "right"
autostart = true           # connect and enable controls when air-gesturesd starts
profile = "default"        # the name of the [gestures] profile
# Band controls: one gesture each (any gesture key, or "none"), above every profile.
# A profile's command on that gesture is kept but paused (the daemon logs it).
toggle = "none"            # controls off/on (works while they're off)
next_profile = "none"      # next profile (needs 2+ profiles)
previous_profile = "none"  # previous profile
draw = "none"              # open a drawing; again: cancel it
# toggle_palm = ["down"]   # <key>_palm: only in these palm positions ("down", "sideways", "up"; 1–2)
on_toggle = ""             # after a gesture toggled controls, with AIR_GESTURES_ENABLED=1|0
on_profile = ""            # after the profile changed, with AIR_GESTURES_PROFILE=<name>
on_draw = 'notify-send -a air-gestures "Drawing" "${AIR_GESTURES_DRAWING:-not recognised}"'  # AIR_GESTURES_DRAWING="" = not recognised

[gestures]                 # the first profile
swipe_left    = "hyprctl dispatch workspace e-1"
swipe_right   = "hyprctl dispatch workspace e+1"
swipe_up      = "wofi --show drun"
swipe_down    = "hyprctl dispatch sendshortcut , Escape, activewindow"
index_tap     = ""
index_double  = "playerctl play-pause"
index_triple  = ""
index_hold    = ""
middle_tap    = ""
middle_double = "wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle"
middle_triple = ""
middle_hold   = ""
# index_drag_left = ""     # …_drag_right, _toward, _away, _up, _down; middle_ likewise
# index_dial  = "volume"   # the turn: "volume" | "brightness" | "off" (default: [dial] target)
# middle_dial = "off"      # with a middle dial, the middle hold acts when you let go
# index_drags = "gestures" # or "sliders": each pair moves a slider while you hold the pinch
# index_slider_left_right = "off"  # also _toward_away, _up_down: "volume" | "brightness" | "off"

# Palm overrides: the same gesture keys, for one position; at most 2 positions per gesture.
# [gestures.palm_up]       # also [gestures.sideways], [gestures.palm_down]
# index_tap = "playerctl next"

# More profiles: same keys; a gesture key you leave out does nothing there.
# [[profile]]
# name = "games"
# swipe_left = "…"
# [profile.palm_up]        # this profile's palm overrides
# [profile.drawings]       # this profile's own drawing actions (win over [drawings])

# Drawings: teach one with `air-gestures draw teach <name>`, then give it an action.
# [drawings]
# lightbulb = "hass-cli light toggle"

[dial]                     # pinch thumb + index, then turn your wrist
target = "volume"          # what the index dial turns by default: "volume" | "brightness" | "off"
sensitivity = 1.0          # 0.5–4; 1.0 = one step per 2° of wrist turn
volume_up       = "wpctl set-volume -l 1.0 @DEFAULT_AUDIO_SINK@ 2%+"
volume_down     = "wpctl set-volume @DEFAULT_AUDIO_SINK@ 2%-"
brightness_up   = "brightnessctl -q set 2%+"
brightness_down = "brightnessctl -q set 2%-"
```

- [ ] **Step 2: `README.md`** — replace everything from "Gestures and the dial are configured in …" up to (not including) "`AIR_GESTURES_LOG=air-gesturesd=debug` …" with:

````markdown
## Configure

Everything is in `~/.config/air-gestures/config.toml` (reloaded on save); every key is in
[`contrib/config.toml`](contrib/config.toml). A gesture runs a shell command; `""` does nothing.
The daemon logs config problems as errors, and every command a band control pauses as a warning.

### Gestures and the pinch

24 gestures: `swipe_left|right|up|down`, and per finger (`index_`, `middle_`) `tap`, `double`,
`triple`, `hold` and six drags, `drag_left|right|toward|away|up|down`.

A pinch (thumb + finger) is one of three things, decided once you've moved 15°:
- let go without moving: the finger's **hold**;
- turn your wrist: the finger's **dial**, `index_dial` / `middle_dial` = `"volume"`,
  `"brightness"` or `"off"` (index: `[dial] target`; middle: off);
- move your hand: a **drag**. With `index_drags = "gestures"` (the default) each drag runs its
  `index_drag_*` command once; with `"sliders"`, the pair you start along (left/right,
  toward/away, up/down) moves `index_slider_left_right|toward_away|up_down` (`"volume"`,
  `"brightness"` or `"off"`) for as long as you hold the pinch.

When a finger's dial and drags do nothing, its hold acts as soon as the band reports it
(the middle hold, by default); otherwise when you let go. `air-gestures dial
volume|brightness|off|cycle` changes the index dial until the profile changes.

**Teach your drags:** `air-gestures teach-drags` takes one pinch per move (the turn, then the
six drags) and keeps them for this band and hand; `--reset` forgets them.

### Band controls

Four controls sit above every profile, each on one gesture you pick in `[band]`:
`toggle` (controls off/on; works while they're off), `next_profile`, `previous_profile` and
`draw`. Each is any gesture key or `"none"` (the default). `<key>_palm = ["up"]` limits one to
one or two palm positions; in the others the gesture does what the profile says.

A profile's command on a band-control gesture is kept but doesn't run ("paused"); the daemon
logs each one, e.g. `middle_double in profile "media" is overridden by [band] next_profile`,
and it runs again once the band control moves. Two band controls on one gesture and position
is a config error. `toggle = "middle_hold"` from older configs keeps working.

### Profiles

`[gestures]` is the first profile (named `default`, or `[band] profile`); more go in
`[[profile]]` tables with a `name` and the same keys. A gesture a profile leaves out does
nothing there:

```toml
[band]
next_profile = "index_triple"

[[profile]]
name = "media"
index_tap = "playerctl play-pause"
index_dial = "volume"
```

Switch with the band controls or `air-gestures profile next|previous|<name>`; the current one
shows in `air-gestures status` and the waybar tooltip. `on_profile` runs on every switch with
`AIR_GESTURES_PROFILE` set.

### Palm position

Any gesture can do something else with the palm up, sideways or down: write it in
`[gestures.palm_up]`, `[gestures.sideways]` or `[gestures.palm_down]` (in a profile:
`[profile.palm_up]` …), at most two positions per gesture; the normal command covers the rest.
`""` there means nothing in that position.

```toml
[gestures.palm_up]
index_tap = "playerctl next"
```

The right hand works out of the box; `air-gestures calibrate` (palm down, then palm up, 2 s
each) fits it to your wrist, and is needed for the left hand (until then every gesture counts
as palm down). It's kept per band and hand; `--reset` forgets it.

### Drawings

Put Draw on a gesture (`[band] draw = "swipe_up"`), make it, and draw with a pinch as the pen,
in the air or pen grip on a table; the drawing ends 1 s after your last stroke. Teach drawings
first, three tries each: `air-gestures draw teach lightbulb` (`draw list`, `draw delete
<name>`). Then give them actions:

```toml
[drawings]                 # every profile
lightbulb = "hass-cli light toggle"

[[profile]]
name = "media"
[profile.drawings]         # this profile's own, wins over [drawings]
lightbulb = "playerctl play-pause"
```

`on_draw` runs after each drawing with `AIR_GESTURES_DRAWING` set to its name (empty when
nothing matched); by default it shows a notification. Templates live in
`~/.local/share/air-gestures/drawings/`.
````

- [ ] **Step 3: `CHANGELOG.md`** — replace the `## Unreleased` bullets with:

```markdown
- **Band controls:** `[band] toggle`, `next_profile`, `previous_profile` and `draw` each sit on
  one gesture you pick (any gesture, `"none"` by default), above every profile, optionally only
  in some palm positions (`toggle_palm = ["up"]`). A profile's command on that gesture is kept
  and paused; the daemon logs each one. `toggle = "middle_hold"` from older configs keeps working.
- **Profiles:** `[[profile]]` tables in config.toml, each with its own gestures. Switch with
  `next_profile` / `previous_profile`, or `air-gestures profile next|previous|<name>`;
  `on_profile` runs on every switch with `AIR_GESTURES_PROFILE` set.
- **The pinch:** index and middle each have a hold, a dial (`index_dial`, `middle_dial`) and six
  drags, as gestures (`index_drag_left` …) or sliders (`index_drags = "sliders"`). New gestures
  `index_hold`, `index_triple`, `middle_triple` and the twelve drags.
  `air-gestures teach-drags` teaches your own drags.
- **Palm position:** `[gestures.palm_up]`, `.sideways` and `.palm_down` override gestures in
  that position; `air-gestures calibrate` fits palm down and up to your wrist (needed for the
  left hand).
- **Drawings:** Draw a shape with a pinch to run its action (`[drawings]`,
  `[profile.drawings]`); teach shapes with `air-gestures draw teach <name>`; `on_draw` shows
  what was drawn.
- Every config load logs warnings (no way to switch profiles, Draw on a tap) and each paused
  command.
- A slow triple tap no longer runs its finger's single tap first.
- Gestures, the dial and the controls now come from band-core's shared controls layer, the
  same code as the Android app.
- A band busy with another device says so instead of "Missing protobuf integer".
```

- [ ] **Step 4: Run the tests** — as Task 1 Step 6 (`the_documented_config_is_the_default` guards the example).

- [ ] **Step 5: Commit** `README.md contrib/config.toml CHANGELOG.md` — message `Docs: band controls, the pinch, palm positions and drawings`.

---

### Task 13: Final check, and a dry-run smoke test

**Files:** none (unless something fails).

- [ ] **Step 1: Everything passes**

```bash
cd ~/Projects/air-gestures
cargo test --workspace 2>&1 | grep -E "^test result|FAILED|panicked|^error" | sort | uniq -c
cargo clippy --workspace --all-targets 2>&1 | grep -E "^(warning|error)" | sort | uniq -c
git status -s     # only "?? air-gestures/examples/"
git -C band-core status -s   # clean; HEAD = the band-core controls tip from Task 1
```

Expected: only `test result: ok`; no clippy lines from air-gesturesd / air-gestures.

- [ ] **Step 2: A dry-run daemon, fully isolated** (never the user service; never the owner's config)

```bash
T=$(mktemp -d)
cat > "$T/config.toml" <<'EOF'
[band]
draw = "swipe_up"
next_profile = "middle_double"
[[profile]]
name = "media"
middle_double = "echo paused"
[gestures.palm_up]
index_tap = "echo up"
[drawings]
lightbulb = "echo light"
EOF
cargo build -q -p air-gesturesd -p air-gestures
E="env XDG_RUNTIME_DIR=$T XDG_STATE_HOME=$T/state XDG_DATA_HOME=$T/data"
$E target/debug/air-gesturesd --dry-run --config "$T/config.toml" > "$T/log" 2>&1 &
D=$!
sleep 2
$E target/debug/air-gestures --dry-run status
$E target/debug/air-gestures --dry-run draw list        # no drawings yet
$E target/debug/air-gestures --dry-run calibrate < /dev/null   # connect the band first
grep -E "config:|drawing" "$T/log"
kill $D; wait $D; rm -rf "$T"
```

Expected log lines: `middle_double in profile "default" is overridden by [band] next_profile`, the same for `"media"`, `drawing "lightbulb" has an action but isn't taught yet`, and the info that no gesture turns the controls off/on. (The dry-run band is never paired, so `status` says how to pair and `calibrate` asks to connect; that's fine here.)

- [ ] **Step 3: Hand over the hardware check to the owner** (don't do it yourself: it needs the band and restarting the service). Checklist for the owner:
  - `air-gestures calibrate` (its last step should read "palm up now"), then a gesture with a palm override, palm up and sideways (index taps sideways: the spec's open question);
  - `air-gestures teach-drags`, then drags and a slider with the palm up;
  - `air-gestures draw teach lightbulb` (air), and a pen-grip check mark on the table; Draw on a non-default gesture; `on_draw`'s notification;
  - a paused command coming back after moving its band control; the lock while off.

---

## Notes for the executor

- **band-core API this plan relies on** (added for it in `5ff7ca7`): `Controls::set_calibration` (Task 7's per-hand swap: no outputs, the hand and any IPC choice stay), `Controls::palm(now)` (Task 8's `palm` request, which `calibrate` confirms with), and `Responder::gyro3` / `Responder::orientation` (Task 7's `SimInject::Gyro3` / `SimInject::Orientation`, used by the end-to-end palm and calibration tests). If Task 1's `grep` for `set_calibration` fails, the submodule isn't at a new enough tip: stop and ask.
- `Options` still holds one hand's calibration, so the daemon swaps it (`refresh_calibration`) whenever the band reports another hand.
- If a daemon test that pinches is flaky, check first that gyro samples keep coming during the pinch (the engine drops a pinch whose motion stops for 0.35 s), and that strokes last ≥ 0.3 s.
