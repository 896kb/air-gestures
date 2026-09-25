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
    // The band still reports Right: one automatic retry, then no more.
    assert_eq!(
        controller.on_event(&Event::Handedness(Hand::Right), 1.6),
        vec![Command::SetHand(Hand::Left)]
    );
    assert!(
        controller
            .on_event(&Event::Handedness(Hand::Right), 1.7)
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
fn a_lost_hand_request_is_retried_once() {
    let mut controller = with_hand(Hand::Right);
    assert_eq!(
        controller.request_hand(Hand::Left),
        vec![Command::SetHand(Hand::Left)]
    );
    // A second request while the first is still outstanding: the controller
    // itself doesn't dedupe (a real session would refuse it as busy).
    assert_eq!(
        controller.request_hand(Hand::Left),
        vec![Command::SetHand(Hand::Left)]
    );
    // The band still reports Right — the write never took. One automatic retry.
    assert_eq!(
        controller.on_event(&Event::Handedness(Hand::Right), 2.0),
        vec![Command::SetHand(Hand::Left)]
    );
    // No further retries once one has already been used.
    assert!(
        controller
            .on_event(&Event::Handedness(Hand::Right), 3.0)
            .is_empty()
    );
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

fn with_index_tap() -> Controller {
    let mut config = Config::default();
    config.gestures.index_tap = "kinesis-dial toggle".into();
    live(config)
}

#[test]
fn an_index_tap_waits_out_its_double_tap_then_runs() {
    let mut controller = with_index_tap();
    assert!(
        controller
            .on_event(&gesture(1, "index", "tap", 2.0), 2.0)
            .is_empty()
    );
    let due = controller.held_tap_due().expect("the tap is held");
    assert!(controller.fire_held_tap(due - 0.01).is_empty());
    assert_eq!(controller.fire_held_tap(due), run("kinesis-dial toggle"));
    assert!(controller.held_tap_due().is_none());
    assert!(controller.fire_held_tap(due + 1.0).is_empty(), "runs once");
}

/// On the band a double tap is: tap, then ~0.3 s later tap + doubletap at once.
#[test]
fn a_double_tap_runs_only_its_own_action() {
    let mut controller = with_index_tap();
    assert!(
        controller
            .on_event(&gesture(1, "index", "tap", 2.0), 2.0)
            .is_empty()
    );
    assert!(
        controller
            .on_event(&gesture(2, "index", "tap", 2.3), 2.3)
            .is_empty()
    );
    assert_eq!(
        controller.on_event(&gesture(3, "index", "doubletap", 2.3), 2.3),
        run("playerctl play-pause")
    );
    assert!(controller.held_tap_due().is_none());
    assert!(controller.fire_held_tap(5.0).is_empty());
}

#[test]
fn a_tap_without_a_double_tap_action_runs_at_once() {
    let mut config = Config::default();
    config.gestures.index_tap = "kinesis-dial toggle".into();
    config.gestures.index_double = String::new();
    let mut controller = live(config);
    assert_eq!(
        controller.on_event(&gesture(1, "index", "tap", 2.0), 2.0),
        run("kinesis-dial toggle")
    );
    assert!(controller.held_tap_due().is_none());
}

#[test]
fn a_disconnect_drops_a_held_tap() {
    let mut controller = with_index_tap();
    controller.on_event(&gesture(1, "index", "tap", 2.0), 2.0);
    controller.on_disconnect();
    assert!(controller.fire_held_tap(5.0).is_empty());
}

fn with_toggle_tap() -> Controller {
    let mut config = Config::default();
    config.band.toggle_tap = kinesisd::config::ToggleTap::Middle;
    config.band.on_toggle = "osd".into();
    live(config)
}

/// On the band: tap; tap + doubletap ~0.25 s later; tap (+ maybe doubletap).
fn triple(controller: &mut Controller, sequence: u64, at: f64) -> Vec<Command> {
    let mut out = controller.on_event(&gesture(sequence, "middle", "tap", at), at);
    out.extend(controller.on_event(
        &gesture(sequence + 1, "middle", "tap", at + 0.25),
        at + 0.25,
    ));
    out.extend(controller.on_event(
        &gesture(sequence + 2, "middle", "doubletap", at + 0.25),
        at + 0.25,
    ));
    out.extend(controller.on_event(&gesture(sequence + 3, "middle", "tap", at + 0.5), at + 0.5));
    out.extend(controller.on_event(
        &gesture(sequence + 4, "middle", "doubletap", at + 0.5),
        at + 0.5,
    ));
    out
}

#[test]
fn a_triple_tap_toggles_the_controls_even_while_off() {
    let mut controller = with_toggle_tap();
    assert_eq!(
        triple(&mut controller, 1, 2.0),
        run("KINESIS_ENABLED=0 osd")
    );
    assert!(controller.status().paused);
    assert!(
        controller.fire_held_tap(10.0).is_empty(),
        "no mute from the triple"
    );
    // Off: other gestures do nothing, but the triple tap still turns it back on.
    assert!(
        controller
            .on_event(&gesture(10, "thumb", "left", 5.0), 5.0)
            .is_empty()
    );
    assert_eq!(
        triple(&mut controller, 20, 6.0),
        run("KINESIS_ENABLED=1 osd")
    );
    assert!(!controller.status().paused);
}

#[test]
fn the_toggle_fingers_double_tap_waits_for_a_possible_third_tap() {
    let mut controller = with_toggle_tap();
    controller.on_event(&gesture(1, "middle", "tap", 2.0), 2.0);
    controller.on_event(&gesture(2, "middle", "tap", 2.25), 2.25);
    assert!(
        controller
            .on_event(&gesture(3, "middle", "doubletap", 2.25), 2.25)
            .is_empty()
    );
    let due = controller.held_tap_due().expect("the double tap is held");
    assert_eq!(
        controller.fire_held_tap(due),
        run("wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle")
    );
    assert!(!controller.status().paused);
}

#[test]
fn slow_taps_are_not_a_triple_tap() {
    let mut controller = with_toggle_tap();
    for (sequence, at) in [(1, 2.0), (2, 2.6), (3, 3.2)] {
        controller.on_event(&gesture(sequence, "middle", "tap", at), at);
    }
    assert!(!controller.status().paused);
}
