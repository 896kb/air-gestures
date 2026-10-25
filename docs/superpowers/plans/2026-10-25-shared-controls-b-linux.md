# Shared controls, plan B: the Linux daemon on band-core's `controls`, with profiles

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `air-gesturesd` drops its own `controller.rs` and runs on band-core's `Controls<String>`. In the process `config.toml` gains:
- `[[profile]]` tables;
- the built-in values `@controls`, `@next-profile` and `@previous-profile`;
- `index_triple` / `middle_triple`;
- `on_profile`;
- `air-gestures profile next|previous|<name>`.

**Architecture:**
- **`config.rs`** reads `[gestures]` and every `[[profile]]` as key→value maps, so it knows which keys were written. It validates them and builds `band_core::controls::Profiles<String>` and `Options`.
- **The daemon** keeps:
  - the dial target (volume, brightness or off, cycled over IPC), putting that target's commands into every profile's `dial`;
  - the hooks: `on_toggle` only for band toggles, as today; `on_profile` on every profile change.
- **The engine:** all gesture logic is the engine's. Its 24 tests already moved to band-core (`tests/controls_linux.rs`).

**Tech Stack:** Rust 2024, tokio, serde/toml, clap. band-core at the `controls` branch (0.5.0, local until pushed).

**Spec:** `~/Projects/band-core/docs/superpowers/specs/2026-10-25-shared-controls-design.md` (the "Linux" section, and the API sketch).

**Rules:**
- Work in `~/Projects/air-gestures` on a new branch `controls` from `master`.
- No push and no tags.
- Don't stop or restart the `air-gesturesd` user service except in Task 6, with the owner.
- Never run `iwctl`. No sudo.
- Never touch the owner's live config `~/arch_rice/.config/air-gestures/config.toml` (Hyprland and the daemon reload it on write).
- Commit messages end with:
  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01MLDqGPYx8bTGKJcVLgoyzo
  ```
- Run `cargo test --workspace` and `cargo clippy --workspace --all-targets` (no new warnings) before every commit.

## The engine's API (band-core `src/controls/mod.rs`; read it for the details)

- **Setup and getters:**
  - `Controls::<String>::new(Options, Profiles<String>)`;
  - `status() -> &ControlStatus<String>`, with fields `paused`, `battery`, `charging`, `hand: Option<Hand>`, `last_gesture: Option<&'static str>`, `last_action`, `gestures`, `profile`;
  - `active()`, `profiles()`, `enabled()`.
- **Setters:** `set_options(Options) -> Vec<Output>` and `set_profiles(Profiles) -> Vec<Output>`. Both keep pending taps and the dial; `set_options` re-requests the hand.
- **Events and time:** `on_event(&Event, now) -> Vec<Output>`, `next_deadline() -> Option<f64>`, `poll(now) -> Vec<Output>`, `on_disconnect()`.
- **Controls and hand:** `pause()`, `resume(now)`, `toggle(now)` (each returns `Vec<Output>`), and `request_hand(Hand) -> Vec<Output>`.
- **Profiles:** `pick(index) -> Vec<Output>`.
- **Outputs:** `Output::{Gesture(Gesture), Run(String), Controls{enabled}, Profile{index, step}, ProfileHeld{app}, SetHand(Hand), Practiced(String)}`.
- **Types:**
  - `Gesture::{ALL, key(), label(), from_key()}` and `Builtin::{key(), from_key()}`, with keys `"controls"`, `"next-profile"`, `"previous-profile"`;
  - `Action::{Builtin(_), Run(_)}`, `Profile { name, gestures, dial }`, `Profiles { list, picked, apps, calls }`;
  - `Profiles::warnings() -> Vec<Warning::{NoControls{profile}, NoWayOut{profile}}>`;
  - `Options { hand: band_core::controls::HandSetting, autostart, dial_sensitivity }`.

---

### Task 1: Use band-core 0.5.0 (the `controls` branch)

**Files:** the `band-core` submodule pointer, `Cargo.lock`.

- [ ] **Step 1: Branch, and move the submodule to the local `controls` commit**

```bash
cd ~/Projects/air-gestures
git status -s                      # only "?? air-gestures/examples/" (left untracked on purpose)
git switch -c controls
git -C band-core fetch -q /home/user888/Projects/band-core controls
git -C band-core checkout -q FETCH_HEAD
git -C band-core log --oneline -1  # the band-core controls HEAD ("controls: tighter tests" or later)
```

- [ ] **Step 2: Still builds and passes (the new module isn't used yet)**

Run: `cargo test --workspace 2>&1 | grep -E "^error|FAILED" | wc -l`

Expected: `0`. `Cargo.lock` now shows band-core 0.5.0.

- [ ] **Step 3: Commit**

```bash
git add band-core Cargo.lock
git commit -q -m "band-core 0.5.0 (the shared controls layer; local until pushed)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01MLDqGPYx8bTGKJcVLgoyzo"
```

---

### Task 2: `config.toml` with profiles and built-ins

**Files:**
- Modify: `air-gesturesd/src/config.rs`
- Modify: `air-gesturesd/tests/config.rs`
- Modify: `contrib/config.toml`

- [ ] **Step 1: Write the failing tests**

Replace `air-gesturesd/tests/config.rs` with the following. It keeps the old tests' intent, but on the new API:

```rust
use air_gesturesd::config::{Config, DialTarget, HandSetting};
use band_core::controls::{Action, Builtin, Gesture};

fn run(command: &str) -> Option<Action<String>> {
    Some(Action::Run(command.into()))
}

fn builtin(builtin: Builtin) -> Option<Action<String>> {
    Some(Action::Builtin(builtin))
}

#[test]
fn defaults_match_the_design() {
    let config = Config::default();
    assert_eq!(config.band.hand, HandSetting::Band);
    assert!(config.band.autostart);
    let profiles = config.profiles(DialTarget::Volume);
    assert_eq!(profiles.list.len(), 1);
    let first = &profiles.list[0];
    assert_eq!(first.name, "default");
    let get = |g| first.gestures.get(&g).cloned();
    assert_eq!(get(Gesture::SwipeLeft), run("hyprctl dispatch workspace e-1"));
    assert_eq!(get(Gesture::SwipeRight), run("hyprctl dispatch workspace e+1"));
    assert_eq!(get(Gesture::SwipeUp), run("wofi --show drun"));
    assert_eq!(get(Gesture::SwipeDown), run("hyprctl dispatch sendshortcut , Escape, activewindow"));
    assert_eq!(get(Gesture::IndexTap), None);
    assert_eq!(get(Gesture::IndexDouble), run("playerctl play-pause"));
    assert_eq!(get(Gesture::MiddleDouble), run("wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle"));
    assert_eq!(get(Gesture::MiddleHold), builtin(Builtin::Controls));
    assert_eq!(get(Gesture::IndexTriple), builtin(Builtin::NextProfile));
    assert_eq!(get(Gesture::MiddleTriple), builtin(Builtin::PreviousProfile));
    let dial = first.dial.clone().unwrap();
    assert_eq!(dial.up, "wpctl set-volume -l 1.0 @DEFAULT_AUDIO_SINK@ 2%+");
    assert_eq!(config.dial.target, DialTarget::Volume);
    assert_eq!(config.dial.sensitivity, 1.0);
    assert!(config.profiles(DialTarget::Off).list[0].dial.is_none());
    assert_eq!(
        config.profiles(DialTarget::Brightness).list[0].dial.clone().unwrap().down,
        "brightnessctl -q set 2%-"
    );
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
    // brightness_up unassigned: no dial for that target.
    assert!(config.profiles(DialTarget::Brightness).list[0].dial.is_none());
}

#[test]
fn whitespace_or_empty_values_are_unassigned() {
    let config = Config::parse("[gestures]\nindex_double = \"   \"\nmiddle_hold = \"\"\n").unwrap();
    let first = &config.profiles(DialTarget::Volume).list[0];
    assert_eq!(first.gestures.get(&Gesture::IndexDouble), None);
    assert_eq!(first.gestures.get(&Gesture::MiddleHold), None);
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
        index_triple = "@next-profile"
        [[profile]]
        name = "media"
        middle_hold = "@controls"
        "#,
    )
    .unwrap();
    let profiles = config.profiles(DialTarget::Volume);
    let names: Vec<&str> = profiles.list.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["desk", "games", "media"]);
    let games = &profiles.list[1];
    assert_eq!(games.gestures.get(&Gesture::SwipeLeft).cloned(), run("echo games"));
    assert_eq!(games.gestures.get(&Gesture::IndexTriple).cloned(), builtin(Builtin::NextProfile));
    assert_eq!(games.gestures.get(&Gesture::SwipeRight), None, "extra profiles don't inherit");
    assert!(games.dial.is_some(), "every profile turns the daemon's dial target");
}

#[test]
fn the_old_toggle_key_binds_the_lock_where_nothing_else_was_written() {
    let config = Config::parse(
        r#"
        [band]
        toggle = "index_triple"
        [gestures]
        middle_hold = "echo hold"
        [[profile]]
        name = "two"
        [[profile]]
        name = "three"
        index_triple = "echo mine"
        "#,
    )
    .unwrap();
    let profiles = config.profiles(DialTarget::Volume);
    let triple = |i: usize| profiles.list[i].gestures.get(&Gesture::IndexTriple).cloned();
    assert_eq!(triple(0), builtin(Builtin::Controls), "overrides the [gestures] default");
    assert_eq!(triple(1), builtin(Builtin::Controls));
    assert_eq!(triple(2), run("echo mine"), "a profile that set the key keeps it");
    assert_eq!(profiles.list[0].gestures.get(&Gesture::MiddleHold).cloned(), run("echo hold"));
}

#[test]
fn mistakes_are_reported() {
    for (text, needle) in [
        ("[band]\ncolour = \"red\"\n", "colour"),
        ("[gestures]\nindex_hold = \"x\"\n", "index_hold"),
        ("[gestures]\nswipe_up = \"@lock\"\n", "@lock"),
        ("[dial]\nsensitivity = 9.0\n", "sensitivity"),
        ("[[profile]]\nswipe_up = \"x\"\n", "name"),
        ("[[profile]]\nname = \"default\"\n", "default"),
        ("[[profile]]\nname = \"a\"\n[[profile]]\nname = \"a\"\n", "\"a\""),
        ("[[profile]]\nname = \"it's\"\n", "it's"),
        ("[[profile]]\nname = \"x\"\nnope = \"y\"\n", "nope"),
    ] {
        let error = Config::parse(text).unwrap_err();
        assert!(error.contains(needle), "{text:?} → {error}");
    }
}

#[test]
fn a_missing_file_means_the_defaults() {
    let config = Config::load(std::path::Path::new("/nonexistent/air-gestures/config.toml")).unwrap();
    assert_eq!(config, Config::default());
}

#[test]
fn the_documented_config_is_the_default() {
    let text = include_str!("../../contrib/config.toml");
    assert_eq!(Config::parse(text).unwrap(), Config::default());
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
}
```

Before deleting the old file, compare it with this one. Any old test not covered above must be added back in the new API.

Run: `cargo test -p air-gesturesd --test config 2>&1 | grep -E "error\[E" | head -3`

Expected: compile errors (no `profiles`, `options`, `band.profile`).

- [ ] **Step 2: Rewrite `air-gesturesd/src/config.rs`**

Keep:
- `HandSetting`, `DialTarget` (with `next`) and `DialConfig` (with `command`), unchanged;
- `home()`, `xdg()` and the `#[cfg(test)] mod tests` at the bottom, unchanged;
- `Config::load` and `Config::default_path`, unchanged.

Remove `GestureConfig` and its `command(Recognized)`. Change the rest as follows.

```rust
//! `~/.config/air-gestures/config.toml`: profiles of gesture → shell command (or a built-in:
//! `@controls`, `@next-profile`, `@previous-profile`; `""` = unassigned), and the dial.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use band_core::controls::{Action, Builtin, Dial, Gesture, Options, Profile, Profiles};
use serde::{Deserialize, Serialize};
```

`ToggleGesture` stays, plus a method:

```rust
impl ToggleGesture {
    /// The gesture the old `toggle` key put the lock on.
    pub fn gesture(self) -> Option<Gesture> {
        match self {
            ToggleGesture::None => None,
            ToggleGesture::MiddleHold => Some(Gesture::MiddleHold),
            ToggleGesture::IndexTriple => Some(Gesture::IndexTriple),
            ToggleGesture::MiddleTriple => Some(Gesture::MiddleTriple),
        }
    }
}
```

`BandConfig` gains `on_profile` and `profile`:

```rust
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BandConfig {
    pub hand: HandSetting,
    /// Connect and enable controls when the daemon starts.
    pub autostart: bool,
    /// Old style: the gesture that turns the controls off and on, in every profile that
    /// doesn't set that key itself. Today: put "@controls" on any gesture.
    pub toggle: ToggleGesture,
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
            toggle: ToggleGesture::None,
            on_toggle: String::new(),
            on_profile: String::new(),
            profile: "default".into(),
        }
    }
}
```

The first profile's defaults and the value parser:

```rust
/// `[gestures]` keys nobody wrote: the first profile's defaults.
fn default_gesture(gesture: Gesture) -> &'static str {
    match gesture {
        Gesture::SwipeLeft => "hyprctl dispatch workspace e-1",
        Gesture::SwipeRight => "hyprctl dispatch workspace e+1",
        Gesture::SwipeUp => "wofi --show drun",
        Gesture::SwipeDown => "hyprctl dispatch sendshortcut , Escape, activewindow",
        Gesture::IndexDouble => "playerctl play-pause",
        Gesture::MiddleDouble => "wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle",
        Gesture::MiddleHold => "@controls",
        Gesture::IndexTriple => "@next-profile",
        Gesture::MiddleTriple => "@previous-profile",
        Gesture::IndexTap | Gesture::MiddleTap => "",
    }
}

/// A gesture's value: `""` (or blanks) = nothing, `@<builtin>`, else a shell command.
pub fn action(value: &str) -> Result<Option<Action<String>>, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if let Some(key) = trimmed.strip_prefix('@') {
        return match Builtin::from_key(key) {
            Some(builtin) => Ok(Some(Action::Builtin(builtin))),
            None => Err(format!(
                "unknown built-in \"{trimmed}\" (use @controls, @next-profile or @previous-profile)"
            )),
        };
    }
    Ok(Some(Action::Run(value.to_owned())))
}

/// Profile names go into `AIR_GESTURES_PROFILE='…'`: letters, digits, space, `.`, `_`, `-`.
fn check_name(name: &str) -> Result<(), String> {
    let fine = !name.trim().is_empty()
        && name.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-'));
    if fine {
        Ok(())
    } else {
        Err(format!("profile name \"{name}\": use letters, digits, spaces, '.', '_' or '-'"))
    }
}
```

`DialConfig` gains:

```rust
impl DialConfig {
    /// Both commands for this target, or `None` when it's off or either is unassigned.
    pub fn dial(&self, target: DialTarget) -> Option<Dial<String>> {
        Some(Dial {
            up: self.command(target, true)?.to_owned(),
            down: self.command(target, false)?.to_owned(),
        })
    }
}
```

The config:

```rust
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub band: BandConfig,
    /// The first profile, as written (unwritten keys take `default_gesture`).
    gestures: BTreeMap<String, String>,
    /// More profiles, in order (`name` plus gesture keys; unwritten keys do nothing).
    profile: Vec<BTreeMap<String, String>>,
    pub dial: DialConfig,
}

impl Config {
    pub fn parse(text: &str) -> Result<Self, String> {
        let config: Config = toml::from_str(text).map_err(|error| error.to_string())?;
        if !(0.5..=4.0).contains(&config.dial.sensitivity) {
            return Err(format!(
                "dial.sensitivity must be between 0.5 and 4 (got {})",
                config.dial.sensitivity
            ));
        }
        config.build(DialTarget::Off)?;
        Ok(config)
    }

    pub fn options(&self) -> Options {
        Options {
            hand: match self.band.hand {
                HandSetting::Band => band_core::controls::HandSetting::Band,
                HandSetting::Left => band_core::controls::HandSetting::Left,
                HandSetting::Right => band_core::controls::HandSetting::Right,
            },
            autostart: self.band.autostart,
            dial_sensitivity: self.dial.sensitivity,
        }
    }

    /// Every profile, each turning the dial at `target`.
    pub fn profiles(&self, target: DialTarget) -> Profiles<String> {
        self.build(target).expect("checked when the config was parsed")
    }

    fn build(&self, target: DialTarget) -> Result<Profiles<String>, String> {
        let dial = self.dial.dial(target);
        let lock = self.band.toggle.gesture();
        let mut list = Vec::new();
        // The first profile: [gestures] over the defaults.
        check_name(&self.band.profile).map_err(|e| format!("band.profile: {e}"))?;
        for key in self.gestures.keys() {
            if Gesture::from_key(key).is_none() {
                return Err(format!("gestures: unknown gesture \"{key}\""));
            }
        }
        let mut first = Profile::new(&self.band.profile);
        for gesture in Gesture::ALL {
            let value = match self.gestures.get(gesture.key()) {
                Some(value) => value.as_str(),
                None if lock == Some(gesture) => "@controls",
                None => default_gesture(gesture),
            };
            if let Some(action) = action(value).map_err(|e| format!("gestures.{}: {e}", gesture.key()))? {
                first.gestures.insert(gesture, action);
            }
        }
        first.dial = dial.clone();
        list.push(first);
        // The other profiles.
        for table in &self.profile {
            let name = table.get("name").ok_or("every [[profile]] needs a name")?;
            check_name(name)?;
            if list.iter().any(|p: &Profile<String>| p.name == *name) {
                return Err(format!("two profiles are called \"{name}\""));
            }
            let mut profile = Profile::new(name);
            for (key, value) in table {
                if key == "name" {
                    continue;
                }
                let gesture = Gesture::from_key(key)
                    .ok_or_else(|| format!("profile \"{name}\": unknown key \"{key}\""))?;
                if let Some(action) = action(value).map_err(|e| format!("profile \"{name}\".{key}: {e}"))? {
                    profile.gestures.insert(gesture, action);
                }
            }
            if let Some(gesture) = lock {
                profile.gestures.entry(gesture).or_insert(Action::Builtin(Builtin::Controls));
            }
            profile.dial = dial.clone();
            list.push(profile);
        }
        Ok(Profiles { list, picked: 0, apps: BTreeMap::new(), calls: BTreeMap::new() })
    }

    // load() and default_path() unchanged.
}
```

The first profile's rule is "`[gestures]` value if written, else the old `toggle`'s `@controls`, else the default". Extra profiles start empty, and the old `toggle` gives `@controls` only where a profile leaves that key out.

Note on `Config::default()`: `gestures` and `profile` are empty, and `band.profile` is `"default"`. So `profiles()` gives one profile from the defaults, which matches what the tests expect.

- [ ] **Step 3: Update `contrib/config.toml` to the new defaults**

```toml
# air-gesturesd config: ~/.config/air-gestures/config.toml (reloaded on save).
# Every gesture runs a shell command through `sh -c`; "" leaves it unassigned.
# Built-in actions work on any gesture:
#   "@controls"          turn the controls off/on (works while they're off)
#   "@next-profile"      switch to the next profile  (needs 2+ profiles)
#   "@previous-profile"  switch to the previous one
# Triple taps: taps <0.45 s apart; with one waiting, that finger's taps wait 0.45 s.

[band]
hand = "band"        # "band" = whatever the band reports | "left" | "right"
autostart = true     # connect and enable controls when air-gesturesd starts
toggle = "none"      # old style; put "@controls" on a gesture instead
on_toggle = ""       # run after a gesture toggled controls, with AIR_GESTURES_ENABLED=1|0
on_profile = ""      # run after the profile changed, with AIR_GESTURES_PROFILE=<name>
profile = "default"  # the name of the [gestures] profile

[gestures]           # the first profile
swipe_left    = "hyprctl dispatch workspace e-1"
swipe_right   = "hyprctl dispatch workspace e+1"
swipe_up      = "wofi --show drun"
swipe_down    = "hyprctl dispatch sendshortcut , Escape, activewindow"
index_tap     = ""
index_double  = "playerctl play-pause"
index_triple  = "@next-profile"
middle_tap    = ""
middle_double = "wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle"
middle_triple = "@previous-profile"
middle_hold   = "@controls"

# More profiles: same keys; a key you leave out does nothing there.
# [[profile]]
# name = "games"
# swipe_left = "…"
# index_triple = "@next-profile"
# middle_hold = "@controls"

[dial]               # pinch thumb + index, hold a moment, then turn your wrist
target = "volume"    # "volume" | "brightness" | "off"
sensitivity = 1.0    # 0.5–4; 1.0 = one step per 2° of wrist turn
volume_up       = "wpctl set-volume -l 1.0 @DEFAULT_AUDIO_SINK@ 2%+"
volume_down     = "wpctl set-volume @DEFAULT_AUDIO_SINK@ 2%-"
brightness_up   = "brightnessctl -q set 2%+"
brightness_down = "brightnessctl -q set 2%-"
```

Careful: `the_documented_config_is_the_default` compares the parsed file with `Config::default()`. A written `[gestures]` map is not equal to the default's empty map. Make the test compare `parse(text).profiles(Volume)` and `options()` with the defaults' instead, plus `band` and `dial`:

```rust
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
```

`a_missing_file_means_the_defaults` keeps `assert_eq!(config, Config::default())`; that's fine.

- [ ] **Step 4: Run the config tests**

Run: `cargo test -p air-gesturesd --test config 2>&1 | grep -E "test result|FAILED|panicked"`

Expected: `ok. 10 passed`.

The daemon and the old controller won't compile against the new `config.rs` yet. That's Task 3, so in this task build only what the config tests need. If `cargo test -p air-gesturesd --test config` can't build because other modules use `GestureConfig`, do Task 2 and Task 3 in one commit instead, and say so in the report.

- [ ] **Step 5: Commit** (only if the workspace builds; see Step 4)

```bash
git add air-gesturesd/src/config.rs air-gesturesd/tests/config.rs contrib/config.toml
git commit -q -m "config: profiles, built-in actions (@controls, @next-profile, @previous-profile), triples

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01MLDqGPYx8bTGKJcVLgoyzo"
```

---

### Task 3: The daemon on `Controls<String>`

**Files:**
- Modify: `air-gesturesd/src/daemon.rs`, `air-gesturesd/src/status.rs`, `air-gesturesd/src/lib.rs`
- Delete: `air-gesturesd/src/controller.rs`, `air-gesturesd/tests/controller.rs` (ported to band-core's `tests/controls_linux.rs`)
- Test: `air-gesturesd/tests/daemon.rs` (existing; must pass), plus the new tests below

- [ ] **Step 1: Write the failing daemon tests**

Add to `air-gesturesd/tests/daemon.rs`. Model them on the existing tests that check "gestures run commands" and "edited config applies": use the same harness (`start`, the fake runner, writing the config file, sending band gestures through the sim transport), and read those tests first.
1. **`a_band_toggle_runs_on_toggle_but_ipc_pause_does_not`:**
   - config `[band] on_toggle = "osd"` (default `middle_hold = "@controls"`);
   - a band middle hold → the runner got `export AIR_GESTURES_ENABLED=0; osd`;
   - an IPC `Pause` / `Resume` → nothing more ran.
2. **`profiles_switch_from_the_band_and_run_on_profile`:**
   - config with `[band] on_profile = "osd"` and one `[[profile]] name = "games"`;
   - a band index triple (three taps 0.2 s apart, the 2nd and 3rd with their double taps, as in band-core's `controls_common::triple`) → the runner got `export AIR_GESTURES_PROFILE='games'; osd`;
   - status `profile` is `Some("games")`.
3. **`a_single_profile_shows_no_profile_in_status`:** default config → status `profile` is `None`.

Run: `cargo test -p air-gesturesd --test daemon 2>&1 | grep -E "error\[E|FAILED" | head`

Expected: compile errors or failures.

- [ ] **Step 2: Status gets the active profile**

In `air-gesturesd/src/status.rs`, add to `Status` (after `band`):

```rust
    /// The active profile's name, when there is more than one profile.
    pub profile: Option<String>,
```

and `profile: None,` in its `Default`. The field is additive, so the status contract stays at version 1.

- [ ] **Step 3: Swap the controller for `Controls`**

In `air-gesturesd/src/daemon.rs`:
1. Replace the `Controller` import with:
   ```rust
   use band_core::controls::{Controls, Output};
   use crate::config::{Config, DialTarget};
   ```
2. Replace the `controller: Controller` field with:
   ```rust
       controls: Controls<String>,
       /// The loaded config (hooks, dial commands, profiles).
       config: Config,
       /// What the dial turns: the file's `[dial] target`, or a runtime choice over IPC.
       dial_target: DialTarget,
   ```
3. In `new`:
   ```rust
           let dial_target = config.dial.target;
           let controls = Controls::new(config.options(), config.profiles(dial_target));
           warn_profiles(&controls);
   ```
   Then `controls, config, dial_target,` in the struct literal (`wants_connection` still reads `config.band.autostart` before the move).

Add these helpers to `impl Daemon`:

```rust
    /// Do what the controls asked. `from_band`: caused by a band gesture, not a request
    /// (`on_toggle` only runs for those, as before).
    fn apply(&mut self, outputs: Vec<Output<String>>, from_band: bool) {
        for output in outputs {
            match output {
                Output::Run(command) => {
                    tracing::info!(%command, "action");
                    self.runner.run(command);
                }
                Output::SetHand(hand) => {
                    self.send_link(LinkCommand::SetHand(hand));
                }
                Output::Gesture(gesture) => {
                    tracing::debug!(gesture = gesture.label(), enabled = self.controls.enabled(), "recognized");
                }
                Output::Controls { enabled } => {
                    tracing::info!(enabled, "controls");
                    let hook = self.config.band.on_toggle.trim();
                    if from_band && !hook.is_empty() {
                        // Exported, not a prefix assignment: the hook's own `$AIR_GESTURES_ENABLED`
                        // expansions must see it.
                        self.runner.run(format!(
                            "export AIR_GESTURES_ENABLED={}; {hook}",
                            u8::from(enabled)
                        ));
                    }
                }
                Output::Profile { index, .. } => {
                    let name = self.controls.profiles().list[index].name.clone();
                    tracing::info!(profile = %name, "profile");
                    let hook = self.config.band.on_profile.trim();
                    if !hook.is_empty() {
                        // Names are checked when the config loads: no quotes inside.
                        self.runner.run(format!("export AIR_GESTURES_PROFILE='{name}'; {hook}"));
                    }
                }
                Output::ProfileHeld { app } => tracing::debug!(%app, "profile held by the app in front"),
                Output::Practiced(_) => {}
            }
        }
    }

    /// The profiles again (the dial target or the config changed).
    fn rebuild_profiles(&mut self) {
        let outputs = self.controls.set_profiles(self.config.profiles(self.dial_target));
        self.apply(outputs, false);
    }
```

and a free function:

```rust
fn warn_profiles(controls: &Controls<String>) {
    use band_core::controls::Warning;
    for warning in controls.profiles().warnings() {
        let (profile, what) = match warning {
            Warning::NoControls { profile } => (profile, "no gesture turns the controls off/on (\"@controls\")"),
            Warning::NoWayOut { profile } => (profile, "no gesture switches away from it (\"@next-profile\" / \"@previous-profile\")"),
        };
        tracing::warn!("profile \"{}\": {what}", controls.profiles().list[profile].name);
    }
}
```

Replace every call site:
- **Held-tap timer** in `run`:
  ```rust
                  _ = until(self.controls.next_deadline(), self.clock) => {
                      let now = self.clock.now();
                      let outputs = self.controls.poll(now);
                      self.apply(outputs, true);
                  }
  ```
- **`on_link_event`:** `let outputs = self.controls.on_event(&event, now); self.apply(outputs, true);`
- **`on_disconnect`:** both call sites become `self.controls.on_disconnect()`.
- **`reload_config`, on `Ok(config)`:**
  ```rust
                  if config.dial.target != self.config.dial.target {
                      // A changed target in the file wins over a runtime choice.
                      self.dial_target = config.dial.target;
                  }
                  self.config = config;
                  let mut outputs = self.controls.set_options(self.config.options());
                  outputs.extend(self.controls.set_profiles(self.config.profiles(self.dial_target)));
                  warn_profiles(&self.controls);
                  self.apply(outputs, false);
  ```
- **IPC in `handle`:**
  ```rust
              Request::Pause => { let outputs = self.controls.pause(); self.apply(outputs, false); }
              Request::Resume => { let outputs = self.controls.resume(now); self.apply(outputs, false); }
              Request::TogglePause => { let outputs = self.controls.toggle(now); self.apply(outputs, false); }
              Request::Dial { target } => { self.dial_target = target; self.rebuild_profiles(); }
              Request::CycleDial => { self.dial_target = self.dial_target.next(); self.rebuild_profiles(); }
  ```
  `Request::Hand`: `let outputs = self.controls.request_hand(hand); self.apply(outputs, false);`
- **`publish`:** build the status from the daemon's own fields instead of cloning the controller's:
  ```rust
          let controls = self.controls.status();
          let mut status = Status {
              paused: controls.paused,
              battery: controls.battery,
              charging: controls.charging,
              hand: controls.hand.map(|hand| match hand {
                  band_core::events::Hand::Left => "left".to_owned(),
                  band_core::events::Hand::Right => "right".to_owned(),
              }),
              dial: self.dial_target,
              last_gesture: controls.last_gesture.map(str::to_owned),
              last_action: controls.last_action.clone(),
              gestures: controls.gestures,
              profile: (self.controls.profiles().list.len() > 1)
                  .then(|| self.controls.profiles().list[self.controls.active()].name.clone()),
              ..Status::default()
          };
  ```
  Then the existing lines that set `state`, `error` and the rest, unchanged.

Delete `air-gesturesd/src/controller.rs`, remove `pub mod controller;` from `air-gesturesd/src/lib.rs`, and delete `air-gesturesd/tests/controller.rs`. Its 24 tests live in band-core's `tests/controls_linux.rs`, where they pass.

Check `grep -rn "controller\|GestureConfig\|ToggleGesture::" air-gesturesd air-gestures`. No leftovers may remain except `ToggleGesture` in `config.rs`.

- [ ] **Step 4: Run everything**

Run: `cargo test --workspace 2>&1 | grep -E "^error|FAILED|test result" | sort | uniq -c`

Expected: no `error` and no `FAILED`, and the 3 new daemon tests pass. If an existing daemon test fails, compare with band-core's ported behaviour (`~/Projects/band-core/tests/controls_linux.rs`):
- if it only expected the old `"triple tap"` label or an `on_toggle` run from an IPC toggle, update the assertion and add a comment naming the design change;
- for anything else, stop and report.

- [ ] **Step 5: Commit**

```bash
git add -A air-gesturesd
git commit -q -m "air-gesturesd runs on band-core's shared controls

The daemon's own controller goes; gestures, the dial, controls on/off and profiles are
band-core's. on_toggle still runs only for band toggles; on_profile runs on every switch.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01MLDqGPYx8bTGKJcVLgoyzo"
```

---

### Task 4: `air-gestures profile next|previous|<name>`, and the profile in status

**Files:**
- Modify: `air-gesturesd/src/ipc.rs` (Request), `air-gesturesd/src/daemon.rs` (handle)
- Modify: `air-gestures/src/main.rs` (CLI), `air-gestures/src/text.rs`, `air-gestures/src/waybar.rs`
- Test: `air-gesturesd/tests/ipc.rs` and `air-gesturesd/tests/daemon.rs`, plus `air-gestures` unit tests for text/waybar if they exist (`grep -rn "fn .*waybar\|#\[test\]" air-gestures/src`)

- [ ] **Step 1: Failing tests**
1. **IPC round-trip**, in `tests/ipc.rs` next to the other request round-trips: `{"cmd":"profile","to":"next"}` ↔ `Request::Profile { to: "next".into() }`.
2. **Daemon test `profiles_switch_over_ipc`:**
   - config with `[[profile]] name = "games"` and `[[profile]] name = "media"`;
   - `Profile{to:"media"}` → status `profile == Some("media")`;
   - `next` → `Some("default")`, wrapping around;
   - `previous` → `Some("media")`;
   - `"nope"` → an error response naming the profiles.
3. **Text and waybar:** a status with `profile: Some("games")` shows a line `profile: games`. Add it to the text output after the dial line, and to the waybar tooltip after `dial:`.

- [ ] **Step 2: Implement**

`ipc.rs`, in `Request` (after `CycleDial`):

```rust
    /// Switch profiles: "next", "previous" or a profile's name.
    Profile {
        to: String,
    },
```

`daemon.rs`, in `handle`:

```rust
            Request::Profile { to } => {
                let count = self.controls.profiles().list.len();
                let active = self.controls.active();
                let index = match to.as_str() {
                    "next" => (active + 1) % count,
                    "previous" => (active + count - 1) % count,
                    name => match self.controls.profiles().list.iter().position(|p| p.name == name) {
                        Some(index) => index,
                        None => {
                            let names: Vec<&str> =
                                self.controls.profiles().list.iter().map(|p| p.name.as_str()).collect();
                            return Response::error(format!("no profile \"{name}\" (profiles: {})", names.join(", ")));
                        }
                    },
                };
                let outputs = self.controls.pick(index);
                self.apply(outputs, false);
            }
```

`air-gestures/src/main.rs`, in `enum Command` after `Dial`:

```rust
    /// Switch profiles: next, previous, or a profile's name.
    Profile { to: String },
```

and in the match: `Command::Profile { to } => simple(&socket, Request::Profile { to }),`

`text.rs`, after the dial line: `if let Some(profile) = &status.profile { lines.push(format!("profile: {profile}")); }`

`waybar.rs`, after the dial line: `if let Some(profile) = &status.profile { lines.push(format!("profile: {}", escape(profile))); }`

- [ ] **Step 3: Run and commit**

```bash
cargo test --workspace 2>&1 | grep -E "^error|FAILED" | wc -l       # 0
git add -A air-gesturesd air-gestures
git commit -q -m "air-gestures profile next|previous|<name>; status shows the profile

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01MLDqGPYx8bTGKJcVLgoyzo"
```

---

### Task 5: Docs and changelog

**Files:** `README.md`, `CHANGELOG.md`

- [ ] **Step 1: README**

Change the README's config section: grep for `config.toml` and the `[gestures]` description. It must say:
- the built-in values;
- `[[profile]]` with an example of two profiles, each with `index_triple = "@next-profile"`;
- `on_profile`;
- `air-gestures profile next|previous|<name>`;
- that `toggle` is the old way.

Keep it short; the README links to `contrib/config.toml` for every key.

- [ ] **Step 2: CHANGELOG**

Add at the top a section `## Unreleased` (the owner picks the version):

```markdown
## Unreleased

- Profiles: `[[profile]]` tables in config.toml, each with its own gestures. Switch them
  from the band with `@next-profile` / `@previous-profile` (by default on the index and
  middle triple taps), or with `air-gestures profile next|previous|<name>`. `on_profile`
  runs on every switch with `AIR_GESTURES_PROFILE` set.
- Turning the controls off and on is a built-in action, `@controls`, that can go on any
  gesture (by default the middle hold). The old `[band] toggle` still works.
- A slow triple tap no longer runs its finger's single tap first.
- Gestures, the dial and the controls now come from band-core's shared controls layer,
  the same code as the Android app.
- A band busy with another device says so instead of "Missing protobuf integer".
```

Note the default change in the same section:

```markdown
- New defaults where your config doesn't say: middle hold turns the controls off/on, and
  the triple taps switch profiles (they do nothing with one profile).
```

- [ ] **Step 3: Commit**

```bash
git add README.md CHANGELOG.md
git commit -q -m "Docs: profiles and built-in actions

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01MLDqGPYx8bTGKJcVLgoyzo"
```

---

### Task 6: On the band (with the owner)

The owner must be at the laptop, and the phone must not hold the band (Bluetooth off there).

- [ ] **Step 1: A test config, outside the owner's live one**

```bash
S=/tmp/claude-1000/-home-user888/0b28f152-ab6d-44d1-b86a-97f9f03c7086/scratchpad/linux-profiles
mkdir -p $S/xdg/air-gestures
cp ~/arch_rice/.config/air-gestures/config.toml $S/xdg/air-gestures/config.toml
cat >> $S/xdg/air-gestures/config.toml <<'EOF'

[[profile]]
name = "test"
swipe_left = "notify-send 'test profile: swipe left'"
index_triple = "@next-profile"
middle_triple = "@previous-profile"
swipe_down = "@controls"
EOF
```

The owner's config sets `toggle = "middle_hold"`, which binds `@controls` on the hold in both profiles. The test profile also puts it on swipe down.

- [ ] **Step 2: Run the built daemon against it for 5 minutes**

```bash
cd ~/Projects/air-gestures && cargo build --release -p air-gesturesd
systemctl --user stop air-gesturesd
XDG_CONFIG_HOME=$S/xdg AIR_GESTURES_LOG=air_gesturesd=debug,info timeout 300 target/release/air-gesturesd > $S/run.log 2>&1 &
```

The owner checks:
- the normal gestures;
- an index triple → the `test` profile (`air-gestures status` shows `profile: test`);
- swipe left → the notification;
- swipe down → controls off, and swipe down again → on;
- a middle triple → back to the first profile;
- `air-gestures profile test`, then `air-gestures profile next`.

- [ ] **Step 3: Read the log and put the released daemon back**

```bash
grep -E "profile|controls|action|WARN|ERROR" $S/run.log | cut -c1-160 | head -40
for p in $(pgrep -x air-gesturesd); do kill $p; done
systemctl --user start air-gesturesd && systemctl --user is-active air-gesturesd
```

Expected:
- the profile switches and control toggles appear in the log;
- no errors;
- the service is `active` again on the released binary.

Merging `controls` into `master`, pushing band-core first and then this repo, and releasing all wait for the owner.
