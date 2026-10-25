//! `~/.config/air-gestures/config.toml`: the band controls (`[band]`), profiles of
//! gesture → shell command (`""` = nothing), and the dial.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use band_core::controls::{
    BandControl, BandControls, Binding, Builtin, Dial, DragMode, FingerSetup, Gesture, Invalid, Options, Palm, PalmSet,
    Palms, Place, Profile, Profiles, Warning,
};
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

fn text<'a>(place: &str, key: &str, value: &'a toml::Value) -> Result<&'a str, String> {
    value.as_str().ok_or_else(|| format!("{place}.{key}: expected a string"))
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub band: BandConfig,
    /// The first profile as written (unwritten gesture keys take `default_gesture`).
    gestures: toml::Table,
    /// More profiles, in order (`name` plus the same keys; unwritten gesture keys do nothing).
    profile: Vec<toml::Table>,
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
        config.build()?;
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
            ..Options::default()
        }
    }

    /// Every profile.
    pub fn profiles(&self) -> Profiles<String> {
        self.build().expect("checked when the config was parsed").0
    }

    /// What each profile's index dial turns (for status), in profile order.
    pub fn index_dials(&self) -> Vec<DialTarget> {
        self.build().expect("checked when the config was parsed").1
    }

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

    fn build(&self) -> Result<(Profiles<String>, Vec<DialTarget>), String> {
        let band = self.band.band_controls()?;
        check_name("profile", &self.band.profile).map_err(|e| format!("band.profile: {e}"))?;
        let (first, target) = self.profile_from(&self.band.profile, &self.gestures, "gestures", true)?;
        let mut dials = vec![target];
        let mut list = vec![first];
        for table in &self.profile {
            let name = table.get("name").and_then(toml::Value::as_str).ok_or("every [[profile]] needs a name")?;
            check_name("profile", name)?;
            if list.iter().any(|p: &Profile<String>| p.name == name) {
                return Err(format!("two profiles are called \"{name}\""));
            }
            let place = format!("profile \"{name}\"");
            let (profile, target) = self.profile_from(name, table, &place, false)?;
            dials.push(target);
            list.push(profile);
        }
        let profiles = Profiles { band, list, picked: 0, apps: BTreeMap::new(), calls: BTreeMap::new(), drawings: BTreeMap::new() };
        Ok((profiles, dials))
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
