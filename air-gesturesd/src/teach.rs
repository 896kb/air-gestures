//! Teaching: the words for what went wrong (shared with `air-gestures`), and the teaching
//! steps that wait for the band.

use band_core::controls::Drag;
use band_core::controls::palm::{CalibrationError, Palm};
use band_core::controls::pinch::{TaughtError, roll_or, up_in_band};
use band_core::controls::{Controls, Finger, Output};
use band_core::events::Event;

use crate::ipc::Taught;

pub fn drag_name(drag: Drag) -> &'static str {
    match drag {
        Drag::Left => "left",
        Drag::Right => "right",
        Drag::Toward => "toward",
        Drag::Away => "away",
        Drag::Up => "up",
        Drag::Down => "down",
    }
}

fn palm_name(palm: Palm) -> &'static str {
    match palm {
        Palm::Down => "palm down",
        Palm::Side => "palm sideways",
        Palm::Up => "palm up",
    }
}

pub fn taught_error_text(error: TaughtError) -> String {
    match error {
        TaughtError::NoTurn => "the turn didn't move: pinch and turn your wrist".into(),
        TaughtError::NoDrag(drag) => format!("the {} drag didn't move", drag_name(drag)),
        TaughtError::TooAlike(a, b) => format!("{} and {} came out alike: make them clearly different", drag_name(a), drag_name(b)),
        TaughtError::LikeTheTurn(drag) => format!("{} came out like the turn: move your hand without turning your wrist", drag_name(drag)),
        TaughtError::NoRoll => "the palm-down roll they were taught in isn't a number".into(),
    }
}

pub fn calibration_error_text(error: CalibrationError) -> String {
    match error {
        CalibrationError::NotStill(palm) => format!("you moved while holding {}: once more, and hold still", palm_name(palm)),
        CalibrationError::Empty(palm) => format!("couldn't read {}: keep your forearm level (not pointing up or down)", palm_name(palm)),
        CalibrationError::TooClose => "palm down and palm up came out less than 60° apart: turn your hand all the way over".into(),
    }
}

/// A teaching step that waits for the band.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// The band's roll for a while (palm calibration); nothing changes in the controls.
    Rolls,
    /// Drag teaching until the next pinch that moves.
    Drag,
    /// One drawing, opened now, taught (no match, no action).
    Draw,
}

impl Step {
    /// The shortest and longest a step may wait (seconds).
    pub fn limits(self) -> (f64, f64) {
        match self {
            Step::Rolls => (0.5, 5.0),
            Step::Drag => (1.0, 60.0),
            Step::Draw => (1.0, 120.0),
        }
    }

    /// Teaching on in `controls` for this step.
    pub fn enter(self, controls: &mut Controls<String>, now: f64) -> Vec<Output<String>> {
        match self {
            Step::Rolls => Vec::new(),
            Step::Drag => controls.set_drag_teaching(true),
            Step::Draw => {
                let mut outputs = controls.cancel_drawing();
                controls.set_teaching(true);
                outputs.extend(controls.start_drawing(now));
                outputs
            }
        }
    }

    /// Teaching off again (whatever ended the step).
    pub fn leave(self, controls: &mut Controls<String>) -> Vec<Output<String>> {
        match self {
            Step::Rolls => Vec::new(),
            Step::Drag => controls.set_drag_teaching(false),
            Step::Draw => {
                controls.set_teaching(false);
                controls.cancel_drawing()
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Done {
    Taught(Taught),
    /// This try didn't work (nothing drawn, no pinch in time): the flow tries it again.
    Retry(String),
    /// The step can't go on (the band went, the client went).
    Failed(String),
}

/// A teaching step waiting for the band.
#[derive(Debug)]
pub struct Wait {
    step: Step,
    until: f64,
    seconds: f64,
    rolls: Vec<f64>,
    samples: usize,
}

impl Wait {
    pub fn new(step: Step, now: f64, seconds: f64) -> Self {
        Self { step, until: now + seconds, seconds, rolls: Vec::new(), samples: 0 }
    }

    pub fn step(&self) -> Step {
        self.step
    }

    pub fn deadline(&self) -> f64 {
        self.until
    }

    /// A band event (rolls: each orientation's roll about the forearm, as the controls read it).
    pub fn on_event(&mut self, event: &Event) {
        if let (Step::Rolls, Event::Orientation(q)) = (self.step, event) {
            self.samples += 1;
            let roll = roll_or(up_in_band(*q), f64::NAN);
            if roll.is_finite() {
                self.rolls.push(roll);
            }
        }
    }

    /// An output of the controls: done when it's this step's.
    pub fn on_output(&mut self, output: &Output<String>) -> Option<Done> {
        match (self.step, output) {
            (Step::Drag, Output::DragSample { finger, direction }) => {
                let finger = match finger {
                    Finger::Index => "index",
                    Finger::Middle => "middle",
                };
                Some(Done::Taught(Taught::Drag { finger: finger.into(), direction: *direction }))
            }
            (Step::Draw, Output::DrawFinished { strokes }) if !strokes.is_empty() => {
                Some(Done::Taught(Taught::Strokes { strokes: strokes.clone() }))
            }
            (Step::Draw, Output::DrawFinished { .. } | Output::DrawCancelled) => {
                Some(Done::Retry("nothing was drawn (pinch to put the pen down; a quick tap isn't a stroke)".into()))
            }
            _ => None,
        }
    }

    /// The deadline passed: the rolls so far, else what didn't happen.
    pub fn timed_out(self) -> Done {
        match self.step {
            Step::Rolls => Done::Taught(Taught::Rolls { rolls: self.rolls, samples: self.samples }),
            Step::Drag => Done::Retry(format!("no pinch that moved in {} s", self.seconds)),
            Step::Draw => Done::Retry(format!("the drawing didn't end within {} s", self.seconds)),
        }
    }
}
