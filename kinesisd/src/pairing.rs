//! Claiming a band for this computer: find it in pairing mode, pair at the
//! Bluetooth level, run the ownership ceremony with Meta's two HTTP steps and
//! save the new owner key — as pending before the band can commit, confirmed
//! once it does (kinesis `BandModel.pairBand` / `performCeremony`).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use band_core::ceremony::{CeremonyHttpRequest, OwnershipCeremony};
use band_core::events::Event;
use band_core::session::BandSession;
use tokio::sync::mpsc;

use crate::clock::Clock;
use crate::link::{LinkCommand, LinkEnd, LinkEvent, run_link};
use crate::meta::{PairClient, PairError};
use crate::store::{BandRecord, Store};
use crate::transport::Transport;

#[derive(Debug, Clone, PartialEq)]
pub enum PairFailure {
    NoBandFound,
    /// The Meta session is gone or expired: sign in again.
    SessionExpired(String),
    /// The band belongs to another Meta account (`0x1042`).
    WrongAccount(String),
    Rejected(String),
    Failed(String),
}

impl std::fmt::Display for PairFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PairFailure::NoBandFound => f.write_str(
                "no band in pairing mode found. hold the band's button for 3 seconds until it flashes, then try again.",
            ),
            PairFailure::SessionExpired(message)
            | PairFailure::WrongAccount(message)
            | PairFailure::Rejected(message)
            | PairFailure::Failed(message) => f.write_str(message),
        }
    }
}

fn from_pair_error(error: PairError) -> PairFailure {
    match error {
        PairError::SessionInvalid(message) => PairFailure::SessionExpired(message),
        PairError::Failed(message) => PairFailure::Failed(message),
    }
}

/// Claim the band at `address` (or the first one found) for this computer.
pub async fn pair_band<T: Transport, C: PairClient>(
    transport: &T,
    client: &C,
    store: &Store,
    address: Option<&str>,
    clock: Clock,
    mut progress: impl FnMut(&str) + Send,
) -> Result<BandRecord, PairFailure> {
    // Restored if this run's claim is abandoned or rejected before the band commits.
    let previous_band = store.load_band().ok().flatten();
    progress("looking for a band in pairing mode");
    let found = transport
        .scan(Duration::from_secs(10))
        .await
        .map_err(PairFailure::Failed)?;
    let chosen = match address {
        Some(address) => found
            .into_iter()
            .find(|band| band.address.eq_ignore_ascii_case(address)),
        None => found.into_iter().next(),
    }
    .ok_or(PairFailure::NoBandFound)?;
    let record = BandRecord {
        address: chosen.address,
        name: chosen.name,
    };

    progress("preparing Bluetooth pairing");
    let _pairing = transport
        .prepare_pairing(&record.address)
        .await
        .map_err(PairFailure::Failed)?;
    progress(&format!("connecting to {}", record.name));
    let io = transport
        .open(&record.address)
        .await
        .map_err(PairFailure::Failed)?;

    let (commands, command_rx) = mpsc::channel(8);
    let (event_tx, mut events) = mpsc::channel(64);
    // The band record is saved together with the pending key (band first):
    // once the band may commit, a later connect needs both to try the key.
    let pending_store = store.clone();
    let pending_record = record.clone();
    let persisted_this_run = Arc::new(AtomicBool::new(false));
    let persisted = persisted_this_run.clone();
    let closure_previous = previous_band.clone();
    let session = BandSession::new(None, Some(OwnershipCeremony::default()), false);
    let link = tokio::spawn(run_link(
        io,
        session,
        clock,
        command_rx,
        event_tx,
        move |identity| {
            pending_store.save_band(&pending_record)?;
            if let Err(error) = pending_store.save_pending(identity) {
                // The claim is abandoned before FinishChangeOwner is written:
                // put the previous band record back (best effort).
                let _ = restore_band(&pending_store, closure_previous.as_ref());
                return Err(error);
            }
            persisted.store(true, Ordering::SeqCst);
            Ok(())
        },
    ));

    let mut record = record;
    let mut failure: Option<PairFailure> = None;
    let mut saved = false;
    let mut resolved = false;
    while let Some(event) = events.recv().await {
        let LinkEvent::Band(event) = event else {
            continue;
        };
        match event {
            Event::CeremonyStage(stage) => progress(&stage),
            Event::CeremonyHttp(CeremonyHttpRequest::PairRequest(request)) => {
                progress("asking Meta to approve the claim");
                let next = match client.pair_request(&request).await {
                    Ok(pending) => LinkCommand::ResumePairRequest {
                        signature: pending.signature,
                        receipt: pending.receipt,
                    },
                    Err(error) => {
                        failure = Some(from_pair_error(error));
                        LinkCommand::Stop
                    }
                };
                let _ = commands.send(next).await;
            }
            Event::CeremonyHttp(CeremonyHttpRequest::Pair(pair)) => {
                progress("confirming the claim with Meta");
                let next = match client.pair(&pair).await {
                    Ok(done) => LinkCommand::ResumePair {
                        signature: done.signature,
                        receipt: done.receipt,
                        device_public_key: done.device_public_key,
                    },
                    Err(error) => {
                        failure = Some(from_pair_error(error));
                        LinkCommand::Stop
                    }
                };
                let _ = commands.send(next).await;
            }
            Event::CeremonyCompleted(identity) => match store
                .save_band(&record)
                .and_then(|()| store.save_identity(&identity))
            {
                Ok(()) => {
                    saved = true;
                    progress("the band accepted this computer as its owner");
                    if !resolved {
                        resolved = true;
                        resolve_identity(transport, store, &mut record).await;
                    }
                }
                Err(error) => {
                    failure = Some(PairFailure::Failed(format!(
                        "could not save the owner key: {error}"
                    )));
                    let _ = commands.send(LinkCommand::Stop).await;
                }
            },
            Event::Connected => {
                progress("connected as the owner");
                if !resolved {
                    resolved = true;
                    resolve_identity(transport, store, &mut record).await;
                }
                let _ = commands.send(LinkCommand::Stop).await;
            }
            _ => {}
        }
    }
    let outcome = link
        .await
        .map_err(|error| PairFailure::Failed(format!("the pairing task failed: {error}")))?;
    if !saved
        && let Some(identity) = &outcome.enrollment
        && store.save_identity(identity).is_ok()
    {
        saved = true;
    }
    let persisted = persisted_this_run.load(Ordering::SeqCst);
    // Band-record rule: band.json is kept when this run confirmed an owner, or
    // saved a pending key the band may have committed to. A pending key from an
    // earlier run never counts. An explicit ownership rejection means the band
    // did not commit: drop this run's pending key and put back the band record
    // that was there before this run (or none), so the old owner key and its
    // band stay paired.
    if !saved && persisted && matches!(outcome.end, LinkEnd::OwnershipRejected { .. }) {
        store.discard_pending().map_err(|error| {
            PairFailure::Failed(format!("could not drop the rejected owner key: {error}"))
        })?;
        restore_band(store, previous_band.as_ref()).map_err(|error| {
            PairFailure::Failed(format!("could not restore the band record: {error}"))
        })?;
    } else if saved || persisted {
        // Belt and braces: the closure / completion paths already saved it.
        store.save_band(&record).map_err(|error| {
            PairFailure::Failed(format!("could not save the band record: {error}"))
        })?;
    }
    if let Some(failure) = failure {
        return Err(failure);
    }
    if saved {
        return Ok(record);
    }
    Err(match outcome.end {
        LinkEnd::OwnershipRejected {
            code: 0x1042,
            message,
        } => PairFailure::WrongAccount(message),
        LinkEnd::OwnershipRejected { message, .. } | LinkEnd::IdentityRejected(message) => {
            PairFailure::Rejected(message)
        }
        LinkEnd::Failed(message) => PairFailure::Failed(message),
        other => PairFailure::Failed(format!("the claim didn't finish: {other}")),
    })
}

/// Once bonded, a band advertising a resolvable private address is known by
/// its identity address: save that, so later connections find it.
async fn resolve_identity<T: Transport>(transport: &T, store: &Store, record: &mut BandRecord) {
    let identity = transport.identity_address(&record.address).await;
    if identity != record.address {
        tracing::info!("the band's identity address is {identity}");
        record.address = identity;
        if let Err(error) = store.save_band(record) {
            tracing::warn!("could not save the band's identity address: {error}");
        }
    }
}

/// Put back the band record from before this pairing run (or none).
fn restore_band(store: &Store, previous: Option<&BandRecord>) -> std::io::Result<()> {
    match previous {
        Some(band) => store.save_band(band),
        None => store.forget_band(),
    }
}
