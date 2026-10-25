//! The guided teaching flows: palm calibration, teach your drags, teach a drawing. They talk
//! to air-gesturesd's teaching steps and check the results with band-core before storing them.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use air_gesturesd::ipc::{Request, Taught, TaughtDrags};
use air_gesturesd::status::{LinkState, Status};
use air_gesturesd::store::StoredPalm;
use air_gesturesd::teach::{calibration_error_text, taught_error_text};
use band_core::controls::palm::{CalibrationError, Palm, PalmCalibration};
use band_core::controls::pinch::{Prototypes, TaughtError};

use crate::client;

/// What a flow says and asks (the terminal, or a test).
pub trait Guide {
    fn say(&mut self, text: &str);
    /// A desktop notification (the user is watching their hand, not the terminal).
    fn notify(&mut self, title: &str, body: &str);
    /// Show `prompt`, wait for Enter; `false` on end of input.
    fn ready(&mut self, prompt: &str) -> bool;
}

pub struct Terminal;

impl Guide for Terminal {
    fn say(&mut self, text: &str) {
        println!("{text}");
    }
    fn notify(&mut self, title: &str, body: &str) {
        // Best effort: without a notification daemon the terminal still says it.
        let _ = std::process::Command::new("notify-send")
            .args(["-a", "air-gestures", title, body])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    fn ready(&mut self, prompt: &str) -> bool {
        use std::io::Write;
        print!("{prompt} ");
        let _ = std::io::stdout().flush();
        std::io::stdin().read_line(&mut String::new()).is_ok_and(|read| read > 0)
    }
}

const ROUNDS: usize = 3;
const HOLD: f64 = 2.0;

fn unexpected() -> String {
    "unexpected answer from air-gesturesd".into()
}

/// The band's hand, once it's connected and has said.
pub fn connected_hand(status: &Status) -> Result<&str, String> {
    if status.state != LinkState::Connected {
        return Err("connect the band first (`air-gestures connect`)".into());
    }
    status.hand.as_deref().ok_or_else(|| "the band hasn't said which wrist it's on yet; try again in a moment".into())
}

async fn hold(socket: &Path, guide: &mut impl Guide, palm: Palm) -> Result<Vec<f64>, String> {
    let (title, way) = match palm {
        Palm::Up => ("Palm up", "palm up"),
        _ => ("Palm down", "palm down"),
    };
    if !guide.ready(&format!("Forearm level in front of you, {way}. Press Enter, then hold still for 2 s.")) {
        return Err("cancelled; nothing was changed".into());
    }
    guide.notify(title, "Hold still for 2 seconds");
    let response = client::request_within(socket, &Request::PalmRolls { seconds: HOLD }, Duration::from_secs(10)).await?;
    match response.teach {
        Some(Taught::Rolls { samples: 0, .. }) => {
            Err("the band sent no orientation: is it on your wrist and connected? (`air-gestures status`)".into())
        }
        Some(Taught::Rolls { rolls, .. }) => Ok(rolls),
        _ => Err(unexpected()),
    }
}

/// The palm position the band reads now.
async fn palm_now(socket: &Path) -> Result<String, String> {
    match client::request(socket, &Request::Palm).await?.teach {
        Some(Taught::Palm { palm }) => Ok(palm),
        _ => Err(unexpected()),
    }
}

/// Palm up once more: does the band read it that way now? Only says so; the calibration
/// is saved either way.
async fn confirm(socket: &Path, guide: &mut impl Guide) -> Result<(), String> {
    if !guide.ready("Check: turn your palm up again and press Enter.") {
        return Ok(());
    }
    match palm_now(socket).await?.as_str() {
        "up" => guide.say("The band reads palm up now. Done."),
        other => guide.say(&format!(
            "The band reads palm {other} with your palm up: if that's wrong, run `air-gestures calibrate` again."
        )),
    }
    Ok(())
}

/// Palm down, then palm up, held still: the calibration for the hand in use, stored and
/// applied, then checked.
pub async fn calibrate(socket: &Path, guide: &mut impl Guide) -> Result<PalmCalibration, String> {
    let status = client::status(socket).await?;
    let hand = connected_hand(&status)?.to_owned();
    guide.say(&format!("Palm positions for your {hand} hand: palm down, then palm up."));
    let (mut down, mut up) = (None, None);
    let mut last = String::new();
    for _ in 0..ROUNDS {
        if down.is_none() {
            down = Some(hold(socket, guide, Palm::Down).await?);
        }
        if up.is_none() {
            up = Some(hold(socket, guide, Palm::Up).await?);
        }
        match PalmCalibration::from_rolls(down.as_deref().unwrap_or_default(), up.as_deref().unwrap_or_default()) {
            Ok(calibration) => {
                client::request(socket, &Request::SetPalm { palm: Some(StoredPalm::from(calibration)) }).await?;
                let text = format!(
                    "Saved: palm down at {:.0}°, palm up at {:.0}° (sideways is halfway).",
                    calibration.down_roll, calibration.up_roll
                );
                guide.say(&text);
                guide.notify("Calibrated", &text);
                confirm(socket, guide).await?;
                return Ok(calibration);
            }
            Err(error) => {
                last = calibration_error_text(error);
                guide.say(&format!("{last}."));
                match error {
                    CalibrationError::NotStill(Palm::Down) | CalibrationError::Empty(Palm::Down) => down = None,
                    CalibrationError::NotStill(_) | CalibrationError::Empty(_) => up = None,
                    CalibrationError::TooClose => (down, up) = (None, None),
                }
            }
        }
    }
    Err(format!("calibration didn't work in {ROUNDS} rounds ({last}); nothing was changed"))
}

/// The moves taught, in order: the turn, then `Drag::ALL`'s.
pub const MOVES: [(&str, &str); 7] = [
    ("turn", "turn your wrist the way that should turn the volume up"),
    ("left", "move your hand left"),
    ("right", "move your hand right"),
    ("toward", "pull your hand toward you"),
    ("away", "push your hand away from you"),
    ("up", "lift your hand"),
    ("down", "lower your hand"),
];

/// Which moves (indexes into `MOVES`) to teach again.
pub fn redo(error: TaughtError) -> Vec<usize> {
    match error {
        TaughtError::NoTurn => vec![0],
        TaughtError::NoDrag(drag) | TaughtError::LikeTheTurn(drag) => vec![1 + drag.index()],
        TaughtError::TooAlike(a, b) => vec![1 + a.index(), 1 + b.index()],
        // Not a move's fault (the flow passes a finite roll): all of them again.
        TaughtError::NoRoll => (0..MOVES.len()).collect(),
    }
}

/// One move's direction, from the next pinch that moves.
pub async fn drag_sample(socket: &Path, guide: &mut impl Guide, index: usize) -> Result<[f64; 3], String> {
    let (name, how) = MOVES[index];
    guide.say(&format!("{}/{} {name}: pinch (thumb + index), {how}, let go.", index + 1, MOVES.len()));
    guide.notify("Teach your drags", &format!("Pinch, {how}, let go"));
    let response = client::request_within(socket, &Request::TeachDrag { seconds: 30.0 }, Duration::from_secs(40)).await?;
    match response.teach {
        Some(Taught::Drag { direction, .. }) => Ok(direction),
        _ => Err(unexpected()),
    }
}

/// The turn and six drags, one pinch each: the hand's prototypes, stored and applied.
pub async fn teach_drags(socket: &Path, guide: &mut impl Guide) -> Result<(), String> {
    let status = client::status(socket).await?;
    let hand = connected_hand(&status)?.to_owned();
    guide.say(&format!("Teach your drags ({hand} hand): one pinch for each move, the way you'll make it."));
    let mut samples: [Option<[f64; 3]>; 7] = [None; 7];
    let mut last = String::new();
    for _ in 0..ROUNDS {
        for (index, sample) in samples.iter_mut().enumerate() {
            if sample.is_none() {
                *sample = Some(drag_sample(socket, guide, index).await?);
            }
        }
        let all = samples.map(|s| s.unwrap_or_default());
        let drags: [[f64; 3]; 6] = std::array::from_fn(|i| all[i + 1]);
        // The palm-down roll doesn't change the check; the daemon sets the real one.
        match Prototypes::taught(all[0], drags, 0.0) {
            Ok(_) => {
                client::request(socket, &Request::SetDrags { drags: Some(TaughtDrags { turn: all[0], drags }) }).await?;
                guide.say("Saved: drags now read the way you make them.");
                guide.notify("Drags taught", "Saved");
                return Ok(());
            }
            Err(error) => {
                last = taught_error_text(error);
                guide.say(&format!("{last}: once more."));
                for index in redo(error) {
                    samples[index] = None;
                }
            }
        }
    }
    Err(format!("teaching didn't work in {ROUNDS} rounds ({last}); nothing was changed"))
}
