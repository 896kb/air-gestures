//! `~/.config/kinesis/config.toml`: every gesture maps to a shell command
//! (`""` = unassigned); the dial runs one of two commands per step.

use std::path::{Path, PathBuf};

use band_core::gestures::{Recognized, Swipe, Tap};
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
    /// The gesture that turns the controls off and on.
    pub toggle: ToggleGesture,
    /// Run after that gesture toggled the controls, with `KINESIS_ENABLED=1`
    /// or `0` in its environment (e.g. an OSD).
    pub on_toggle: String,
}

impl Default for BandConfig {
    fn default() -> Self {
        Self {
            hand: HandSetting::Band,
            autostart: true,
            toggle: ToggleGesture::None,
            on_toggle: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GestureConfig {
    pub swipe_left: String,
    pub swipe_right: String,
    pub swipe_up: String,
    pub swipe_down: String,
    pub index_tap: String,
    pub index_double: String,
    pub middle_tap: String,
    pub middle_double: String,
    pub middle_hold: String,
}

impl Default for GestureConfig {
    fn default() -> Self {
        Self {
            swipe_left: "hyprctl dispatch workspace e-1".into(),
            swipe_right: "hyprctl dispatch workspace e+1".into(),
            swipe_up: "wofi --show drun".into(),
            swipe_down: "hyprctl dispatch sendshortcut , Escape, activewindow".into(),
            index_tap: String::new(),
            index_double: "playerctl play-pause".into(),
            middle_tap: String::new(),
            middle_double: "wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle".into(),
            middle_hold: String::new(),
        }
    }
}

impl GestureConfig {
    /// The configured command, or `""` when it's unassigned (including whitespace-only).
    pub fn command(&self, gesture: Recognized) -> &str {
        let command = match gesture {
            Recognized::Swipe(Swipe::Left) => &self.swipe_left,
            Recognized::Swipe(Swipe::Right) => &self.swipe_right,
            Recognized::Swipe(Swipe::Up) => &self.swipe_up,
            Recognized::Swipe(Swipe::Down) => &self.swipe_down,
            Recognized::Tap(Tap::IndexTap) => &self.index_tap,
            Recognized::Tap(Tap::IndexDoubleTap) => &self.index_double,
            Recognized::Tap(Tap::MiddleTap) => &self.middle_tap,
            Recognized::Tap(Tap::MiddleDoubleTap) => &self.middle_double,
            Recognized::Tap(Tap::MiddleHold) => &self.middle_hold,
        };
        if command.trim().is_empty() {
            ""
        } else {
            command
        }
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
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub band: BandConfig,
    pub gestures: GestureConfig,
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
        Ok(config)
    }

    /// Load `path`; a missing file means the defaults.
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).map_err(|error| format!("{}: {error}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(format!("{}: {error}", path.display())),
        }
    }

    /// `$XDG_CONFIG_HOME/kinesis/config.toml`, else `~/.config/kinesis/config.toml`.
    pub fn default_path() -> PathBuf {
        let base = xdg("XDG_CONFIG_HOME").unwrap_or_else(|| home().join(".config"));
        base.join("kinesis").join("config.toml")
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
        assert_eq!(xdg("KINESISD_TEST_XDG_UNSET"), None);
        // SAFETY: these env vars are private to this test and touched only here.
        unsafe {
            std::env::set_var("KINESISD_TEST_XDG_EMPTY", "");
            std::env::set_var("KINESISD_TEST_XDG_RELATIVE", "relative/path");
            std::env::set_var("KINESISD_TEST_XDG_ABSOLUTE", "/abs/path");
        }
        assert_eq!(xdg("KINESISD_TEST_XDG_EMPTY"), None);
        assert_eq!(xdg("KINESISD_TEST_XDG_RELATIVE"), None);
        assert_eq!(
            xdg("KINESISD_TEST_XDG_ABSOLUTE"),
            Some(PathBuf::from("/abs/path"))
        );
        // SAFETY: cleanup of the same test-private env vars.
        unsafe {
            std::env::remove_var("KINESISD_TEST_XDG_EMPTY");
            std::env::remove_var("KINESISD_TEST_XDG_RELATIVE");
            std::env::remove_var("KINESISD_TEST_XDG_ABSOLUTE");
        }
    }
}
