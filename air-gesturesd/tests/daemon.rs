use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use air_gesturesd::actions::RecordingRunner;
use air_gesturesd::config::DialTarget;
use air_gesturesd::daemon::Daemon;
use air_gesturesd::ipc::{Call, HandChoice, Request, Response};
use air_gesturesd::ipc::{Taught, TaughtDrags};
use air_gesturesd::store::{HandCalibration, StoredPalm};
use band_core::controls::pinch::Prototypes;
use band_core::events::Hand;
use air_gesturesd::meta::{
    ClientFactory, FakeFactory, Final, HttpFactory, MetaSession, PairClient, PairError, Pending,
};
use air_gesturesd::status::{LinkState, Status};
use air_gesturesd::store::{BandRecord, Store};
use air_gesturesd::transport::{SIM_ADDRESS, SIM_NAME, SimInject, SimTransport};
use band_core::ceremony::{CeremonyPair, CeremonyPairRequest};
use band_core::identity::{EnrollmentIdentity, verifying_key_from_point64};
use band_core::responder::SimMode;
use p256::ecdsa::SigningKey;
use rand_core::OsRng;
use tokio::sync::{broadcast, mpsc, oneshot, watch};

const SWIPE_LEFT: SimInject = SimInject::Gesture {
    action: 8,
    finger: 1,
    derived: 0,
};

struct Harness {
    calls: mpsc::Sender<Call>,
    status: watch::Receiver<Status>,
    semg: broadcast::Sender<Vec<u8>>,
    runner: RecordingRunner,
    transport: SimTransport,
    store: Store,
    config_path: PathBuf,
    drawings: PathBuf,
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
    let drawings = dir.path().join("drawings");
    if paired {
        store
            .save_band(&BandRecord {
                address: SIM_ADDRESS.into(),
                name: SIM_NAME.into(),
                scheme_guess: None,
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
        drawings.clone(),
    );
    let status = daemon.status();
    let semg = daemon.semg();
    let (calls, call_rx) = mpsc::channel(8);
    tokio::spawn(daemon.run(call_rx, std::future::pending()));
    Harness {
        calls,
        status,
        semg,
        runner,
        transport,
        store,
        config_path,
        drawings,
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
    assert!(status.error.unwrap().contains("air-gestures pair"));
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
    let status = daemon.status.borrow().clone();
    assert!(status.error.is_none());
    assert!(status.pairing_error.is_none());
    assert!(status.last_pair.is_some_and(|last| last.ok));
    assert_eq!(status.version, 1);
}

#[tokio::test(start_paused = true)]
async fn pairing_without_a_meta_session_asks_to_sign_in() {
    let transport = SimTransport::new(SimMode::Legacy);
    let daemon = start(transport, false, HttpFactory);
    let response = daemon.call(Request::Pair { address: None }).await;
    assert!(!response.ok);
    assert!(response.error.unwrap().contains("sign in"));
    assert_eq!(
        response.code.as_deref(),
        Some(air_gesturesd::ipc::CODE_NEEDS_SIGN_IN)
    );
}

#[tokio::test(start_paused = true)]
async fn a_pairing_that_finds_an_expired_session_asks_to_sign_in_again() {
    let transport = SimTransport::new(SimMode::Legacy);
    let factory = FakeFactory {
        device_point: Some(transport.band_point()),
        fail: Some(PairError::SessionInvalid(
            "your meta session expired.".into(),
        )),
    };
    let mut daemon = start(transport, false, factory);
    daemon
        .wait("the unpaired error", |s| s.error.is_some())
        .await;
    assert!(daemon.call(Request::Pair { address: None }).await.ok);
    let status = daemon
        .wait("the pairing failure", |s| s.last_pair.is_some())
        .await;
    let last = status.last_pair.unwrap();
    assert!(!last.ok && last.needs_sign_in, "{last:?}");
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
            s.config_error
                .as_deref()
                .is_some_and(|e| e.contains("config"))
        })
        .await;
    assert_eq!(status.state, LinkState::Connected);
    assert!(status.error.is_none(), "config errors are reported apart");
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
            scheme_guess: None,
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
                .is_some_and(|e| e.contains("air-gestures forget"))
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
        error.contains("already paired") && error.contains("air-gestures forget"),
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
                && s.pairing_error
                    .as_deref()
                    .is_some_and(|e| e.contains("no band in pairing mode"))
        })
        .await;
    assert_eq!(status.state, LinkState::Idle);
    assert!(status.error.is_none(), "pairing errors are reported apart");
    let last = status.last_pair.expect("the pairing's result");
    assert!(!last.ok && last.message.contains("no band in pairing mode"));
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
    assert!(response.error.unwrap().contains("air-gestures pair"));
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
    let waiting = daemon
        .wait("the countdown", |s| {
            s.state == LinkState::Idle && s.reconnect_in.is_some()
        })
        .await;
    assert!(waiting.reconnect_in.unwrap() > 0 && waiting.reconnect_in.unwrap() <= 10);
    daemon.opened(3).await;
    transport.set_reject_identity(None);
    daemon.connected().await;
    let status = daemon.status.borrow().clone();
    assert!(status.error.is_none());
    assert!(
        status.reconnect_in.is_none(),
        "no countdown while connected"
    );
}

#[tokio::test(start_paused = true)]
async fn semg_without_a_viewer_is_turned_off_again() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    assert!(daemon.call(Request::Semg { on: true }).await.ok);
    daemon.wait("sEMG on", |s| s.semg).await;
    let on_at = tokio::time::Instant::now();
    daemon.wait("sEMG off", |s| !s.semg).await;
    let after = on_at.elapsed();
    assert!(
        after <= Duration::from_secs(5),
        "turned off after {after:?}"
    );
    // Not turned back on by a reconnect either.
    daemon.transport.inject(SimInject::Close);
    daemon
        .wait("the drop", |s| s.state != LinkState::Connected)
        .await;
    daemon.connected().await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(!daemon.status.borrow().semg);
}

#[tokio::test(start_paused = true)]
async fn semg_stays_on_while_a_viewer_is_attached() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    let viewer = daemon.semg.subscribe();
    assert!(daemon.call(Request::Semg { on: true }).await.ok);
    daemon.wait("sEMG on", |s| s.semg).await;
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert!(daemon.status.borrow().semg, "kept on for the viewer");
    drop(viewer);
    daemon.wait("sEMG off", |s| !s.semg).await;
}

const MIDDLE_HOLD: SimInject = SimInject::Gesture {
    action: 0,
    finger: 3,
    derived: 3,
};

const INDEX_TAP: SimInject = SimInject::Gesture {
    action: 3,
    finger: 2,
    derived: 0,
};

const INDEX_DOUBLE: SimInject = SimInject::Gesture {
    action: 4,
    finger: 2,
    derived: 0,
};

impl Harness {
    /// Write the config and let the daemon pick it up.
    async fn configure(&self, text: &str) {
        std::fs::write(&self.config_path, text).unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    /// Three index taps the way the band reports them: the 2nd and 3rd also come with a
    /// double tap (band-core's `controls_common::triple`).
    async fn index_triple(&self) {
        self.transport.inject(INDEX_TAP);
        tokio::time::sleep(Duration::from_millis(200)).await;
        self.transport.inject(INDEX_TAP);
        self.transport.inject(INDEX_DOUBLE);
        tokio::time::sleep(Duration::from_millis(200)).await;
        self.transport.inject(INDEX_TAP);
        self.transport.inject(INDEX_DOUBLE);
    }
}

#[tokio::test(start_paused = true)]
async fn a_band_toggle_runs_on_toggle_but_ipc_pause_does_not() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    // The lock is a band control now; none by default.
    daemon.configure("[band]\ntoggle = \"middle_hold\"\non_toggle = \"osd\"\n").await;
    daemon.transport.inject(MIDDLE_HOLD);
    daemon.wait("the band's toggle", |s| s.paused).await;
    let toggled = vec!["export AIR_GESTURES_ENABLED=0; osd".to_string()];
    assert_eq!(daemon.runner.commands(), toggled);
    assert!(!daemon.call(Request::Resume).await.status.unwrap().paused);
    assert!(daemon.call(Request::Pause).await.status.unwrap().paused);
    assert!(!daemon.call(Request::Resume).await.status.unwrap().paused);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(
        daemon.runner.commands(),
        toggled,
        "IPC pause/resume runs no hook"
    );
}

#[tokio::test(start_paused = true)]
async fn profiles_switch_from_the_band_and_run_on_profile() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    daemon
        .configure("[band]\non_profile = \"osd\"\nnext_profile = \"index_triple\"\n[[profile]]\nname = \"games\"\n")
        .await;
    assert_eq!(daemon.status.borrow().profile.as_deref(), Some("default"));
    daemon.index_triple().await;
    daemon
        .wait("the next profile", |s| {
            s.profile.as_deref() == Some("games")
        })
        .await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(
        daemon.runner.commands(),
        vec!["export AIR_GESTURES_PROFILE='games'; osd".to_string()]
    );
}

#[tokio::test(start_paused = true)]
async fn a_single_profile_shows_no_profile_in_status() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    assert_eq!(daemon.status.borrow().profile, None);
}

#[tokio::test(start_paused = true)]
async fn profiles_switch_over_ipc() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    daemon
        .configure("[[profile]]\nname = \"games\"\n[[profile]]\nname = \"media\"\n")
        .await;
    let to = |name: &str| Request::Profile { to: name.into() };
    // Responses may be built before the switch lands, so wait on the watch channel.
    assert!(daemon.call(to("media")).await.error.is_none());
    daemon
        .wait("media", |s| s.profile.as_deref() == Some("media"))
        .await;
    assert!(daemon.call(to("next")).await.error.is_none());
    daemon
        .wait("wrapped to default", |s| {
            s.profile.as_deref() == Some("default")
        })
        .await;
    assert!(daemon.call(to("previous")).await.error.is_none());
    daemon
        .wait("back to media", |s| s.profile.as_deref() == Some("media"))
        .await;
    let refused = daemon.call(to("nope")).await;
    let error = refused.error.expect("an unknown profile is an error");
    assert!(
        error.contains("nope") && error.contains("games") && error.contains("media"),
        "{error}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_band_control_wins_over_the_profiles_command() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    // swipe_left's default command is paused; with one profile, next does nothing.
    daemon.configure("[band]\nnext_profile = \"swipe_left\"\n").await;
    daemon.transport.inject(SWIPE_LEFT);
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(daemon.runner.commands().is_empty(), "{:?}", daemon.runner.commands());
}

#[tokio::test(start_paused = true)]
async fn the_dial_override_lasts_until_the_profile_changes() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    daemon.configure("[[profile]]\nname = \"quiet\"\nindex_dial = \"off\"\n").await;
    assert_eq!(daemon.status.borrow().dial, DialTarget::Volume);
    daemon.call(Request::Dial { target: DialTarget::Brightness }).await;
    daemon.wait("brightness", |s| s.dial == DialTarget::Brightness).await;
    daemon.call(Request::Profile { to: "quiet".into() }).await;
    daemon.wait("quiet's own dial", |s| s.dial == DialTarget::Off).await;
    daemon.call(Request::CycleDial).await;
    daemon.wait("cycled from off", |s| s.dial == DialTarget::Volume).await;
    daemon.call(Request::Profile { to: "default".into() }).await;
    daemon.wait("default's own dial", |s| s.dial == DialTarget::Volume && s.profile.as_deref() == Some("default")).await;
}

const PRESS: u64 = 1;
const RELEASE: u64 = 2;
const INDEX: u64 = 2;
const SWIPE_UP: SimInject = SimInject::Gesture { action: 6, finger: 1, derived: 0 };

impl Harness {
    /// An index pinch turning the wrist about the band's x axis: `raw` gyro units for
    /// `samples` × 10 ms. With no fresh orientation that's a vertical stroke in a drawing,
    /// and a drag away from you outside one.
    async fn pinch_moving(&self, raw: i16, samples: usize) {
        self.transport.inject(SimInject::Gesture { action: PRESS, finger: INDEX, derived: 0 });
        for _ in 0..samples {
            self.transport.inject(SimInject::Gyro(raw));
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        self.transport.inject(SimInject::Gesture { action: RELEASE, finger: INDEX, derived: 0 });
    }

    /// Orientation samples with this roll about the forearm, every 50 ms for `seconds`.
    async fn hold_roll(&self, degrees: f64, seconds: f64) {
        for _ in 0..(seconds / 0.05).round() as usize {
            self.transport.inject(SimInject::Orientation(rolled(degrees)));
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// A taught drawing called `name`: three vertical lines.
    fn teach_line(&self, name: &str) {
        let line = vec![vec![[0.0, 0.0], [0.0, 15.0]]];
        let drawing = band_core::controls::draw::Drawing { name: name.into(), tries: vec![line.clone(), line.clone(), line] };
        air_gesturesd::drawings::save(&self.drawings, &drawing).unwrap();
    }
}

const DRAWING_CONFIG: &str = "[band]\ndraw = \"swipe_up\"\non_draw = \"osd\"\n[drawings]\nline = \"echo line\"\n";

/// An orientation (w, x, y, z) whose roll about the forearm is `degrees`.
fn rolled(degrees: f64) -> [f32; 4] {
    let half = (-degrees).to_radians() / 2.0;
    [half.cos() as f32, 0.0, half.sin() as f32, 0.0]
}

/// The right hand's measured palm down and palm up (band-core's `PalmCalibration::right_hand`).
const PALM_DOWN: f64 = -142.3;
const PALM_UP: f64 = -11.3;

#[tokio::test(start_paused = true)]
async fn a_palm_override_runs_with_the_palm_that_way() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    daemon.wait("the right hand", |s| s.hand.as_deref() == Some("right")).await;
    daemon.configure("[gestures.palm_up]\nswipe_left = \"echo up\"\n").await;
    daemon.hold_roll(PALM_UP, 0.3).await;
    daemon.transport.inject(SWIPE_LEFT);
    daemon.wait("the palm-up command", |s| s.last_action.as_deref() == Some("echo up")).await;
    daemon.hold_roll(PALM_DOWN, 0.6).await; // also past the 0.4 s action gate
    daemon.transport.inject(SWIPE_LEFT);
    daemon.wait("the normal command", |s| s.last_action.as_deref() == Some("hyprctl dispatch workspace e-1")).await;
}

#[tokio::test(start_paused = true)]
async fn a_stored_calibration_applies_to_the_hand_in_use() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start_seeded(transport.clone(), true, fake(&transport), |store| {
        // A wrist where -11.3° is palm down: the measured palm-up roll no longer reads as up.
        let palm = air_gesturesd::store::StoredPalm { down_roll: -11.3, up_roll: 120.0 };
        let calibration = air_gesturesd::store::HandCalibration { palm: Some(palm), drags: None };
        store.save_calibration(SIM_ADDRESS, band_core::events::Hand::Right, &calibration).unwrap();
    });
    daemon.connected().await;
    daemon.wait("the right hand", |s| s.hand.as_deref() == Some("right")).await;
    daemon.configure("[gestures.palm_up]\nswipe_left = \"echo up\"\n").await;
    daemon.hold_roll(PALM_UP, 0.3).await;
    daemon.transport.inject(SWIPE_LEFT);
    daemon.wait("the normal command", |s| s.last_action.as_deref() == Some("hyprctl dispatch workspace e-1")).await;
}

#[tokio::test(start_paused = true)]
async fn a_drawing_runs_on_draw_then_its_action() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    daemon.teach_line("line"); // after start: the library reloads on the tick
    daemon.configure(DRAWING_CONFIG).await;
    daemon.transport.inject(SWIPE_UP);
    tokio::time::sleep(Duration::from_millis(100)).await;
    daemon.pinch_moving(600, 40).await;
    daemon.wait("the drawing's action", |s| s.last_action.as_deref() == Some("echo line")).await;
    assert_eq!(
        daemon.runner.commands(),
        ["export AIR_GESTURES_DRAWING='line'; osd".to_string(), "echo line".to_string()],
        "swipe_up's own command is paused by [band] draw"
    );
}

#[tokio::test(start_paused = true)]
async fn an_unrecognised_drawing_runs_on_draw_with_nothing() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    daemon.teach_line("line");
    daemon.configure(DRAWING_CONFIG).await;
    daemon.transport.inject(SWIPE_UP);
    tokio::time::sleep(Duration::from_millis(100)).await;
    daemon.pinch_moving(50, 40).await; // 1.4°: too small to read
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(daemon.runner.commands(), ["export AIR_GESTURES_DRAWING=''; osd".to_string()]);
}

#[tokio::test(start_paused = true)]
async fn a_hand_chosen_over_ipc_survives_its_calibration_swap() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    daemon.configure("[band]\nhand = \"right\"\n").await;
    daemon.wait("the right hand", |s| s.hand.as_deref() == Some("right")).await;
    daemon.call(Request::Hand { hand: HandChoice::Left }).await;
    daemon.wait("the left hand", |s| s.hand.as_deref() == Some("left")).await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(daemon.status.borrow().hand.as_deref(), Some("left"), "the left hand's options didn't undo the choice");
}

impl Harness {
    /// Send a call and hand back where its answer will arrive.
    async fn call_later(&self, request: Request) -> oneshot::Receiver<Response> {
        let (reply, response) = oneshot::channel();
        self.calls.send((request, reply)).await.unwrap();
        response
    }

    async fn right_hand(&mut self) {
        self.connected().await;
        self.wait("the right hand", |s| s.hand.as_deref() == Some("right")).await;
    }
}

#[tokio::test(start_paused = true)]
async fn teaching_a_drag_answers_with_the_pinchs_direction_and_runs_nothing() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.right_hand().await;
    daemon.configure("[gestures]\nindex_drag_away = \"echo away\"\n").await;
    let answer = daemon.call_later(Request::TeachDrag { seconds: 30.0 }).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    daemon.pinch_moving(600, 50).await;
    let response = answer.await.unwrap();
    let Some(Taught::Drag { finger, direction }) = response.teach else { panic!("{response:?}") };
    assert_eq!(finger, "index");
    // Band x, palm down (roll −142.3°), in the forearm frame.
    assert!(direction[0] < -0.75 && direction[1].abs() < 0.05 && direction[2] < -0.55, "{direction:?}");
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(daemon.runner.commands().is_empty(), "teaching acts on nothing");
    // Teaching is off again: the same pinch is the drag gesture.
    daemon.pinch_moving(600, 50).await;
    daemon.wait("the drag", |s| s.last_action.as_deref() == Some("echo away")).await;
}

#[tokio::test(start_paused = true)]
async fn a_drag_step_with_no_pinch_times_out() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    let response = daemon.call(Request::TeachDrag { seconds: 2.0 }).await;
    assert!(response.error.unwrap().contains("no pinch"));
}

#[tokio::test(start_paused = true)]
async fn a_drawing_try_answers_with_its_strokes_and_matches_nothing() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    daemon.teach_line("line");
    daemon.configure(DRAWING_CONFIG).await;
    let answer = daemon.call_later(Request::DrawTry { seconds: 30.0 }).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    daemon.pinch_moving(600, 40).await;
    let response = answer.await.unwrap();
    let Some(Taught::Strokes { strokes }) = response.teach else { panic!("{response:?}") };
    assert_eq!(strokes.len(), 1);
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(daemon.runner.commands().is_empty(), "no on_draw, no action while teaching");
}

#[tokio::test(start_paused = true)]
async fn palm_rolls_without_orientation_come_back_empty() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    let response = daemon.call(Request::PalmRolls { seconds: 2.0 }).await;
    assert_eq!(response.teach, Some(Taught::Rolls { rolls: Vec::new(), samples: 0 }));
}

#[tokio::test(start_paused = true)]
async fn palm_rolls_and_the_palm_now_come_from_the_bands_orientation() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.right_hand().await;
    let answer = daemon.call_later(Request::PalmRolls { seconds: 2.0 }).await;
    daemon.hold_roll(-140.0, 2.5).await;
    let Some(Taught::Rolls { rolls, samples }) = answer.await.unwrap().teach else { panic!() };
    assert!(samples >= 30 && rolls.len() == samples, "{samples} samples, {} rolls", rolls.len());
    assert!(rolls.iter().all(|r| (r + 140.0).abs() < 0.01), "{rolls:?}");
    daemon.hold_roll(PALM_UP, 0.3).await;
    assert_eq!(daemon.call(Request::Palm).await.teach, Some(Taught::Palm { palm: "up".into() }));
    tokio::time::sleep(Duration::from_secs(1)).await;
    let stale = daemon.call(Request::Palm).await.teach;
    assert_eq!(stale, Some(Taught::Palm { palm: "down".into() }), "no orientation for 0.5 s reads as down");
}

#[tokio::test(start_paused = true)]
async fn teaching_needs_a_band_and_one_step_at_a_time() {
    let lonely = SimTransport::new(SimMode::Enrolled);
    let alone = start(lonely.clone(), false, fake(&lonely));
    assert!(alone.call(Request::PalmRolls { seconds: 2.0 }).await.error.unwrap().contains("connect the band first"));
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.connected().await;
    let _first = daemon.call_later(Request::TeachDrag { seconds: 30.0 }).await;
    let second = daemon.call(Request::DrawTry { seconds: 30.0 }).await;
    assert!(second.error.unwrap().contains("another teaching step"));
}

#[tokio::test(start_paused = true)]
async fn set_palm_and_set_drags_store_for_the_band_and_hand() {
    let transport = SimTransport::new(SimMode::Enrolled);
    let mut daemon = start(transport.clone(), true, fake(&transport));
    daemon.right_hand().await;
    let palm = StoredPalm { down_roll: -140.0, up_roll: -10.0 };
    assert!(daemon.call(Request::SetPalm { palm: Some(palm) }).await.ok);
    let p = Prototypes::right_hand();
    let drags = TaughtDrags { turn: p.turn, drags: p.drags.unwrap() };
    assert!(daemon.call(Request::SetDrags { drags: Some(drags.clone()) }).await.ok);
    let stored = daemon.store.load_calibration(SIM_ADDRESS, Hand::Right).unwrap();
    assert_eq!(stored.palm, Some(palm));
    assert_eq!(stored.drags.unwrap().palm_down_roll, -140.0, "taught under the calibration's palm down");
    let mut alike = drags;
    alike.drags[1] = alike.drags[0];
    assert!(daemon.call(Request::SetDrags { drags: Some(alike) }).await.error.unwrap().contains("alike"));
    let close = StoredPalm { down_roll: -140.0, up_roll: -120.0 };
    assert!(daemon.call(Request::SetPalm { palm: Some(close) }).await.error.unwrap().contains("60°"));
    daemon.call(Request::SetPalm { palm: None }).await;
    daemon.call(Request::SetDrags { drags: None }).await;
    assert_eq!(daemon.store.load_calibration(SIM_ADDRESS, Hand::Right).unwrap(), HandCalibration::default());
}
