use air_gesturesd::config::{Config, DialTarget, HandSetting};
use band_core::controls::{Action, Builtin, Gesture};

fn run(command: &str) -> Option<Action<String>> {
    Some(Action::Run(command.into()))
}

fn builtin(builtin: Builtin) -> Option<Action<String>> {
    Some(Action::Builtin(builtin))
}

#[test]
fn defaults_match_the_design() {
    let config = Config::default();
    assert_eq!(config.band.hand, HandSetting::Band);
    assert!(config.band.autostart);
    let profiles = config.profiles(DialTarget::Volume);
    assert_eq!(profiles.list.len(), 1);
    let first = &profiles.list[0];
    assert_eq!(first.name, "default");
    let get = |g| first.gestures.get(&g).cloned();
    assert_eq!(
        get(Gesture::SwipeLeft),
        run("hyprctl dispatch workspace e-1")
    );
    assert_eq!(
        get(Gesture::SwipeRight),
        run("hyprctl dispatch workspace e+1")
    );
    assert_eq!(get(Gesture::SwipeUp), run("wofi --show drun"));
    assert_eq!(
        get(Gesture::SwipeDown),
        run("hyprctl dispatch sendshortcut , Escape, activewindow")
    );
    assert_eq!(get(Gesture::IndexTap), None);
    assert_eq!(get(Gesture::IndexDouble), run("playerctl play-pause"));
    assert_eq!(
        get(Gesture::MiddleDouble),
        run("wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle")
    );
    assert_eq!(get(Gesture::MiddleHold), builtin(Builtin::Controls));
    assert_eq!(get(Gesture::IndexTriple), builtin(Builtin::NextProfile));
    assert_eq!(
        get(Gesture::MiddleTriple),
        builtin(Builtin::PreviousProfile)
    );
    let dial = first.dial.clone().unwrap();
    assert_eq!(dial.up, "wpctl set-volume -l 1.0 @DEFAULT_AUDIO_SINK@ 2%+");
    assert_eq!(config.dial.target, DialTarget::Volume);
    assert_eq!(config.dial.sensitivity, 1.0);
    assert!(config.profiles(DialTarget::Off).list[0].dial.is_none());
    assert_eq!(
        config.profiles(DialTarget::Brightness).list[0]
            .dial
            .clone()
            .unwrap()
            .down,
        "brightnessctl -q set 2%-"
    );
}

#[test]
fn partial_files_override_only_what_they_name() {
    let config = Config::parse(
        r#"
        [band]
        hand = "left"
        [gestures]
        swipe_left = "echo left"
        [dial]
        target = "brightness"
        sensitivity = 2.0
        brightness_up = ""
        "#,
    )
    .unwrap();
    assert_eq!(config.band.hand, HandSetting::Left);
    assert!(config.band.autostart, "unnamed keys keep their defaults");
    let first = &config.profiles(DialTarget::Brightness).list[0];
    assert_eq!(
        first.gestures.get(&Gesture::SwipeLeft).cloned(),
        run("echo left")
    );
    assert_eq!(
        first.gestures.get(&Gesture::SwipeRight).cloned(),
        run("hyprctl dispatch workspace e+1")
    );
    assert_eq!(config.dial.target, DialTarget::Brightness);
    assert_eq!(config.options().dial_sensitivity, 2.0);
    // brightness_up unassigned: no dial for that target.
    assert!(
        config.profiles(DialTarget::Brightness).list[0]
            .dial
            .is_none()
    );
}

#[test]
fn whitespace_or_empty_values_are_unassigned() {
    let config = Config::parse("[gestures]\nindex_double = \"   \"\nmiddle_hold = \"\"\n").unwrap();
    let first = &config.profiles(DialTarget::Volume).list[0];
    assert_eq!(first.gestures.get(&Gesture::IndexDouble), None);
    assert_eq!(first.gestures.get(&Gesture::MiddleHold), None);
}

#[test]
fn a_whitespace_only_dial_command_is_unassigned() {
    // Kept from the old suite: the dial's commands follow the same rule.
    let config = Config::parse("[dial]\nvolume_up = \"\\t\\n\"\n").unwrap();
    assert_eq!(config.dial.command(DialTarget::Volume, true), None);
    assert!(config.profiles(DialTarget::Volume).list[0].dial.is_none());
}

#[test]
fn profiles_come_in_order_and_start_empty() {
    let config = Config::parse(
        r#"
        [band]
        profile = "desk"
        [[profile]]
        name = "games"
        swipe_left = "echo games"
        index_triple = "@next-profile"
        [[profile]]
        name = "media"
        middle_hold = "@controls"
        "#,
    )
    .unwrap();
    let profiles = config.profiles(DialTarget::Volume);
    let names: Vec<&str> = profiles.list.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["desk", "games", "media"]);
    let games = &profiles.list[1];
    assert_eq!(
        games.gestures.get(&Gesture::SwipeLeft).cloned(),
        run("echo games")
    );
    assert_eq!(
        games.gestures.get(&Gesture::IndexTriple).cloned(),
        builtin(Builtin::NextProfile)
    );
    assert_eq!(
        games.gestures.get(&Gesture::SwipeRight),
        None,
        "extra profiles don't inherit"
    );
    assert!(
        games.dial.is_some(),
        "every profile turns the daemon's dial target"
    );
}

#[test]
fn the_old_toggle_key_binds_the_lock_where_nothing_else_was_written() {
    let config = Config::parse(
        r#"
        [band]
        toggle = "index_triple"
        [gestures]
        middle_hold = "echo hold"
        [[profile]]
        name = "two"
        [[profile]]
        name = "three"
        index_triple = "echo mine"
        "#,
    )
    .unwrap();
    let profiles = config.profiles(DialTarget::Volume);
    let triple = |i: usize| {
        profiles.list[i]
            .gestures
            .get(&Gesture::IndexTriple)
            .cloned()
    };
    assert_eq!(
        triple(0),
        builtin(Builtin::Controls),
        "overrides the [gestures] default"
    );
    assert_eq!(triple(1), builtin(Builtin::Controls));
    assert_eq!(
        triple(2),
        run("echo mine"),
        "a profile that set the key keeps it"
    );
    assert_eq!(
        profiles.list[0].gestures.get(&Gesture::MiddleHold).cloned(),
        run("echo hold")
    );
}

#[test]
fn mistakes_are_reported() {
    for (text, needle) in [
        ("[band]\ncolour = \"red\"\n", "colour"),
        ("[gestures]\nindex_hold = \"x\"\n", "index_hold"),
        ("[gestures]\nswipe_up = \"@lock\"\n", "@lock"),
        ("[dial]\nsensitivity = 9.0\n", "sensitivity"),
        ("[[profile]]\nswipe_up = \"x\"\n", "name"),
        ("[[profile]]\nname = \"default\"\n", "default"),
        (
            "[[profile]]\nname = \"a\"\n[[profile]]\nname = \"a\"\n",
            "\"a\"",
        ),
        ("[[profile]]\nname = \"it's\"\n", "it's"),
        ("[[profile]]\nname = \"x\"\nnope = \"y\"\n", "nope"),
    ] {
        let error = Config::parse(text).unwrap_err();
        assert!(error.contains(needle), "{text:?} → {error}");
    }
}

#[test]
fn mistakes_carry_the_bad_value() {
    // Kept from the old suite.
    let error = Config::parse("[dial]\nsensitivity = 9.0").unwrap_err();
    assert!(
        error.contains("(got 9"),
        "should report the bad value: {error}"
    );
    assert!(Config::parse("[band]\nhand = \"both\"").is_err());
}

#[test]
fn a_missing_file_means_the_defaults() {
    let config = Config::load(std::path::Path::new(
        "/nonexistent/air-gestures/config.toml",
    ))
    .unwrap();
    assert_eq!(config, Config::default());
}

#[test]
fn an_existing_file_is_loaded() {
    // Kept from the old suite.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[dial]\ntarget = \"off\"\n").unwrap();
    assert_eq!(Config::load(&path).unwrap().dial.target, DialTarget::Off);
}

#[test]
fn the_documented_config_is_the_default() {
    let documented = Config::parse(include_str!("../../contrib/config.toml")).unwrap();
    let default = Config::default();
    assert_eq!(documented.band, default.band);
    assert_eq!(documented.dial, default.dial);
    assert_eq!(documented.options(), default.options());
    for target in [DialTarget::Volume, DialTarget::Brightness, DialTarget::Off] {
        assert_eq!(documented.profiles(target), default.profiles(target));
    }
}

#[test]
fn the_dial_target_cycles() {
    assert_eq!(DialTarget::Volume.next(), DialTarget::Brightness);
    assert_eq!(DialTarget::Brightness.next(), DialTarget::Volume);
    assert_eq!(DialTarget::Off.next(), DialTarget::Volume);
}

#[test]
fn options_carry_hand_autostart_and_sensitivity() {
    let config =
        Config::parse("[band]\nhand = \"right\"\nautostart = false\n[dial]\nsensitivity = 0.5\n")
            .unwrap();
    let options = config.options();
    assert_eq!(options.hand, band_core::controls::HandSetting::Right);
    assert!(!options.autostart);
    assert_eq!(options.dial_sensitivity, 0.5);
}

/// Configs from before profiles: the old `toggle` took the gesture over whatever
/// `[gestures]` said for it (the old controller never ran that command), and the old
/// documented config wrote `middle_hold = ""`. Both must keep their lock.
#[test]
fn the_old_toggle_key_still_wins_in_gestures_like_it_did() {
    for written in ["\"\"", "\"echo never\""] {
        let config = Config::parse(&format!(
            "[band]\ntoggle = \"middle_hold\"\n[gestures]\nmiddle_hold = {written}\n"
        ))
        .unwrap();
        let first = &config.profiles(DialTarget::Volume).list[0];
        assert_eq!(
            first.gestures.get(&Gesture::MiddleHold).cloned(),
            builtin(Builtin::Controls),
            "middle_hold = {written}"
        );
    }
}
