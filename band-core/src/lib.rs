//! Sans-IO protocol core for the Meta Neural Band.
//!
//! Ported from callbacked/kinesis `Sources/KinesisCore` (MIT). Callers feed
//! received bytes plus a monotonic time in seconds and get outgoing bytes and
//! typed events back; nothing here touches sockets, clocks or files.

pub mod airshield;
pub mod datax;
pub mod error;
pub mod proto;

pub use error::{BandError, Result};
