//! Sliders set to a command: while you slide, each slider's steps are gathered and its command
//! runs at most once per `WINDOW` with their net count, so a smart light follows a quick slide
//! smoothly (as the Android app sends a dial's or slider's steps to automation). The first step
//! runs at once; the rest wait for the window to end, or for the pinch to.

use std::collections::BTreeMap;

use crate::config::SliderStep;

/// The shortest time between two runs of one slider's command (seconds).
pub const WINDOW: f64 = 0.25;

/// One run of a slider's command.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub slider: String,
    pub command: String,
    pub profile: String,
    /// The net steps: + right / away / up, − the other way; never 0.
    pub steps: i32,
}

impl Run {
    /// The command with `AIR_GESTURES_SLIDER`, `AIR_GESTURES_STEPS` (`+3`, `-2`) and
    /// `AIR_GESTURES_PROFILE` exported.
    pub fn shell(&self) -> String {
        // Slider and profile names are checked when the config loads: no quotes inside.
        format!(
            "export AIR_GESTURES_SLIDER={} AIR_GESTURES_STEPS={:+} AIR_GESTURES_PROFILE='{}'; {}",
            self.slider, self.steps, self.profile, self.command
        )
    }
}

/// One slider's gathered steps.
#[derive(Debug)]
struct Batch {
    pending: i32,
    /// A step came since the last run (it may have netted to 0).
    waiting: bool,
    last_run: f64,
}

impl Batch {
    /// What's gathered, unless it netted to 0; a run starts the window again at `now`.
    fn take(&mut self, (slider, profile, command): &Key, now: f64) -> Option<Run> {
        let steps = std::mem::take(&mut self.pending);
        self.waiting = false;
        if steps == 0 {
            return None;
        }
        self.last_run = now;
        Some(Run { slider: slider.clone(), command: command.clone(), profile: profile.clone(), steps })
    }

    fn due(&self) -> Option<f64> {
        self.waiting.then_some(self.last_run + WINDOW)
    }
}

/// A batch's slider, profile and command: steps made in another profile, or before a reload
/// changed the command, run with what they were made with.
type Key = (String, String, String);

/// Every command slider's batch.
#[derive(Debug, Default)]
pub struct Sliders {
    batches: BTreeMap<Key, Batch>,
}

impl Sliders {
    /// A step at `now`: its slider's run now when its window has passed, else it waits for `due`.
    pub fn step(&mut self, step: &SliderStep, now: f64) -> Option<Run> {
        let key = (step.slider.clone(), step.profile.clone(), step.command.clone());
        let batch = self.batches.entry(key.clone()).or_insert(Batch { pending: 0, waiting: false, last_run: f64::NEG_INFINITY });
        batch.pending += step.step;
        batch.waiting = true;
        if now - batch.last_run >= WINDOW { batch.take(&key, now) } else { None }
    }

    /// When the earliest waiting steps run.
    pub fn due(&self) -> Option<f64> {
        self.batches.values().filter_map(Batch::due).min_by(f64::total_cmp)
    }

    /// The runs whose window has ended by `now`.
    pub fn flush_due(&mut self, now: f64) -> Vec<Run> {
        self.batches
            .iter_mut()
            .filter(|(_, batch)| batch.due().is_some_and(|due| due <= now))
            .filter_map(|(key, batch)| batch.take(key, now))
            .collect()
    }

    /// Every waiting step's run now (the pinch ended); the next pinch's first step runs at once.
    pub fn flush(&mut self, now: f64) -> Vec<Run> {
        let runs = self.batches.iter_mut().filter(|(_, batch)| batch.waiting).filter_map(|(key, batch)| batch.take(key, now)).collect();
        self.batches.clear();
        runs
    }

    /// Forget the waiting steps (the band went, or the controls went off).
    pub fn drop_pending(&mut self) {
        self.batches.clear();
    }
}
