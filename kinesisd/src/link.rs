//! One connection to a band: the I/O loop around a `BandSession`
//! (kinesis `BandConnection.swift` scheduling + `BandModel.swift` stall check).

use std::time::Duration;

use band_core::BandError;
use band_core::events::{Event, Hand};
use band_core::identity::EnrollmentIdentity;
use band_core::session::BandSession;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::clock::Clock;

const TICK: Duration = Duration::from_millis(50);
const STARTUP_TIMEOUT: f64 = 30.0;
/// The ownership ceremony includes two HTTP round trips.
const CEREMONY_TIMEOUT: f64 = 180.0;
const BATTERY_EVERY: f64 = 5.0;
const QUIET_QUERY: f64 = 2.0;
const STALL_LIVE: f64 = 8.0;
const STALL_BEFORE_INPUT: f64 = 50.0;
const STOP_WAIT: f64 = 3.0;

#[derive(Debug)]
pub enum LinkCommand {
    /// Stop the streams and close (also what dropping the sender does).
    Stop,
    SetHand(Hand),
    RawEmg(bool),
    /// The `pair_request` HTTP reply.
    ResumePairRequest {
        signature: Vec<u8>,
        receipt: String,
    },
    /// The `pair` HTTP reply.
    ResumePair {
        signature: Vec<u8>,
        receipt: String,
        device_public_key: Option<Vec<u8>>,
    },
}

#[derive(Debug)]
pub enum LinkEvent {
    Band(Event),
    /// A command the session refused for now (`BandError::Busy`); the link is fine.
    Refused(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum LinkEnd {
    /// Stopped on request (acknowledged or not).
    Closed,
    /// Went quiet for too long; stopped.
    Stalled,
    /// Didn't finish starting up in time.
    StartupTimeout,
    /// The band closed the channel.
    StreamEnded,
    /// The band rejected our stored owner key.
    IdentityRejected(String),
    /// The band rejected the ownership claim (`0x1042` = a different Meta account).
    OwnershipRejected {
        code: u32,
        message: String,
    },
    Failed(String),
}

impl std::fmt::Display for LinkEnd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinkEnd::Closed => f.write_str("disconnected"),
            LinkEnd::Stalled => f.write_str("the band stopped responding"),
            LinkEnd::StartupTimeout => f.write_str("the band took too long to respond"),
            LinkEnd::StreamEnded => f.write_str("the band closed the connection"),
            LinkEnd::IdentityRejected(message) | LinkEnd::Failed(message) => f.write_str(message),
            LinkEnd::OwnershipRejected { message, .. } => f.write_str(message),
        }
    }
}

pub struct LinkOutcome {
    pub end: LinkEnd,
    /// The session's owner identity at the end — after a claim, the new one,
    /// even if the connection failed right after the band confirmed it.
    pub enrollment: Option<EnrollmentIdentity>,
}

fn from_error(error: BandError) -> LinkEnd {
    match error {
        BandError::IdentityMismatch(message) => LinkEnd::IdentityRejected(message),
        BandError::OwnershipRejected { code, message } => {
            LinkEnd::OwnershipRejected { code, message }
        }
        BandError::Protocol(message) | BandError::Busy(message) => LinkEnd::Failed(message),
    }
}

async fn send<IO: AsyncWrite + Unpin>(io: &mut IO, bytes: &[u8]) -> Result<(), LinkEnd> {
    if bytes.is_empty() {
        return Ok(());
    }
    io.write_all(bytes)
        .await
        .map_err(|error| LinkEnd::Failed(format!("write to the band failed: {error}")))?;
    io.flush()
        .await
        .map_err(|error| LinkEnd::Failed(format!("write to the band failed: {error}")))
}

/// Run one connection until it ends. `persist_pending` is called with the
/// claimed identity before FinishChangeOwner is written; if it fails the
/// claim is abandoned before the band can commit.
pub async fn run_link<IO>(
    mut io: IO,
    mut session: BandSession,
    clock: Clock,
    mut commands: mpsc::Receiver<LinkCommand>,
    events: mpsc::Sender<LinkEvent>,
    mut persist_pending: impl FnMut(&EnrollmentIdentity) -> std::io::Result<()>,
) -> LinkOutcome
where
    IO: AsyncRead + AsyncWrite + Unpin,
{
    let end = drive(
        &mut io,
        &mut session,
        clock,
        &mut commands,
        &events,
        &mut persist_pending,
    )
    .await;
    let _ = io.shutdown().await;
    LinkOutcome {
        end,
        enrollment: session.enrollment().cloned(),
    }
}

struct Stopping {
    until: f64,
    reason: LinkEnd,
}

async fn drive<IO: AsyncRead + AsyncWrite + Unpin>(
    io: &mut IO,
    session: &mut BandSession,
    clock: Clock,
    commands: &mut mpsc::Receiver<LinkCommand>,
    events: &mpsc::Sender<LinkEvent>,
    persist_pending: &mut impl FnMut(&EnrollmentIdentity) -> std::io::Result<()>,
) -> LinkEnd {
    let start = clock.now();
    let ceremony = session.ceremony().is_some();
    let mut deadline = start
        + if ceremony {
            CEREMONY_TIMEOUT
        } else {
            STARTUP_TIMEOUT
        };
    let mut live = false;
    let mut last_read = start;
    let mut last_query = start;
    let mut last_input: Option<f64> = None;
    let mut next_battery = start;
    let mut stopping: Option<Stopping> = None;
    let mut buffer = vec![0u8; 1 << 16];
    let mut ticker = tokio::time::interval(TICK);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let request = match session.request() {
        Ok(request) => request,
        Err(error) => return from_error(error),
    };
    if let Err(end) = send(io, &request).await {
        return end;
    }

    // Begin a graceful stop; returns the end right away when there's nothing to wait for.
    macro_rules! begin_stop {
        ($reason:expr) => {{
            let reason = $reason;
            match session.stop() {
                Ok(bytes) if bytes.is_empty() => return reason,
                Ok(bytes) => {
                    if let Err(end) = send(io, &bytes).await {
                        return end;
                    }
                    stopping = Some(Stopping {
                        until: clock.now() + STOP_WAIT,
                        reason,
                    });
                }
                Err(_) => return reason,
            }
        }};
    }

    loop {
        tokio::select! {
            read = io.read(&mut buffer) => {
                let count = match read {
                    Ok(0) => return stopping.map_or(LinkEnd::StreamEnded, |stop| stop.reason),
                    Ok(count) => count,
                    Err(error) => return LinkEnd::Failed(format!("read from the band failed: {error}")),
                };
                let now = clock.now();
                last_read = now;
                let result = match session.feed(&buffer[..count], now) {
                    Ok(result) => result,
                    Err(error) => return from_error(error),
                };
                if let Err(end) = send(io, &result.outgoing).await {
                    return end;
                }
                for event in result.events {
                    match event {
                        Event::Connected => {
                            live = true;
                            deadline = f64::INFINITY;
                            last_input = Some(now);
                        }
                        Event::Heartbeat | Event::DataSeen => last_input = Some(now),
                        _ => {}
                    }
                    if stopping.is_none() {
                        let _ = events.send(LinkEvent::Band(event)).await;
                    }
                }
                if let Some(stop) = &stopping
                    && session.stop_acknowledged()
                {
                    return stop.reason.clone();
                }
            }
            _ = ticker.tick() => {
                let now = clock.now();
                if let Some(stop) = &stopping {
                    if now >= stop.until {
                        return stop.reason.clone();
                    }
                    continue;
                }
                if now >= deadline {
                    return LinkEnd::StartupTimeout;
                }
                for event in session.tick(now) {
                    let _ = events.send(LinkEvent::Band(event)).await;
                }
                let mut outgoing = Vec::new();
                if session.streams_enabled() && now >= next_battery {
                    next_battery = now + BATTERY_EVERY;
                    match session.query_battery_status(now) {
                        Ok(bytes) => outgoing.extend(bytes),
                        Err(error) => return from_error(error),
                    }
                }
                if session.streams_enabled() && now - last_read >= QUIET_QUERY && now - last_query >= QUIET_QUERY {
                    last_query = now;
                    match session.query_stream_state() {
                        Ok(bytes) => outgoing.extend(bytes),
                        Err(error) => return from_error(error),
                    }
                }
                if let Err(end) = send(io, &outgoing).await {
                    return end;
                }
                let silent = now - last_input.unwrap_or(start);
                let limit = if live { STALL_LIVE } else { STALL_BEFORE_INPUT };
                if (live || !ceremony) && silent > limit {
                    begin_stop!(LinkEnd::Stalled);
                }
            }
            command = commands.recv(), if stopping.is_none() => {
                let now = clock.now();
                let bytes = match command {
                    None | Some(LinkCommand::Stop) => {
                        begin_stop!(LinkEnd::Closed);
                        continue;
                    }
                    Some(LinkCommand::SetHand(hand)) => session.set_handedness(hand, now),
                    Some(LinkCommand::RawEmg(on)) => session.set_raw_emg_enabled(on, now),
                    Some(LinkCommand::ResumePairRequest { signature, receipt }) => {
                        session.ceremony_pair_request_completed(&signature, &receipt)
                    }
                    Some(LinkCommand::ResumePair { signature, receipt, device_public_key }) => {
                        match session.ceremony_pair_completed(&signature, &receipt, device_public_key.as_deref()) {
                            Ok(bytes) => {
                                // The band may commit as soon as it reads these bytes.
                                if let Some(identity) = session.pending_identity()
                                    && let Err(error) = persist_pending(&identity)
                                {
                                    return LinkEnd::Failed(format!("could not save the pending owner key: {error}"));
                                }
                                Ok(bytes)
                            }
                            Err(error) => Err(error),
                        }
                    }
                };
                match bytes {
                    Ok(bytes) => {
                        if let Err(end) = send(io, &bytes).await {
                            return end;
                        }
                    }
                    Err(BandError::Busy(message)) => {
                        let _ = events.send(LinkEvent::Refused(message)).await;
                    }
                    Err(error) => return from_error(error),
                }
            }
        }
    }
}
