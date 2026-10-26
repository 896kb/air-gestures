use air_gesturesd::config::{Action, Config, DialTarget, HandSetting, Level, SliderStep, notes};
use band_core::controls::{BandControl, BandControls, Binding, Dial, DragMode, Gesture, Palm, PalmSet, Palms};

fn run(command: &str) -> Option<Binding<Action>> {
    Some(Binding::from(Action::from(command)))
}

fn texts(config: &Config) -> Vec<(Level, String)> {
    notes(config.profiles()).into_iter().map(|n| (n.level, n.text)).collect()
}

#[test]
fn defaults_match_the_design() {
    let config = Config::default();
    assert_eq!(config.band.hand, HandSetting::Band);
    assert!(config.band.autostart);
    let profiles = config.profiles();
    assert_eq!(profiles.band, BandControls::default(), "Linux: no band controls unless asked");
    assert_eq!(profiles.list.len(), 1);
    let first = &profiles.list[0];
    assert_eq!(first.name, "default");
    let get = |g| first.gestures.get(&g).cloned();
    assert_eq!(get(Gesture::SwipeLeft), run("hyprctl dispatch workspace e-1"));
    assert_eq!(get(Gesture::SwipeRight), run("hyprctl dispatch workspace e+1"));
    assert_eq!(get(Gesture::SwipeUp), run("wofi --show drun"));
    assert_eq!(get(Gesture::SwipeDown), run("hyprctl dispatch sendshortcut , Escape, activewindow"));
    assert_eq!(get(Gesture::IndexDouble), run("playerctl play-pause"));
    assert_eq!(get(Gesture::MiddleDouble), run("wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle"));
    for nothing in [Gesture::IndexTap, Gesture::MiddleTap, Gesture::MiddleHold, Gesture::IndexHold, Gesture::IndexTriple, Gesture::MiddleTriple, Gesture::IndexDragLeft] {
        assert_eq!(get(nothing), None, "{nothing:?}: 0.3.10's defaults");
    }
    assert_eq!(config.dial.target, DialTarget::Volume);
    assert_eq!(config.dial.sensitivity, 1.0);
    assert_eq!(config.band.on_draw, r#"notify-send -a air-gestures "Drawing" "${AIR_GESTURES_DRAWING:-not recognised}""#);
    let first = &config.profiles().list[0];
    assert_eq!(first.index.dial.clone().unwrap().up.command(), "wpctl set-volume -l 1.0 @DEFAULT_AUDIO_SINK@ 2%+");
    assert!(first.middle.dial.is_none());
    assert_eq!(first.index.drags, DragMode::Gestures);
    assert_eq!(config.index_dials(), [DialTarget::Volume]);
    let off = Config::parse("[dial]\ntarget = \"off\"\n").unwrap();
    assert!(off.profiles().list[0].index.dial.is_none());
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
    let first = &config.profiles().list[0];
    assert_eq!(first.gestures.get(&Gesture::SwipeLeft).cloned(), run("echo left"));
    assert_eq!(first.gestures.get(&Gesture::SwipeRight).cloned(), run("hyprctl dispatch workspace e+1"));
    assert_eq!(config.dial.target, DialTarget::Brightness);
    assert_eq!(config.options().dial_sensitivity, 2.0);
    assert!(first.index.dial.is_none(), "brightness_up unassigned: no dial for that target");
}

#[test]
fn whitespace_or_empty_values_are_unassigned() {
    let config = Config::parse("[gestures]\nindex_double = \"   \"\nmiddle_hold = \"\"\n").unwrap();
    let first = &config.profiles().list[0];
    assert_eq!(first.gestures.get(&Gesture::IndexDouble), None);
    assert_eq!(first.gestures.get(&Gesture::MiddleHold), None);
}

#[test]
fn a_whitespace_only_dial_command_is_unassigned() {
    let config = Config::parse("[dial]\nvolume_up = \"\\t\\n\"\n").unwrap();
    assert_eq!(config.dial.command(DialTarget::Volume, true), None);
    assert!(config.profiles().list[0].index.dial.is_none());
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
        index_hold = "echo held"
        [[profile]]
        name = "media"
        "#,
    )
    .unwrap();
    let profiles = config.profiles();
    let names: Vec<&str> = profiles.list.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["desk", "games", "media"]);
    let games = &profiles.list[1];
    assert_eq!(games.gestures.get(&Gesture::SwipeLeft).cloned(), run("echo games"));
    assert_eq!(games.gestures.get(&Gesture::IndexHold).cloned(), run("echo held"));
    assert_eq!(games.gestures.get(&Gesture::SwipeRight), None, "extra profiles don't inherit");
    assert!(games.index.dial.is_some(), "every profile turns the dial target");
}

#[test]
fn band_controls_take_any_gesture_and_palm_positions() {
    let config = Config::parse(
        r#"
        [band]
        toggle = "middle_hold"
        next_profile = "index_triple"
        next_profile_palm = ["up"]
        previous_profile = "swipe_down"
        draw = "index_drag_up"
        draw_palm = ["down", "sideways"]
        "#,
    )
    .unwrap();
    let band = config.profiles().band;
    assert_eq!(band.controls, Some(BandControl::on(Gesture::MiddleHold)));
    assert_eq!(band.next, Some(BandControl { gesture: Gesture::IndexTriple, palm: PalmSet::Only(Palms::from([Palm::Up])) }));
    assert_eq!(band.previous, Some(BandControl::on(Gesture::SwipeDown)));
    assert_eq!(band.draw, Some(BandControl { gesture: Gesture::IndexDragUp, palm: PalmSet::Only(Palms::from([Palm::Down, Palm::Side])) }));
}

#[test]
fn two_band_controls_may_share_a_gesture_only_in_different_positions() {
    let error = Config::parse("[band]\ntoggle = \"middle_hold\"\ndraw = \"middle_hold\"\n").unwrap_err();
    assert!(error.contains("[band] toggle and [band] draw are both on middle_hold"), "{error}");
    let error = Config::parse("[band]\ntoggle = \"middle_hold\"\ndraw = \"middle_hold\"\ndraw_palm = [\"up\"]\n").unwrap_err();
    assert!(error.contains("with the palm up"), "{error}");
    let fine = "[band]\ntoggle = \"middle_hold\"\ntoggle_palm = [\"down\"]\ndraw = \"middle_hold\"\ndraw_palm = [\"up\", \"sideways\"]\n";
    assert!(Config::parse(fine).is_ok());
}

/// Configs from before band controls: `toggle = "middle_hold"` keeps the lock there, and a
/// command the old documented config wrote on that gesture is kept but paused (logged).
#[test]
fn the_old_toggle_key_is_a_band_control_and_its_gestures_value_is_paused() {
    for (written, paused) in [("\"\"", 0), ("\"echo never\"", 1)] {
        let config = Config::parse(&format!("[band]\ntoggle = \"middle_hold\"\n[gestures]\nmiddle_hold = {written}\n")).unwrap();
        assert_eq!(config.profiles().band.controls, Some(BandControl::on(Gesture::MiddleHold)));
        let count = texts(&config).iter().filter(|(_, t)| t.contains("overridden")).count();
        assert_eq!(count, paused, "middle_hold = {written}");
    }
}

#[test]
fn every_config_load_notes_warnings_and_paused_entries() {
    let config = Config::parse(
        r#"
        [band]
        next_profile = "middle_double"
        [[profile]]
        name = "media"
        middle_double = "echo paused"
        "#,
    )
    .unwrap();
    let texts = texts(&config);
    let warn = |text: &str| (Level::Warn, text.to_owned());
    assert!(texts.contains(&warn("middle_double in profile \"default\" is overridden by [band] next_profile")), "{texts:?}");
    assert!(texts.contains(&warn("middle_double in profile \"media\" is overridden by [band] next_profile")), "{texts:?}");
    assert!(texts.iter().any(|(l, t)| *l == Level::Info && t.contains("[band] toggle")), "no lock: {texts:?}");
    let two = texts_of("[[profile]]\nname = \"b\"\n");
    assert!(two.iter().any(|(l, t)| *l == Level::Warn && t.contains("air-gestures profile next") && t.contains("[band] next_profile")), "{two:?}");
    let tap = texts_of("[band]\ndraw = \"index_tap\"\n");
    assert!(tap.iter().any(|(l, t)| *l == Level::Warn && t.contains("single tap")), "{tap:?}");
    let hold = texts_of("[band]\ndraw = \"index_hold\"\n");
    assert!(hold.iter().any(|(l, t)| *l == Level::Error && t.contains("index_hold") && t.contains("can't cancel")), "{hold:?}");
}

fn texts_of(text: &str) -> Vec<(Level, String)> {
    texts(&Config::parse(text).unwrap())
}

#[test]
fn mistakes_are_reported() {
    for (text, needle) in [
        ("[band]\ncolour = \"red\"\n", "[band] colour: unknown key"),
        ("[band]\nautostart = \"yes\"\n", "[band] autostart: invalid type"),
        ("[dial]\nloud = 1\n", "[dial] loud: unknown key"),
        ("[dial]\nsensitivity = \"x\"\n", "[dial] sensitivity: invalid type"),
        ("nope = 1\n", "nope: unknown key"),
        ("[nope]\nx = 1\n", "[nope]: unknown table"),
        ("band = 3\n", "band: expected a table"),
        ("profile = 3\n", "profile: expected [[profile]] tables"),
        ("[band]\ntoggle = \"pinky_hold\"\n", "[band] toggle: unknown gesture \"pinky_hold\""),
        ("[band]\ntoggle_palm = [\"up\"]\n", "[band] toggle_palm: set [band] toggle to a gesture first"),
        ("[band]\ntoggle = \"middle_hold\"\ntoggle_palm = [\"left\"]\n", "[band] toggle_palm: unknown position \"left\""),
        ("[band]\ntoggle = \"middle_hold\"\ntoggle_palm = [\"down\", \"sideways\", \"up\"]\n", "[band] toggle_palm: list one or two"),
        ("[band]\ntoggle = \"middle_hold\"\ntoggle_palm = []\n", "[band] toggle_palm: list one or two"),
        ("[band]\ntoggle = \"middle_hold\"\ntoggle_palm = [\"up\", \"up\"]\n", "[band] toggle_palm: \"up\" is listed twice"),
        ("[band]\nprofile = \"it's\"\n", "[band] profile: profile name \"it's\""),
        ("[gestures]\npinky_tap = \"x\"\n", "[gestures] pinky_tap: unknown key"),
        ("[gestures]\nmiddle_hold = \"@controls\"\n", "set in [band] now: toggle = \"middle_hold\""),
        ("[gestures]\nindex_triple = \"@next-profile\"\n", "next_profile = \"index_triple\""),
        ("[gestures]\nmiddle_triple = \"@previous-profile\"\n", "previous_profile = \"middle_triple\""),
        ("[gestures]\nswipe_up = \"@draw\"\n", "draw = \"swipe_up\""),
        ("[gestures]\nswipe_up = \"@lock\"\n", "[gestures] swipe_up = \"@lock\""),
        ("[dial]\nsensitivity = 9.0\n", "[dial] sensitivity must be between 0.5 and 4"),
        ("[[profile]]\nswipe_up = \"x\"\n", "[[profile]] #1: every [[profile]] needs a name"),
        ("[[profile]]\nname = 3\n", "[[profile]] #1 name: a profile's name must be text"),
        ("[[profile]]\nname = \"default\"\n", "[[profile]] #1 name: two profiles are called \"default\""),
        ("[[profile]]\nname = \"a\"\n[[profile]]\nname = \"a\"\n", "[[profile]] #2 name: two profiles are called \"a\""),
        ("[[profile]]\nname = \"a\"\n[[profile]]\nname = \"it's\"\n", "[[profile]] #2 name: profile name \"it's\""),
        ("[[profile]]\nname = \".hidden\"\n", "profile name \".hidden\""),
        ("[[profile]]\nname = \"x\"\nnope = \"y\"\n", "[profile \"x\"] nope: unknown key"),
        ("[[profile]]\nname = \"x\"\nindex_tap = \"@next-profile\"\n", "[profile \"x\"] index_tap = \"@next-profile\": band controls are set in [band] now: next_profile = \"index_tap\""),
        ("[gestures]\nindex_dial = \"loud\"\n", "[gestures] index_dial: \"loud\" isn't a dial"),
        ("[gestures]\nmiddle_drags = \"both\"\n", "[gestures] middle_drags: \"both\" isn't a drag mode (\"gestures\" or \"sliders\")"),
        ("[gestures]\nindex_slider_sideways = \"volume\"\n", "[gestures] index_slider_sideways: unknown key"),
        ("[gestures]\nring_dial = \"volume\"\n", "[gestures] ring_dial: unknown key"),
        ("[gestures.palm_up]\nnope = \"x\"\n", "[gestures.palm_up] nope: unknown key"),
        ("[gestures.palm_up]\nindex_tap = \"@controls\"\n", "toggle = \"index_tap\", toggle_palm = [\"up\"]"),
        ("[gestures.palm_up]\nindex_dial = \"volume\"\n", "[gestures.palm_up] index_dial: dials don't take palm overrides"),
        ("[gestures.sideways]\nmiddle_drags = \"sliders\"\n", "[gestures.sideways] middle_drags: drag modes and sliders don't take palm overrides"),
        ("[gestures]\nindex_tap = 3\n", "[gestures] index_tap: expected a string"),
        ("[gestures]\nnope = 3\n", "[gestures] nope: unknown key"),
        ("[gestures.palm_left]\nindex_tap = \"x\"\n", "[gestures] palm_left: unknown table"),
        ("[[profile]]\nname = \"x\"\n[profile.drawing]\nz = \"x\"\n", "[profile \"x\"] drawing: unknown table"),
        ("[gestures]\npalm_up = \"x\"\n", "[gestures] palm_up: expected a table"),
        ("[drawings]\n\"it's\" = \"x\"\n", "[drawings] drawing name \"it's\""),
        ("[drawings]\nz = \"@draw\"\n", "[drawings] z = \"@draw\": a drawing runs a command; band controls are set in [band]"),
        ("[[profile]]\nname = \"x\"\n[profile.drawings]\nz = \"@draw\"\n", "[profile \"x\".drawings] z = \"@draw\": a drawing runs a command"),
        ("[drawings]\nz = 1\n", "[drawings] z: expected a string"),
        ("[[profile]]\nname = \"x\"\ndrawings = \"z\"\n", "[profile \"x\"] drawings: expected a table"),
    ] {
        let error = Config::parse(text).unwrap_err();
        assert!(error.contains(needle), "{text:?} → {error}");
    }
}

#[test]
fn mistakes_carry_the_bad_value() {
    let error = Config::parse("[dial]\nsensitivity = 9.0").unwrap_err();
    assert!(error.contains("(got 9"), "should report the bad value: {error}");
    let error = Config::parse("[band]\nhand = \"both\"").unwrap_err();
    assert!(error.contains("[band] hand: ") && error.contains("both"), "{error}");
}

#[test]
fn a_missing_file_means_the_defaults() {
    let config = Config::load(std::path::Path::new("/nonexistent/air-gestures/config.toml")).unwrap();
    assert_eq!(config, Config::default());
}

#[test]
fn an_existing_file_is_loaded() {
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
    assert_eq!(documented.profiles(), default.profiles());
}

#[test]
fn the_dial_target_cycles() {
    assert_eq!(DialTarget::Volume.next(), DialTarget::Brightness);
    assert_eq!(DialTarget::Brightness.next(), DialTarget::Volume);
    assert_eq!(DialTarget::Off.next(), DialTarget::Volume);
}

#[test]
fn options_carry_hand_autostart_and_sensitivity() {
    let config = Config::parse("[band]\nhand = \"right\"\nautostart = false\n[dial]\nsensitivity = 0.5\n").unwrap();
    let options = config.options();
    assert_eq!(options.hand, band_core::controls::HandSetting::Right);
    assert!(!options.autostart);
    assert_eq!(options.dial_sensitivity, 0.5);
    assert!(options.palm.is_none() && options.drags.is_none(), "the daemon adds what's taught");
}

#[test]
fn fingers_have_dials_drag_modes_and_sliders() {
    let config = Config::parse(
        r#"
        [gestures]
        index_dial = "brightness"
        middle_dial = "volume"
        index_drag_left = "echo left"
        middle_drags = "sliders"
        middle_slider_up_down = "brightness"
        index_slider_left_right = "volume"
        [[profile]]
        name = "b"
        "#,
    )
    .unwrap();
    let profiles = config.profiles();
    let dial = |target| config.dial.dial(target);
    let first = &profiles.list[0];
    assert_eq!(first.index.dial, dial(DialTarget::Brightness));
    assert_eq!(first.middle.dial, dial(DialTarget::Volume));
    assert_eq!(first.index.drags, DragMode::Gestures, "sliders written, mode gestures: unused");
    assert_eq!(first.middle.drags, DragMode::Sliders([None, None, dial(DialTarget::Brightness)]));
    assert_eq!(first.gestures.get(&Gesture::IndexDragLeft).cloned(), run("echo left"));
    let b = &profiles.list[1];
    assert_eq!(b.index.dial, dial(DialTarget::Volume), "[dial] target is every profile's index default");
    assert!(b.middle.dial.is_none());
    assert_eq!(config.index_dials(), [DialTarget::Brightness, DialTarget::Volume]);
}

/// The slider `<finger>_<pair>` in `profile`, running `command` with its steps.
fn command_slider(slider: &str, command: &str, profile: &str) -> Option<Dial<Action>> {
    let step = |step| Action::Slider(SliderStep { slider: slider.into(), command: command.into(), profile: profile.into(), step });
    Some(Dial { up: step(1), down: step(-1) })
}

#[test]
fn a_slider_can_run_its_own_command() {
    let config = Config::parse(
        r#"
        [gestures]
        index_drags = "sliders"
        index_slider_left_right = "~/bin/lights"
        index_slider_toward_away = ""
        index_slider_up_down = "volume"
        [[profile]]
        name = "Living room"
        middle_drags = "sliders"
        middle_slider_left_right = "loud"
        middle_slider_up_down = "hass-light up-down"
        "#,
    )
    .unwrap();
    let profiles = config.profiles();
    assert_eq!(
        profiles.list[0].index.drags,
        DragMode::Sliders([command_slider("index_left_right", "~/bin/lights", "default"), None, config.dial.dial(DialTarget::Volume)]),
        "\"\" is off; the presets stay presets"
    );
    assert_eq!(
        profiles.list[1].middle.drags,
        DragMode::Sliders([
            command_slider("middle_left_right", "loud", "Living room"),
            None,
            command_slider("middle_up_down", "hass-light up-down", "Living room"),
        ]),
        "any other word is a command"
    );
    assert!(!texts(&config).iter().any(|(_, text)| text.contains("slider")), "{:?}", texts(&config));
}

#[test]
fn a_band_control_on_a_drag_pauses_that_pairs_slider() {
    let config = Config::parse(
        "[band]\ndraw = \"middle_drag_up\"\n[gestures]\nmiddle_drags = \"sliders\"\nmiddle_slider_up_down = \"volume\"\n",
    )
    .unwrap();
    let notes = notes(config.profiles());
    assert!(
        notes.iter().any(|n| n.level == Level::Warn
            && n.text == "middle_slider_up_down in profile \"default\" is overridden by [band] draw on middle_drag_up"),
        "{notes:?}"
    );
}

#[test]
fn palm_sub_tables_override_a_gesture_in_that_position() {
    let config = Config::parse(
        r#"
        [gestures]
        index_tap = "echo tap"
        [gestures.palm_up]
        index_tap = "echo up"
        swipe_left = ""
        [[profile]]
        name = "b"
        [profile.sideways]
        middle_tap = "echo side"
        "#,
    )
    .unwrap();
    let profiles = config.profiles();
    let tap = &profiles.list[0].gestures[&Gesture::IndexTap];
    assert_eq!(tap.action(Palm::Down).map(Action::command), Some("echo tap"));
    assert_eq!(tap.action(Palm::Up).map(Action::command), Some("echo up"));
    let left = &profiles.list[0].gestures[&Gesture::SwipeLeft];
    assert_eq!(left.action(Palm::Up), None, "\"\" = nothing with the palm up");
    assert_eq!(left.action(Palm::Side).map(Action::command), Some("hyprctl dispatch workspace e-1"));
    let side = &profiles.list[1].gestures[&Gesture::MiddleTap];
    assert_eq!(side.normal, None);
    assert_eq!(side.action(Palm::Side).map(Action::command), Some("echo side"));
}

#[test]
fn three_palm_overrides_on_one_gesture_are_an_error() {
    let error = Config::parse(
        "[gestures.palm_up]\nindex_tap = \"a\"\n[gestures.sideways]\nindex_tap = \"b\"\n[gestures.palm_down]\nindex_tap = \"c\"\n",
    )
    .unwrap_err();
    assert!(error.contains("[gestures] index_tap: palm overrides in") && error.contains("at most 2"), "{error}");
}

#[test]
fn an_override_a_band_control_pauses_is_named_with_its_position() {
    let config = Config::parse(
        r#"
        [band]
        draw = "index_tap"
        draw_palm = ["up"]
        [gestures]
        index_tap = "echo tap"
        [gestures.palm_up]
        index_tap = "echo up"
        "#,
    )
    .unwrap();
    let texts: Vec<String> = notes(config.profiles()).into_iter().map(|n| n.text).collect();
    assert!(texts.contains(&"index_tap (palm_up) in profile \"default\" is overridden by [band] draw with the palm up".to_owned()), "{texts:?}");
    assert!(!texts.iter().any(|t| t.starts_with("index_tap in")), "the normal action isn't used with the palm up: {texts:?}");
}

#[test]
fn drawings_have_shared_and_profile_actions() {
    let config = Config::parse(
        r#"
        [drawings]
        lightbulb = "hass-cli light toggle"
        z = "echo z"
        nothing = ""
        [[profile]]
        name = "media"
        [profile.drawings]
        z = "playerctl next"
        "#,
    )
    .unwrap();
    let p = config.profiles();
    assert_eq!(p.drawing_action(0, "z").map(Action::command), Some("echo z"));
    assert_eq!(p.drawing_action(1, "z").map(Action::command), Some("playerctl next"), "a profile's own wins");
    assert_eq!(p.drawing_action(1, "lightbulb").map(Action::command), Some("hass-cli light toggle"));
    assert_eq!(p.drawing_action(0, "nothing"), None);
}

#[test]
fn drawings_with_an_action_but_no_template_are_noted() {
    let config = Config::parse("[drawings]\nlightbulb = \"x\"\nz = \"y\"\n").unwrap();
    let notes = air_gesturesd::config::missing_drawings(config.profiles(), &["z"]);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(notes[0].level, Level::Warn);
    assert!(notes[0].text.contains("air-gestures draw teach lightbulb"), "{notes:?}");
}

#[test]
fn the_first_profiles_own_drawings_are_in_gestures_drawings() {
    let config = Config::parse(
        "[gestures.drawings]\nz = \"echo first\"\n[drawings]\nz = \"echo shared\"\n[[profile]]\nname = \"b\"\n",
    )
    .unwrap();
    let p = config.profiles();
    assert_eq!(p.drawing_action(0, "z").map(Action::command), Some("echo first"));
    assert_eq!(p.drawing_action(1, "z").map(Action::command), Some("echo shared"));
}

#[test]
fn three_palm_overrides_in_a_profile_are_an_error() {
    let error = Config::parse(
        "[[profile]]\nname = \"b\"\n[profile.palm_up]\nindex_tap = \"a\"\n[profile.sideways]\nindex_tap = \"b\"\n[profile.palm_down]\nindex_tap = \"c\"\n",
    )
    .unwrap_err();
    assert!(error.contains("[profile \"b\"] index_tap: palm overrides in") && error.contains("at most 2"), "{error}");
}

#[test]
fn an_empty_override_without_a_normal_action_leaves_nothing() {
    let config = Config::parse(
        "[gestures.palm_up]\nindex_tap = \"\"\nswipe_left = \"\"\n[[profile]]\nname = \"b\"\n[profile.sideways]\nmiddle_tap = \"\"\n",
    )
    .unwrap();
    let profiles = config.profiles();
    assert!(!profiles.list[0].gestures.contains_key(&Gesture::IndexTap));
    assert!(profiles.list[0].gestures.contains_key(&Gesture::SwipeLeft), "its normal action stays");
    assert!(profiles.list[1].gestures.is_empty(), "{:?}", profiles.list[1].gestures);
}

#[test]
fn a_band_control_on_a_drag_in_some_positions_pauses_the_slider_there() {
    let config = Config::parse(
        "[band]\ndraw = \"middle_drag_up\"\ndraw_palm = [\"up\"]\n[gestures]\nmiddle_drags = \"sliders\"\nmiddle_slider_up_down = \"volume\"\n",
    )
    .unwrap();
    let notes = notes(config.profiles());
    assert!(
        notes.iter().any(|n| n.level == Level::Warn
            && n.text == "middle_slider_up_down in profile \"default\" is overridden by [band] draw on middle_drag_up with the palm up"),
        "{notes:?}"
    );
}
