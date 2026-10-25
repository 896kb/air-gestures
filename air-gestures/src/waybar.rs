//! The waybar `custom/air-gestures` module: one JSON line per state.

use std::path::PathBuf;
use std::time::Duration;

use air_gesturesd::config::DialTarget;
use air_gesturesd::status::{LinkState, Status};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::client::Subscription;

/// Material Design hand-back-right and its struck-through "off" twin (both
/// in Nerd Fonts, so the two states match), and Font Awesome's bolt.
const HAND: &str = "\u{f0a4f}";
const HAND_OFF: &str = "\u{f1831}";
const BOLT: &str = "\u{f0e7}";

pub fn line(status: Option<&Status>) -> Value {
    let Some(status) = status else {
        return json!({
            "text": HAND,
            "tooltip": "air-gesturesd isn't running (systemctl --user start air-gesturesd)",
            "class": "off",
            "alt": "off",
        });
    };
    // Controls off: the hand is struck through.
    let hand = if status.state == LinkState::Connected && status.paused {
        HAND_OFF
    } else {
        HAND
    };
    let mut text = hand.to_string();
    if status.state == LinkState::Connected
        && let Some(level) = status.battery
    {
        text.push_str(&format!(" {level}%"));
        if status.charging == Some(true) {
            text.push(' ');
            text.push_str(BOLT);
        }
    }
    let base = match status.state {
        LinkState::Connected if status.paused => "paused",
        LinkState::Connected => "connected",
        LinkState::Connecting => "connecting",
        LinkState::Pairing => "pairing",
        LinkState::Idle => "off",
    };
    let low =
        status.state == LinkState::Connected && status.battery.is_some_and(|level| level < 20);
    let class = if low {
        json!([base, "low"])
    } else {
        json!(base)
    };
    json!({ "text": escape(&text), "tooltip": tooltip(status), "class": class, "alt": base })
}

/// Waybar renders `text` and `tooltip` as Pango markup: anything free-form
/// (band name, error text, pairing stage, gesture name) must be escaped so a
/// literal `&`, `<` or `>` in it can't be read as markup.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn tooltip(status: &Status) -> String {
    let mut lines = vec![escape(status.band.as_deref().unwrap_or("Neural Band"))];
    lines.push(match status.state {
        LinkState::Idle => "not connected".into(),
        LinkState::Connecting => "connecting…".into(),
        LinkState::Connected if status.paused => "connected · controls paused".into(),
        LinkState::Connected => "connected".into(),
        LinkState::Pairing => format!(
            "pairing: {}",
            escape(status.pairing.as_deref().unwrap_or("…"))
        ),
    });
    if let Some(hand) = &status.hand {
        lines.push(format!("hand: {}", escape(hand)));
    }
    let dial = match status.dial {
        DialTarget::Volume => "volume",
        DialTarget::Brightness => "brightness",
        DialTarget::Off => "off",
    };
    lines.push(format!("dial: {dial}"));
    if let Some(profile) = &status.profile {
        lines.push(format!("profile: {}", escape(profile)));
    }
    if let Some(gesture) = &status.last_gesture {
        lines.push(format!("last: {} ({})", escape(gesture), status.gestures));
    }
    if let Some(seconds) = status.reconnect_in {
        lines.push(format!("reconnecting in {seconds} s"));
    }
    for message in [&status.error, &status.pairing_error, &status.config_error]
        .into_iter()
        .flatten()
    {
        lines.push(format!("⚠ {}", escape(message)));
    }
    lines.push("click: connect/disconnect · right: pause · middle: dial".into());
    lines.join("\n")
}

/// Status lines for waybar, forever: re-subscribes every 2 s while air-gesturesd is down.
pub fn next_lines(socket: PathBuf) -> mpsc::Receiver<String> {
    let (sender, receiver) = mpsc::channel(16);
    tokio::spawn(async move {
        let mut last = String::new();
        loop {
            if let Ok(mut subscription) = Subscription::status(&socket).await {
                while let Some(status) = subscription.next_status().await {
                    let text = line(Some(&status)).to_string();
                    if text != last {
                        last = text.clone();
                        if sender.send(text).await.is_err() {
                            return;
                        }
                    }
                }
            }
            let text = line(None).to_string();
            if text != last {
                last = text.clone();
                if sender.send(text).await.is_err() {
                    return;
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
    receiver
}
