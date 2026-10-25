//! `~/.config/air-gestures/config.toml`: profiles of gesture → shell command (or a built-in:
//! `@controls`, `@next-profile`, `@previous-profile`; `""` = unassigned), and the dial.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use band_core::controls::{Action, Builtin, Dial, Gesture, Options, Profile, Profiles};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HandSetting {
    /// Use whatever hand the band reports.
    #[default]
    Band,
    Left,
    Right,
}

/// The gesture that turns the controls off and on (it works while they are off).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToggleGesture {
    #[default]
    None,
    /// The band's own middle-finger hold (long press).
    MiddleHold,
    /// Three taps, each within 0.45 s of the last; that finger's double tap
    /// then waits 0.45 s for a third. Fast tapping only.
    IndexTriple,
    MiddleTriple,
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DialTarget {
    #[default]
    Volume,
    Brightness,
    Off,
}

impl DialTarget {
    pub fn next(self) -> Self {
        match self {
            DialTarget::Volume => DialTarget::Brightness,
            DialTarget::Brightness | DialTarget::Off => DialTarget::Volume,
        }
    }
}

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
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-'));
    if fine {
        Ok(())
    } else {
        Err(format!(
            "profile name \"{name}\": use letters, digits, spaces, '.', '_' or '-'"
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DialConfig {
    pub target: DialTarget,
    /// 1.0 = one step per 2° of relative wrist rotation; 0.5–4.
    pub sensitivity: f64,
    pub volume_up: String,
    pub volume_down: String,
    pub brightness_up: String,
    pub brightness_down: String,
}

impl Default for DialConfig {
    fn default() -> Self {
        Self {
            target: DialTarget::Volume,
            sensitivity: 1.0,
            volume_up: "wpctl set-volume -l 1.0 @DEFAULT_AUDIO_SINK@ 2%+".into(),
            volume_down: "wpctl set-volume @DEFAULT_AUDIO_SINK@ 2%-".into(),
            brightness_up: "brightnessctl -q set 2%+".into(),
            brightness_down: "brightnessctl -q set 2%-".into(),
        }
    }
}

impl DialConfig {
    /// The command for one step, or `None` when the target is off or the
    /// command is unassigned (including whitespace-only).
    pub fn command(&self, target: DialTarget, up: bool) -> Option<&str> {
        let command = match (target, up) {
            (DialTarget::Off, _) => return None,
            (DialTarget::Volume, true) => &self.volume_up,
            (DialTarget::Volume, false) => &self.volume_down,
            (DialTarget::Brightness, true) => &self.brightness_up,
            (DialTarget::Brightness, false) => &self.brightness_down,
        };
        (!command.trim().is_empty()).then_some(command.as_str())
    }

    /// Both commands for this target, or `None` when it's off or either is unassigned.
    pub fn dial(&self, target: DialTarget) -> Option<Dial<String>> {
        Some(Dial {
            up: self.command(target, true)?.to_owned(),
            down: self.command(target, false)?.to_owned(),
        })
    }
}

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
        self.build(target)
            .expect("checked when the config was parsed")
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
            if let Some(action) =
                action(value).map_err(|e| format!("gestures.{}: {e}", gesture.key()))?
            {
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
                if let Some(action) =
                    action(value).map_err(|e| format!("profile \"{name}\".{key}: {e}"))?
                {
                    profile.gestures.insert(gesture, action);
                }
            }
            if let Some(gesture) = lock {
                profile
                    .gestures
                    .entry(gesture)
                    .or_insert(Action::Builtin(Builtin::Controls));
            }
            profile.dial = dial.clone();
            list.push(profile);
        }
        Ok(Profiles {
            list,
            picked: 0,
            apps: BTreeMap::new(),
            calls: BTreeMap::new(),
        })
    }

    /// Load `path`; a missing file means the defaults.
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).map_err(|error| format!("{}: {error}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(format!("{}: {error}", path.display())),
        }
    }

    /// `$XDG_CONFIG_HOME/air-gestures/config.toml`, else `~/.config/air-gestures/config.toml`.
    pub fn default_path() -> PathBuf {
        let base = xdg("XDG_CONFIG_HOME").unwrap_or_else(|| home().join(".config"));
        base.join("air-gestures").join("config.toml")
    }
}

pub(crate) fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// `$name`, but only when it's set, non-empty and absolute — the XDG base
/// directory spec says relative values must be ignored.
pub(crate) fn xdg(name: &str) -> Option<PathBuf> {
    let value = std::env::var_os(name)?;
    if value.is_empty() {
        return None;
    }
    let path = PathBuf::from(value);
    path.is_absolute().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xdg_ignores_unset_empty_and_relative_values() {
        assert_eq!(xdg("AIR_GESTURESD_TEST_XDG_UNSET"), None);
        // SAFETY: these env vars are private to this test and touched only here.
        unsafe {
            std::env::set_var("AIR_GESTURESD_TEST_XDG_EMPTY", "");
            std::env::set_var("AIR_GESTURESD_TEST_XDG_RELATIVE", "relative/path");
            std::env::set_var("AIR_GESTURESD_TEST_XDG_ABSOLUTE", "/abs/path");
        }
        assert_eq!(xdg("AIR_GESTURESD_TEST_XDG_EMPTY"), None);
        assert_eq!(xdg("AIR_GESTURESD_TEST_XDG_RELATIVE"), None);
        assert_eq!(
            xdg("AIR_GESTURESD_TEST_XDG_ABSOLUTE"),
            Some(PathBuf::from("/abs/path"))
        );
        // SAFETY: cleanup of the same test-private env vars.
        unsafe {
            std::env::remove_var("AIR_GESTURESD_TEST_XDG_EMPTY");
            std::env::remove_var("AIR_GESTURESD_TEST_XDG_RELATIVE");
            std::env::remove_var("AIR_GESTURESD_TEST_XDG_ABSOLUTE");
        }
    }
}
