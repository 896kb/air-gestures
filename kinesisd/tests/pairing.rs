use std::sync::{Arc, Mutex};
use std::time::Duration;

use band_core::events::Event;
use band_core::responder::SimMode;
use band_core::session::BandSession;
use kinesisd::clock::Clock;
use kinesisd::link::{LinkEvent, run_link};
use kinesisd::meta::{ClientFactory, FakeFactory, PairError};
use kinesisd::pairing::{PairFailure, pair_band};
use kinesisd::store::Store;
use kinesisd::transport::{SIM_ADDRESS, SimTransport, Transport};
use tokio::sync::mpsc;

fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("state")).unwrap();
    (dir, store)
}

#[tokio::test(start_paused = true)]
async fn a_claim_saves_the_owner_key_and_band_then_connects_as_owner() {
    let transport = SimTransport::new(SimMode::Legacy);
    let client = FakeFactory {
        device_point: Some(transport.band_point()),
        fail: None,
    }
    .client(None)
    .unwrap();
    let (_dir, store) = store();
    let stages = Arc::new(Mutex::new(Vec::new()));
    let sink = stages.clone();
    let record = pair_band(
        &transport,
        &client,
        &store,
        None,
        Clock::new(),
        move |stage: &str| sink.lock().unwrap().push(stage.to_owned()),
    )
    .await
    .unwrap();
    assert_eq!(record.address, SIM_ADDRESS);
    assert_eq!(store.load_band().unwrap(), Some(record));
    assert!(store.load_pending().unwrap().is_none());
    let identity = store.load_identity().unwrap().expect("owner key saved");
    assert!(identity.band_public_key.is_some());
    assert!(
        stages
            .lock()
            .unwrap()
            .iter()
            .any(|stage| stage.contains("Meta"))
    );

    // The next connection authenticates as the new owner.
    let io = transport.open(SIM_ADDRESS).await.unwrap();
    let (_commands, command_rx) = mpsc::channel(4);
    let (event_tx, mut events) = mpsc::channel(64);
    tokio::spawn(run_link(
        io,
        BandSession::new(Some(identity), None, false),
        Clock::new(),
        command_rx,
        event_tx,
        |_| Ok(()),
    ));
    tokio::time::timeout(Duration::from_secs(30), async {
        while let Some(event) = events.recv().await {
            if matches!(event, LinkEvent::Band(Event::Connected)) {
                return;
            }
        }
        panic!("link ended");
    })
    .await
    .unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_band_owned_by_another_account_is_reported_and_nothing_is_kept() {
    let transport = SimTransport::new(SimMode::Legacy);
    transport.set_reject_claim(Some(0x03001042));
    let client = FakeFactory {
        device_point: None,
        fail: None,
    }
    .client(None)
    .unwrap();
    let (_dir, store) = store();
    let failure = pair_band(
        &transport,
        &client,
        &store,
        None,
        Clock::new(),
        |_: &str| {},
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&failure, PairFailure::WrongAccount(message) if message.contains("different meta account")),
        "{failure:?}"
    );
    assert!(store.load_identity().unwrap().is_none());
    assert!(store.load_band().unwrap().is_none());
}

#[tokio::test(start_paused = true)]
async fn an_expired_meta_session_asks_for_a_fresh_sign_in() {
    let transport = SimTransport::new(SimMode::Legacy);
    let fail = Some(PairError::SessionInvalid(
        "your meta session expired.".into(),
    ));
    let client = FakeFactory {
        device_point: None,
        fail,
    }
    .client(None)
    .unwrap();
    let (_dir, store) = store();
    let failure = pair_band(
        &transport,
        &client,
        &store,
        None,
        Clock::new(),
        |_: &str| {},
    )
    .await
    .unwrap_err();
    assert!(matches!(failure, PairFailure::SessionExpired(_)));
    assert!(store.load_identity().unwrap().is_none() && store.load_pending().unwrap().is_none());
}

#[tokio::test(start_paused = true)]
async fn a_band_that_isnt_there_is_not_found() {
    let transport = SimTransport::new(SimMode::Legacy);
    let client = FakeFactory {
        device_point: None,
        fail: None,
    }
    .client(None)
    .unwrap();
    let (_dir, store) = store();
    let failure = pair_band(
        &transport,
        &client,
        &store,
        Some("11:22:33:44:55:66"),
        Clock::new(),
        |_: &str| {},
    )
    .await
    .unwrap_err();
    assert_eq!(failure, PairFailure::NoBandFound);
}

#[tokio::test(start_paused = true)]
async fn a_lost_confirmation_keeps_the_pending_key_and_band() {
    let transport = SimTransport::new(SimMode::Legacy);
    transport.set_drop_after_finish(true);
    let client = FakeFactory {
        device_point: Some(transport.band_point()),
        fail: None,
    }
    .client(None)
    .unwrap();
    let (_dir, store) = store();
    let failure = pair_band(
        &transport,
        &client,
        &store,
        None,
        Clock::new(),
        |_: &str| {},
    )
    .await
    .unwrap_err();
    assert!(
        !matches!(failure, PairFailure::WrongAccount(_)),
        "{failure:?}"
    );
    let pending = store.load_pending().unwrap().expect("pending key kept");
    assert!(store.load_band().unwrap().is_some());
    assert!(store.load_identity().unwrap().is_none());

    // The band committed the claim: the pending key connects as its owner.
    let io = transport.open(SIM_ADDRESS).await.unwrap();
    let (_commands, command_rx) = mpsc::channel(4);
    let (event_tx, mut events) = mpsc::channel(64);
    tokio::spawn(run_link(
        io,
        BandSession::new(Some(pending), None, false),
        Clock::new(),
        command_rx,
        event_tx,
        |_| Ok(()),
    ));
    tokio::time::timeout(Duration::from_secs(30), async {
        while let Some(event) = events.recv().await {
            if matches!(event, LinkEvent::Band(Event::Connected)) {
                return;
            }
        }
        panic!("link ended");
    })
    .await
    .unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_stale_pending_key_does_not_keep_a_failed_claims_band() {
    let transport = SimTransport::new(SimMode::Legacy);
    let fail = Some(PairError::SessionInvalid(
        "your meta session expired.".into(),
    ));
    let client = FakeFactory {
        device_point: None,
        fail,
    }
    .client(None)
    .unwrap();
    let (_dir, store) = store();
    let stale = band_core::identity::EnrollmentIdentity::new(
        p256::ecdsa::SigningKey::random(&mut rand_core::OsRng),
        None,
    );
    store.save_pending(&stale).unwrap();
    let failure = pair_band(
        &transport,
        &client,
        &store,
        None,
        Clock::new(),
        |_: &str| {},
    )
    .await
    .unwrap_err();
    assert!(matches!(failure, PairFailure::SessionExpired(_)));
    assert!(store.load_band().unwrap().is_none());
}

#[tokio::test(start_paused = true)]
async fn a_claim_rejected_at_finish_restores_the_previous_band() {
    let transport = SimTransport::new(SimMode::Legacy);
    transport.set_reject_finish(Some(0x03001045));
    let client = FakeFactory {
        device_point: Some(transport.band_point()),
        fail: None,
    }
    .client(None)
    .unwrap();
    let (_dir, store) = store();
    let previous = kinesisd::store::BandRecord {
        address: "11:22:33:44:55:66".into(),
        name: "Meta Band A".into(),
    };
    let old = band_core::identity::EnrollmentIdentity::new(
        p256::ecdsa::SigningKey::random(&mut rand_core::OsRng),
        None,
    );
    store.save_band(&previous).unwrap();
    store.save_identity(&old).unwrap();
    let failure = pair_band(
        &transport,
        &client,
        &store,
        None,
        Clock::new(),
        |_: &str| {},
    )
    .await
    .unwrap_err();
    assert!(matches!(failure, PairFailure::Rejected(_)), "{failure:?}");
    assert_eq!(store.load_band().unwrap(), Some(previous));
    assert!(store.load_pending().unwrap().is_none());
    assert_eq!(
        store.load_identity().unwrap().unwrap().private_bytes(),
        old.private_bytes()
    );
}
