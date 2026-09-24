//! What to do with each band event — a port of the event handling in kinesis
//! `BandModel.swift` (~L840–990). Pure: no I/O; time is passed in.

use band_core::events::{Event, Hand};
use band_core::gestures::{ActionGate, DialRouter, GestureRouter, Recognized, Swipe, Tap};

use crate::config::{Config, DialTarget, HandSetting};
use crate::status::Status;

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Run this shell command.
    Run(String),
    /// Ask the band to switch hands.
    SetHand(Hand),
}

fn label(gesture: Recognized) -> &'static str {
    match gesture {
        Recognized::Swipe(Swipe::Left) => "swipe left",
        Recognized::Swipe(Swipe::Right) => "swipe right",
        Recognized::Swipe(Swipe::Up) => "swipe up",
        Recognized::Swipe(Swipe::Down) => "swipe down",
        Recognized::Tap(Tap::IndexTap) => "index tap",
        Recognized::Tap(Tap::IndexDoubleTap) => "index double tap",
        Recognized::Tap(Tap::MiddleTap) => "middle tap",
        Recognized::Tap(Tap::MiddleDoubleTap) => "middle double tap",
        Recognized::Tap(Tap::MiddleHold) => "middle hold",
    }
}

fn hand_name(hand: Hand) -> &'static str {
    match hand {
        Hand::Left => "left",
        Hand::Right => "right",
    }
}

pub struct Controller {
    config: Config,
    dial_target: DialTarget,
    router: GestureRouter,
    gate: ActionGate,
    dial_gate: ActionGate,
    dial_router: DialRouter,
    /// Controls are on (the user hasn't paused).
    enabled: bool,
    /// The band's input subscription is up.
    live: bool,
    hand: Option<Hand>,
    hand_confirmed: bool,
    hand_requested: bool,
    dial_armed: bool,
    dial_engaged: bool,
    dial_turned: bool,
    dial_ended_at: f64,
    last_dial_action: f64,
    status: Status,
}

impl Controller {
    pub fn new(config: Config) -> Self {
        let enabled = config.band.autostart;
        let status = Status {
            paused: !enabled,
            dial: config.dial.target,
            ..Status::default()
        };
        Self {
            dial_target: config.dial.target,
            config,
            router: GestureRouter::default(),
            gate: ActionGate::default(),
            dial_gate: ActionGate::new(0.0),
            dial_router: DialRouter::default(),
            enabled,
            live: false,
            hand: None,
            hand_confirmed: false,
            hand_requested: false,
            dial_armed: false,
            dial_engaged: false,
            dial_turned: false,
            dial_ended_at: f64::NEG_INFINITY,
            last_dial_action: f64::NEG_INFINITY,
            status,
        }
    }

    /// Gesture, dial and control fields of the status (the daemon adds the rest).
    pub fn status(&self) -> &Status {
        &self.status
    }

    /// Apply a reloaded config. A changed dial target in the file wins over a
    /// runtime switch; a changed hand setting is applied if the band is live.
    pub fn set_config(&mut self, config: Config) -> Vec<Command> {
        if config.dial.target != self.config.dial.target {
            self.dial_target = config.dial.target;
            self.status.dial = config.dial.target;
        }
        self.config = config;
        self.hand_requested = false;
        self.reset_dial();
        self.maybe_request_hand()
    }

    pub fn resume(&mut self, now: f64) {
        self.enabled = true;
        self.status.paused = false;
        if self.live {
            self.gate.arm(now);
        }
        self.reset_dial();
    }

    pub fn pause(&mut self) {
        self.enabled = false;
        self.status.paused = true;
        self.suspend();
    }

    pub fn toggle(&mut self, now: f64) {
        if self.enabled {
            self.pause()
        } else {
            self.resume(now)
        }
    }

    pub fn set_dial_target(&mut self, target: DialTarget) {
        self.dial_target = target;
        self.status.dial = target;
        self.reset_dial();
    }

    pub fn cycle_dial(&mut self) {
        self.set_dial_target(self.dial_target.next());
    }

    pub fn on_disconnect(&mut self) {
        self.live = false;
        self.hand_confirmed = false;
        self.hand_requested = false;
        self.suspend();
        self.router.reset();
        self.status.charging = None;
    }

    pub fn on_event(&mut self, event: &Event, now: f64) -> Vec<Command> {
        match event {
            Event::Connected => {
                self.live = true;
                if self.enabled {
                    self.gate.arm(now);
                }
                self.maybe_request_hand()
            }
            Event::Handedness(hand) => {
                self.hand = Some(*hand);
                self.hand_confirmed = true;
                self.status.hand = Some(hand_name(*hand).into());
                self.maybe_request_hand()
            }
            Event::HandednessFailure(_) => {
                self.hand_confirmed = false;
                Vec::new()
            }
            Event::BatteryStatus(Some(status)) => {
                self.status.battery = Some(status.level);
                self.status.charging = status.charging;
                Vec::new()
            }
            Event::BatteryStatus(None) => {
                self.status.charging = None;
                Vec::new()
            }
            Event::Gesture(message) => self.gesture(message, now),
            Event::DialState(engaged) => {
                self.dial_state(*engaged, now);
                Vec::new()
            }
            Event::DialTurn(rotation) => self.dial_turn(*rotation, now),
            _ => Vec::new(),
        }
    }

    fn gesture(&mut self, message: &band_core::events::GestureMessage, now: f64) -> Vec<Command> {
        let age = now - message.received_at;
        if !(-0.1..=0.35).contains(&age) {
            return Vec::new();
        }
        let Some(gesture) = self.router.gesture(message, now) else {
            return Vec::new();
        };
        // Releasing a wrist turn must not also trigger an index-finger assignment.
        if let Recognized::Tap(tap) = gesture
            && tap.finger() == "index"
            && (now - self.last_dial_action <= 0.6 || now - self.dial_ended_at <= 0.7)
        {
            return Vec::new();
        }
        self.status.gestures += 1;
        self.status.last_gesture = Some(label(gesture).into());
        let command = self.config.gestures.command(gesture).to_owned();
        if !self.enabled
            || command.is_empty()
            || !self.gate.allows(message.received_at, now, self.live, true)
        {
            return Vec::new();
        }
        self.status.last_action = Some(command.clone());
        vec![Command::Run(command)]
    }

    fn dial_state(&mut self, engaged: bool, now: f64) {
        if !self.live {
            self.reset_dial();
            return;
        }
        // A pinch that turned has just ended: its release is not a tap.
        if !engaged && self.dial_turned {
            self.dial_ended_at = now;
        }
        self.dial_turned = false;
        self.dial_engaged = engaged;
        self.reset_dial();
        if engaged && self.enabled {
            self.dial_armed = true;
            self.dial_gate.arm(now);
        }
    }

    fn dial_turn(&mut self, rotation: f64, now: f64) -> Vec<Command> {
        if !self.live || !self.hand_confirmed || !self.dial_engaged || !rotation.is_finite() {
            return Vec::new();
        }
        // The same intended turn produces the opposite gyro sign on the left wrist.
        let delta = if self.hand == Some(Hand::Left) {
            -rotation
        } else {
            rotation
        };
        self.dial_turned = true;
        if !self.enabled
            || !self.dial_armed
            || self.config.dial.command(self.dial_target, true).is_none()
        {
            return Vec::new();
        }
        if !self.dial_gate.allows(now, now, self.live, true) {
            return Vec::new();
        }
        let steps = self
            .dial_router
            .turn(delta, self.config.dial.sensitivity, now);
        if steps == 0 {
            return Vec::new();
        }
        self.last_dial_action = now;
        match self.config.dial.command(self.dial_target, steps > 0) {
            Some(command) => {
                self.status.last_action = Some(command.to_owned());
                vec![Command::Run(command.to_owned())]
            }
            None => Vec::new(),
        }
    }

    fn desired_hand(&self) -> Option<Hand> {
        match self.config.band.hand {
            HandSetting::Band => None,
            HandSetting::Left => Some(Hand::Left),
            HandSetting::Right => Some(Hand::Right),
        }
    }

    fn maybe_request_hand(&mut self) -> Vec<Command> {
        match (self.desired_hand(), self.hand) {
            (Some(wanted), Some(current))
                if self.live && wanted != current && !self.hand_requested =>
            {
                self.hand_requested = true;
                vec![Command::SetHand(wanted)]
            }
            _ => Vec::new(),
        }
    }

    fn reset_dial(&mut self) {
        self.dial_armed = false;
        self.dial_gate.pause();
        self.dial_router.reset();
    }

    fn suspend(&mut self) {
        self.gate.pause();
        self.reset_dial();
        self.dial_engaged = false;
    }
}
