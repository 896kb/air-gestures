use std::os::unix::fs::PermissionsExt;

use band_core::identity::EnrollmentIdentity;
use kinesisd::status::{LinkState, PairResult, Status};
use kinesisd::store::{BandRecord, IdentitySource, Store};
use p256::ecdsa::SigningKey;
use rand_core::OsRng;

fn identity(with_band_key: bool) -> EnrollmentIdentity {
    let band = SigningKey::random(&mut OsRng);
    EnrollmentIdentity::new(
        SigningKey::random(&mut OsRng),
        with_band_key.then(|| *band.verifying_key()),
    )
}

fn mode(path: &std::path::Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn the_directory_and_every_file_are_private() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("kinesis");
    let store = Store::open(&dir).unwrap();
    assert_eq!(mode(&dir), 0o700);
    store.save_identity(&identity(true)).unwrap();
    store
        .save_band(&BandRecord {
            address: "AA:BB:CC:DD:EE:FF".into(),
            name: "Meta Band 1234".into(),
        })
        .unwrap();
    // A single `owner.key` now holds the private key plus any band key
    // (there's no more separate `band.pub` file).
    for name in ["owner.key", "band.json"] {
        assert_eq!(mode(&dir.join(name)), 0o600, "{name}");
    }
    assert!(!dir.join("band.pub").exists());
}

#[test]
fn identities_round_trip_with_and_without_a_band_key() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    assert!(store.load_identity().unwrap().is_none());
    let with = identity(true);
    store.save_identity(&with).unwrap();
    let loaded = store.load_identity().unwrap().unwrap();
    assert_eq!(loaded.private_bytes(), with.private_bytes());
    assert_eq!(loaded.band_key_x963(), with.band_key_x963());
    let without = identity(false);
    store.save_identity(&without).unwrap();
    assert!(
        store
            .load_identity()
            .unwrap()
            .unwrap()
            .band_public_key
            .is_none(),
        "a stale band key is removed"
    );
}

#[test]
fn a_pending_identity_is_used_until_one_is_confirmed() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    let pending = identity(true);
    store.save_pending(&pending).unwrap();
    assert!(store.load_identity().unwrap().is_none());
    let (loaded, source) = store.connect_identity().unwrap().unwrap();
    assert_eq!(loaded.private_bytes(), pending.private_bytes());
    assert_eq!(source, IdentitySource::Pending);

    // A pending claim is always newer: even an older confirmed identity
    // sitting alongside it must not win.
    let older_confirmed = identity(true);
    store.save_identity(&older_confirmed).unwrap();
    store.save_pending(&pending).unwrap();
    let (loaded, source) = store.connect_identity().unwrap().unwrap();
    assert_eq!(loaded.private_bytes(), pending.private_bytes());
    assert_eq!(source, IdentitySource::Pending);

    let confirmed = identity(true);
    store.save_identity(&confirmed).unwrap();
    assert!(
        store.load_pending().unwrap().is_none(),
        "confirming clears the pending key"
    );
    let (loaded, source) = store.connect_identity().unwrap().unwrap();
    assert_eq!(loaded.private_bytes(), confirmed.private_bytes());
    assert_eq!(source, IdentitySource::Confirmed);
}

#[test]
fn pending_identities_can_be_promoted_or_discarded() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();

    // promote_pending is a no-op with nothing pending.
    store.promote_pending().unwrap();
    assert!(store.load_identity().unwrap().is_none());

    let pending = identity(true);
    store.save_pending(&pending).unwrap();
    store.promote_pending().unwrap();
    assert!(store.load_pending().unwrap().is_none());
    assert_eq!(
        store.load_identity().unwrap().unwrap().private_bytes(),
        pending.private_bytes()
    );

    let other_pending = identity(false);
    store.save_pending(&other_pending).unwrap();
    store.discard_pending().unwrap();
    assert!(store.load_pending().unwrap().is_none());
    // Discarding the pending key must not touch the confirmed one.
    assert_eq!(
        store.load_identity().unwrap().unwrap().private_bytes(),
        pending.private_bytes()
    );
}

#[test]
fn an_identity_file_with_an_invalid_length_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    // Neither a bare 32-byte key nor a 32+65-byte key plus band key.
    std::fs::write(root.path().join("owner.key"), [7u8; 50]).unwrap();
    assert_eq!(
        store.load_identity().unwrap_err().kind(),
        std::io::ErrorKind::InvalidData
    );
}

#[test]
fn the_band_record_round_trips_and_forget_removes_everything() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    assert!(store.load_band().unwrap().is_none());
    let band = BandRecord {
        address: "AA:BB:CC:DD:EE:FF".into(),
        name: "Meta Band 1234".into(),
    };
    store.save_band(&band).unwrap();
    assert_eq!(store.load_band().unwrap(), Some(band));
    store.save_identity(&identity(true)).unwrap();
    store.save_pending(&identity(true)).unwrap();
    store.forget().unwrap();
    assert!(store.load_band().unwrap().is_none());
    assert!(store.connect_identity().unwrap().is_none());
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn a_corrupt_key_is_an_error_not_a_missing_one() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    std::fs::write(root.path().join("owner.key"), [1, 2, 3]).unwrap();
    assert_eq!(
        store.load_identity().unwrap_err().kind(),
        std::io::ErrorKind::InvalidData
    );
}

#[test]
fn status_serializes_for_clients() {
    let status = Status {
        state: LinkState::Connected,
        battery: Some(84),
        ..Status::default()
    };
    let json = serde_json::to_value(&status).unwrap();
    assert_eq!(json["state"], "connected");
    assert_eq!(json["battery"], 84);
    assert_eq!(json["dial"], "volume");
    let back: Status = serde_json::from_value(json).unwrap();
    assert_eq!(back, status);
}

#[test]
fn status_contract_v1() {
    let status = Status::default();
    assert_eq!(status.version, 1);
    let json = serde_json::to_value(&status).unwrap();
    assert_eq!(json["version"], 1);
    for field in [
        "error",
        "pairing_error",
        "config_error",
        "reconnect_in",
        "last_pair",
    ] {
        assert!(json.get(field).is_some(), "{field} missing from {json}");
    }
    // Older / partial JSON still parses, as version 1.
    let partial: Status = serde_json::from_str(r#"{"state":"idle"}"#).unwrap();
    assert_eq!(partial.version, 1);
    let full = Status {
        pairing_error: Some("no band".into()),
        config_error: Some("config: bad".into()),
        reconnect_in: Some(4.0),
        last_pair: Some(PairResult {
            ok: false,
            message: "no band".into(),
        }),
        ..Status::default()
    };
    let json = serde_json::to_value(&full).unwrap();
    assert_eq!(json["last_pair"]["ok"], false);
    assert_eq!(json["reconnect_in"], 4.0);
    let back: Status = serde_json::from_value(json).unwrap();
    assert_eq!(back, full);
}
