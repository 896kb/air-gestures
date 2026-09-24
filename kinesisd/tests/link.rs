use std::sync::{Arc, Mutex};
use std::time::Duration;

use band_core::ceremony::{CeremonyHttpRequest, OwnershipCeremony};
use band_core::events::{Event, Hand};
use band_core::identity::EnrollmentIdentity;
use band_core::responder::SimMode;
use band_core::session::BandSession;
use kinesisd::clock::Clock;
use kinesisd::link::{LinkCommand, LinkEnd, LinkEvent, LinkOutcome, run_link};
use kinesisd::transport::{SIM_ADDRESS, SimInject, SimTransport, Transport};
use p256::ecdsa::SigningKey;
use rand_core::OsRng;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

struct Running {
    commands: mpsc::Sender<LinkCommand>,
    events: mpsc::Receiver<LinkEvent>,
    task: JoinHandle<LinkOutcome>,
}

async fn open(transport: &SimTransport, session: BandSession) -> Running {
    open_with(transport, session, |_| Ok(())).await
}

async fn open_with(
    transport: &SimTransport,
    session: BandSession,
    persist: impl FnMut(&EnrollmentIdentity) -> std::io::Result<()> + Send + 'static,
) -> Running {
    let io = transport.open(SIM_ADDRESS).await.unwrap();
    let (commands, command_rx) = mpsc::channel(16);
    let (event_tx, events) = mpsc::channel(256);
    let task = tokio::spawn(run_link(
        io,
        session,
        Clock::new(),
        command_rx,
        event_tx,
        persist,
    ));
    Running {
        commands,
        events,
        task,
    }
}

/// Collect events until `until` matches one (fails after a minute of virtual time).
async fn wait_for(
    events: &mut mpsc::Receiver<LinkEvent>,
    until: impl Fn(&LinkEvent) -> bool,
) -> Vec<LinkEvent> {
    let mut seen = Vec::new();
    tokio::time::timeout(Duration::from_secs(60), async {
        while let Some(event) = events.recv().await {
            let done = until(&event);
            seen.push(event);
            if done {
                return;
            }
        }
        panic!("link ended before the expected event: {seen:?}");
    })
    .await
    .expect("timed out waiting for a link event");
    seen
}

fn band(event: &LinkEvent, test: impl Fn(&Event) -> bool) -> bool {
    matches!(event, LinkEvent::Band(inner) if test(inner))
}

#[tokio::test(start_paused = true)]
async fn a_legacy_band_connects_and_reports_hand_and_battery() {
    let transport = SimTransport::new(SimMode::Legacy);
    let mut link = open(&transport, BandSession::new(None, None, false)).await;
    wait_for(&mut link.events, |e| {
        band(e, |e| matches!(e, Event::Connected))
    })
    .await;
    wait_for(&mut link.events, |e| {
        band(e, |e| matches!(e, Event::Handedness(Hand::Right)))
    })
    .await;
    wait_for(&mut link.events, |e| {
        band(e, |e| matches!(e, Event::BatteryStatus(Some(_))))
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn injected_gestures_arrive_and_stop_is_acknowledged() {
    let transport = SimTransport::new(SimMode::Legacy);
    let mut link = open(&transport, BandSession::new(None, None, false)).await;
    wait_for(&mut link.events, |e| {
        band(e, |e| matches!(e, Event::Connected))
    })
    .await;
    assert!(transport.inject(SimInject::Gesture {
        action: 8,
        finger: 1,
        derived: 0
    }));
    wait_for(&mut link.events, |e| {
        band(e, |e| matches!(e, Event::Gesture(g) if g.action == "left"))
    })
    .await;
    link.commands.send(LinkCommand::Stop).await.unwrap();
    assert_eq!(link.task.await.unwrap().end, LinkEnd::Closed);
}

#[tokio::test(start_paused = true)]
async fn a_band_that_goes_quiet_is_given_up_on() {
    let transport = SimTransport::new(SimMode::Legacy);
    let mut link = open(&transport, BandSession::new(None, None, false)).await;
    wait_for(&mut link.events, |e| {
        band(e, |e| matches!(e, Event::Connected))
    })
    .await;
    transport.inject(SimInject::Silence);
    let outcome = tokio::time::timeout(Duration::from_secs(30), link.task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outcome.end, LinkEnd::Stalled);
}

#[tokio::test(start_paused = true)]
async fn a_dropped_connection_ends_the_link() {
    let transport = SimTransport::new(SimMode::Legacy);
    let mut link = open(&transport, BandSession::new(None, None, false)).await;
    wait_for(&mut link.events, |e| {
        band(e, |e| matches!(e, Event::Connected))
    })
    .await;
    transport.inject(SimInject::Close);
    assert_eq!(link.task.await.unwrap().end, LinkEnd::StreamEnded);
}

#[tokio::test(start_paused = true)]
async fn a_band_that_never_answers_times_out() {
    let (host, _band) = tokio::io::duplex(1024);
    let (_commands, command_rx) = mpsc::channel(4);
    let (event_tx, _events) = mpsc::channel(4);
    let session = BandSession::new(None, None, false);
    let outcome = run_link(
        host,
        session,
        Clock::new(),
        command_rx,
        event_tx,
        |_| Ok(()),
    )
    .await;
    assert_eq!(outcome.end, LinkEnd::StartupTimeout);
}

#[tokio::test(start_paused = true)]
async fn a_rejected_identity_is_reported_as_such() {
    let transport = SimTransport::new(SimMode::Enrolled);
    transport.set_reject_identity(Some(0x03001043));
    let identity = EnrollmentIdentity::new(SigningKey::random(&mut OsRng), None);
    let link = open(&transport, BandSession::new(Some(identity), None, false)).await;
    assert!(matches!(
        link.task.await.unwrap().end,
        LinkEnd::IdentityRejected(_)
    ));
}

#[tokio::test(start_paused = true)]
async fn a_hand_change_before_the_band_is_ready_is_refused_not_fatal() {
    let transport = SimTransport::new(SimMode::Legacy);
    let mut link = open(&transport, BandSession::new(None, None, false)).await;
    link.commands
        .send(LinkCommand::SetHand(Hand::Left))
        .await
        .unwrap();
    wait_for(&mut link.events, |e| matches!(e, LinkEvent::Refused(_))).await;
    wait_for(&mut link.events, |e| {
        band(e, |e| matches!(e, Event::Connected))
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_ceremony_persists_the_pending_identity_and_hands_out_the_final_one() {
    let transport = SimTransport::new(SimMode::Legacy);
    transport.prepare_pairing(SIM_ADDRESS).await.unwrap();
    let persisted = Arc::new(Mutex::new(Vec::new()));
    let sink = persisted.clone();
    let session = BandSession::new(None, Some(OwnershipCeremony::default()), false);
    let mut link = open_with(&transport, session, move |identity| {
        sink.lock().unwrap().push(identity.private_bytes());
        Ok(())
    })
    .await;
    let mut completed = None;
    loop {
        let event = tokio::time::timeout(Duration::from_secs(60), link.events.recv())
            .await
            .unwrap()
            .unwrap();
        match event {
            LinkEvent::Band(Event::CeremonyHttp(CeremonyHttpRequest::PairRequest(_))) => {
                link.commands
                    .send(LinkCommand::ResumePairRequest {
                        signature: vec![0x30, 1],
                        receipt: "{\"p\":1}".into(),
                    })
                    .await
                    .unwrap();
            }
            LinkEvent::Band(Event::CeremonyHttp(CeremonyHttpRequest::Pair(_))) => {
                assert!(persisted.lock().unwrap().is_empty());
                link.commands
                    .send(LinkCommand::ResumePair {
                        signature: vec![0x30, 2],
                        receipt: "{\"f\":1}".into(),
                        device_public_key: Some(transport.band_point()),
                    })
                    .await
                    .unwrap();
            }
            LinkEvent::Band(Event::CeremonyCompleted(identity)) => {
                completed = Some(identity.private_bytes())
            }
            LinkEvent::Band(Event::Connected) => break,
            _ => {}
        }
    }
    assert_eq!(persisted.lock().unwrap().as_slice(), &[completed.unwrap()]);
    link.commands.send(LinkCommand::Stop).await.unwrap();
    let outcome = link.task.await.unwrap();
    assert_eq!(outcome.end, LinkEnd::Closed);
    assert_eq!(
        outcome.enrollment.unwrap().private_bytes(),
        completed.unwrap()
    );
}

#[tokio::test(start_paused = true)]
async fn failing_to_persist_the_pending_identity_aborts_before_finishing() {
    let transport = SimTransport::new(SimMode::Legacy);
    transport.prepare_pairing(SIM_ADDRESS).await.unwrap();
    let session = BandSession::new(None, Some(OwnershipCeremony::default()), false);
    let mut link = open_with(&transport, session, |_| {
        Err(std::io::Error::other("disk full"))
    })
    .await;
    loop {
        match tokio::time::timeout(Duration::from_secs(60), link.events.recv())
            .await
            .unwrap()
        {
            Some(LinkEvent::Band(Event::CeremonyHttp(CeremonyHttpRequest::PairRequest(_)))) => {
                link.commands
                    .send(LinkCommand::ResumePairRequest {
                        signature: vec![0x30],
                        receipt: "{}".into(),
                    })
                    .await
                    .unwrap();
            }
            Some(LinkEvent::Band(Event::CeremonyHttp(CeremonyHttpRequest::Pair(_)))) => {
                link.commands
                    .send(LinkCommand::ResumePair {
                        signature: vec![0x30],
                        receipt: "{}".into(),
                        device_public_key: None,
                    })
                    .await
                    .unwrap();
            }
            Some(LinkEvent::Band(Event::CeremonyCompleted(_))) => {
                panic!("the band must never be asked to finish")
            }
            Some(_) => {}
            None => break,
        }
    }
    let outcome = link.task.await.unwrap();
    assert!(matches!(outcome.end, LinkEnd::Failed(message) if message.contains("disk full")));
    assert!(outcome.enrollment.is_none());
}
