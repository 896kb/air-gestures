//! The guided teaching flows: palm calibration, teach your drags, teach a drawing. They talk
//! to air-gesturesd's teaching steps and check the results with band-core before storing them.
//!
//! A try that didn't work (the daemon's `try_again`: nothing drawn, no pinch in time; or a
//! check here: not still, too small) is said and done again, within `ROUNDS`; anything else
//! (the band or the daemon gone, another teaching step running) ends the flow, changing
//! nothing.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use air_gesturesd::config::Config;
use air_gesturesd::drawings;
use air_gesturesd::ipc::{CODE_TRY_AGAIN, Request, Response, Taught, TaughtDrags};
use air_gesturesd::status::{LinkState, Status};
use air_gesturesd::store::StoredPalm;
use air_gesturesd::teach::{calibration_error_text, taught_error_text};
use band_core::controls::draw::{self, Drawing, Library, Stroke, TeachError};
use band_core::controls::palm::{CalibrationError, Palm, PalmCalibration};
use band_core::controls::pinch::{Drag, Prototypes, TaughtError};

use crate::client::{self, RequestError};

/// What a flow says and asks (the terminal, or a test).
pub trait Guide {
    fn say(&mut self, text: &str);
    /// A desktop notification (the user is watching their hand, not the terminal).
    fn notify(&mut self, title: &str, body: &str);
    /// Show `prompt`, wait for Enter; `false` on end of input.
    fn ready(&mut self, prompt: &str) -> bool;
}

/// The terminal, with notifications from `notify_command` (`notify-send`).
pub struct Terminal {
    pub notify_command: String,
}

impl Default for Terminal {
    fn default() -> Self {
        Self { notify_command: "notify-send".into() }
    }
}

impl Guide for Terminal {
    fn say(&mut self, text: &str) {
        println!("{text}");
    }
    fn notify(&mut self, title: &str, body: &str) {
        // Best effort, and never waited for (a slow notification daemon mustn't delay the
        // step it announces): without one the terminal still says it.
        let child = std::process::Command::new(&self.notify_command)
            .args(["-a", "air-gestures", title, body])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if let Ok(mut child) = child {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
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
/// How long a drag or drawing try waits for the band.
const TRY_WAIT: f64 = 30.0;

fn unexpected() -> String {
    "unexpected answer from air-gesturesd".into()
}

fn unchanged(error: impl Into<String>) -> String {
    format!("{}; nothing was changed", error.into())
}

fn unsaved(error: impl Into<String>) -> String {
    format!("{}; nothing was saved", error.into())
}

/// The band's hand, once it's connected and has said.
pub fn connected_hand(status: &Status) -> Result<&str, String> {
    if status.state != LinkState::Connected {
        return Err(air_gesturesd::daemon::CONNECT_FIRST.into());
    }
    status.hand.as_deref().ok_or_else(|| "the band hasn't said which wrist it's on yet; try again in a moment".into())
}

/// One try: what the band gave, or why to do it again.
#[derive(Debug, Clone, PartialEq)]
pub enum Try<T> {
    Got(T),
    Again(String),
}

/// A teaching step's answer: `Again` for a try that didn't work, `Err` for the rest.
fn step_answer(answer: Result<Response, RequestError>) -> Result<Try<Taught>, String> {
    match answer {
        Ok(response) => response.teach.map(Try::Got).ok_or_else(unexpected),
        Err(error) if error.code.as_deref() == Some(CODE_TRY_AGAIN) => Ok(Try::Again(error.message)),
        Err(error) => Err(error.message),
    }
}

async fn hold(socket: &Path, guide: &mut impl Guide, palm: Palm) -> Result<Vec<f64>, String> {
    let (title, way) = match palm {
        Palm::Up => ("Palm up", "palm up"),
        _ => ("Palm down", "palm down"),
    };
    if !guide.ready(&format!("Forearm level in front of you, {way}. Press Enter, then hold still for 2 s.")) {
        return Err(unchanged("cancelled"));
    }
    guide.notify(title, "Hold still for 2 seconds");
    let response = client::request_within(socket, &Request::PalmRolls { seconds: HOLD }, Duration::from_secs(10))
        .await
        .map_err(unchanged)?;
    guide.notify("Done", &format!("{way}: you can move"));
    match response.teach {
        Some(Taught::Rolls { samples: 0, .. }) => {
            Err(unchanged("the band sent no orientation: is it on your wrist and connected? (`air-gestures status`)"))
        }
        Some(Taught::Rolls { rolls, .. }) => Ok(rolls),
        _ => Err(unchanged(unexpected())),
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
                client::request(socket, &Request::SetPalm { palm: Some(StoredPalm::from(calibration)) })
                    .await
                    .map_err(unchanged)?;
                let text = format!(
                    "Saved: palm down at {:.0}°, palm up at {:.0}° (sideways is halfway).",
                    calibration.down_roll, calibration.up_roll
                );
                guide.say(&text);
                guide.notify("Calibrated", &text);
                if let Err(error) = confirm(socket, guide).await {
                    guide.say(&format!("Saved, but the check couldn't run: {error}."));
                }
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
    Err(unchanged(format!("calibration didn't work in {ROUNDS} rounds ({last})")))
}

/// `--reset`: forget the palm calibration for the band and the hand in use.
pub async fn reset_palm(socket: &Path) -> Result<String, String> {
    let response = client::request(socket, &Request::SetPalm { palm: None }).await?;
    Ok(format!("palm calibration forgotten for your {} hand", hand_of(&response)))
}

/// `--reset`: forget the taught drags for the band and the hand in use.
pub async fn reset_drags(socket: &Path) -> Result<String, String> {
    let response = client::request(socket, &Request::SetDrags { drags: None }).await?;
    Ok(format!("taught drags forgotten for your {} hand", hand_of(&response)))
}

fn hand_of(response: &Response) -> &str {
    response.status.as_ref().and_then(|status| status.hand.as_deref()).unwrap_or("current")
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

/// The drag `MOVES` calls `name` (`"left"` … `"down"`; not the turn).
pub fn drag_named(name: &str) -> Option<Drag> {
    MOVES[1..].iter().position(|(n, _)| *n == name).map(|i| Drag::ALL[i])
}

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

/// `redo`, after `before` (the last round's refusal): a drag like the turn twice running
/// means the turn may be what's off, so it's taught again too.
pub fn redo_round(error: TaughtError, before: Option<TaughtError>) -> Vec<usize> {
    let mut moves = redo(error);
    if matches!((error, before), (TaughtError::LikeTheTurn(_), Some(TaughtError::LikeTheTurn(_)))) && !moves.contains(&0) {
        moves.push(0);
    }
    moves
}

/// One move's direction, from the next pinch that moves within `seconds`.
pub async fn drag_sample(socket: &Path, guide: &mut impl Guide, index: usize, seconds: f64) -> Result<Try<[f64; 3]>, String> {
    let (name, how) = MOVES[index];
    guide.say(&format!("{}/{} {name}: pinch (thumb + index), {how}, let go.", index + 1, MOVES.len()));
    guide.notify("Teach your drags", &format!("Pinch, {how}, let go"));
    let wait = Duration::from_secs_f64(seconds + 10.0);
    match step_answer(client::request_within(socket, &Request::TeachDrag { seconds }, wait).await)? {
        Try::Got(Taught::Drag { direction, .. }) => Ok(Try::Got(direction)),
        Try::Got(_) => Err(unexpected()),
        Try::Again(why) => Ok(Try::Again(why)),
    }
}

/// The turn and six drags, one pinch each: the hand's prototypes, stored and applied. The
/// drags in `skip` aren't taught: they're never read (a move like one does nothing).
pub async fn teach_drags(socket: &Path, guide: &mut impl Guide, skip: &[Drag]) -> Result<(), String> {
    teach_drags_within(socket, guide, skip, TRY_WAIT).await
}

/// `teach_drags`, each pinch waited for `seconds`.
pub async fn teach_drags_within(socket: &Path, guide: &mut impl Guide, skip: &[Drag], seconds: f64) -> Result<(), String> {
    let status = client::status(socket).await?;
    let hand = connected_hand(&status)?.to_owned();
    guide.say(&format!("Teach your drags ({hand} hand): one pinch for each move, the way you'll make it."));
    let skipped: [bool; 7] = std::array::from_fn(|index| index > 0 && skip.contains(&Drag::ALL[index - 1]));
    if skipped.contains(&true) {
        let names: Vec<&str> = (0..MOVES.len()).filter(|&i| skipped[i]).map(|i| MOVES[i].0).collect();
        guide.say(&format!("Skipped: {} (never read; that move does nothing).", names.join(", ")));
    }
    let mut samples: [Option<[f64; 3]>; 7] = [None; 7];
    let mut last = String::new();
    let mut refused = None;
    for _ in 0..ROUNDS {
        for (index, sample) in samples.iter_mut().enumerate() {
            if sample.is_some() || skipped[index] {
                continue;
            }
            match drag_sample(socket, guide, index, seconds).await.map_err(unchanged)? {
                Try::Got(direction) => *sample = Some(direction),
                Try::Again(why) => {
                    guide.say(&format!("{why}. Once more later."));
                    last = why;
                }
            }
        }
        if (0..MOVES.len()).any(|i| samples[i].is_none() && !skipped[i]) {
            continue;
        }
        let turn = samples[0].unwrap_or_default();
        let drags: [Option<[f64; 3]>; 6] = std::array::from_fn(|i| samples[i + 1]);
        // The palm-down roll doesn't change the check; the daemon sets the real one.
        match Prototypes::taught(turn, drags, 0.0) {
            Ok(_) => {
                client::request(socket, &Request::SetDrags { drags: Some(TaughtDrags { turn, drags }) })
                    .await
                    .map_err(unchanged)?;
                guide.say("Saved: drags now read the way you make them.");
                guide.notify("Drags taught", "Saved");
                return Ok(());
            }
            Err(error) => {
                last = taught_error_text(error);
                guide.say(&format!("{last}. Once more."));
                for index in redo_round(error, refused) {
                    samples[index] = None;
                }
                refused = Some(error);
            }
        }
    }
    Err(unchanged(format!("teaching didn't work in {ROUNDS} rounds ({last})")))
}

/// Tries a drawing is taught with.
pub const TRIES: usize = 3;

/// `name` as a TOML key.
fn toml_key(name: &str) -> String {
    if name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') { name.to_owned() } else { format!("\"{name}\"") }
}

/// What to add after a drawing is saved, when Draw isn't on a gesture yet.
pub fn draw_hint(config: &Config) -> Option<String> {
    matches!(config.band.draw.trim(), "none" | "")
        .then(|| "Draw isn't on a gesture yet: set [band] draw to one, e.g. draw = \"swipe_up\".".to_owned())
}

async fn draw_try(socket: &Path, guide: &mut impl Guide, name: &str, index: usize, seconds: f64) -> Result<Try<Vec<Stroke>>, String> {
    guide.say(&format!("Try {} of {TRIES}: draw it now.", index + 1));
    guide.notify("Draw", &format!("{name}: try {} of {TRIES}", index + 1));
    let wait = Duration::from_secs_f64(seconds + 10.0);
    match step_answer(client::request_within(socket, &Request::DrawTry { seconds }, wait).await)? {
        Try::Got(Taught::Strokes { strokes }) => Ok(Try::Got(strokes)),
        Try::Got(_) => Err(unexpected()),
        Try::Again(why) => Ok(Try::Again(why)),
    }
}

/// Three alike tries of a new drawing, saved in `dir` (where the daemon reads drawings).
pub async fn teach_drawing(socket: &Path, dir: &Path, name: &str, guide: &mut impl Guide) -> Result<(), String> {
    teach_drawing_within(socket, dir, name, guide, TRY_WAIT).await
}

/// `teach_drawing`, each try waited for `seconds`.
pub async fn teach_drawing_within(socket: &Path, dir: &Path, name: &str, guide: &mut impl Guide, seconds: f64) -> Result<(), String> {
    drawings::check_name(name)?;
    // A file there that doesn't load (corrupt, another version) still takes the name.
    if dir.join(format!("{name}.json")).exists() {
        return Err(format!("\"{name}\" is already taught: `air-gestures draw delete {name}` first"));
    }
    let (existing, _) = drawings::load(dir);
    connected_hand(&client::status(socket).await?)?;
    guide.say(&format!(
        "Draw \"{name}\" {TRIES} times, the way you'll draw it later (in the air). \
         Pinch to put the pen down, let go to lift it; each try ends 1 s after your last stroke."
    ));
    let mut tries: Vec<Option<Vec<Stroke>>> = vec![None; TRIES];
    let mut last = String::from("they didn't come out alike");
    for _ in 0..ROUNDS {
        for (index, slot) in tries.iter_mut().enumerate() {
            if slot.is_some() {
                continue;
            }
            match draw_try(socket, guide, name, index, seconds).await.map_err(unsaved)? {
                Try::Got(strokes) => *slot = Some(strokes),
                Try::Again(why) => {
                    guide.say(&format!("Try {}: {why}. Once more later.", index + 1));
                    last = why;
                }
            }
        }
        if tries.iter().any(Option::is_none) {
            continue;
        }
        let all: Vec<Vec<Stroke>> = tries.iter().map(|t| t.clone().unwrap_or_default()).collect();
        match draw::check_tries(&all) {
            Ok(()) => {
                if let Some(other) = Library::new(&existing).looks_like(&all) {
                    return Err(unsaved(format!("that looks like \"{other}\": teach something more different")));
                }
                drawings::save(dir, &Drawing { name: name.to_owned(), tries: all })
                    .map_err(|error| unsaved(format!("could not save \"{name}\": {error}")))?;
                guide.say(&format!("Saved \"{name}\". Give it an action in config.toml:\n  [drawings]\n  {} = \"…\"", toml_key(name)));
                guide.notify("Drawing taught", name);
                return Ok(());
            }
            Err(TeachError::TooSmall(index)) => {
                last = format!("try {} was too small to read", index + 1);
                guide.say(&format!("Try {} was too small to read: draw bigger.", index + 1));
                tries[index] = None;
            }
            Err(TeachError::NotAlike { .. } | TeachError::NoTries) => {
                last = "they didn't come out alike".into();
                guide.say("These don't look alike: try again, or draw bigger.");
                tries.iter_mut().for_each(|t| *t = None);
            }
        }
    }
    Err(unsaved(format!("\"{name}\" didn't work in {ROUNDS} rounds ({last})")))
}

/// One line per drawing, or how to make one.
pub fn list_drawings(dir: &Path) -> Result<String, String> {
    let (all, problems) = drawings::load(dir);
    for problem in problems {
        eprintln!("{problem}");
    }
    if all.is_empty() {
        return Ok("no drawings yet: `air-gestures draw teach <name>`".into());
    }
    Ok(all.iter().map(|d| format!("{} ({} tries)", d.name, d.tries.len())).collect::<Vec<_>>().join("\n"))
}

pub fn delete_drawing(dir: &Path, name: &str) -> Result<(), String> {
    match drawings::delete(dir, name) {
        Ok(true) => Ok(()),
        Ok(false) => Err(format!("no drawing \"{name}\" (`air-gestures draw list`)")),
        Err(error) => Err(format!("could not delete \"{name}\": {error}")),
    }
}
