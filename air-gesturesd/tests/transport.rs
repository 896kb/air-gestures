//! Drives a full `BandSession` against `SimTransport` over its duplex pipe —
//! the same "sans-IO core plus real transport" pattern `link.rs` (Task 7) uses.

use air_gesturesd::transport::{SIM_ADDRESS, SimTransport, Transport};
use band_core::Result;
use band_core::ceremony::{CeremonyHttpRequest, OwnershipCeremony};
use band_core::error::BandError;
use band_core::events::Event;
use band_core::identity::EnrollmentIdentity;
use band_core::responder::SimMode;
use band_core::session::BandSession;
use p256::ecdsa::SigningKey;
use rand_core::OsRng;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

const PENDING_RECEIPT: &str = r#"{"receipt_type":"ServerPendingOwnershipReceipt"}"#;
const FINAL_RECEIPT: &str = r#"{"receipt_type":"ServerFinalOwnershipReceipt"}"#;

/// Feed the session with whatever the band sends, answering any HTTP step with
/// fabricated (unvalidated) data, until it connects or the session errors out.
async fn pump(
    io: &mut DuplexStream,
    session: &mut BandSession,
    transport: &SimTransport,
) -> Result<Vec<Event>> {
    let mut buffer = vec![0u8; 1 << 16];
    let mut all_events = Vec::new();
    io.write_all(&session.request()?)
        .await
        .expect("write to the simulated band");
    loop {
        let count = io
            .read(&mut buffer)
            .await
            .expect("read from the simulated band");
        if count == 0 {
            return Ok(all_events);
        }
        let result = session.feed(&buffer[..count], 0.0)?;
        let mut outgoing = result.outgoing;
        for event in &result.events {
            match event {
                Event::CeremonyHttp(CeremonyHttpRequest::PairRequest(_)) => {
                    outgoing.extend(
                        session.ceremony_pair_request_completed(&[3; 71], PENDING_RECEIPT)?,
                    );
                }
                Event::CeremonyHttp(CeremonyHttpRequest::Pair(_)) => {
                    let band_point = transport.band_point();
                    outgoing.extend(session.ceremony_pair_completed(
                        &[5; 71],
                        FINAL_RECEIPT,
                        Some(&band_point),
                    )?);
                }
                _ => {}
            }
        }
        if !outgoing.is_empty() {
            io.write_all(&outgoing)
                .await
                .expect("write to the simulated band");
        }
        let connected = result
            .events
            .iter()
            .any(|event| matches!(event, Event::Connected));
        all_events.extend(result.events);
        if connected {
            return Ok(all_events);
        }
    }
}

#[tokio::test]
async fn a_completed_ceremony_leaves_the_band_owned() {
    let transport = SimTransport::new(SimMode::Legacy);
    transport.prepare_pairing(SIM_ADDRESS).await.unwrap();
    let mut io = transport.open(SIM_ADDRESS).await.unwrap();

    let mut session = BandSession::new(None, Some(OwnershipCeremony::default()), false);
    let events = pump(&mut io, &mut session, &transport)
        .await
        .expect("the ceremony to complete");
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::CeremonyCompleted(_)))
    );
    assert!(transport.owner().is_some());
}

#[tokio::test]
async fn a_rejected_claim_leaves_the_band_unowned_and_still_legacy() {
    let transport = SimTransport::new(SimMode::Legacy);
    transport.set_reject_claim(Some(0x03001042));
    transport.prepare_pairing(SIM_ADDRESS).await.unwrap();
    let mut io = transport.open(SIM_ADDRESS).await.unwrap();

    let mut session = BandSession::new(None, Some(OwnershipCeremony::default()), false);
    match pump(&mut io, &mut session, &transport).await {
        Err(BandError::OwnershipRejected { code, .. }) => assert_eq!(code, 0x1042),
        other => panic!("expected an ownership rejection, got {other:?}"),
    }
    assert!(transport.owner().is_none());

    // The band never enrolled, so a later connection still refuses an
    // enrolled session — it did not silently adopt the rejected claim.
    let identity = EnrollmentIdentity::new(SigningKey::random(&mut OsRng), None);
    let mut io = transport.open(SIM_ADDRESS).await.unwrap();
    let mut session = BandSession::new(Some(identity), None, false);
    match pump(&mut io, &mut session, &transport).await {
        Err(BandError::IdentityMismatch { code: 0x1043, .. }) => {}
        other => panic!("expected an identity mismatch, got {other:?}"),
    }
}

#[tokio::test]
async fn a_rejected_claim_on_an_already_enrolled_band_still_resets_to_pairing_mode() {
    // A factory reset puts a previously-enrolled band back in pairing mode
    // with no owner; `prepare_pairing` must reflect that even if the
    // ceremony that follows doesn't finish.
    let transport = SimTransport::new(SimMode::Enrolled);
    transport.set_reject_claim(Some(0x03001042));
    transport.prepare_pairing(SIM_ADDRESS).await.unwrap();
    let mut io = transport.open(SIM_ADDRESS).await.unwrap();

    let mut session = BandSession::new(None, Some(OwnershipCeremony::default()), false);
    match pump(&mut io, &mut session, &transport).await {
        Err(BandError::OwnershipRejected { code, .. }) => assert_eq!(code, 0x1042),
        other => panic!("expected an ownership rejection, got {other:?}"),
    }
    assert!(transport.owner().is_none());

    // The next connection still refuses an enrolled session — the previous
    // owner was reset by the factory reset, not silently kept.
    let identity = EnrollmentIdentity::new(SigningKey::random(&mut OsRng), None);
    let mut io = transport.open(SIM_ADDRESS).await.unwrap();
    let mut session = BandSession::new(Some(identity), None, false);
    match pump(&mut io, &mut session, &transport).await {
        Err(BandError::IdentityMismatch { code: 0x1043, .. }) => {}
        other => panic!("expected an identity mismatch, got {other:?}"),
    }
}
