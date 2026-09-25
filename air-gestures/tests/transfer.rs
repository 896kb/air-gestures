use std::os::unix::fs::PermissionsExt;

use air_gestures::transfer;
use air_gesturesd::store::{self, BandRecord, Store};
use band_core::identity::EnrollmentIdentity;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use p256::ecdsa::SigningKey;
use rand_core::OsRng;

fn identity(with_band_key: bool) -> EnrollmentIdentity {
    let band = with_band_key.then(|| *SigningKey::random(&mut OsRng).verifying_key());
    EnrollmentIdentity::new(SigningKey::random(&mut OsRng), band)
}

fn band() -> BandRecord {
    BandRecord {
        address: "AA:BB:CC:DD:EE:FF".into(),
        name: "Meta Band 1234".into(),
        scheme_guess: Some(2),
    }
}

fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("state")).unwrap();
    (dir, store)
}

#[test]
fn a_bundle_round_trips_the_key_and_band() {
    for with_band_key in [false, true] {
        let (dir, from) = store();
        let key = identity(with_band_key);
        from.save_identity(&key).unwrap();
        from.save_band(&band()).unwrap();
        let path = dir.path().join(transfer::DEFAULT_FILE);
        transfer::export(&from, &path, false).unwrap();

        let (_other, to) = store();
        to.save_pending(&identity(false)).unwrap();
        let imported = transfer::parse(&std::fs::read(&path).unwrap()).unwrap();
        transfer::import(&to, &imported).unwrap();
        assert_eq!(
            store::identity_bytes(&to.load_identity().unwrap().unwrap()),
            store::identity_bytes(&key)
        );
        assert_eq!(to.load_band().unwrap(), Some(band()));
        assert!(!to.has_pending(), "the imported key replaces a pending one");
    }
}

#[test]
fn the_export_is_private_and_not_overwritten_without_force() {
    let (dir, store) = store();
    store.save_identity(&identity(true)).unwrap();
    let path = dir.path().join("band.json");
    std::fs::write(&path, "old").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let error = transfer::export(&store, &path, false).unwrap_err();
    assert!(error.contains("--force"), "{error}");
    transfer::export(&store, &path, true).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);

    let fresh = dir.path().join("fresh.json");
    transfer::export(&store, &fresh, false).unwrap();
    let mode = std::fs::metadata(&fresh).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    // No band record here: the bundle has only the key.
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&fresh).unwrap()).unwrap();
    assert_eq!(json["format"], "air-gestures-band");
    assert_eq!(json["version"], 1);
    assert!(json.get("band").is_none());
}

#[test]
fn export_refuses_an_unconfirmed_claim() {
    let (dir, store) = store();
    store.save_pending(&identity(false)).unwrap();
    let path = dir.path().join(transfer::DEFAULT_FILE);
    let error = transfer::export(&store, &path, false).unwrap_err();
    assert!(error.contains("isn't confirmed"), "{error}");
    assert!(!path.exists());
}

#[test]
fn a_raw_owner_key_imports_without_touching_the_band() {
    let (_dir, store) = store();
    store.save_band(&band()).unwrap();
    let key = identity(true);
    let imported = transfer::parse(&store::identity_bytes(&key)).unwrap();
    assert!(imported.band.is_none());
    transfer::import(&store, &imported).unwrap();
    assert_eq!(
        store::identity_bytes(&store.load_identity().unwrap().unwrap()),
        store::identity_bytes(&key)
    );
    assert_eq!(store.load_band().unwrap(), Some(band()));
}

fn bundle_json(format: &str, version: u64, key: &[u8]) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "format": format,
        "version": version,
        "owner_key": STANDARD.encode(key),
        "band": { "address": "AA:BB:CC:DD:EE:FF", "name": "Meta Band 1234", "scheme_guess": 0 },
    }))
    .unwrap()
}

#[test]
fn bad_files_are_rejected() {
    let key = store::identity_bytes(&identity(false));
    let parsed = transfer::parse(&bundle_json("air-gestures-band", 1, &key)).unwrap();
    assert_eq!(parsed.band.unwrap().scheme_guess, Some(0));

    let error = transfer::parse(&bundle_json("something-else", 1, &key))
        .err()
        .unwrap();
    assert!(error.contains("format"), "{error}");
    let error = transfer::parse(&bundle_json("air-gestures-band", 2, &key))
        .err()
        .unwrap();
    assert!(error.contains("newer"), "{error}");
    let error = transfer::parse(&bundle_json("air-gestures-band", 1, &key[..31]))
        .err()
        .unwrap();
    assert!(error.contains("31 bytes"), "{error}");
    // 32 bytes that aren't a valid P-256 scalar.
    let error = transfer::parse(&bundle_json("air-gestures-band", 1, &[0; 32]))
        .err()
        .unwrap();
    assert!(error.contains("owner key"), "{error}");
    let error = transfer::parse(&[1; 40]).err().unwrap();
    assert!(error.contains("not an air-gestures band file"), "{error}");
}
