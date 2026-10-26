use air_gesturesd::outcome::describe;
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

#[test]
fn each_gesture_is_one_line_with_its_palm_and_what_it_did() {
    let out = [
        gesture(Gesture::IndexTap, Palm::Up),
        Output::Run("echo up".into()),
        gesture(Gesture::SwipeLeft, Palm::Down),
        Output::Unused { gesture: Gesture::SwipeLeft, why: Unused::Unbound },
        gesture(Gesture::MiddleHold, Palm::Side),
        Output::Controls { enabled: false },
    ];
    assert_eq!(
        describe(&out, &profiles()),
        [
            "index tap · palm up → echo up",
            "swipe left · palm down → nothing (nothing bound here)",
            "middle hold · palm sideways → controls off",
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
    ];
    assert_eq!(
        describe(&out, &two()),
        [
            "index triple tap · palm down → profile \"media\"",
            "swipe up · palm down → drawing",
            "swipe right · palm down → nothing (its command is paused by [band] next_profile, which did nothing now)",
            "swipe down · palm down → nothing (the controls are off)",
            "index double tap · palm down → nothing (too soon after another action)",
        ]
    );
}

#[test]
fn a_stepping_pinch_is_one_line_when_it_ends() {
    let out = [
        Output::Run("vol-up".into()),
        Output::Run("vol-up".into()),
        Output::PinchEnded { finger: Finger::Index, class: Class::Turn, palm: Palm::Up, up: 2, down: 1 },
        Output::PinchEnded { finger: Finger::Middle, class: Class::Drag(Drag::Left), palm: Palm::Down, up: 0, down: 3 },
    ];
    assert_eq!(
        describe(&out, &profiles()),
        ["index dial · palm up → 2 up, 1 down", "middle slider left/right · palm down → 0 up, 3 down"]
    );
}
