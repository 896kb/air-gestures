use air_gesturesd::meta::{
    ClientFactory, FakeFactory, MetaSession, PairClient, PairError, is_session_failure,
    pair_fields, pair_request_fields, parse_final, parse_pending, url_form,
};
use air_gesturesd::store::Store;
use band_core::ceremony::{BandIdentityInfo, CeremonyPair, CeremonyPairRequest};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde_json::json;

fn session() -> MetaSession {
    MetaSession::new("test-access-token", "1234567890")
}

#[test]
fn pair_request_encodes_the_exact_reference_field_set() {
    let request = CeremonyPairRequest {
        identity: BandIdentityInfo {
            device_certificate: vec![1, 2, 3, 4],
            serial: "TESTSERIAL0001".into(),
            secondary_certificate: vec![5, 6, 7, 8],
        },
        nonce: (0..16).collect(),
        app_public_key: vec![9; 64],
    };
    let fields = pair_request_fields(&request, &session());
    let names: Vec<&str> = fields.iter().map(|(name, _)| *name).collect();
    assert_eq!(
        names,
        [
            "access_token",
            "user_access_token",
            "user_token_universe",
            "pair_protocol_version",
            "device_cert",
            "serial_number",
            "additional_data"
        ]
    );
    assert_eq!(
        fields[0].1,
        "HW|1312539125771114|98588f106d5d542adbf590619ca071fe"
    );
    assert_eq!(fields[1].1, "test-access-token");
    assert_eq!(fields[2].1, "ar");
    assert_eq!(fields[3].1, "3");
    assert_eq!(fields[4].1, B64.encode([1, 2, 3, 4]));
    assert_eq!(fields[5].1, "TESTSERIAL0001");
    let nonce: Vec<u8> = (0..16).collect();
    assert_eq!(
        fields[6].1,
        format!(
            "{{\"device_nonce\":\"{}\",\"app_pubkey\":\"{}\",\"secondary_cert\":\"{}\"}}",
            B64.encode(&nonce),
            B64.encode([9u8; 64]),
            B64.encode([5, 6, 7, 8])
        )
    );
}

#[test]
fn pair_encodes_the_exact_reference_field_set() {
    let receipt = r#"{"receipt_type":"DevicePendingOwnershipReceipt"}"#;
    let pair = CeremonyPair {
        receipt: receipt.into(),
        signature: vec![10, 11, 12],
    };
    let fields = pair_fields(&pair, &session());
    let names: Vec<&str> = fields.iter().map(|(name, _)| *name).collect();
    assert_eq!(
        names,
        [
            "access_token",
            "user_access_token",
            "user_token_universe",
            "pair_protocol_version",
            "device_pending_ownership_receipt",
            "device_pending_ownership_receipt_signature"
        ]
    );
    assert_eq!(fields[4].1, receipt);
    assert_eq!(fields[5].1, B64.encode([10, 11, 12]));
}

#[test]
fn url_form_escapes_everything_but_unreserved_characters() {
    let body = url_form(&[("a", "x+y=z 1".into()), ("b", r#"{"q":"\/"}"#.into())]);
    assert_eq!(body, "a=x%2By%3Dz%201&b=%7B%22q%22%3A%22%5C%2F%22%7D");
}

#[test]
fn pending_and_final_receipts_parse_with_the_band_key() {
    let pending = parse_pending(&json!({
        "pending_ownership_receipt": r#"{"receipt_type":"ServerPendingOwnershipReceipt"}"#,
        "receipt_signature": B64.encode([1, 2, 3]),
    }))
    .unwrap();
    assert_eq!(pending.signature, vec![1, 2, 3]);
    assert!(pending.receipt.contains("ServerPendingOwnershipReceipt"));
    let point: Vec<u8> = (0..64).collect();
    let final_ = parse_final(&json!({
        "final_ownership_receipt": r#"{"receipt_type":"ServerFinalOwnershipReceipt"}"#,
        "receipt_signature": B64.encode([4, 5]),
        "additional_data": { "device_ec_public_key": B64.encode(&point) },
    }))
    .unwrap();
    assert_eq!(final_.device_public_key.as_deref(), Some(point.as_slice()));
    assert_eq!(final_.signature, vec![4, 5]);
    // A server that embeds the key in the receipt string still parses.
    let receipt = format!(
        "{{\"additional_data\":{{\"device_ec_public_key\":\"{}\"}}}}",
        B64.encode(&point)
    );
    let embedded = parse_final(
        &json!({ "final_ownership_receipt": receipt, "receipt_signature": B64.encode([4, 5]) }),
    )
    .unwrap();
    assert_eq!(
        embedded.device_public_key.as_deref(),
        Some(point.as_slice())
    );
    // The live server may omit the band key; that's tolerated.
    let keyless =
        parse_final(&json!({ "final_ownership_receipt": "{}", "receipt_signature": "AAAA" }))
            .unwrap();
    assert!(keyless.device_public_key.is_none());
    assert!(matches!(
        parse_pending(&json!({ "receipt_signature": "AAAA" })),
        Err(PairError::Failed(_))
    ));
}

#[test]
fn session_failures_cover_auth_statuses_and_expired_token_errors() {
    assert!(is_session_failure(401, None));
    assert!(is_session_failure(403, None));
    assert!(is_session_failure(400, Some(&json!({ "code": 190 }))));
    assert!(!is_session_failure(400, Some(&json!({ "code": 1 }))));
    assert!(!is_session_failure(500, None));
    assert!(!is_session_failure(200, None));
}

#[test]
fn the_session_never_prints_its_token() {
    assert!(!format!("{:?}", session()).contains("test-access-token"));
}

#[test]
fn the_meta_session_round_trips_through_the_store() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    assert!(store.load_meta_session().unwrap().is_none());
    store.save_meta_session(&session()).unwrap();
    let loaded = store.load_meta_session().unwrap().unwrap();
    assert_eq!(loaded.access_token, "test-access-token");
    assert_eq!(loaded.universe, "ar");
    store.delete_meta_session().unwrap();
    assert!(store.load_meta_session().unwrap().is_none());
}

#[tokio::test]
async fn the_fake_client_answers_both_steps() {
    let client = FakeFactory {
        device_point: Some(vec![7; 64]),
        fail: None,
    }
    .client(None)
    .unwrap();
    let request = CeremonyPairRequest {
        identity: BandIdentityInfo {
            device_certificate: vec![1],
            serial: "S".into(),
            secondary_certificate: vec![2],
        },
        nonce: vec![0; 16],
        app_public_key: vec![9; 64],
    };
    let pending = client.pair_request(&request).await.unwrap();
    assert!(!pending.signature.is_empty() && !pending.receipt.is_empty());
    let final_ = client
        .pair(&CeremonyPair {
            receipt: "{}".into(),
            signature: vec![1],
        })
        .await
        .unwrap();
    assert_eq!(final_.device_public_key, Some(vec![7; 64]));
}

#[test]
fn an_empty_band_key_falls_back_to_the_receipt_or_none() {
    let point: Vec<u8> = (0..64).collect();
    let receipt = format!(
        "{{\"additional_data\":{{\"device_ec_public_key\":\"{}\"}}}}",
        B64.encode(&point)
    );
    let embedded = parse_final(&json!({
        "final_ownership_receipt": receipt,
        "receipt_signature": "AAAA",
        "additional_data": { "device_ec_public_key": "" },
    }))
    .unwrap();
    assert_eq!(
        embedded.device_public_key.as_deref(),
        Some(point.as_slice())
    );
    let empty = parse_final(&json!({
        "final_ownership_receipt": "{\"additional_data\":{\"device_ec_public_key\":\"\"}}",
        "receipt_signature": "AAAA",
        "additional_data": { "device_ec_public_key": "" },
    }))
    .unwrap();
    assert!(empty.device_public_key.is_none());
}
