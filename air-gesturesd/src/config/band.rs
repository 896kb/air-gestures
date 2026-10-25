//! `[band]`: the hand, autostart, the hooks and the band controls.

use band_core::controls::{BandControl, BandControls, Builtin, Gesture, Palm, PalmSet, Palms};
use serde::Deserialize;

use super::HandSetting;

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
    /// Run after a drawing, with AIR_GESTURES_DRAWING=<name> exported (empty: nothing matched).
    pub on_draw: String,
    /// The name of the `[gestures]` profile (the first one).
    pub profile: String,
}

/// The default `on_draw`: the drawing's name (or "not recognised") as a notification.
pub const DEFAULT_ON_DRAW: &str = r#"notify-send -a air-gestures "Drawing" "${AIR_GESTURES_DRAWING:-not recognised}""#;

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
            on_draw: DEFAULT_ON_DRAW.into(),
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
pub(super) fn palm_suffix(palms: Palms) -> String {
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
        let palm = Palm::from_key(name).ok_or_else(|| {
            format!("[band] {key}_palm: unknown position \"{name}\" (\"down\", \"sideways\" or \"up\")")
        })?;
        if set.contains(&palm) {
            return Err(format!("[band] {key}_palm: \"{name}\" is listed twice"));
        }
        set.push(palm);
    }
    let set: Palms = set.into_iter().collect();
    if !(1..=PalmSet::MAX_ONLY).contains(&set.len()) {
        return Err(format!("[band] {key}_palm: list one or two positions (leave it out for all three)"));
    }
    Ok(PalmSet::Only(set))
}

impl BandConfig {
    /// One band control as written.
    fn band_control(&self, builtin: Builtin) -> Result<Option<BandControl>, String> {
        let key = band_key(builtin);
        let (value, palm) = match builtin {
            Builtin::Controls => (&self.toggle, self.toggle_palm.as_ref()),
            Builtin::NextProfile => (&self.next_profile, self.next_profile_palm.as_ref()),
            Builtin::PreviousProfile => (&self.previous_profile, self.previous_profile_palm.as_ref()),
            Builtin::Draw => (&self.draw, self.draw_palm.as_ref()),
        };
        match value.trim() {
            "none" | "" => match palm {
                Some(_) => Err(format!("[band] {key}_palm: set [band] {key} to a gesture first")),
                None => Ok(None),
            },
            name => Ok(Some(BandControl {
                gesture: Gesture::from_key(name).ok_or_else(|| {
                    format!("[band] {key}: unknown gesture \"{name}\" (a gesture key like \"middle_hold\", or \"none\")")
                })?,
                palm: palm_set(key, palm)?,
            })),
        }
    }

    /// The band controls; two on one gesture in one position is an error.
    pub fn band_controls(&self) -> Result<BandControls, String> {
        let band = BandControls {
            controls: self.band_control(Builtin::Controls)?,
            next: self.band_control(Builtin::NextProfile)?,
            previous: self.band_control(Builtin::PreviousProfile)?,
            draw: self.band_control(Builtin::Draw)?,
        };
        for (i, &a) in Builtin::ALL.iter().enumerate() {
            for &b in &Builtin::ALL[i + 1..] {
                let (Some(x), Some(y)) = (band.control(a), band.control(b)) else { continue };
                let shared: Palms = Palm::ALL.into_iter().filter(|&p| x.palm.contains(p) && y.palm.contains(p)).collect();
                if x.gesture == y.gesture && !shared.is_empty() {
                    return Err(format!(
                        "[band] {} and [band] {} are both on {}{}: move one to another gesture or palm position",
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
