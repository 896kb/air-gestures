//! Teaching: the words for what went wrong (shared with `air-gestures`), and (Task 8) the
//! teaching steps that wait for the band.

use band_core::controls::Drag;
use band_core::controls::palm::{CalibrationError, Palm};
use band_core::controls::pinch::TaughtError;

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
