//! What each gesture did, as one line for the log: "index tap · palm up → <command>" (from
//! band-core's `Output::Gesture`, what follows it, `Output::Unused`, `Output::PinchEnded` and
//! a drawing's `Output::Drawn`).

use band_core::controls::{Class, Finger, Output, Palm, Profiles, Unused};

use crate::config::{Action, DialConfig, band_key};
use crate::sliders::Run;

/// The longest a command is shown (in characters).
const SHOWN: usize = 60;

/// A command as the log shows it: the dial's own by its name ("volume up"), else the shell line,
/// cut to `SHOWN` characters.
pub fn short(command: &str, dial: &DialConfig) -> String {
    for (name, own) in [
        ("volume up", &dial.volume_up),
        ("volume down", &dial.volume_down),
        ("brightness up", &dial.brightness_up),
        ("brightness down", &dial.brightness_down),
    ] {
        if command == own {
            return name.to_owned();
        }
    }
    if command.chars().count() <= SHOWN {
        return command.to_owned();
    }
    let mut cut: String = command.chars().take(SHOWN - 1).collect();
    cut.push('…');
    cut
}

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
        Unused::Unbound => "nothing (nothing bound here)".into(),
        Unused::ControlsOff => "nothing (the controls are off)".into(),
        Unused::Paused(builtin) => {
            format!("nothing (its command is paused by [band] {}, which did nothing now)", band_key(builtin))
        }
        Unused::TooSoon => "nothing (too soon after another action)".into(),
        Unused::Teaching => "teaching drags".into(),
        Unused::Writing => "writing".into(),
        Unused::Drawing => "drawing".into(),
    }
}

/// "volume up ×2, volume down ×1": each command once, in the order first run.
fn counted(commands: &[String]) -> String {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for command in commands {
        match counts.iter_mut().find(|(c, _)| c == command) {
            Some((_, n)) => *n += 1,
            None => counts.push((command, 1)),
        }
    }
    counts.iter().map(|(c, n)| format!("{c} ×{n}")).collect::<Vec<_>>().join(", ")
}

/// Turns the controls' outputs into log lines, call after call: a dial's or slider's steps
/// come one at a time, and are named when their pinch ends.
#[derive(Debug, Default)]
pub struct Outcomes {
    /// The steps since the last stepping pinch ended (shown names).
    steps: Vec<String>,
    /// What each command slider ran since then (steps + and −), in the order first run; in
    /// practice, the steps it would have.
    sliders: Vec<(String, u32, u32)>,
}

impl Outcomes {
    /// A command slider ran (its steps are named when its pinch ends).
    pub fn slider_ran(&mut self, run: &Run) {
        self.slider_steps(&run.slider, run.steps);
    }

    fn slider_steps(&mut self, slider: &str, steps: i32) {
        let at = match self.sliders.iter().position(|(s, ..)| s == slider) {
            Some(at) => at,
            None => {
                self.sliders.push((slider.to_owned(), 0, 0));
                self.sliders.len() - 1
            }
        };
        let (_, up, down) = &mut self.sliders[at];
        if steps > 0 { *up += steps.unsigned_abs() } else { *down += steps.unsigned_abs() }
    }

    /// The pinch is gone without a `PinchEnded` (the band went): its steps aren't named.
    pub fn forget_pinch(&mut self) {
        self.steps.clear();
        self.sliders.clear();
    }

    /// One line per gesture and per drawing in `outputs` (and per stepping pinch that ended),
    /// in order.
    pub fn lines(&mut self, outputs: &[Output<Action>], profiles: &Profiles<Action>, dial: &DialConfig) -> Vec<String> {
        let mut lines = Vec::new();
        // What's being described ("index tap · palm up", "drawing \"z\""), and what it did.
        let mut current: Option<(String, Vec<String>)> = None;
        for output in outputs {
            let did = match output {
                Output::Gesture { gesture, palm } => {
                    flush(&mut current, &mut lines);
                    current = Some((format!("{} · {}", gesture.label(), palm_text(*palm)), Vec::new()));
                    continue;
                }
                Output::Drawn { name, .. } => {
                    flush(&mut current, &mut lines);
                    current = Some((format!("drawing \"{name}\""), Vec::new()));
                    continue;
                }
                Output::NotDrawn => {
                    flush(&mut current, &mut lines);
                    lines.push("drawing → not recognised".into());
                    continue;
                }
                Output::PinchEnded { finger, class, palm, practiced, .. } => {
                    flush(&mut current, &mut lines);
                    let what = match class {
                        Class::Turn => "dial".to_owned(),
                        Class::Drag(drag) => format!("slider {}", ["left/right", "toward/away", "up/down"][drag.index() / 2]),
                        Class::Unknown => "pinch".to_owned(),
                    };
                    let mut steps = vec![counted(&std::mem::take(&mut self.steps))];
                    steps.extend(self.sliders.drain(..).map(|(slider, up, down)| {
                        let mut text = format!("slider {slider}");
                        if up > 0 {
                            text += &format!(" +{up}");
                        }
                        if down > 0 {
                            text += &format!(" -{down}");
                        }
                        text
                    }));
                    let steps = steps.into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ");
                    let steps = match (steps.is_empty(), *practiced) {
                        (true, _) => "nothing".to_owned(),
                        (false, true) => format!("would: {steps}"),
                        (false, false) => steps,
                    };
                    lines.push(format!("{} {what} · {} → {steps}", finger_text(*finger), palm_text(*palm)));
                    continue;
                }
                // Counted as they run (`slider_ran`): gathered, or dropped.
                Output::Run(Action::Slider(_)) => continue,
                Output::Practiced(Action::Slider(step)) => {
                    self.slider_steps(&step.slider, step.step);
                    continue;
                }
                Output::Run(command) | Output::Practiced(command) => {
                    let shown = short(command.command(), dial);
                    if current.is_none() {
                        // A dial's or slider's step: named when its pinch ends.
                        self.steps.push(shown);
                        continue;
                    }
                    if matches!(output, Output::Practiced(_)) { format!("would: {shown}") } else { shown }
                }
                Output::Controls { enabled } => {
                    if !enabled {
                        // A pinch's end came first; anything left is from steps that were dropped.
                        self.forget_pinch();
                    }
                    format!("controls {}", if *enabled { "on" } else { "off" })
                }
                Output::Profile { index, .. } => {
                    format!("profile \"{}\"", profiles.list.get(*index).map_or("?", |p| p.name.as_str()))
                }
                Output::ProfileHeld { app } => format!("profile held by {app}"),
                Output::DrawStarted => "open a drawing".into(),
                Output::DrawCancelled => "drawing cancelled".into(),
                Output::Unused { why, .. } => unused_text(*why),
                _ => continue,
            };
            if let Some((_, done)) = &mut current {
                done.push(did);
            }
        }
        flush(&mut current, &mut lines);
        lines
    }
}

/// The line for what's being described, if anything.
fn flush(current: &mut Option<(String, Vec<String>)>, lines: &mut Vec<String>) {
    if let Some((what, did)) = current.take() {
        let did = if did.is_empty() { "nothing".to_owned() } else { did.join(", ") };
        lines.push(format!("{what} → {did}"));
    }
}
