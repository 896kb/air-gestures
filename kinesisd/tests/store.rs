use std::os::unix::fs::PermissionsExt;

use band_core::identity::EnrollmentIdentity;
use kinesisd::status::{LinkState, Status};
use kinesisd::store::{BandRecord, Store};
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
    for name in ["owner.key", "band.pub", "band.json"] {
        assert_eq!(mode(&dir.join(name)), 0o600, "{name}");
    }
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
    assert_eq!(
        store.connect_identity().unwrap().unwrap().private_bytes(),
        pending.private_bytes()
    );
    let confirmed = identity(true);
    store.save_identity(&confirmed).unwrap();
    assert!(
        store.load_pending().unwrap().is_none(),
        "confirming clears the pending key"
    );
    assert_eq!(
        store.connect_identity().unwrap().unwrap().private_bytes(),
        confirmed.private_bytes()
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
