//! What clients (waybar, kinesisctl) see.

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

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
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
    pub error: Option<String>,
    /// Raw sEMG streaming is on.
    pub semg: bool,
    /// The paired band's name.
    pub band: Option<String>,
}
