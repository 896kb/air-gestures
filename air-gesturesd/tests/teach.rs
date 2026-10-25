use air_gesturesd::ipc::Taught;
use air_gesturesd::teach::{Done, Step, Wait, calibration_error_text, taught_error_text};
use band_core::controls::palm::{CalibrationError, Palm};
use band_core::controls::pinch::TaughtError;
use band_core::controls::{Drag, Finger, Output};
use band_core::events::Event;

/// An orientation whose roll about the forearm is `degrees` (a turn about the band's y axis).
fn rolled(degrees: f64) -> Event {
    let half = (-degrees).to_radians() / 2.0;
    Event::Orientation([half.cos() as f32, 0.0, half.sin() as f32, 0.0])
}

#[test]
fn rolls_are_collected_until_the_deadline() {
    let mut wait = Wait::new(Step::Rolls, 10.0, 2.0);
    assert_eq!(wait.deadline(), 12.0);
    for _ in 0..5 {
        wait.on_event(&rolled(-140.0));
    }
    // Forearm pointing straight up: the roll can't be read.
    let half = std::f64::consts::FRAC_PI_4;
    wait.on_event(&Event::Orientation([half.cos() as f32, half.sin() as f32, 0.0, 0.0]));
    assert_eq!(wait.on_output(&Output::DrawCancelled), None, "not its step");
    let Done::Taught(Taught::Rolls { rolls, samples }) = wait.timed_out() else { panic!() };
    assert_eq!(samples, 6);
    assert_eq!(rolls.len(), 5);
    assert!(rolls.iter().all(|r| (r + 140.0).abs() < 0.01), "{rolls:?}");
}

#[test]
fn a_drag_step_takes_the_first_sample() {
    let mut wait = Wait::new(Step::Drag, 0.0, 30.0);
    wait.on_event(&rolled(0.0));
    assert_eq!(wait.on_output(&Output::Run("x".into())), None);
    let done = wait.on_output(&Output::DragSample { finger: Finger::Middle, direction: [0.0, 1.0, 0.0] });
    assert_eq!(done, Some(Done::Taught(Taught::Drag { finger: "middle".into(), direction: [0.0, 1.0, 0.0] })));
    let Done::Retry(why) = Wait::new(Step::Drag, 0.0, 30.0).timed_out() else { panic!("a missed try is tried again") };
    assert!(why.contains("30 s"), "{why}");
}

#[test]
fn a_draw_step_takes_the_strokes_or_fails_on_nothing() {
    let strokes = vec![vec![[0.0, 0.0], [0.0, 10.0]]];
    let mut wait = Wait::new(Step::Draw, 0.0, 30.0);
    assert_eq!(wait.on_output(&Output::DrawStarted), None);
    assert_eq!(
        wait.on_output(&Output::DrawFinished { strokes: strokes.clone() }),
        Some(Done::Taught(Taught::Strokes { strokes }))
    );
    let mut cancelled = Wait::new(Step::Draw, 0.0, 30.0);
    assert!(matches!(cancelled.on_output(&Output::DrawCancelled), Some(Done::Retry(_))));
    assert!(matches!(cancelled.on_output(&Output::DrawFinished { strokes: Vec::new() }), Some(Done::Retry(_))));
}

#[test]
fn errors_say_what_to_do() {
    assert!(taught_error_text(TaughtError::TooAlike(Drag::Left, Drag::Right)).contains("left and right"));
    assert!(taught_error_text(TaughtError::LikeTheTurn(Drag::Up)).contains("without turning"));
    assert!(calibration_error_text(CalibrationError::NotStill(Palm::Up)).contains("hold still"));
    assert!(calibration_error_text(CalibrationError::TooClose).contains("60°"));
    assert!(calibration_error_text(CalibrationError::Empty(Palm::Down)).contains("level"));
}
