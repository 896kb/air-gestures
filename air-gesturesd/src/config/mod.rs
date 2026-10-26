//! `~/.config/air-gestures/config.toml`: the band controls (`[band]`), profiles of
//! gesture → shell command (`""` = nothing), and the dial.
//!
//! The profiles are built (and every mistake found) once, when the file is parsed.

mod band;
mod notes;
mod profile;

use std::path::{Path, PathBuf};

use band_core::controls::{Dial, Options, Profiles};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub use band::{BandConfig, DEFAULT_ON_DRAW, band_key};
pub use notes::{Level, Note, missing_drawings, notes};
pub use profile::{check_name, palm_section};

/// What a binding does.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Command(String),
}

impl Action {
    /// The shell command it runs.
    pub fn command(&self) -> &str {
        match self {
            Action::Command(command) => command,
        }
    }
}

impl From<&str> for Action {
    fn from(command: &str) -> Self {
        Action::Command(command.to_owned())
    }
}

impl From<String> for Action {
    fn from(command: String) -> Self {
        Action::Command(command)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HandSetting {
    /// Use whatever hand the band reports.
    #[default]
    Band,
    Left,
    Right,
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
    pub fn dial(&self, target: DialTarget) -> Option<Dial<Action>> {
        Some(Dial {
            up: self.command(target, true)?.into(),
            down: self.command(target, false)?.into(),
        })
    }
}

/// The file as written.
#[derive(Default)]
struct Raw {
    band: BandConfig,
    /// The first profile as written (unwritten gesture keys take the defaults).
    gestures: toml::Table,
    /// More profiles, in order (`name` plus the same keys; unwritten gesture keys do nothing).
    profile: Vec<toml::Table>,
    /// Drawing name → command in every profile without its own.
    drawings: toml::Table,
    dial: DialConfig,
}

/// `[band]`'s and `[dial]`'s keys (their structs' fields).
const BAND_KEYS: [&str; 14] = [
    "hand",
    "autostart",
    "toggle",
    "next_profile",
    "previous_profile",
    "draw",
    "toggle_palm",
    "next_profile_palm",
    "previous_profile_palm",
    "draw_palm",
    "on_toggle",
    "on_profile",
    "on_draw",
    "profile",
];
const DIAL_KEYS: [&str; 6] = ["target", "sensitivity", "volume_up", "volume_down", "brightness_up", "brightness_down"];

/// A `[name]` table into `T`, each key checked on its own so a mistake names its key.
fn section<T: DeserializeOwned>(name: &str, table: toml::Table, keys: &[&str]) -> Result<T, String> {
    for (key, value) in &table {
        if !keys.contains(&key.as_str()) {
            return Err(format!("[{name}] {key}: unknown key"));
        }
        let one = toml::Table::from_iter([(key.clone(), value.clone())]);
        toml::Value::Table(one).try_into::<T>().map_err(|error| format!("[{name}] {key}: {}", error.message()))?;
    }
    toml::Value::Table(table).try_into::<T>().map_err(|error| format!("[{name}]: {}", error.message()))
}

impl Raw {
    fn from_table(mut table: toml::Table) -> Result<Self, String> {
        let mut sub_table = |name: &str| match table.remove(name) {
            None => Ok(toml::Table::new()),
            Some(toml::Value::Table(sub)) => Ok(sub),
            Some(_) => Err(format!("{name}: expected a table ([{name}])")),
        };
        let band = sub_table("band")?;
        let dial = sub_table("dial")?;
        let gestures = sub_table("gestures")?;
        let drawings = sub_table("drawings")?;
        let profile = match table.remove("profile") {
            None => Vec::new(),
            Some(toml::Value::Array(items)) => items
                .into_iter()
                .enumerate()
                .map(|(i, item)| match item {
                    toml::Value::Table(profile) => Ok(profile),
                    _ => Err(format!("[[profile]] #{}: expected a table", i + 1)),
                })
                .collect::<Result<_, _>>()?,
            Some(_) => return Err("profile: expected [[profile]] tables".into()),
        };
        if let Some((key, value)) = table.iter().next() {
            return Err(match value.is_table() {
                true => format!("[{key}]: unknown table"),
                false => format!("{key}: unknown key (at the top of the file, before any [table])"),
            });
        }
        Ok(Self {
            band: section("band", band, &BAND_KEYS)?,
            gestures,
            profile,
            drawings,
            dial: section("dial", dial, &DIAL_KEYS)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub band: BandConfig,
    pub dial: DialConfig,
    profiles: Profiles<Action>,
    /// What each profile's index dial turns, in profile order.
    index_dials: Vec<DialTarget>,
}

impl Default for Config {
    fn default() -> Self {
        Self::from_raw(Raw::default()).expect("the defaults are a valid config")
    }
}

impl Config {
    pub fn parse(text: &str) -> Result<Self, String> {
        let table: toml::Table = toml::from_str(text).map_err(|error| error.to_string())?;
        Self::from_raw(Raw::from_table(table)?)
    }

    fn from_raw(raw: Raw) -> Result<Self, String> {
        if !(0.5..=4.0).contains(&raw.dial.sensitivity) {
            return Err(format!("[dial] sensitivity must be between 0.5 and 4 (got {})", raw.dial.sensitivity));
        }
        let (profiles, index_dials) = profile::build(&raw.band, &raw.dial, &raw.gestures, &raw.profile, &raw.drawings)?;
        Ok(Self { band: raw.band, dial: raw.dial, profiles, index_dials })
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
            ..Options::default()
        }
    }

    /// Every profile.
    pub fn profiles(&self) -> &Profiles<Action> {
        &self.profiles
    }

    /// What each profile's index dial turns (for status), in profile order.
    pub fn index_dials(&self) -> &[DialTarget] {
        &self.index_dials
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
