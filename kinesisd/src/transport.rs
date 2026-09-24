//! How the daemon reaches a band. `BluezTransport` (Task 12) talks to real
//! hardware; `SimTransport` runs an in-process `band_core::responder::Responder`
//! behind a duplex pipe for tests and `--dry-run`.

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use band_core::identity::{EnrollmentIdentity, point64};
use band_core::responder::{Responder, SimMode};
use p256::ecdsa::SigningKey;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream};
use tokio::sync::mpsc;

/// A band seen while scanning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub address: String,
    pub name: String,
    pub rssi: Option<i16>,
}

pub trait Transport: Send + Sync + 'static {
    type Io: AsyncRead + AsyncWrite + Unpin + Send + 'static;
    /// Held while a pairing runs (e.g. the scoped BlueZ agent).
    type PairingGuard: Send + 'static;

    /// Bands advertising a "Meta Band…" name, strongest first.
    fn scan(&self, timeout: Duration) -> impl Future<Output = Result<Vec<Found>, String>> + Send;
    /// Get ready to pair with `address`: drop a stale bond, accept only its pairing request.
    fn prepare_pairing(
        &self,
        address: &str,
    ) -> impl Future<Output = Result<Self::PairingGuard, String>> + Send;
    /// Connect and open the band's input channel.
    fn open(&self, address: &str) -> impl Future<Output = Result<Self::Io, String>> + Send;
    /// Remove the system's bond with `address`.
    fn forget(&self, address: &str) -> impl Future<Output = Result<(), String>> + Send;
}

/// Things a test can make the simulated band do on the open connection.
#[derive(Debug, Clone)]
pub enum SimInject {
    /// A recognized gesture (band enum values: thumb=1 index=2 middle=3; left=8 right=9 up=6 down=7 doubletap=4).
    Gesture {
        action: u64,
        finger: u64,
        derived: u64,
    },
    Gyro(i16),
    RawEmg,
    /// Stop answering requests.
    Silence,
    /// Drop the connection.
    Close,
}

pub const SIM_ADDRESS: &str = "00:00:5E:00:53:01";
pub const SIM_NAME: &str = "Meta Band (simulated)";

struct SimState {
    mode: SimMode,
    next_mode: Option<SimMode>,
    band_key: SigningKey,
    reject_identity: Option<u32>,
    reject_claim: Option<u32>,
    /// SHA-256 of the owner's app point; kept across connections like a real band.
    owner: Option<[u8; 32]>,
    /// ConfigResp hand (0 right, 1 left); kept across connections.
    hand: u64,
    inject: Option<mpsc::UnboundedSender<SimInject>>,
    opens: usize,
    /// Drop the connection right after the band commits a claim, before it confirms.
    drop_after_finish: bool,
}

#[derive(Clone)]
pub struct SimTransport {
    state: Arc<Mutex<SimState>>,
}

impl SimTransport {
    /// `mode` is how the band answers ordinary connections. `prepare_pairing`
    /// switches the next connection to the ceremony; afterwards the band is enrolled.
    pub fn new(mode: SimMode) -> Self {
        let mut scalar = [0u8; 32];
        scalar[31] = 2;
        let band_key = SigningKey::from_slice(&scalar).expect("scalar 2 is a valid key");
        Self {
            state: Arc::new(Mutex::new(SimState {
                mode,
                next_mode: None,
                band_key,
                reject_identity: None,
                reject_claim: None,
                owner: None,
                hand: 0,
                inject: None,
                opens: 0,
                drop_after_finish: false,
            })),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SimState> {
        self.state.lock().expect("sim transport lock")
    }

    /// The simulated band's identity key point (what the fake ownership server returns).
    pub fn band_point(&self) -> Vec<u8> {
        point64(self.lock().band_key.verifying_key())
    }

    pub fn set_reject_identity(&self, code: Option<u32>) {
        self.lock().reject_identity = code;
    }

    pub fn set_reject_claim(&self, code: Option<u32>) {
        self.lock().reject_claim = code;
    }

    /// Make the band already owned by `identity` (as if a claim had happened).
    pub fn set_owner(&self, identity: &EnrollmentIdentity) {
        let hash: [u8; 32] = Sha256::digest(identity.app_point()).into();
        self.lock().owner = Some(hash);
    }

    /// Test knob: when the band reads FinishChangeOwner it commits the claim,
    /// then drops the connection without sending its confirmation.
    pub fn set_drop_after_finish(&self, drop: bool) {
        self.lock().drop_after_finish = drop;
    }

    /// The band's current owner hash (None until a claim or the first enrolled connection).
    pub fn owner(&self) -> Option<[u8; 32]> {
        self.lock().owner
    }

    /// Act on the most recently opened connection; false if there is none.
    pub fn inject(&self, what: SimInject) -> bool {
        self.lock()
            .inject
            .as_ref()
            .is_some_and(|sender| sender.send(what).is_ok())
    }

    /// How many connections have been opened.
    pub fn opens(&self) -> usize {
        self.lock().opens
    }
}

impl Transport for SimTransport {
    type Io = DuplexStream;
    type PairingGuard = ();

    async fn scan(&self, _timeout: Duration) -> Result<Vec<Found>, String> {
        Ok(vec![Found {
            address: SIM_ADDRESS.into(),
            name: SIM_NAME.into(),
            rssi: Some(-40),
        }])
    }

    async fn prepare_pairing(&self, _address: &str) -> Result<(), String> {
        let mut state = self.lock();
        state.next_mode = Some(SimMode::Ceremony);
        // A factory reset puts the band back in pairing mode with no owner;
        // if the ceremony that follows doesn't finish, a later connection
        // must see a plain pairing-mode band, not whatever it was before.
        state.owner = None;
        state.mode = SimMode::Legacy;
        Ok(())
    }

    async fn open(&self, address: &str) -> Result<DuplexStream, String> {
        if address != SIM_ADDRESS {
            return Err(format!("no simulated band at {address}"));
        }
        let (host, band) = tokio::io::duplex(1 << 16);
        let (sender, receiver) = mpsc::unbounded_channel();
        let responder = {
            let mut state = self.lock();
            let mode = state.next_mode.take().unwrap_or(state.mode);
            state.inject = Some(sender);
            state.opens += 1;
            let mut responder = Responder::new(mode, state.band_key.clone());
            responder.reject_identity = state.reject_identity;
            responder.reject_claim = state.reject_claim;
            responder.set_owner(state.owner);
            responder.hand = state.hand;
            responder
        };
        tokio::spawn(serve(band, responder, receiver, self.state.clone()));
        Ok(host)
    }

    async fn forget(&self, _address: &str) -> Result<(), String> {
        Ok(())
    }
}

/// Persist whatever changed since the last sync: what a real band remembers
/// survives the connection, including the mode once the ceremony commits.
fn sync_state(
    band: &Responder,
    synced: &mut (Option<[u8; 32]>, u64, bool),
    state: &Mutex<SimState>,
) {
    let current = (band.owner(), band.hand, band.finish_seen());
    if current != *synced {
        let mut state = state.lock().expect("sim transport lock");
        state.owner = current.0;
        state.hand = current.1;
        if current.2 && !synced.2 {
            state.mode = SimMode::Enrolled;
        }
        *synced = current;
    }
}

async fn serve(
    mut io: DuplexStream,
    mut band: Responder,
    mut inject: mpsc::UnboundedReceiver<SimInject>,
    state: Arc<Mutex<SimState>>,
) {
    let mut buffer = vec![0u8; 1 << 16];
    let mut synced = (band.owner(), band.hand, band.finish_seen());
    loop {
        tokio::select! {
            read = io.read(&mut buffer) => {
                let count = match read {
                    Ok(0) | Err(_) => return,
                    Ok(count) => count,
                };
                match band.respond(&buffer[..count]) {
                    Ok(reply) => {
                        let finished_now = band.finish_seen() && !synced.2;
                        sync_state(&band, &mut synced, &state);
                        if finished_now && state.lock().expect("sim transport lock").drop_after_finish {
                            return;
                        }
                        if !reply.is_empty() && io.write_all(&reply).await.is_err() {
                            return;
                        }
                    }
                    Err(_) => return,
                }
            }
            what = inject.recv() => {
                let bytes = match what {
                    None | Some(SimInject::Close) => return,
                    Some(SimInject::Silence) => {
                        band.silent = true;
                        continue;
                    }
                    Some(SimInject::Gesture { action, finger, derived }) => band.gesture(action, finger, derived),
                    Some(SimInject::Gyro(x)) => band.gyro(x),
                    Some(SimInject::RawEmg) => band.raw_emg(),
                };
                sync_state(&band, &mut synced, &state);
                if let Ok(bytes) = bytes
                    && io.write_all(&bytes).await.is_err()
                {
                    return;
                }
            }
        }
    }
}
