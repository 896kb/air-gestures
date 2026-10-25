use std::os::unix::fs::PermissionsExt;

use air_gesturesd::meta::MetaSession;
use air_gesturesd::status::{LinkState, PairResult, Status};
use air_gesturesd::store::{BandRecord, IdentitySource, Store};
use band_core::identity::EnrollmentIdentity;
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
    let dir = root.path().join("air-gestures");
    let store = Store::open(&dir).unwrap();
    assert_eq!(mode(&dir), 0o700);
    store.save_identity(&identity(true)).unwrap();
    store
        .save_band(&BandRecord {
            address: "AA:BB:CC:DD:EE:FF".into(),
            name: "Meta Band 1234".into(),
            scheme_guess: None,
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
fn the_band_record_round_trips_and_forget_removes_everything_but_the_meta_session() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    assert!(store.load_band().unwrap().is_none());
    let band = BandRecord {
        address: "AA:BB:CC:DD:EE:FF".into(),
        name: "Meta Band 1234".into(),
        scheme_guess: None,
    };
    store.save_band(&band).unwrap();
    assert_eq!(store.load_band().unwrap(), Some(band));
    store.save_identity(&identity(true)).unwrap();
    store.save_pending(&identity(true)).unwrap();
    store
        .save_meta_session(&MetaSession::new("test-access-token", "1234567890"))
        .unwrap();
    store.forget().unwrap();
    assert!(store.load_band().unwrap().is_none());
    assert!(store.connect_identity().unwrap().is_none());
    // Forgetting a band keeps the Meta sign-in; signing out is separate
    // (`Store::delete_meta_session`).
    assert!(store.load_meta_session().unwrap().is_some());
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
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
        reconnect_in: Some(4),
        last_pair: Some(PairResult {
            ok: false,
            message: "no band".into(),
            needs_sign_in: false,
        }),
        ..Status::default()
    };
    let json = serde_json::to_value(&full).unwrap();
    assert_eq!(json["last_pair"]["ok"], false);
    assert_eq!(json["reconnect_in"], 4);
    let back: Status = serde_json::from_value(json).unwrap();
    assert_eq!(back, full);
}

#[test]
fn a_corrupt_pending_key_does_not_block_the_confirmed_one() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    let confirmed = identity(true);
    store.save_identity(&confirmed).unwrap();
    std::fs::write(root.path().join("owner.pending.key"), [9; 5]).unwrap();
    let (found, source) = store.connect_identity().unwrap().unwrap();
    assert_eq!(source, IdentitySource::Confirmed);
    assert_eq!(found.private_bytes(), confirmed.private_bytes());
}

#[test]
fn a_corrupt_pending_key_with_nothing_confirmed_is_still_an_error() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    std::fs::write(root.path().join("owner.pending.key"), [9; 5]).unwrap();
    assert_eq!(
        store.connect_identity().unwrap_err().kind(),
        std::io::ErrorKind::InvalidData
    );
}

#[test]
fn forget_also_removes_leftover_temp_files() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    store.save_identity(&identity(true)).unwrap();
    let temps = [
        ".owner.key.tmp",
        ".owner.pending.key.tmp",
        ".band.json.tmp",
        ".meta-session.json.tmp",
    ];
    for name in temps {
        std::fs::write(root.path().join(name), b"half-written").unwrap();
    }
    store.forget().unwrap();
    for name in temps {
        assert!(!root.path().join(name).exists(), "{name} left behind");
    }
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

use air_gesturesd::store::{HandCalibration, StoredDrags, StoredPalm};
use band_core::controls::palm::PalmCalibration;
use band_core::controls::pinch::Prototypes;
use band_core::events::Hand;

fn taught() -> HandCalibration {
    HandCalibration {
        palm: Some(StoredPalm { down_roll: -140.0, up_roll: -10.0 }),
        drags: StoredDrags::from_prototypes(&Prototypes::right_hand()),
    }
}

#[test]
fn calibrations_round_trip_per_band_and_hand_and_survive_forget() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    assert_eq!(store.load_calibration("AA", Hand::Right).unwrap(), HandCalibration::default());
    store.save_calibration("AA", Hand::Right, &taught()).unwrap();
    let left = HandCalibration { palm: Some(StoredPalm { down_roll: 30.0, up_roll: 150.0 }), drags: None };
    store.save_calibration("BB", Hand::Left, &left).unwrap();
    assert_eq!(store.load_calibration("AA", Hand::Right).unwrap(), taught());
    assert_eq!(store.load_calibration("AA", Hand::Left).unwrap(), HandCalibration::default());
    assert_eq!(store.load_calibration("BB", Hand::Left).unwrap(), left);
    assert_eq!(mode(&root.path().join("calibration.json")), 0o600);
    store.forget().unwrap();
    assert_eq!(store.load_calibration("AA", Hand::Right).unwrap(), taught(), "it describes the wrist, not the key");
    store.save_calibration("AA", Hand::Right, &HandCalibration::default()).unwrap();
    assert_eq!(store.load_calibration("AA", Hand::Right).unwrap(), HandCalibration::default());
}

#[test]
fn a_corrupt_calibration_file_is_an_error_and_a_save_sets_it_aside() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    let path = root.path().join("calibration.json");
    for corrupt in ["{nope", r#"{"version":9,"bands":{}}"#, r#"{"bands":{}}"#] {
        std::fs::write(&path, corrupt).unwrap();
        let error = store.load_calibration("AA", Hand::Right).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData, "{corrupt}");
        store.save_calibration("AA", Hand::Right, &taught()).unwrap();
        assert_eq!(std::fs::read_to_string(root.path().join("calibration.json.bad")).unwrap(), corrupt, "kept aside");
        assert_eq!(store.load_calibration("AA", Hand::Right).unwrap(), taught(), "started afresh");
    }
}

#[test]
fn the_calibration_file_is_versioned_and_one_bad_entry_spoils_nothing_else() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    store.save_calibration("AA", Hand::Left, &taught()).unwrap();
    let path = root.path().join("calibration.json");
    let mut json: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(json["version"], 1);
    json["bands"]["AA"]["right"] = serde_json::json!({"palm": "nope"});
    std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
    assert_eq!(store.load_calibration("AA", Hand::Right).unwrap(), HandCalibration::default(), "skipped");
    assert_eq!(store.load_calibration("AA", Hand::Left).unwrap(), taught());
    store.save_calibration("BB", Hand::Right, &taught()).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(json["bands"]["AA"]["right"]["palm"], "nope", "an entry nobody changed is written back as it was");
    assert_eq!(store.load_calibration("BB", Hand::Right).unwrap(), taught());
}

#[test]
fn a_calibration_is_checked_on_the_way_in() {
    let (palm, drags, problems) = taught().apply();
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(palm, Some(PalmCalibration { down_roll: -140.0, up_roll: -10.0 }));
    let measured = Prototypes::right_hand().drags.unwrap();
    for (a, b) in drags.unwrap().drags.unwrap().iter().zip(measured) {
        assert!((0..3).all(|k| (a[k] - b[k]).abs() < 1e-12));
    }
    let mut bad = taught();
    let stored = bad.drags.as_mut().unwrap();
    stored.drags[1] = stored.drags[0];
    bad.palm = Some(StoredPalm { down_roll: -140.0, up_roll: -120.0 });
    let (palm, drags, problems) = bad.apply();
    assert_eq!((palm, drags.is_none()), (None, true));
    assert_eq!(problems.len(), 2, "{problems:?}");
    assert!(problems.iter().any(|p| p.contains("teach-drags")), "{problems:?}");
    assert!(problems.iter().any(|p| p.contains("air-gestures calibrate") && p.contains("60°")), "{problems:?}");
    assert_eq!(HandCalibration::default().apply(), (None, None, Vec::new()), "nothing taught: the core's defaults");
}
