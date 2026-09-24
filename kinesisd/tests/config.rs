use band_core::gestures::{Recognized, Swipe, Tap};
use kinesisd::config::{Config, DialTarget, HandSetting};

#[test]
fn defaults_match_the_design() {
    let config = Config::default();
    assert_eq!(config.band.hand, HandSetting::Band);
    assert!(config.band.autostart);
    assert_eq!(
        config.gestures.command(Recognized::Swipe(Swipe::Left)),
        "hyprctl dispatch workspace e-1"
    );
    assert_eq!(
        config.gestures.command(Recognized::Swipe(Swipe::Right)),
        "hyprctl dispatch workspace e+1"
    );
    assert_eq!(
        config.gestures.command(Recognized::Swipe(Swipe::Up)),
        "wofi --show drun"
    );
    assert_eq!(
        config.gestures.command(Recognized::Swipe(Swipe::Down)),
        "hyprctl dispatch sendshortcut , Escape, activewindow"
    );
    assert_eq!(
        config
            .gestures
            .command(Recognized::Tap(Tap::IndexDoubleTap)),
        "playerctl play-pause"
    );
    assert_eq!(
        config
            .gestures
            .command(Recognized::Tap(Tap::MiddleDoubleTap)),
        "wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle"
    );
    assert_eq!(
        config.gestures.command(Recognized::Tap(Tap::MiddleHold)),
        ""
    );
    assert_eq!(config.dial.target, DialTarget::Volume);
    assert_eq!(config.dial.sensitivity, 1.0);
    assert_eq!(
        config.dial.command(DialTarget::Volume, true),
        Some("wpctl set-volume -l 1.0 @DEFAULT_AUDIO_SINK@ 2%+")
    );
    assert_eq!(
        config.dial.command(DialTarget::Brightness, false),
        Some("brightnessctl -q set 2%-")
    );
    assert_eq!(config.dial.command(DialTarget::Off, true), None);
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
    assert_eq!(
        config.gestures.command(Recognized::Swipe(Swipe::Left)),
        "echo left"
    );
    assert_eq!(
        config.gestures.command(Recognized::Swipe(Swipe::Right)),
        "hyprctl dispatch workspace e+1"
    );
    assert_eq!(config.dial.target, DialTarget::Brightness);
    assert_eq!(
        config.dial.command(DialTarget::Brightness, true),
        None,
        "an empty command is unassigned"
    );
}

#[test]
fn mistakes_are_reported() {
    assert!(
        Config::parse("[gestures]\nswipe_sideways = \"x\"")
            .unwrap_err()
            .contains("swipe_sideways")
    );
    assert!(
        Config::parse("[dial]\nsensitivity = 9.0")
            .unwrap_err()
            .contains("sensitivity")
    );
    assert!(Config::parse("[band]\nhand = \"both\"").is_err());
}

#[test]
fn a_missing_file_means_defaults() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        Config::load(&dir.path().join("nope.toml")).unwrap(),
        Config::default()
    );
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[dial]\ntarget = \"off\"\n").unwrap();
    assert_eq!(Config::load(&path).unwrap().dial.target, DialTarget::Off);
}

#[test]
fn the_dial_target_cycles_between_volume_and_brightness() {
    assert_eq!(DialTarget::Volume.next(), DialTarget::Brightness);
    assert_eq!(DialTarget::Brightness.next(), DialTarget::Volume);
    assert_eq!(DialTarget::Off.next(), DialTarget::Volume);
}
