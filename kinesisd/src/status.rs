//! What clients (waybar, kinesisctl) see: status contract version 1.

use serde::{Deserialize, Serialize};

use crate::config::DialTarget;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkState {
    #[default]
    Idle,
    Connecting,
    Connected,
    Pairing,
}

/// The status contract version clients can check.
pub const STATUS_VERSION: u32 = 1;

/// How the last pairing ended.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct PairResult {
    pub ok: bool,
    pub message: String,
    /// The failure was a missing or expired Meta session: sign in and pair again.
    #[serde(default)]
    pub needs_sign_in: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
    /// Always `STATUS_VERSION` (1).
    pub version: u32,
    pub state: LinkState,
    /// Controls are off (the link can still be up).
    pub paused: bool,
    pub battery: Option<u8>,
    pub charging: Option<bool>,
    /// "left" or "right" as confirmed by the band.
    pub hand: Option<String>,
    pub dial: DialTarget,
    pub last_gesture: Option<String>,
    pub last_action: Option<String>,
    /// Recognized gestures this run.
    pub gestures: u64,
    /// The current pairing step while `state == Pairing`.
    pub pairing: Option<String>,
    /// The link / daemon error (not pairing or config problems).
    pub error: Option<String>,
    /// Why the last pairing failed; kept until the next pairing or `forget`.
    pub pairing_error: Option<String>,
    /// The config file is broken (the previous config stays in use).
    pub config_error: Option<String>,
    /// Whole seconds until the next reconnect attempt (rounded up), while
    /// idle and wanting a connection.
    pub reconnect_in: Option<u32>,
    /// How the last pairing ended (kept until the next pairing or `forget`).
    pub last_pair: Option<PairResult>,
    /// Raw sEMG streaming is on.
    pub semg: bool,
    /// The paired band's name.
    pub band: Option<String>,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            version: STATUS_VERSION,
            state: LinkState::default(),
            paused: false,
            battery: None,
            charging: None,
            hand: None,
            dial: DialTarget::default(),
            last_gesture: None,
            last_action: None,
            gestures: 0,
            pairing: None,
            error: None,
            pairing_error: None,
            config_error: None,
            reconnect_in: None,
            last_pair: None,
            semg: false,
            band: None,
        }
    }
}
