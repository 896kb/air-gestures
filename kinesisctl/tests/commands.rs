mod support;

use band_core::responder::SimMode;
use kinesisctl::client;
use kinesisctl::text::describe;
use kinesisd::config::DialTarget;
use kinesisd::ipc::{HandChoice, Request};
use kinesisd::status::{LinkState, Status};

#[tokio::test]
async fn status_reports_a_connected_band() {
    let daemon = support::start(SimMode::Enrolled, true).await;
    let status = daemon
        .wait("connected", |s| s.state == LinkState::Connected)
        .await;
    let text = describe(&status);
    assert!(text.contains("connected"), "{text}");
    assert!(text.contains("Meta Band (simulated)"), "{text}");
}

#[tokio::test]
async fn without_a_band_the_status_says_how_to_pair() {
    let daemon = support::start(SimMode::Enrolled, false).await;
    let status = daemon.wait("the error", |s| s.error.is_some()).await;
    assert!(describe(&status).contains("kinesisctl pair"));
}

#[tokio::test]
async fn simple_requests_change_the_status() {
    let daemon = support::start(SimMode::Enrolled, true).await;
    daemon
        .wait("connected", |s| s.state == LinkState::Connected)
        .await;
    let paused = client::request(&daemon.socket, &Request::Pause)
        .await
        .unwrap();
    assert!(paused.status.unwrap().paused);
    let dial = client::request(
        &daemon.socket,
        &Request::Dial {
            target: DialTarget::Brightness,
        },
    )
    .await
    .unwrap();
    assert_eq!(dial.status.unwrap().dial, DialTarget::Brightness);
    let hand = client::request(
        &daemon.socket,
        &Request::Hand {
            hand: HandChoice::Left,
        },
    )
    .await
    .unwrap();
    assert!(hand.ok);
    daemon
        .wait("the left hand", |s| s.hand.as_deref() == Some("left"))
        .await;
}

#[tokio::test]
async fn a_daemon_error_is_returned_as_err() {
    let daemon = support::start(SimMode::Enrolled, false).await;
    let error = client::request(&daemon.socket, &Request::Connect)
        .await
        .unwrap_err();
    assert!(error.contains("kinesisctl pair"), "{error}");
}

#[tokio::test]
async fn a_missing_daemon_is_a_clear_error() {
    let dir = tempfile::tempdir().unwrap();
    let error = client::status(&dir.path().join("nope.sock"))
        .await
        .unwrap_err();
    assert!(error.contains("kinesisd isn't running"), "{error}");
}

#[tokio::test]
async fn subscriptions_deliver_changes() {
    let daemon = support::start(SimMode::Enrolled, true).await;
    let mut subscription = client::Subscription::status(&daemon.socket).await.unwrap();
    let first: Status = subscription.next_status().await.unwrap();
    assert_eq!(first.version, 1);

    let mut state = first.state;
    while state != LinkState::Connected {
        let next = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            subscription.next_status(),
        )
        .await
        .unwrap()
        .unwrap();
        state = next.state;
    }

    // Now that we're connected (whether the first status already was, or we
    // waited for it), trigger a change and make sure the subscription
    // actually reports it, rather than replaying a status that predates it.
    client::request(&daemon.socket, &Request::Pause)
        .await
        .unwrap();
    loop {
        let next = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            subscription.next_status(),
        )
        .await
        .unwrap()
        .unwrap();
        if next.paused {
            break;
        }
    }
}

#[tokio::test]
async fn a_rejected_subscribe_does_not_hang() {
    use tokio::io::AsyncWriteExt;

    let dir = tempfile::tempdir().unwrap();
    let socket_path = dir.path().join("reject.sock");
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (_read, mut write) = stream.into_split();
        write
            .write_all(b"{\"ok\":false,\"error\":\"bad request\"}\n")
            .await
            .unwrap();
        // Keep the connection open: a prompt `None` here must come from
        // seeing `ok: false`, not from the daemon hanging up (EOF).
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    });

    let mut subscription = client::Subscription::status(&socket_path).await.unwrap();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        subscription.next_status(),
    )
    .await
    .expect("next_status() should return promptly on a rejected subscribe, not hang");
    assert!(result.is_none());
    server.abort();
}

#[test]
fn the_text_status_shows_battery_hand_dial_and_pause() {
    let status = Status {
        state: LinkState::Connected,
        battery: Some(64),
        charging: Some(true),
        hand: Some("right".into()),
        paused: true,
        band: Some("Meta Band 1234".into()),
        ..Status::default()
    };
    let text = describe(&status);
    for part in [
        "Meta Band 1234",
        "connected",
        "64%",
        "charging",
        "right",
        "volume",
        "paused",
    ] {
        assert!(text.contains(part), "missing {part} in {text}");
    }
}
