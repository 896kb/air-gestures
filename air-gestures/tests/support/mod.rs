#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use air_gesturesd::actions::RecordingRunner;
use air_gesturesd::daemon::Daemon;
use air_gesturesd::ipc::{self, Handles};
use air_gesturesd::meta::{FakeFactory, PairError};
use air_gesturesd::store::{BandRecord, Store};
use air_gesturesd::transport::{SIM_ADDRESS, SIM_NAME, SimTransport};
use band_core::identity::{EnrollmentIdentity, verifying_key_from_point64};
use band_core::responder::SimMode;
use p256::ecdsa::SigningKey;
use rand_core::OsRng;
use tokio::sync::mpsc;

pub struct TestDaemon {
    pub socket: PathBuf,
    pub transport: SimTransport,
    pub runner: RecordingRunner,
    pub store: Store,
    pub dir: tempfile::TempDir,
}

/// A air-gesturesd on a simulated band, serving a socket in a temp dir.
/// `paired` pre-claims the band (store + sim owner).
pub async fn start(mode: SimMode, paired: bool) -> TestDaemon {
    start_failing(mode, paired, None).await
}

/// Like `start`, but a claim's Meta HTTP steps answer with `fail` instead of
/// succeeding (e.g. `PairError::SessionInvalid` for a "needs sign-in" pairing).
pub async fn start_failing(mode: SimMode, paired: bool, fail: Option<PairError>) -> TestDaemon {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("air-gestures.sock");
    let (transport, runner, store) = spawn(&socket, dir.path(), mode, paired, fail).await;
    TestDaemon {
        socket,
        transport,
        runner,
        store,
        dir,
    }
}

/// Like `start`, but binds the daemon's control socket at `socket` instead of
/// picking a fresh temp dir for it — for tests that need to see what a
/// client observes at a path before any daemon exists there. The caller owns
/// (and must keep alive) `socket`'s directory; the daemon's own state (store,
/// config) lives in a separate private temp dir, held by the returned
/// `TestDaemon` so it isn't cleaned up while the daemon is still running.
pub async fn start_at(socket: PathBuf, mode: SimMode, paired: bool) -> TestDaemon {
    let dir = tempfile::tempdir().unwrap();
    let (transport, runner, store) = spawn(&socket, dir.path(), mode, paired, None).await;
    TestDaemon {
        socket,
        transport,
        runner,
        store,
        dir,
    }
}

/// Set up the store/transport/daemon and start serving `socket`; `state_dir`
/// holds the store and config file (a subdirectory of it each).
async fn spawn(
    socket: &std::path::Path,
    state_dir: &std::path::Path,
    mode: SimMode,
    paired: bool,
    fail: Option<PairError>,
) -> (SimTransport, RecordingRunner, Store) {
    let store = Store::open(state_dir.join("state")).unwrap();
    let transport = SimTransport::new(mode);
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
    let runner = RecordingRunner::default();
    let factory = FakeFactory {
        device_point: Some(transport.band_point()),
        fail,
    };
    let daemon = Daemon::new(
        Arc::new(transport.clone()),
        runner.clone(),
        factory,
        store.clone(),
        state_dir.join("config.toml"),
        state_dir.join("drawings"),
    );
    let listener = ipc::bind(socket).unwrap();
    let (calls, call_rx) = mpsc::channel(16);
    tokio::spawn(ipc::serve(
        listener,
        Handles {
            calls,
            status: daemon.status(),
            semg: daemon.semg(),
        },
    ));
    tokio::spawn(daemon.run(call_rx, std::future::pending()));
    (transport, runner, store)
}

impl TestDaemon {
    /// Where this daemon reads its drawings.
    pub fn drawings(&self) -> PathBuf {
        self.dir.path().join("drawings")
    }

    /// Poll `status` until `until` holds (real time, 20 s cap).
    pub async fn wait(
        &self,
        what: &str,
        until: impl Fn(&air_gesturesd::status::Status) -> bool,
    ) -> air_gesturesd::status::Status {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        loop {
            if let Ok(status) = air_gestures::client::status(&self.socket).await
                && until(&status)
            {
                return status;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for {what}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

/// An orientation (w, x, y, z) whose roll about the forearm is `degrees`.
pub fn rolled(degrees: f64) -> [f32; 4] {
    let half = (-degrees).to_radians() / 2.0;
    [half.cos() as f32, 0.0, half.sin() as f32, 0.0]
}

/// The band's orientation every 50 ms, rolled as `roll` says now (until aborted).
pub async fn feed_roll(transport: SimTransport, roll: std::sync::Arc<std::sync::Mutex<f64>>) {
    use air_gesturesd::transport::SimInject;
    loop {
        let degrees = *roll.lock().unwrap();
        transport.inject(SimInject::Orientation(rolled(degrees)));
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The right hand's measured drag away (forearm frame, unit).
pub fn away() -> [f64; 3] {
    band_core::controls::pinch::Prototypes::right_hand().drags.unwrap()[band_core::controls::Drag::Away.index()].unwrap()
}

/// An index pinch moving `raw` gyro units along the drag away, the band palm down (no fresh
/// orientation): outside a drawing a drag away, in one a straight stroke.
pub async fn pinch_moving(transport: &SimTransport, raw: i16, samples: usize) {
    use air_gesturesd::transport::SimInject;
    use band_core::controls::palm::RIGHT_PALM_DOWN_ROLL;
    let axes = band_core::controls::pinch::from_forearm(away(), RIGHT_PALM_DOWN_ROLL).map(|v| (v * f64::from(raw)).round() as i16);
    transport.inject(SimInject::Gesture { action: 1, finger: 2, derived: 0 });
    for _ in 0..samples {
        transport.inject(SimInject::Gyro3(axes));
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    transport.inject(SimInject::Gesture { action: 2, finger: 2, derived: 0 });
}

/// An index pinch turning the wrist about these band axes (raw gyro units per sample).
pub async fn pinch_moving3(transport: &SimTransport, axes: [i16; 3], samples: usize) {
    use air_gesturesd::transport::SimInject;
    transport.inject(SimInject::Gesture { action: 1, finger: 2, derived: 0 });
    for _ in 0..samples {
        transport.inject(SimInject::Gyro3(axes));
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    transport.inject(SimInject::Gesture { action: 2, finger: 2, derived: 0 });
}
