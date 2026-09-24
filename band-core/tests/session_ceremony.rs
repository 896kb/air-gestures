mod common;

use band_core::ceremony::{CeremonyHttpRequest, OwnershipCeremony};
use band_core::datax::{encode_frame, DataXFrame};
use band_core::events::Event;
use band_core::identity::{point64, trust_digest};
use band_core::proto::{field_bytes as fb, ProtoFields};
use band_core::session::BandSession;
use band_core::sim::SimBand;
use band_core::{BandError, Result};
use p256::ecdsa::signature::hazmat::PrehashVerifier;
use p256::ecdsa::{Signature, SigningKey};
use rand_core::OsRng;
use sha2::{Digest, Sha256};

const SERIAL: &str = "TESTSERIAL0001";
const PENDING_RECEIPT: &str = r#"{"serial":"TESTSERIAL0001","receipt_type":"ServerPendingOwnershipReceipt"}"#;
const DEVICE_RECEIPT: &str = r#"{"receipt_type":"DevicePendingOwnershipReceipt","additional_data":"{}"}"#;
const FINAL_RECEIPT: &str = r#"{"receipt_type":"ServerFinalOwnershipReceipt"}"#;

fn nonce() -> Vec<u8> {
    (0..16).collect()
}

fn identity_response() -> Vec<u8> {
    let payload = [fb(1, &[1; 967]), fb(2, SERIAL.as_bytes()), fb(5, &[2; 889])].concat();
    encode_frame(2, &[0x02003001], &payload).unwrap()
}

fn skip_challenge_response() -> Vec<u8> {
    encode_frame(2, &[0x02002001], &[vec![0x0a, 0x10], nonce()].concat()).unwrap()
}

/// A pairing-mode band running the ceremony (kinesis `CeremonyPeer`).
struct CeremonyPeer {
    session: BandSession,
    band: SimBand,
    startup: Vec<DataXFrame>,
}

impl CeremonyPeer {
    fn new() -> CeremonyPeer {
        let mut session = BandSession::new(None, Some(OwnershipCeremony::default()), false);
        let (band, startup) = SimBand::connect(&mut session).unwrap();
        CeremonyPeer { session, band, startup }
    }

    fn send(&mut self, frame: &[u8]) -> Result<(Vec<Event>, Vec<DataXFrame>)> {
        let record = self.band.encrypt(frame);
        let result = self.session.feed(&record, 100.0)?;
        Ok((result.events, self.band.requests(&result.outgoing)?))
    }
}

#[test]
fn the_session_runs_the_whole_ceremony_and_rejoins_the_enrolled_startup() {
    let mut peer = CeremonyPeer::new();
    let app_point = peer.session.ceremony().unwrap().app_public_key();
    // After the transport handshake the session opens the identity service.
    assert_eq!(peer.startup.len(), 1);
    assert_eq!(peer.startup[0].channel, 0x8002);
    assert_eq!(peer.startup[0].words, vec![0x81000024, 0x02003000]);
    // IdentityResponse -> SkipChallenge.
    let skip = peer.send(&identity_response()).unwrap().1;
    assert!(skip.len() == 1 && skip[0].channel == 0x8002 && skip[0].words == vec![0x02002000]);
    // SkipChallengeResponse -> pair_request event with the nonce.
    let challenge = peer.send(&skip_challenge_response()).unwrap().0;
    let request = challenge
        .iter()
        .find_map(|event| match event {
            Event::CeremonyHttp(CeremonyHttpRequest::PairRequest(request)) => Some(request.clone()),
            _ => None,
        })
        .expect("pair_request event");
    assert_eq!(request.nonce, nonce());
    assert_eq!(request.app_public_key, app_point);
    assert_eq!(request.identity.serial, SERIAL);
    // The HTTP reply becomes StartChangeOwner on channel 0x8002.
    let pending_signature = [vec![0x30, 0x45, 0x02, 0x20], vec![3; 67]].concat();
    let start_bytes = peer.session.ceremony_pair_request_completed(&pending_signature, PENDING_RECEIPT).unwrap();
    let start = peer.band.requests(&start_bytes).unwrap().remove(0);
    assert!(start.channel == 0x8002 && start.words == vec![0x02002002]);
    assert_eq!(ProtoFields::parse(&start.payload).unwrap().bytes(2).unwrap(), PENDING_RECEIPT.as_bytes());
    // StartChangeOwnerResponse -> pair event with the verbatim receipt.
    let device_signature = [vec![0x30, 0x46, 0x02, 0x21], vec![4; 66]].concat();
    let response = encode_frame(2, &[0x02002003], &[fb(1, &device_signature), fb(2, DEVICE_RECEIPT.as_bytes())].concat()).unwrap();
    let events = peer.send(&response).unwrap().0;
    let pair = events
        .iter()
        .find_map(|event| match event {
            Event::CeremonyHttp(CeremonyHttpRequest::Pair(pair)) => Some(pair.clone()),
            _ => None,
        })
        .expect("pair event");
    assert_eq!((pair.receipt.as_str(), pair.signature.clone()), (DEVICE_RECEIPT, device_signature));
    // The final receipt becomes FinishChangeOwner; the empty confirmation hands
    // the new identity out and opens the enrolled trust flow.
    let band_key = SigningKey::random(&mut OsRng);
    let band_point = point64(band_key.verifying_key());
    let final_signature = [vec![0x30, 0x45, 0x02, 0x21], vec![5; 67]].concat();
    let finish_bytes = peer.session.ceremony_pair_completed(&final_signature, FINAL_RECEIPT, Some(&band_point)).unwrap();
    let finish = peer.band.requests(&finish_bytes).unwrap().remove(0);
    assert!(finish.channel == 0x8002 && finish.words == vec![0x02002004]);
    let (events, requests) = peer.send(&encode_frame(2, &[0x02002005], &[]).unwrap()).unwrap();
    let identity = events
        .iter()
        .find_map(|event| match event {
            Event::CeremonyCompleted(identity) => Some(identity.clone()),
            _ => None,
        })
        .expect("ceremony completed event");
    assert_eq!(identity.app_point(), app_point);
    assert_eq!(identity.band_public_key, Some(*band_key.verifying_key()));
    // The service is already open on 0x8002, so the trust proof uses one word.
    let trust = &requests[0];
    assert!(trust.channel == 0x8002 && trust.words == vec![0x02001000]);
    let trust_fields = ProtoFields::parse(&trust.payload).unwrap();
    assert_eq!(trust_fields.bytes(1).unwrap(), Sha256::digest(&app_point).as_slice());
    let digest = trust_digest(&peer.band.band_challenge, &peer.band.transport_point, &peer.band.host_seed, &peer.band.host_point);
    let signature = Signature::from_slice(trust_fields.bytes_len(2, 64).unwrap()).unwrap();
    identity.private_key.verifying_key().verify_prehash(&digest, &signature).unwrap();
    // From here the standard enrolled startup continues: band proof accepted, then EndLinkSetup.
    assert!(peer.send(&encode_frame(2, &[0x03001000], &[]).unwrap()).unwrap().1.is_empty());
    let proof = peer.band.trust_proof(&band_key).unwrap();
    let acknowledged = peer.send(&proof).unwrap().1;
    assert_eq!(acknowledged.first().unwrap().channel, 3);
    let end = acknowledged.last().unwrap();
    assert!(end.channel == 0x8001 && end.words == vec![0x02001000]);
}

#[test]
fn identity_result_codes_fail_the_ceremony_session() {
    let mut peer = CeremonyPeer::new();
    peer.send(&identity_response()).unwrap();
    match peer.send(&encode_frame(2, &[0x03001043], &[]).unwrap()) {
        Err(BandError::Protocol(message)) => assert!(message.contains("identity"), "{message}"),
        other => panic!("expected a protocol error, got {other:?}"),
    }
}
