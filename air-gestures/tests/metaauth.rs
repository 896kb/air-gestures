use air_gestures::metaauth::{
    auth_entry_url, callback_matches, expected_callback_token, is_callback, jazoest, make_lsd,
    multipart_body, parse_callback, parse_login, parse_tokens,
};
use serde_json::json;

#[test]
fn lsd_follows_the_reference_shape() {
    let lsd = make_lsd();
    assert_eq!(lsd.len(), 9);
    assert!(lsd.starts_with("S0."));
    assert!(lsd[3..].chars().all(|c| c.is_ascii_digit()));
    let sum: u32 = lsd.chars().map(|c| c as u32).sum();
    assert_eq!(jazoest(&lsd), format!("2{sum}"));
}

#[test]
fn multipart_bodies_wrap_every_field_once() {
    let body = String::from_utf8(multipart_body(
        &[("blob", "b"), ("lsd", "S0.1")],
        "boundary",
    ))
    .unwrap();
    let expected = "--boundary\r\nContent-Disposition: form-data; name=\"blob\"\r\n\r\nb\r\n\
                    --boundary\r\nContent-Disposition: form-data; name=\"lsd\"\r\n\r\nS0.1\r\n--boundary--\r\n";
    assert_eq!(body, expected);
}

#[test]
fn callback_validation_compares_the_truncated_token_digest() {
    let native = "epfEORXEWkN2obB5";
    let expected = expected_callback_token(native);
    assert_eq!(expected.len(), 16);
    assert!(callback_matches(Some(&expected), native));
    let mut wrong = expected.clone();
    wrong.pop();
    wrong.push(if expected.ends_with('0') { '1' } else { '0' });
    assert!(!callback_matches(Some(&wrong), native));
    assert!(!callback_matches(None, native));
    assert!(!callback_matches(Some("short"), native));
}

#[test]
fn the_callback_is_recognized_and_parsed() {
    let url = "fb-viewapp://frl_login?token=0123456789abcdef&blob=AbC%2B%2F%3D";
    assert!(is_callback(url));
    assert!(is_callback("https://m.meta.com/frl_login/?token=x&blob=y"));
    assert!(!is_callback("https://auth.meta.com/login"));
    let (token, blob) = parse_callback(url).unwrap();
    assert_eq!(token.as_deref(), Some("0123456789abcdef"));
    assert_eq!(blob.as_deref(), Some("AbC+/="));
}

#[test]
fn responses_parse_or_fail_clearly() {
    let tokens =
        parse_tokens(&json!({ "native_sso_token": "n", "native_sso_etoken": "e" })).unwrap();
    assert_eq!(
        (tokens.native_sso_token.as_str(), tokens.etoken.as_str()),
        ("n", "e")
    );
    assert!(auth_entry_url(&tokens).ends_with("native_sso_etoken=e"));
    assert!(
        auth_entry_url(&tokens).starts_with("https://auth.meta.com/?native_app_id=388177446008673")
    );
    assert!(parse_tokens(&json!({ "native_sso_token": "" })).is_err());
    let session = parse_login(&json!({ "access_token": "t", "user_id": 42 }), "device").unwrap();
    assert_eq!(
        (session.access_token.as_str(), session.user_id.as_str()),
        ("t", "42")
    );
    assert!(parse_login(&json!({ "access_token": "t" }), "device").is_err());
}

#[test]
fn a_raw_plus_in_the_blob_stays_a_plus() {
    // Like Swift's URLComponents.queryItems: only %XX is decoded, `+` is literal.
    let (token, blob) = parse_callback("fb-viewapp://frl_login?blob=a+b/c%3D&token=t").unwrap();
    assert_eq!(blob.as_deref(), Some("a+b/c="));
    assert_eq!(token.as_deref(), Some("t"));
}
