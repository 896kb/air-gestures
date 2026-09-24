#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use band_core::identity::{EnrollmentIdentity, verifying_key_from_point64};
use band_core::responder::SimMode;
use kinesisd::actions::RecordingRunner;
use kinesisd::daemon::Daemon;
use kinesisd::ipc::{self, Handles};
use kinesisd::meta::FakeFactory;
use kinesisd::store::{BandRecord, Store};
use kinesisd::transport::{SIM_ADDRESS, SIM_NAME, SimTransport};
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

/// A kinesisd on a simulated band, serving a socket in a temp dir.
/// `paired` pre-claims the band (store + sim owner).
pub async fn start(mode: SimMode, paired: bool) -> TestDaemon {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("kinesis.sock");
    let store = Store::open(dir.path().join("state")).unwrap();
    let transport = SimTransport::new(mode);
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
    let runner = RecordingRunner::default();
    let factory = FakeFactory {
        device_point: Some(transport.band_point()),
        fail: None,
    };
    let daemon = Daemon::new(
        Arc::new(transport.clone()),
        runner.clone(),
        factory,
        store.clone(),
        dir.path().join("config.toml"),
    );
    let listener = ipc::bind(&socket).unwrap();
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
    TestDaemon {
        socket,
        transport,
        runner,
        store,
        dir,
    }
}

impl TestDaemon {
    /// Poll `status` until `until` holds (real time, 20 s cap).
    pub async fn wait(
        &self,
        what: &str,
        until: impl Fn(&kinesisd::status::Status) -> bool,
    ) -> kinesisd::status::Status {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        loop {
            if let Ok(status) = kinesisctl::client::status(&self.socket).await
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
