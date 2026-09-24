mod support;

use band_core::responder::SimMode;
use kinesisctl::pair::{Outcome, follow_pairing, start_pairing};
use kinesisd::status::LinkState;

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
        kinesisctl::client::status(&daemon.socket)
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
    assert!(error.contains("forget"), "{error}");
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

#[test]
fn a_sign_in_request_is_recognized() {
    assert!(kinesisctl::pair::needs_sign_in(
        "sign in to Meta first (`kinesisctl pair` opens the sign-in window)"
    ));
    assert!(!kinesisctl::pair::needs_sign_in(
        "a pairing is already running"
    ));
}
