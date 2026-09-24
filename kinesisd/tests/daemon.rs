use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use band_core::ceremony::{CeremonyPair, CeremonyPairRequest};
use band_core::identity::{EnrollmentIdentity, verifying_key_from_point64};
use band_core::responder::SimMode;
use kinesisd::actions::RecordingRunner;
use kinesisd::daemon::Daemon;
use kinesisd::ipc::{Call, HandChoice, Request, Response};
use kinesisd::meta::{
    ClientFactory, FakeFactory, Final, HttpFactory, MetaSession, PairClient, PairError, Pending,
};
use kinesisd::status::{LinkState, Status};
use kinesisd::store::{BandRecord, Store};
use kinesisd::transport::{SIM_ADDRESS, SIM_NAME, SimInject, SimTransport};
use p256::ecdsa::SigningKey;
use rand_core::OsRng;
use tokio::sync::{mpsc, oneshot, watch};

const SWIPE_LEFT: SimInject = SimInject::Gesture {
    action: 8,
    finger: 1,
    derived: 0,
};

struct Harness {
    calls: mpsc::Sender<Call>,
    status: watch::Receiver<Status>,
    runner: RecordingRunner,
    transport: SimTransport,
    store: Store,
    config_path: PathBuf,
    _dir: tempfile::TempDir,
}

fn start<F: ClientFactory>(transport: SimTransport, paired: bool, factory: F) -> Harness {
    start_seeded(transport, paired, factory, |_| {})
}

/// Like `start`, with `seed` run on the store before the daemon exists.
fn start_seeded<F: ClientFactory>(
    transport: SimTransport,
    paired: bool,
    factory: F,
    seed: impl FnOnce(&Store),
) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("state")).unwrap();
    let config_path = dir.path().join("config.toml");
    if paired {
        store
            .save_band(&BandRecord {
                address: SIM_ADDRESS.into(),
                name: SIM_NAME.into(),
            })
            .unwrap();
        let band_key = verifying_key_from_point64(&transport.band_point()).unwrap();
        let identity = EnrollmentIdentity::new(SigningKey::random(&mut OsRng), Some(band_key));
        store.save_identity(&identity).unwrap();
        transport.set_owner(&identity);
    }
    seed(&store);
    let runner = RecordingRunner::default();
    let daemon = Daemon::new(
        Arc::new(transport.clone()),
        runner.clone(),
        factory,
        store.clone(),
        config_path.clone(),
    );
    let status = daemon.status();
    let (calls, call_rx) = mpsc::channel(8);
    tokio::spawn(daemon.run(call_rx, std::future::pending()));
    Harness {
        calls,
        status,
        runner,
        transport,
        store,
        config_path,
        _dir: dir,
    }
}

fn fake(transport: &SimTransport) -> FakeFactory {
    FakeFactory {
        device_point: Some(transport.band_point()),
        fail: None,
    }
}

impl Harness {
    async fn call(&self, request: Request) -> Response {
        let (reply, response) = oneshot::channel();
        self.calls.send((request, reply)).await.unwrap();
        response.await.unwrap()
    }

    async fn wait(&mut self, what: &str, until: impl Fn(&Status) -> bool) -> Status {
        let found = tokio::time::timeout(Duration::from_secs(120), async {
            loop {
                let current = self.status.borrow_and_update().clone();
                if until(&current) {
                    return current;
                }
                self.status.changed().await.unwrap();
            }
        })
        .await;
        found.unwrap_or_else(|_| {
            panic!(
                "timed out waiting for {what}; last status {:?}",
                self.status.borrow()
            )
        })
    }

    /// Let (paused) time run until the band has been opened `count` times.
    async fn opened(&self, count: usize) {
        for _ in 0..600 {
            if self.transport.opens() >= count {
                return;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        panic!(
            "only {} opens; last status {:?}",
            self.transport.opens(),
            self.status.borrow()
        );
    }

    async fn connected(&mut self) {
        self.wait("connected", |s| s.state == LinkState::Connected)
            .await;
    }
}

#[tokio::test(start_paused = true)]
async fn without_a_paired_band_it_says_how_to_pair() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), false, fake(&transport));
    let status = daemon.wait("an error", |s| s.error.is_some()).await;
    assert!(status.error.unwrap().contains("kinesisctl pair"));
    assert_eq!(status.state, LinkState::Idle);
}

#[tokio::test(start_paused = true)]
async fn a_paired_band_connects_and_gestures_run_their_commands() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    assert_eq!(daemon.status.borrow().band.as_deref(), Some(SIM_NAME));
    daemon.transport.inject(SWIPE_LEFT);
    daemon
        .wait("the swipe", |s| {
            s.last_action.as_deref() == Some("hyprctl dispatch workspace e-1")
        })
        .await;
    assert_eq!(
        daemon.runner.commands(),
        vec!["hyprctl dispatch workspace e-1".to_string()]
    );
    daemon.wait("battery", |s| s.battery == Some(80)).await;
}

#[tokio::test(start_paused = true)]
async fn pausing_stops_actions_until_resumed() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    assert!(daemon.call(Request::Pause).await.status.unwrap().paused);
    daemon.transport.inject(SWIPE_LEFT);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(daemon.runner.commands().is_empty());
    daemon.call(Request::Resume).await;
    daemon.transport.inject(SWIPE_LEFT);
    daemon.wait("the swipe", |s| s.last_action.is_some()).await;
    assert_eq!(daemon.runner.commands().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_dropped_band_is_reconnected() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    daemon.transport.inject(SimInject::Close);
    daemon
        .wait("the drop", |s| s.state != LinkState::Connected)
        .await;
    daemon.connected().await;
    assert_eq!(daemon.transport.opens(), 2);
}

#[tokio::test(start_paused = true)]
async fn pairing_claims_the_band_then_connects_as_its_owner() {
    let transport = SimTransport::new(SimMode::Legacy);
    let mut daemon = start(transport.clone(), false, fake(&transport));
    daemon
        .wait("the unpaired error", |s| s.error.is_some())
        .await;
    let response = daemon.call(Request::Pair { address: None }).await;
    assert!(response.ok, "{response:?}");
    assert!(response.message.unwrap().contains("pairing mode"));
    daemon.connected().await;
    assert!(
        daemon
            .store
            .load_identity()
            .unwrap()
            .unwrap()
            .band_public_key
            .is_some()
    );
    assert_eq!(
        daemon.store.load_band().unwrap().unwrap().address,
        SIM_ADDRESS
    );
    assert!(daemon.status.borrow().error.is_none());
}

#[tokio::test(start_paused = true)]
async fn pairing_without_a_meta_session_asks_to_sign_in() {
    let transport = SimTransport::new(SimMode::Legacy);
    let daemon = start(transport, false, HttpFactory);
    let response = daemon.call(Request::Pair { address: None }).await;
    assert!(!response.ok);
    assert!(response.error.unwrap().contains("sign in"));
}

#[tokio::test(start_paused = true)]
async fn an_edited_config_applies_without_a_restart() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    std::fs::write(
        &daemon.config_path,
        "[gestures]\nswipe_left = \"echo reloaded\"\n",
    )
    .unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    daemon.transport.inject(SWIPE_LEFT);
    daemon
        .wait("the reloaded command", |s| {
            s.last_action.as_deref() == Some("echo reloaded")
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn a_broken_config_is_reported_and_the_old_one_kept() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    std::fs::write(&daemon.config_path, "[gestures]\nswipe_sideways = \"x\"\n").unwrap();
    let status = daemon
        .wait("the config error", |s| {
            s.error.as_deref().is_some_and(|e| e.contains("config"))
        })
        .await;
    assert_eq!(status.state, LinkState::Connected);
    daemon.transport.inject(SWIPE_LEFT);
    daemon
        .wait("the old command", |s| {
            s.last_action.as_deref() == Some("hyprctl dispatch workspace e-1")
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn forgetting_removes_everything_and_stops() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    let response = daemon.call(Request::Forget).await;
    assert!(response.message.unwrap().contains("factory reset"));
    assert_eq!(response.status.unwrap().state, LinkState::Idle);
    assert!(daemon.store.load_band().unwrap().is_none());
    assert!(daemon.store.connect_identity().unwrap().is_none());
    tokio::time::sleep(Duration::from_secs(30)).await;
    assert_eq!(
        daemon.transport.opens(),
        1,
        "no reconnects after forgetting"
    );
}

#[tokio::test(start_paused = true)]
async fn a_pending_key_the_band_accepts_becomes_the_confirmed_one() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), false, fake(&transport));
    daemon
        .store
        .save_band(&BandRecord {
            address: SIM_ADDRESS.into(),
            name: SIM_NAME.into(),
        })
        .unwrap();
    let band_key = verifying_key_from_point64(&transport.band_point()).unwrap();
    let pending = EnrollmentIdentity::new(SigningKey::random(&mut OsRng), Some(band_key));
    daemon.store.save_pending(&pending).unwrap();
    transport.set_owner(&pending);
    daemon.call(Request::Connect).await;
    daemon.connected().await;
    assert!(daemon.store.load_pending().unwrap().is_none());
    assert_eq!(
        daemon
            .store
            .load_identity()
            .unwrap()
            .unwrap()
            .private_bytes(),
        pending.private_bytes()
    );
}

#[tokio::test(start_paused = true)]
async fn a_pending_key_the_band_never_committed_falls_back_to_the_confirmed_one() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    daemon.call(Request::Disconnect).await;
    let band_key = verifying_key_from_point64(&transport.band_point()).unwrap();
    daemon
        .store
        .save_pending(&EnrollmentIdentity::new(
            SigningKey::random(&mut OsRng),
            Some(band_key),
        ))
        .unwrap();
    daemon.call(Request::Connect).await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    daemon.connected().await;
    assert!(
        daemon.store.load_pending().unwrap().is_none(),
        "the stale pending key is dropped"
    );
    assert!(daemon.status.borrow().error.is_none());
}

#[tokio::test(start_paused = true)]
async fn a_rejected_owner_key_stops_retrying() {
    let transport = SimTransport::new(SimMode::Enrolled);
    transport.set_reject_identity(Some(0x03001043));
    let mut daemon = start(transport.clone(), true, fake(&transport));
    let status = daemon
        .wait("the rejection", |s| {
            s.error
                .as_deref()
                .is_some_and(|e| e.contains("claim the band again"))
        })
        .await;
    assert_eq!(status.state, LinkState::Idle);
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(daemon.transport.opens(), 1);
}

#[tokio::test(start_paused = true)]
async fn the_hand_can_be_switched_from_the_socket() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    daemon
        .wait("the band's hand", |s| s.hand.as_deref() == Some("right"))
        .await;
    assert!(
        daemon
            .call(Request::Hand {
                hand: HandChoice::Left
            })
            .await
            .ok
    );
    daemon
        .wait("the new hand", |s| s.hand.as_deref() == Some("left"))
        .await;
}

#[tokio::test(start_paused = true)]
async fn disconnect_and_connect_on_request() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    assert_eq!(
        daemon.call(Request::Disconnect).await.status.unwrap().state,
        LinkState::Idle
    );
    tokio::time::sleep(Duration::from_secs(30)).await;
    assert_eq!(daemon.transport.opens(), 1);
    daemon.call(Request::Connect).await;
    daemon.connected().await;
}

/// A Meta client that never answers, so a pairing stays running.
struct Hanging;

impl PairClient for Hanging {
    async fn pair_request(&self, _request: &CeremonyPairRequest) -> Result<Pending, PairError> {
        std::future::pending().await
    }

    async fn pair(&self, _pair: &CeremonyPair) -> Result<Final, PairError> {
        std::future::pending().await
    }
}

struct HangingFactory;

impl ClientFactory for HangingFactory {
    type Client = Hanging;
    fn client(&self, _session: Option<MetaSession>) -> Result<Hanging, Option<String>> {
        Ok(Hanging)
    }
}

#[tokio::test(start_paused = true)]
async fn pairing_is_refused_while_a_band_is_paired() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    let response = daemon.call(Request::Pair { address: None }).await;
    assert!(!response.ok, "{response:?}");
    let error = response.error.unwrap();
    assert!(
        error.contains("already paired") && error.contains("kinesisctl forget"),
        "{error}"
    );
    // The paired band's link is untouched.
    assert_eq!(daemon.status.borrow().state, LinkState::Connected);
    assert_eq!(daemon.transport.opens(), 1);
    assert!(daemon.store.load_identity().unwrap().is_some());
}

#[tokio::test(start_paused = true)]
async fn a_band_can_be_paired_again_after_forgetting_it() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    let old = daemon.store.load_identity().unwrap().unwrap();
    assert!(daemon.call(Request::Forget).await.ok);
    let response = daemon.call(Request::Pair { address: None }).await;
    assert!(response.ok, "{response:?}");
    daemon.connected().await;
    let new = daemon.store.load_identity().unwrap().unwrap();
    assert_ne!(new.private_bytes(), old.private_bytes());
}

#[tokio::test(start_paused = true)]
async fn a_failed_pairing_keeps_the_failure_visible() {
    let transport = SimTransport::new(SimMode::Legacy);
    let mut daemon = start(transport.clone(), false, fake(&transport));
    daemon
        .wait("the unpaired error", |s| s.error.is_some())
        .await;
    // No band at that address.
    let response = daemon
        .call(Request::Pair {
            address: Some("AA:BB:CC:DD:EE:FF".into()),
        })
        .await;
    assert!(response.ok, "{response:?}");
    let status = daemon
        .wait("the pairing failure", |s| {
            s.state != LinkState::Pairing
                && s.error
                    .as_deref()
                    .is_some_and(|e| e.contains("no band in pairing mode"))
        })
        .await;
    assert_eq!(status.state, LinkState::Idle);
    assert_eq!(daemon.transport.opens(), 0);
}

#[tokio::test(start_paused = true)]
async fn forget_and_disconnect_are_refused_while_pairing() {
    let transport = SimTransport::new(SimMode::Legacy);
    let mut daemon = start(transport.clone(), false, HangingFactory);
    daemon
        .wait("the unpaired error", |s| s.error.is_some())
        .await;
    assert!(daemon.call(Request::Pair { address: None }).await.ok);
    let forget = daemon.call(Request::Forget).await;
    assert!(!forget.ok && forget.error.unwrap().contains("a pairing is running"));
    let disconnect = daemon.call(Request::Disconnect).await;
    assert!(!disconnect.ok && disconnect.error.unwrap().contains("a pairing is running"));
}

#[tokio::test(start_paused = true)]
async fn connect_without_a_paired_band_is_an_error() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let daemon = start(transport.clone(), false, fake(&transport));
    let response = daemon.call(Request::Connect).await;
    assert!(!response.ok);
    assert!(response.error.unwrap().contains("kinesisctl pair"));
}

fn random_identity(transport: &SimTransport) -> EnrollmentIdentity {
    let band_key = verifying_key_from_point64(&transport.band_point()).unwrap();
    EnrollmentIdentity::new(SigningKey::random(&mut OsRng), Some(band_key))
}

#[tokio::test(start_paused = true)]
async fn a_pending_key_rejected_with_another_code_is_kept_and_retried() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let pending = random_identity(&transport);
    let mut daemon = start_seeded(transport.clone(), true, fake(&transport), |store| {
        store.save_pending(&pending).unwrap();
    });
    // The band has committed the pending claim, but answers 0x1044 for now.
    transport.set_owner(&pending);
    transport.set_reject_identity(Some(0x03001044));
    daemon.opened(3).await;
    assert!(
        daemon.store.load_pending().unwrap().is_some(),
        "only 0x1043 proves the claim didn't commit"
    );
    assert_eq!(daemon.status.borrow().state, LinkState::Idle);
    transport.set_reject_identity(None);
    daemon.connected().await;
    assert!(daemon.store.load_pending().unwrap().is_none());
    assert_eq!(
        daemon
            .store
            .load_identity()
            .unwrap()
            .unwrap()
            .private_bytes(),
        pending.private_bytes()
    );
}

#[tokio::test(start_paused = true)]
async fn a_confirmed_key_rejected_with_another_code_is_retried() {
    let transport = SimTransport::new(SimMode::Enrolled);
    transport.set_reject_identity(Some(0x03001044));
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon
        .wait("the transient rejection", |s| {
            s.error
                .as_deref()
                .is_some_and(|e| e.contains("rejected the stored identity"))
        })
        .await;
    daemon.opened(3).await;
    transport.set_reject_identity(None);
    daemon.connected().await;
    assert!(daemon.status.borrow().error.is_none());
}
