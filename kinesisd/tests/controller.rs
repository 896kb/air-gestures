use band_core::events::{BatteryStatus, Event, GestureMessage, Hand};
use kinesisd::config::{Config, DialTarget, HandSetting};
use kinesisd::controller::{Command, Controller};

fn gesture(sequence: u64, finger: &str, action: &str, at: f64) -> Event {
    Event::Gesture(GestureMessage {
        sequence,
        timestamp_us: sequence * 1000,
        finger: finger.into(),
        action: action.into(),
        derived_action: "unknown".into(),
        synthetic: false,
        received_at: at,
    })
}

fn run(command: &str) -> Vec<Command> {
    vec![Command::Run(command.into())]
}

fn live(config: Config) -> Controller {
    let mut controller = Controller::new(config);
    controller.on_event(&Event::Connected, 1.0);
    controller
}

fn with_hand(hand: Hand) -> Controller {
    let mut controller = live(Config::default());
    controller.on_event(&Event::Handedness(hand), 1.0);
    controller
}

#[test]
fn a_swipe_runs_its_command_once() {
    let mut controller = live(Config::default());
    let swipe = gesture(1, "thumb", "left", 2.0);
    assert_eq!(
        controller.on_event(&swipe, 2.0),
        run("hyprctl dispatch workspace e-1")
    );
    assert!(
        controller.on_event(&swipe, 2.01).is_empty(),
        "a repeated message is not a second swipe"
    );
    assert_eq!(controller.status().gestures, 1);
    assert_eq!(
        controller.status().last_gesture.as_deref(),
        Some("swipe left")
    );
    assert_eq!(
        controller.status().last_action.as_deref(),
        Some("hyprctl dispatch workspace e-1")
    );
}

#[test]
fn nothing_runs_before_the_band_is_live_or_while_paused() {
    let mut controller = Controller::new(Config::default());
    assert!(
        controller
            .on_event(&gesture(1, "thumb", "left", 0.5), 0.5)
            .is_empty()
    );
    controller.on_event(&Event::Connected, 1.0);
    controller.pause();
    assert!(controller.status().paused);
    assert!(
        controller
            .on_event(&gesture(2, "thumb", "left", 2.0), 2.0)
            .is_empty()
    );
    controller.resume(3.0);
    assert!(
        controller
            .on_event(&gesture(3, "thumb", "left", 2.9), 3.0)
            .is_empty(),
        "input from before resuming never acts"
    );
    assert_eq!(
        controller.on_event(&gesture(4, "thumb", "right", 3.5), 3.5),
        run("hyprctl dispatch workspace e+1")
    );
}

#[test]
fn without_autostart_controls_start_paused() {
    let mut config = Config::default();
    config.band.autostart = false;
    let mut controller = live(config);
    assert!(controller.status().paused);
    assert!(
        controller
            .on_event(&gesture(1, "thumb", "up", 2.0), 2.0)
            .is_empty()
    );
    controller.toggle(3.0);
    assert_eq!(
        controller.on_event(&gesture(2, "thumb", "up", 3.5), 3.5),
        run("wofi --show drun")
    );
}

#[test]
fn stale_gestures_never_act() {
    let mut controller = live(Config::default());
    assert!(
        controller
            .on_event(&gesture(1, "thumb", "left", 1.0), 2.0)
            .is_empty()
    );
}

#[test]
fn the_dial_steps_volume_both_ways() {
    let mut controller = with_hand(Hand::Right);
    controller.on_event(&Event::DialState(true), 2.0);
    assert_eq!(
        controller.on_event(&Event::DialTurn(2.0), 2.1),
        run("wpctl set-volume -l 1.0 @DEFAULT_AUDIO_SINK@ 2%+")
    );
    assert_eq!(
        controller.status().last_gesture.as_deref(),
        Some("wrist turn +")
    );
    assert_eq!(
        controller.on_event(&Event::DialTurn(-2.0), 2.2),
        run("wpctl set-volume @DEFAULT_AUDIO_SINK@ 2%-")
    );
    assert_eq!(
        controller.status().last_gesture.as_deref(),
        Some("wrist turn \u{2212}")
    );
}

#[test]
fn the_left_hand_reverses_the_dial() {
    let mut controller = with_hand(Hand::Left);
    controller.on_event(&Event::DialState(true), 2.0);
    assert_eq!(
        controller.on_event(&Event::DialTurn(2.0), 2.1),
        run("wpctl set-volume @DEFAULT_AUDIO_SINK@ 2%-")
    );
}

#[test]
fn the_dial_waits_for_a_confirmed_hand() {
    let mut controller = live(Config::default());
    controller.on_event(&Event::DialState(true), 2.0);
    assert!(controller.on_event(&Event::DialTurn(2.0), 2.1).is_empty());
}

#[test]
fn the_dial_target_can_be_switched_or_turned_off() {
    let mut controller = with_hand(Hand::Right);
    controller.set_dial_target(DialTarget::Off);
    controller.on_event(&Event::DialState(true), 2.0);
    assert!(controller.on_event(&Event::DialTurn(2.0), 2.1).is_empty());
    controller.cycle_dial();
    assert_eq!(controller.status().dial, DialTarget::Volume);
    controller.cycle_dial();
    controller.on_event(&Event::DialState(true), 3.0);
    assert_eq!(
        controller.on_event(&Event::DialTurn(2.0), 3.1),
        run("brightnessctl -q set 2%+")
    );
}

#[test]
fn releasing_a_wrist_turn_is_not_an_index_tap() {
    let mut controller = with_hand(Hand::Right);
    controller.on_event(&Event::DialState(true), 2.0);
    controller.on_event(&Event::DialTurn(2.0), 2.1);
    controller.on_event(&Event::DialState(false), 2.3);
    assert!(
        controller
            .on_event(&gesture(1, "index", "doubletap", 2.5), 2.5)
            .is_empty()
    );
    assert_eq!(
        controller.on_event(&gesture(2, "middle", "doubletap", 2.5), 2.5),
        run("wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle")
    );
    assert_eq!(
        controller.on_event(&gesture(3, "index", "doubletap", 3.5), 3.5),
        run("playerctl play-pause")
    );
}

#[test]
fn a_configured_hand_is_written_once_after_connecting() {
    let mut config = Config::default();
    config.band.hand = HandSetting::Left;
    let mut controller = live(config);
    assert_eq!(
        controller.on_event(&Event::Handedness(Hand::Right), 1.5),
        vec![Command::SetHand(Hand::Left)]
    );
    assert!(
        controller
            .on_event(&Event::Handedness(Hand::Right), 1.6)
            .is_empty()
    );
    assert!(
        controller
            .on_event(&Event::Handedness(Hand::Left), 2.0)
            .is_empty()
    );
    assert_eq!(controller.status().hand.as_deref(), Some("left"));
}

#[test]
fn a_disconnect_stops_everything_until_the_next_connect() {
    let mut controller = live(Config::default());
    controller.on_disconnect();
    assert!(
        controller
            .on_event(&gesture(1, "thumb", "left", 2.0), 2.0)
            .is_empty()
    );
    controller.on_event(&Event::Connected, 3.0);
    assert_eq!(
        controller.on_event(&gesture(2, "thumb", "left", 3.5), 3.5),
        run("hyprctl dispatch workspace e-1")
    );
}

#[test]
fn battery_reports_update_the_status() {
    let mut controller = live(Config::default());
    controller.on_event(
        &Event::BatteryStatus(Some(BatteryStatus {
            level: 42,
            charging: Some(true),
        })),
        2.0,
    );
    assert_eq!(
        (controller.status().battery, controller.status().charging),
        (Some(42), Some(true))
    );
    controller.on_event(&Event::BatteryStatus(None), 3.0);
    assert_eq!(
        (controller.status().battery, controller.status().charging),
        (Some(42), None)
    );
}

#[test]
fn a_stale_hand_is_forgotten_on_reconnect() {
    let mut controller = live(Config::default());
    controller.on_event(&Event::Handedness(Hand::Right), 1.0);
    controller.on_disconnect();
    let mut config = Config::default();
    config.band.hand = HandSetting::Left;
    assert!(controller.set_config(config).is_empty());
    assert!(controller.on_event(&Event::Connected, 3.0).is_empty());
    assert_eq!(
        controller.on_event(&Event::Handedness(Hand::Right), 3.5),
        vec![Command::SetHand(Hand::Left)]
    );
}

#[test]
fn an_explicit_hand_request_overrides_the_config_until_it_reloads() {
    let mut config = Config::default();
    config.band.hand = HandSetting::Left;
    let mut controller = live(config);
    assert!(
        controller
            .on_event(&Event::Handedness(Hand::Left), 1.0)
            .is_empty()
    );
    assert_eq!(
        controller.request_hand(Hand::Right),
        vec![Command::SetHand(Hand::Right)]
    );
    assert!(
        controller
            .on_event(&Event::Handedness(Hand::Right), 1.5)
            .is_empty()
    );
}
