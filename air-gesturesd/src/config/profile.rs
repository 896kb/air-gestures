//! Profile tables (`[gestures]`, each `[[profile]]`): gesture → command, the per-finger
//! keys, palm overrides (`.palm_down`, `.sideways`, `.palm_up`) and drawings' actions.

use std::collections::BTreeMap;

use band_core::controls::{BandControls, Binding, Builtin, DragMode, FingerSetup, Gesture, Palm, Profile, Profiles};

use super::band::{BandConfig, band_key};
use super::{DialConfig, DialTarget};

/// The slider keys' pair names, in `DragMode::Sliders` order.
pub(super) const SLIDER_PAIRS: [&str; 3] = ["left_right", "toward_away", "up_down"];

/// The sub-table a palm override is written in.
pub fn palm_section(palm: Palm) -> &'static str {
    match palm {
        Palm::Down => "palm_down",
        Palm::Side => "sideways",
        Palm::Up => "palm_up",
    }
}

/// `[gestures]`, `[gestures.palm_up]`, `[profile "games"]`, `[profile "games".drawings]`.
fn section(base: &str, sub: Option<&str>) -> String {
    match sub {
        None => format!("[{base}]"),
        Some(sub) => format!("[{base}.{sub}]"),
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

/// What a command value is on, for the hint about the first build's `@…` values.
#[derive(Clone, Copy)]
enum On {
    /// A gesture's action (in this palm position's sub-table, if any).
    Gesture(Option<Palm>),
    Drawing,
}

/// A command value at `key` in `at`: blank = nothing. `@…` was the first build's built-in syntax.
fn command(at: &str, key: &str, value: &str, on: On) -> Result<Option<String>, String> {
    let trimmed = value.trim();
    if let Some(name) = trimmed.strip_prefix('@') {
        let start = format!("{at} {key} = \"{trimmed}\"");
        return Err(match (on, Builtin::from_key(name)) {
            (On::Drawing, _) => format!(
                "{start}: a drawing runs a command; band controls are set in [band] (toggle, next_profile, previous_profile, draw)"
            ),
            (On::Gesture(palm), Some(builtin)) => {
                let band = band_key(builtin);
                let palm = palm.map_or(String::new(), |palm| format!(", {band}_palm = [\"{}\"]", palm.key()));
                format!("{start}: band controls are set in [band] now: {band} = \"{key}\"{palm}")
            }
            (On::Gesture(_), None) => format!(
                "{start}: band controls are set in [band] now (toggle, next_profile, previous_profile or draw = \"{key}\")"
            ),
        });
    }
    Ok((!trimmed.is_empty()).then(|| value.to_owned()))
}

fn text<'a>(at: &str, key: &str, value: &'a toml::Value) -> Result<&'a str, String> {
    value.as_str().ok_or_else(|| format!("{at} {key}: expected a string"))
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

/// A per-finger key's kind.
#[derive(Clone, Copy)]
enum FingerKey {
    Dial,
    Drags,
    Slider(usize),
}

/// `index_dial`, `middle_drags`, `index_slider_up_down`, …: the finger slot (0 index,
/// 1 middle) and the kind; `None` for any other key.
fn finger_key(key: &str) -> Option<(usize, FingerKey)> {
    let (slot, rest) = match (key.strip_prefix("index_"), key.strip_prefix("middle_")) {
        (Some(rest), _) => (0, rest),
        (_, Some(rest)) => (1, rest),
        _ => return None,
    };
    let kind = match rest {
        "dial" => FingerKey::Dial,
        "drags" => FingerKey::Drags,
        _ => FingerKey::Slider(SLIDER_PAIRS.iter().position(|pair| rest.strip_prefix("slider_") == Some(*pair))?),
    };
    Some((slot, kind))
}

/// A profile's per-finger keys.
struct Fingers {
    dial: [Option<DialTarget>; 2],
    sliders_mode: [bool; 2],
    sliders: [[DialTarget; 3]; 2],
}

impl Default for Fingers {
    fn default() -> Self {
        Self { dial: [None; 2], sliders_mode: [false; 2], sliders: [[DialTarget::Off; 3]; 2] }
    }
}

fn parse_target(at: &str, key: &str, value: &str) -> Result<DialTarget, String> {
    match value.trim() {
        "volume" => Ok(DialTarget::Volume),
        "brightness" => Ok(DialTarget::Brightness),
        "off" => Ok(DialTarget::Off),
        other => Err(format!("{at} {key}: \"{other}\" isn't a dial (\"volume\", \"brightness\" or \"off\")")),
    }
}

impl Fingers {
    fn set(&mut self, at: &str, key: &str, (slot, kind): (usize, FingerKey), value: &str) -> Result<(), String> {
        match kind {
            FingerKey::Dial => self.dial[slot] = Some(parse_target(at, key, value)?),
            FingerKey::Drags => {
                self.sliders_mode[slot] = match value.trim() {
                    "gestures" => false,
                    "sliders" => true,
                    other => return Err(format!("{at} {key}: \"{other}\" isn't a drag mode (\"gestures\" or \"sliders\")")),
                }
            }
            FingerKey::Slider(pair) => self.sliders[slot][pair] = parse_target(at, key, value)?,
        }
        Ok(())
    }

    /// Both fingers' setups, and what the index dial turns.
    fn setups(&self, dial: &DialConfig) -> (FingerSetup<String>, FingerSetup<String>, DialTarget) {
        let index = self.dial[0].unwrap_or(dial.target);
        let middle = self.dial[1].unwrap_or(DialTarget::Off);
        let setup = |slot: usize, target: DialTarget| FingerSetup {
            dial: dial.dial(target),
            drags: if self.sliders_mode[slot] {
                DragMode::Sliders(self.sliders[slot].map(|t| dial.dial(t)))
            } else {
                DragMode::Gestures
            },
        };
        (setup(0, index), setup(1, middle), index)
    }
}

fn drawing_actions(at: &str, table: &toml::Table) -> Result<BTreeMap<String, String>, String> {
    let mut actions = BTreeMap::new();
    for (name, value) in table {
        check_name("drawing", name).map_err(|e| format!("{at} {e}"))?;
        if let Some(command) = command(at, name, text(at, name, value)?, On::Drawing)? {
            actions.insert(name.clone(), command);
        }
    }
    Ok(actions)
}

/// A sub-table key of a profile table: a palm position's, or `drawings`.
fn is_sub_table(key: &str) -> bool {
    key == "drawings" || Palm::ALL.into_iter().any(|palm| palm_section(palm) == key)
}

/// One profile from its table (`base` names it in errors: `gestures` or `profile "<name>"`).
/// `first` starts from the defaults. Also returns what its index dial turns.
fn profile_from(dial: &DialConfig, name: &str, table: &toml::Table, base: &str, first: bool) -> Result<(Profile<String>, DialTarget), String> {
    let at = section(base, None);
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
        if key == "name" && !first {
            continue;
        }
        if is_sub_table(key) {
            if !value.is_table() {
                let of = if key == "drawings" { "drawing" } else { "gesture" };
                return Err(format!("{at} {key}: expected a table of {of} = command"));
            }
            continue;
        }
        let finger = finger_key(key);
        let gesture = Gesture::from_key(key);
        if finger.is_none() && gesture.is_none() {
            return Err(match value.is_table() {
                true => format!("{at} {key}: unknown table (palm_down, sideways, palm_up or drawings)"),
                false => format!("{at} {key}: unknown key"),
            });
        }
        let value = text(&at, key, value)?;
        match (finger, gesture) {
            (Some(finger), _) => fingers.set(&at, key, finger, value)?,
            (None, Some(gesture)) => {
                match command(&at, key, value, On::Gesture(None))? {
                    Some(command) => profile.gestures.insert(gesture, Binding::from(command)),
                    None => profile.gestures.remove(&gesture),
                };
            }
            (None, None) => unreachable!("checked above"),
        }
    }
    for palm in Palm::ALL {
        let sub_name = palm_section(palm);
        let Some(sub) = table.get(sub_name).and_then(toml::Value::as_table) else { continue };
        let sub_at = section(base, Some(sub_name));
        for (key, value) in sub {
            let Some(gesture) = Gesture::from_key(key) else {
                return Err(match finger_key(key) {
                    Some((_, FingerKey::Dial)) => format!("{sub_at} {key}: dials don't take palm overrides"),
                    Some(_) => format!("{sub_at} {key}: drag modes and sliders don't take palm overrides"),
                    None if value.is_table() => format!("{sub_at} {key}: unknown table"),
                    None => format!("{sub_at} {key}: unknown key"),
                });
            };
            let action = command(&sub_at, key, text(&sub_at, key, value)?, On::Gesture(Some(palm)))?;
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
            "{at} {}: palm overrides in palm_down, sideways and palm_up; at most 2 (the normal action covers the third position)",
            gesture.key()
        ));
    }
    // "" in a palm table over no normal action: nothing anywhere.
    profile.gestures.retain(|_, binding| binding.normal.is_some() || binding.overrides.values().any(Option::is_some));
    if let Some(sub) = table.get("drawings").and_then(toml::Value::as_table) {
        profile.drawings = drawing_actions(&section(base, Some("drawings")), sub)?;
    }
    let (index, middle, target) = fingers.setups(dial);
    profile.index = index;
    profile.middle = middle;
    Ok((profile, target))
}

/// Every profile, and what each one's index dial turns.
pub(super) fn build(
    band: &BandConfig,
    dial: &DialConfig,
    gestures: &toml::Table,
    tables: &[toml::Table],
    drawings: &toml::Table,
) -> Result<(Profiles<String>, Vec<DialTarget>), String> {
    let controls: BandControls = band.band_controls()?;
    check_name("profile", &band.profile).map_err(|e| format!("[band] profile: {e}"))?;
    let (first, target) = profile_from(dial, &band.profile, gestures, "gestures", true)?;
    let mut dials = vec![target];
    let mut list = vec![first];
    for (i, table) in tables.iter().enumerate() {
        let at = format!("[[profile]] #{}", i + 1);
        let name = match table.get("name") {
            None => return Err(format!("{at}: every [[profile]] needs a name")),
            Some(name) => name.as_str().ok_or_else(|| format!("{at} name: a profile's name must be text"))?,
        };
        check_name("profile", name).map_err(|e| format!("{at} name: {e}"))?;
        if list.iter().any(|p: &Profile<String>| p.name == name) {
            return Err(format!("{at} name: two profiles are called \"{name}\""));
        }
        let (profile, target) = profile_from(dial, name, table, &format!("profile \"{name}\""), false)?;
        dials.push(target);
        list.push(profile);
    }
    let profiles = Profiles {
        band: controls,
        list,
        picked: 0,
        apps: BTreeMap::new(),
        calls: BTreeMap::new(),
        drawings: drawing_actions("[drawings]", drawings)?,
    };
    Ok((profiles, dials))
}
