use air_gesturesd::config::DialConfig;
use air_gesturesd::outcome::{Outcomes, short};
use band_core::controls::{Builtin, Class, Drag, Finger, Gesture, Output, Palm, Profile, Profiles, Unused};

fn profiles() -> Profiles<String> {
    Profiles::single(Profile::new("default"))
}

fn two() -> Profiles<String> {
    let mut p = profiles();
    p.list.push(Profile::new("media"));
    p
}

fn gesture(gesture: Gesture, palm: Palm) -> Output<String> {
    Output::Gesture { gesture, palm }
}

fn lines(out: &[Output<String>], profiles: &Profiles<String>) -> Vec<String> {
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
    assert!(outcomes.lines(&[Output::Run(dial.volume_up.clone()), Output::Run(dial.volume_up.clone())], &profiles(), &dial).is_empty());
    let out = [Output::Run(dial.volume_down.clone()), ended(false)];
    assert_eq!(outcomes.lines(&out, &profiles(), &dial), ["index dial · palm up → volume up ×2, volume down ×1"]);
    let out = [Output::Practiced(dial.brightness_up.clone()), ended(true)];
    assert_eq!(outcomes.lines(&out, &profiles(), &dial), ["index dial · palm up → would: brightness up ×1"]);
    let slider = Output::PinchEnded { finger: Finger::Middle, class: Class::Drag(Drag::Left), palm: Palm::Down, up: 0, down: 1, practiced: false };
    let out = [Output::Run("echo left".into()), slider];
    assert_eq!(outcomes.lines(&out, &profiles(), &dial), ["middle slider left/right · palm down → echo left ×1"]);
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
