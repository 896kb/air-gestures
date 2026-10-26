//! What the daemon logs about a loaded config.

use band_core::controls::{Finger, Invalid, Place, Profiles, Warning};

use super::Action;
use super::band::{band_key, palm_suffix};
use super::profile::{SLIDER_PAIRS, palm_section};

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

fn place_text(profiles: &Profiles<Action>, place: Place) -> String {
    match place {
        Place::Profile(index) => format!("profile \"{}\"", profiles.list[index].name),
        Place::Calls => "the call layer".into(),
    }
}

/// What to log on every config load: the core's warnings (no lock gesture is Linux's
/// default, so that one is info), what's invalid (errors), and every entry a band control
/// pauses (warnings).
pub fn notes(profiles: &Profiles<Action>) -> Vec<Note> {
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
            Finger::Index => "index",
            Finger::Middle => "middle",
        };
        let pair = SLIDER_PAIRS[slider.pair];
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

/// Drawings with an action whose template isn't taught (yet).
pub fn missing_drawings(profiles: &Profiles<Action>, taught: &[&str]) -> Vec<Note> {
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
