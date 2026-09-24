//! kinesisd: drives a Meta Neural Band through `band-core` and turns its
//! gestures into desktop actions.

pub mod actions;
pub mod bluez;
pub mod clock;
pub mod config;
pub mod controller;
pub mod daemon;
pub mod ipc;
pub mod link;
pub mod meta;
pub mod pairing;
pub mod status;
pub mod store;
pub mod transport;
