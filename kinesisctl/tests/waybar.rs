mod support;

use band_core::responder::SimMode;
use kinesisctl::waybar::{line, next_lines};
use kinesisd::status::{LinkState, PairResult, Status};

const HAND: &str = "\u{f256}";
const BOLT: &str = "\u{f0e7}";

#[test]
fn a_connected_band_shows_its_battery() {
    let status = Status {
        state: LinkState::Connected,
        battery: Some(64),
        band: Some("Meta Band".into()),
        ..Status::default()
    };
    let json = line(Some(&status));
    assert_eq!(json["text"], format!("{HAND} 64%"));
    assert_eq!(json["class"], "connected");
    assert!(json["tooltip"].as_str().unwrap().contains("Meta Band"));
}

#[test]
fn charging_low_paused_and_pairing_have_their_own_look() {
    let charging = Status {
        state: LinkState::Connected,
        battery: Some(12),
        charging: Some(true),
        ..Status::default()
    };
    let json = line(Some(&charging));
    assert_eq!(json["text"], format!("{HAND} 12% {BOLT}"));
    assert_eq!(json["class"], serde_json::json!(["connected", "low"]));
    let paused = Status {
        state: LinkState::Connected,
        paused: true,
        battery: Some(80),
        ..Status::default()
    };
    assert_eq!(line(Some(&paused))["class"], "paused");
    let pairing = Status {
        state: LinkState::Pairing,
        pairing: Some("confirming the claim with Meta".into()),
        ..Status::default()
    };
    let json = line(Some(&pairing));
    assert_eq!(json["class"], "pairing");
    assert!(
        json["tooltip"]
            .as_str()
            .unwrap()
            .contains("confirming the claim")
    );
}

#[test]
fn idle_or_missing_daemon_is_off_and_errors_go_in_the_tooltip() {
    let idle = Status {
        error: Some("no band paired yet — run `kinesisctl pair`".into()),
        ..Status::default()
    };
    let json = line(Some(&idle));
    assert_eq!(json["text"], HAND);
    assert_eq!(json["class"], "off");
    assert!(
        json["tooltip"]
            .as_str()
            .unwrap()
            .contains("kinesisctl pair")
    );
    let gone = line(None);
    assert_eq!(gone["class"], "off");
    assert!(
        gone["tooltip"]
            .as_str()
            .unwrap()
            .contains("kinesisd isn't running")
    );
    let failed = Status {
        state: LinkState::Connected,
        battery: Some(50),
        last_pair: Some(PairResult {
            ok: false,
            message: "no band found".into(),
        }),
        pairing_error: Some("no band found".into()),
        ..Status::default()
    };
    let json = line(Some(&failed));
    assert_eq!(
        json["class"], "connected",
        "a note doesn't make a working band look broken"
    );
    assert!(json["tooltip"].as_str().unwrap().contains("no band found"));
}

#[tokio::test]
async fn follow_emits_a_line_per_change() {
    let daemon = support::start(SimMode::Enrolled, true).await;
    let mut lines = next_lines(daemon.socket.clone());
    let mut classes = Vec::new();
    while !classes
        .iter()
        .any(|class: &String| class.contains("connected"))
    {
        let line = tokio::time::timeout(std::time::Duration::from_secs(20), lines.recv())
            .await
            .unwrap()
            .unwrap();
        let json: serde_json::Value = serde_json::from_str(&line).unwrap();
        classes.push(json["class"].to_string());
    }
    assert_ne!(
        classes.first().map(String::as_str),
        Some("\"connected\""),
        "starts from the pre-connect state"
    );
}
