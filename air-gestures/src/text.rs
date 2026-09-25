//! Human-readable status for `air-gestures status`.

use air_gesturesd::config::DialTarget;
use air_gesturesd::status::{LinkState, Status};

fn state(status: &Status) -> &'static str {
    match status.state {
        LinkState::Idle => "not connected",
        LinkState::Connecting => "connecting",
        LinkState::Connected => "connected",
        LinkState::Pairing => "pairing",
    }
}

pub fn describe(status: &Status) -> String {
    let mut lines = Vec::new();
    let band = status.band.as_deref().unwrap_or("no band paired");
    lines.push(format!("{band}: {}", state(status)));
    if let Some(level) = status.battery {
        let charging = if status.charging == Some(true) {
            ", charging"
        } else {
            ""
        };
        lines.push(format!("battery {level}%{charging}"));
    }
    if let Some(hand) = &status.hand {
        lines.push(format!("hand: {hand}"));
    }
    let dial = match status.dial {
        DialTarget::Volume => "volume",
        DialTarget::Brightness => "brightness",
        DialTarget::Off => "off",
    };
    lines.push(format!(
        "dial: {dial}{}",
        if status.paused {
            " · controls paused"
        } else {
            ""
        }
    ));
    if let Some(gesture) = &status.last_gesture {
        lines.push(format!(
            "last gesture: {gesture} ({} this run)",
            status.gestures
        ));
    }
    if let Some(stage) = &status.pairing {
        lines.push(format!("pairing: {stage}"));
    }
    if let Some(seconds) = status.reconnect_in {
        lines.push(format!("reconnecting in {seconds} s"));
    }
    for (label, message) in [
        ("error", &status.error),
        ("pairing failed", &status.pairing_error),
    ] {
        if let Some(message) = message {
            lines.push(format!("{label}: {message}"));
        }
    }
    // Already reads "config: ..." (the daemon prefixes it itself).
    if let Some(message) = &status.config_error {
        lines.push(message.clone());
    }
    lines.join("\n")
}
