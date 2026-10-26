use air_gesturesd::config::SliderStep;
use air_gesturesd::sliders::{Run, Sliders, WINDOW};

fn step(slider: &str, step: i32) -> SliderStep {
    step_in("default", "~/bin/lights", slider, step)
}

fn step_in(profile: &str, command: &str, slider: &str, step: i32) -> SliderStep {
    SliderStep { slider: slider.into(), command: command.into(), profile: profile.into(), step }
}

fn run(slider: &str, steps: i32) -> Run {
    run_in("default", "~/bin/lights", slider, steps)
}

fn run_in(profile: &str, command: &str, slider: &str, steps: i32) -> Run {
    Run { slider: slider.into(), command: command.into(), profile: profile.into(), steps }
}

const LR: &str = "index_left_right";

#[test]
fn the_first_step_runs_at_once_and_the_rest_wait_for_the_window_to_end() {
    let mut sliders = Sliders::default();
    assert_eq!(sliders.step(&step(LR, 1), 10.0), Some(run(LR, 1)));
    assert_eq!(sliders.due(), None, "nothing waits");
    assert_eq!(sliders.step(&step(LR, 1), 10.05), None);
    assert_eq!(sliders.step(&step(LR, 1), 10.1), None);
    assert_eq!(sliders.due(), Some(10.0 + WINDOW));
    assert!(sliders.flush_due(10.2).is_empty(), "not yet");
    assert_eq!(sliders.flush_due(10.25), [run(LR, 2)]);
    assert_eq!(sliders.due(), None);
    // The next window starts at that run.
    assert_eq!(sliders.step(&step(LR, 1), 10.4), None);
    assert_eq!(sliders.due(), Some(10.5));
    assert_eq!(sliders.flush_due(10.5), [run(LR, 1)]);
    // A step after a quiet window runs at once again.
    assert_eq!(sliders.step(&step(LR, -1), 11.0), Some(run(LR, -1)));
}

#[test]
fn steps_are_netted_and_a_net_zero_runs_nothing() {
    let mut sliders = Sliders::default();
    assert_eq!(sliders.step(&step(LR, -1), 0.0), Some(run(LR, -1)));
    for at in [0.05, 0.1, 0.15] {
        assert_eq!(sliders.step(&step(LR, -1), at), None);
    }
    assert_eq!(sliders.step(&step(LR, 1), 0.2), None);
    assert_eq!(sliders.flush_due(0.25), [run(LR, -2)]);
    assert_eq!(sliders.step(&step(LR, 1), 0.3), None);
    assert_eq!(sliders.step(&step(LR, -1), 0.35), None);
    assert!(sliders.flush_due(0.5).is_empty(), "+1 −1: nothing to run");
    assert_eq!(sliders.due(), None);
    // Nothing ran, so the window didn't start again: the next step runs at once.
    assert_eq!(sliders.step(&step(LR, 1), 0.5), Some(run(LR, 1)));
}

#[test]
fn the_pinchs_end_runs_what_waits() {
    let mut sliders = Sliders::default();
    sliders.step(&step(LR, 1), 0.0);
    sliders.step(&step(LR, 1), 0.1);
    sliders.step(&step(LR, 1), 0.12);
    assert_eq!(sliders.flush(0.15), [run(LR, 2)]);
    assert_eq!(sliders.due(), None);
    assert!(sliders.flush(0.2).is_empty(), "nothing left");
    // The next pinch's first step runs at once.
    assert_eq!(sliders.step(&step(LR, 1), 0.2), Some(run(LR, 1)));
}

/// A profile switch or a reload mid-slide: the steps made before run with the command and
/// profile they were made with, in their own window.
#[test]
fn steps_keep_their_own_command_and_profile() {
    let mut sliders = Sliders::default();
    assert_eq!(sliders.step(&step_in("a", "x", LR, 1), 0.0), Some(run_in("a", "x", LR, 1)));
    assert_eq!(sliders.step(&step_in("a", "x", LR, 1), 0.1), None);
    assert_eq!(sliders.step(&step_in("b", "x", LR, 1), 0.15), Some(run_in("b", "x", LR, 1)));
    assert_eq!(sliders.step(&step_in("b", "y", LR, -1), 0.16), Some(run_in("b", "y", LR, -1)));
    assert_eq!(sliders.step(&step_in("b", "y", LR, -1), 0.2), None);
    assert_eq!(sliders.flush_due(0.25), [run_in("a", "x", LR, 1)]);
    assert_eq!(sliders.flush_due(0.45), [run_in("b", "y", LR, -1)]);
}

#[test]
fn each_slider_has_its_own_window() {
    let mut sliders = Sliders::default();
    assert_eq!(sliders.step(&step(LR, 1), 0.0), Some(run(LR, 1)));
    assert_eq!(sliders.step(&step("middle_up_down", -1), 0.1), Some(run("middle_up_down", -1)));
    sliders.step(&step(LR, 1), 0.2);
    sliders.step(&step("middle_up_down", -1), 0.2);
    assert_eq!(sliders.due(), Some(0.25));
    assert_eq!(sliders.flush_due(0.3), [run(LR, 1)]);
    assert_eq!(sliders.flush_due(0.35), [run("middle_up_down", -1)]);
}

#[test]
fn dropped_steps_never_run() {
    let mut sliders = Sliders::default();
    sliders.step(&step(LR, 1), 0.0);
    sliders.step(&step(LR, 1), 0.1);
    sliders.drop_pending();
    assert_eq!(sliders.due(), None);
    assert!(sliders.flush(0.3).is_empty());
    assert!(sliders.flush_due(0.3).is_empty());
}

#[test]
fn a_run_exports_the_slider_its_steps_and_the_profile() {
    let printed = |run: &Run| {
        let output = std::process::Command::new("sh").arg("-c").arg(run.shell()).output().unwrap();
        assert!(output.status.success(), "{}", run.shell());
        String::from_utf8(output.stdout).unwrap()
    };
    let echo = r#"printf '%s|%s|%s|%s' "$AIR_GESTURES_SLIDER" "$AIR_GESTURES_STEPS" "$AIR_GESTURES_PROFILE" $((AIR_GESTURES_STEPS * 5))"#;
    let run = |steps| Run { slider: "middle_up_down".into(), command: echo.into(), profile: "Living room".into(), steps };
    assert_eq!(printed(&run(3)), "middle_up_down|+3|Living room|15");
    assert_eq!(printed(&run(-2)), "middle_up_down|-2|Living room|-10");
}
