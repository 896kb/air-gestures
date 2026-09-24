mod support;

use band_core::responder::SimMode;
use kinesisctl::client;
use kinesisctl::waybar::{line, next_lines};
use kinesisd::ipc::Request;
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

#[test]
fn free_form_text_is_escaped_for_pango_markup() {
    let status = Status {
        error: Some("a & <b>".into()),
        ..Status::default()
    };
    let json = line(Some(&status));
    let tooltip = json["tooltip"].as_str().unwrap();
    assert!(tooltip.contains("a &amp; &lt;b&gt;"), "{tooltip}");
    assert!(!tooltip.contains("<b>"), "{tooltip}");
}

/// Waits (real time, 20 s cap) for a `next_lines` line whose `class` contains `target`
/// (a plain string or a `[base, "low"]` array both stringify to something `contains` matches).
async fn wait_for_class(lines: &mut tokio::sync::mpsc::Receiver<String>, target: &str) -> String {
    loop {
        let line = tokio::time::timeout(std::time::Duration::from_secs(20), lines.recv())
            .await
            .unwrap()
            .unwrap();
        let json: serde_json::Value = serde_json::from_str(&line).unwrap();
        if json["class"].to_string().contains(target) {
            return line;
        }
    }
}

#[tokio::test]
async fn follow_emits_a_line_per_change() {
    let daemon = support::start(SimMode::Enrolled, true).await;
    daemon
        .wait("connected", |s| s.state == LinkState::Connected)
        .await;
    let mut lines = next_lines(daemon.socket.clone());
    wait_for_class(&mut lines, "connected").await;
    client::request(&daemon.socket, &Request::Pause)
        .await
        .unwrap();
    wait_for_class(&mut lines, "paused").await;
}

#[tokio::test]
async fn follow_reconnects_once_the_daemon_appears_without_repeating_a_line() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("kinesis.sock");
    let mut lines = next_lines(socket.clone());

    // No daemon yet: the first line is "off".
    let first = tokio::time::timeout(std::time::Duration::from_secs(20), lines.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&first).unwrap()["class"],
        "off"
    );

    // Bind a daemon at that same path; follow should notice and move on from "off".
    let _daemon = support::start_at(socket, SimMode::Enrolled, true).await;

    let mut previous = first;
    loop {
        let line = tokio::time::timeout(std::time::Duration::from_secs(20), lines.recv())
            .await
            .unwrap()
            .unwrap();
        assert_ne!(
            line, previous,
            "no two consecutive lines are ever identical"
        );
        let class = serde_json::from_str::<serde_json::Value>(&line).unwrap()["class"].clone();
        previous = line;
        if class != "off" {
            break;
        }
    }
}
