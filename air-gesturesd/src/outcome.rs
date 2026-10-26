//! What each gesture did, as one line for the log: "index tap · palm up → <command>" (from
//! band-core's `Output::Gesture`, what follows it, `Output::Unused` and `Output::PinchEnded`).

use band_core::controls::{Class, Finger, Output, Palm, Profiles, Unused};

use crate::config::band_key;

fn palm_text(palm: Palm) -> String {
    format!("palm {}", palm.key())
}

fn finger_text(finger: Finger) -> &'static str {
    match finger {
        Finger::Index => "index",
        Finger::Middle => "middle",
    }
}

fn unused_text(why: Unused) -> String {
    match why {
        Unused::Unbound => "nothing bound here".into(),
        Unused::ControlsOff => "the controls are off".into(),
        Unused::Paused(builtin) => format!("its command is paused by [band] {}, which did nothing now", band_key(builtin)),
        Unused::TooSoon => "too soon after another action".into(),
    }
}

/// The line for the gesture being described, if any.
fn flush(current: &mut Option<(String, Vec<String>)>, lines: &mut Vec<String>) {
    if let Some((gesture, did)) = current.take() {
        let did = if did.is_empty() { "nothing".to_owned() } else { did.join(", ") };
        lines.push(format!("{gesture} → {did}"));
    }
}

/// One line per gesture in `outputs` (and per stepping pinch that ended), in order. Outputs
/// that don't follow a gesture (a dial's steps, IPC changes) aren't lines of their own.
pub fn describe(outputs: &[Output<String>], profiles: &Profiles<String>) -> Vec<String> {
    let mut lines = Vec::new();
    // The gesture being described, and what it did so far.
    let mut current: Option<(String, Vec<String>)> = None;
    for output in outputs {
        let did = match output {
            Output::Gesture { gesture, palm } => {
                flush(&mut current, &mut lines);
                current = Some((format!("{} · {}", gesture.label(), palm_text(*palm)), Vec::new()));
                continue;
            }
            Output::PinchEnded { finger, class, palm, up, down } => {
                flush(&mut current, &mut lines);
                let what = match class {
                    Class::Turn => "dial".to_owned(),
                    Class::Drag(drag) => format!("slider {}", ["left/right", "toward/away", "up/down"][drag.index() / 2]),
                    Class::Unknown => "pinch".to_owned(),
                };
                lines.push(format!("{} {what} · {} → {up} up, {down} down", finger_text(*finger), palm_text(*palm)));
                continue;
            }
            Output::Run(command) => command.clone(),
            Output::Practiced(command) => format!("practice: {command}"),
            Output::Controls { enabled } => format!("controls {}", if *enabled { "on" } else { "off" }),
            Output::Profile { index, .. } => {
                format!("profile \"{}\"", profiles.list.get(*index).map_or("?", |p| p.name.as_str()))
            }
            Output::ProfileHeld { app } => format!("profile held by {app}"),
            Output::DrawStarted => "drawing".into(),
            Output::DrawCancelled => "drawing cancelled".into(),
            Output::Unused { why, .. } => format!("nothing ({})", unused_text(*why)),
            _ => continue,
        };
        if let Some((_, done)) = &mut current {
            done.push(did);
        }
    }
    flush(&mut current, &mut lines);
    lines
}
