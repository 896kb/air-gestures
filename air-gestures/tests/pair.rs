mod support;

use std::time::Duration;

use air_gestures::pair::{Outcome, follow_pairing, start_pairing};
use air_gesturesd::meta::PairError;
use air_gesturesd::status::LinkState;
use band_core::responder::SimMode;

#[tokio::test]
async fn a_pairing_is_followed_to_a_connected_owner() {
    let daemon = support::start(SimMode::Legacy, false).await;
    daemon
        .wait("the unpaired error", |s| s.error.is_some())
        .await;
    start_pairing(&daemon.socket, None).await.unwrap();
    let mut stages = Vec::new();
    let outcome = follow_pairing(&daemon.socket, |stage| stages.push(stage.to_owned()))
        .await
        .unwrap();
    assert!(
        matches!(outcome, Outcome::Paired { ref band } if band.contains("simulated")),
        "{outcome:?}"
    );
    assert!(stages.iter().any(|s| s.contains("Meta")), "{stages:?}");
    assert_eq!(
        air_gestures::client::status(&daemon.socket)
            .await
            .unwrap()
            .state,
        LinkState::Connected
    );
}

#[tokio::test]
async fn pairing_is_refused_while_a_band_is_paired() {
    let daemon = support::start(SimMode::Enrolled, true).await;
    let error = start_pairing(&daemon.socket, None).await.unwrap_err();
    assert!(error.message.contains("forget"), "{error}");
}

#[tokio::test]
async fn a_failed_pairing_is_reported() {
    let daemon = support::start(SimMode::Legacy, false).await;
    daemon.transport.set_reject_claim(Some(0x03001042));
    start_pairing(&daemon.socket, None).await.unwrap();
    let outcome = follow_pairing(&daemon.socket, |_| {}).await.unwrap();
    assert!(
        matches!(outcome, Outcome::Failed { ref message } if message.contains("different meta account")),
        "{outcome:?}"
    );
}

/// A pairing whose Meta claim finds the saved session already expired ends
/// in `Outcome::NeedsSignIn`, not a generic `Failed` — so `air-gestures pair`
/// knows to open the sign-in window and try again, rather than just giving up.
#[tokio::test]
async fn a_pairing_with_an_expired_session_needs_a_fresh_sign_in() {
    let daemon = support::start_failing(
        SimMode::Legacy,
        false,
        Some(PairError::SessionInvalid(
            "your meta session expired.".into(),
        )),
    )
    .await;
    daemon
        .wait("the unpaired error", |s| s.error.is_some())
        .await;
    start_pairing(&daemon.socket, None).await.unwrap();
    let outcome = follow_pairing(&daemon.socket, |_| {}).await.unwrap();
    assert_eq!(outcome, Outcome::NeedsSignIn, "{outcome:?}");
}

/// `follow_pairing` may be called after the pairing (and its settling
/// reconnect) has already finished — a fast, e.g. simulated, pairing easily
/// beats a fresh `subscribe` connection to the punch. It must still report
/// the outcome immediately rather than waiting out the 60 s connect timeout.
#[tokio::test]
async fn an_already_settled_pairing_is_reported_immediately() {
    let daemon = support::start(SimMode::Legacy, false).await;
    daemon
        .wait("the unpaired error", |s| s.error.is_some())
        .await;
    start_pairing(&daemon.socket, None).await.unwrap();
    daemon
        .wait("connected", |s| s.state == LinkState::Connected)
        .await;
    let outcome = tokio::time::timeout(
        Duration::from_secs(5),
        follow_pairing(&daemon.socket, |_| {}),
    )
    .await
    .expect("follow_pairing should return promptly, not wait out the connect timeout")
    .unwrap();
    assert!(
        matches!(outcome, Outcome::Paired { ref band } if band.contains("simulated")),
        "{outcome:?}"
    );
}
