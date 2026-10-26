use air_gesturesd::config::{Action, DialConfig, SliderStep};
use air_gesturesd::outcome::{Outcomes, short};
use air_gesturesd::sliders::Run;
use band_core::controls::{Builtin, Class, Drag, Finger, Gesture, Output, Palm, Profile, Profiles, Unused};

fn profiles() -> Profiles<Action> {
    Profiles::single(Profile::new("default"))
}

fn two() -> Profiles<Action> {
    let mut p = profiles();
    p.list.push(Profile::new("media"));
    p
}

fn gesture(gesture: Gesture, palm: Palm) -> Output<Action> {
    Output::Gesture { gesture, palm }
}

fn lines(out: &[Output<Action>], profiles: &Profiles<Action>) -> Vec<String> {
    Outcomes::default().lines(out, profiles, &DialConfig::default())
}

#[test]
fn each_gesture_is_one_line_with_its_palm_and_what_it_did() {
    let out = [
        gesture(Gesture::IndexTap, Palm::Up),
        Output::Run("echo up".into()),
        gesture(Gesture::SwipeLeft, Palm::Down),
        Output::Unused { gesture: Gesture::SwipeLeft, why: Unused::Unbound },
        gesture(Gesture::MiddleHold, Palm::Side),
        Output::Controls { enabled: false },
        gesture(Gesture::SwipeUp, Palm::Down),
        Output::Practiced("wofi --show drun".into()),
    ];
    assert_eq!(
        lines(&out, &profiles()),
        [
            "index tap · palm up → echo up",
            "swipe left · palm down → nothing (nothing bound here)",
            "middle hold · palm sideways → controls off",
            "swipe up · palm down → would: wofi --show drun",
        ]
    );
}

#[test]
fn band_controls_profiles_drawings_and_reasons_read_plainly() {
    let out = [
        gesture(Gesture::IndexTriple, Palm::Down),
        Output::Profile { index: 1, step: 1 },
        gesture(Gesture::SwipeUp, Palm::Down),
        Output::DrawStarted,
        gesture(Gesture::SwipeRight, Palm::Down),
        Output::Unused { gesture: Gesture::SwipeRight, why: Unused::Paused(Builtin::NextProfile) },
        gesture(Gesture::SwipeDown, Palm::Down),
        Output::Unused { gesture: Gesture::SwipeDown, why: Unused::ControlsOff },
        gesture(Gesture::IndexDouble, Palm::Down),
        Output::Unused { gesture: Gesture::IndexDouble, why: Unused::TooSoon },
        gesture(Gesture::SwipeLeft, Palm::Down),
        Output::Unused { gesture: Gesture::SwipeLeft, why: Unused::Teaching },
        gesture(Gesture::SwipeLeft, Palm::Down),
        Output::Unused { gesture: Gesture::SwipeLeft, why: Unused::Drawing },
        gesture(Gesture::SwipeLeft, Palm::Down),
        Output::Unused { gesture: Gesture::SwipeLeft, why: Unused::Writing },
    ];
    assert_eq!(
        lines(&out, &two()),
        [
            "index triple tap · palm down → profile \"media\"",
            "swipe up · palm down → open a drawing",
            "swipe right · palm down → nothing (its command is paused by [band] next_profile, which did nothing now)",
            "swipe down · palm down → nothing (the controls are off)",
            "index double tap · palm down → nothing (too soon after another action)",
            "swipe left · palm down → teaching drags",
            "swipe left · palm down → drawing",
            "swipe left · palm down → writing",
        ]
    );
}

#[test]
fn a_drawing_says_what_it_ran() {
    let out = [
        Output::DrawFinished { strokes: Vec::new() },
        Output::Drawn { name: "lightbulb".into(), distance: 1.0 },
        Output::Run("hass-cli light toggle".into()),
        Output::DrawFinished { strokes: Vec::new() },
        Output::NotDrawn,
    ];
    assert_eq!(lines(&out, &profiles()), ["drawing \"lightbulb\" → hass-cli light toggle", "drawing → not recognised"]);
}

#[test]
fn a_stepping_pinch_names_what_it_stepped_when_it_ends() {
    let dial = DialConfig::default();
    let mut outcomes = Outcomes::default();
    let ended = |practiced| Output::PinchEnded { finger: Finger::Index, class: Class::Turn, palm: Palm::Up, up: 2, down: 1, practiced };
    assert!(outcomes.lines(&[Output::Run(dial.volume_up.as_str().into()), Output::Run(dial.volume_up.as_str().into())], &profiles(), &dial).is_empty());
    let out = [Output::Run(dial.volume_down.as_str().into()), ended(false)];
    assert_eq!(outcomes.lines(&out, &profiles(), &dial), ["index dial · palm up → volume up ×2, volume down ×1"]);
    let out = [Output::Practiced(dial.brightness_up.as_str().into()), ended(true)];
    assert_eq!(outcomes.lines(&out, &profiles(), &dial), ["index dial · palm up → would: brightness up ×1"]);
    let slider = Output::PinchEnded { finger: Finger::Middle, class: Class::Drag(Drag::Left), palm: Palm::Down, up: 0, down: 1, practiced: false };
    let out = [Output::Run("echo left".into()), slider];
    assert_eq!(outcomes.lines(&out, &profiles(), &dial), ["middle slider left/right · palm down → echo left ×1"]);
}

#[test]
fn a_command_sliders_pinch_names_the_slider_and_what_ran() {
    let dial = DialConfig::default();
    let mut outcomes = Outcomes::default();
    let step = |step| Action::Slider(SliderStep { slider: "index_left_right".into(), command: "~/bin/lights".into(), profile: "default".into(), step });
    let ran = |steps| Run { slider: "index_left_right".into(), command: "~/bin/lights".into(), profile: "default".into(), steps };
    let ended = |practiced| Output::PinchEnded { finger: Finger::Index, class: Class::Drag(Drag::Right), palm: Palm::Down, up: 4, down: 1, practiced };
    // The steps are counted as they run (gathered), not as they're made.
    outcomes.slider_ran(&ran(1));
    outcomes.slider_ran(&ran(2));
    let out = [Output::Run(step(1)), Output::Run(step(-1)), Output::Run(step(1)), ended(false)];
    assert_eq!(outcomes.lines(&out, &profiles(), &dial), ["index slider left/right · palm down → slider index_left_right +3"]);
    outcomes.slider_ran(&ran(2));
    outcomes.slider_ran(&ran(-2));
    assert_eq!(outcomes.lines(&[ended(false)], &profiles(), &dial), ["index slider left/right · palm down → slider index_left_right +2 -2"]);
    assert_eq!(outcomes.lines(&[ended(false)], &profiles(), &dial), ["index slider left/right · palm down → nothing"], "its steps were dropped");
    let out = [Output::Practiced(step(-1)), Output::Practiced(step(-1)), ended(true)];
    assert_eq!(outcomes.lines(&out, &profiles(), &dial), ["index slider left/right · palm down → would: slider index_left_right -2"]);
}

#[test]
fn a_pinch_the_band_or_the_controls_dropped_leaves_nothing_for_the_next() {
    let dial = DialConfig::default();
    let mut outcomes = Outcomes::default();
    let ran = Run { slider: "index_left_right".into(), command: "x".into(), profile: "default".into(), steps: 3 };
    let turn = Output::PinchEnded { finger: Finger::Index, class: Class::Turn, palm: Palm::Up, up: 1, down: 0, practiced: false };
    outcomes.slider_ran(&ran);
    assert!(outcomes.lines(&[Output::Run(dial.volume_up.as_str().into())], &profiles(), &dial).is_empty());
    outcomes.forget_pinch(); // the band went: no `PinchEnded`
    let out = [Output::Run(dial.volume_down.as_str().into()), turn.clone()];
    assert_eq!(outcomes.lines(&out, &profiles(), &dial), ["index dial · palm up → volume down ×1"]);
    outcomes.slider_ran(&ran);
    outcomes.lines(&[Output::Run(dial.volume_up.as_str().into()), Output::Controls { enabled: false }], &profiles(), &dial);
    let out = [Output::Run(dial.volume_down.as_str().into()), turn];
    assert_eq!(outcomes.lines(&out, &profiles(), &dial), ["index dial · palm up → volume down ×1"], "the controls went off");
}

#[test]
fn long_commands_are_cut_short() {
    let dial = DialConfig::default();
    assert_eq!(short(&dial.volume_up, &dial), "volume up");
    let long = "x".repeat(100);
    let cut = short(&long, &dial);
    assert_eq!(cut.chars().count(), 60);
    assert!(cut.ends_with('…'));
    assert_eq!(short("echo hi", &dial), "echo hi");
}
