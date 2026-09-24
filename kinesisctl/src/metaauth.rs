//! Meta sign-in HTTP steps (kinesis `MetaAuth.swift`). The client ids below
//! are Meta's own public app identifiers (the same in every copy of Meta's
//! apps), not secrets. The resulting access token never reaches logs.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use kinesisd::meta::{MetaSession, url_form};
use rand_core::{OsRng, RngCore};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const FRL_CLIENT: &str = "FRL|388177446008673|083800dd7efbbd42eab18c9886d79c18";
pub const AR_CLIENT: &str = "AR|306760944872162|a919421a55a8ea18080ab2f10f57f1be";
const TOKENS_QUERY_URL: &str = "https://meta.graph.meta.com/webview_tokens_query";
const BLOBS_DECRYPT_URL: &str = "https://meta.graph.meta.com/webview_blobs_decrypt";
const LOGIN_URL: &str = "https://ar-genai.graph.meta.com/login";
const AUTH_ENTRY_BASE: &str = "https://auth.meta.com/?native_app_id=388177446008673&source_app_id=388177446008673&native_sso_etoken=";

pub struct SsoTokens {
    pub native_sso_token: String,
    pub etoken: String,
}

pub fn auth_entry_url(tokens: &SsoTokens) -> String {
    format!("{AUTH_ENTRY_BASE}{}", tokens.etoken)
}

pub fn make_lsd() -> String {
    let mut digits = String::with_capacity(6);
    while digits.len() < 6 {
        let mut byte = [0u8; 1];
        OsRng.fill_bytes(&mut byte);
        // 250 = 25 × 10: rejecting the rest keeps every digit equally likely.
        if byte[0] < 250 {
            digits.push(char::from(b'0' + byte[0] % 10));
        }
    }
    format!("S0.{digits}")
}

pub fn jazoest(lsd: &str) -> String {
    format!("2{}", lsd.chars().map(|c| c as u32).sum::<u32>())
}

pub fn multipart_body(fields: &[(&str, &str)], boundary: &str) -> Vec<u8> {
    let mut body = Vec::new();
    for (name, value) in fields {
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n")
                .as_bytes(),
        );
        body.extend_from_slice(value.as_bytes());
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

/// The token the login callback must present: the first 16 hex digits of SHA-256(native token).
pub fn expected_callback_token(native_sso_token: &str) -> String {
    Sha256::digest(native_sso_token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()[..16]
        .to_string()
}

pub fn callback_matches(token: Option<&str>, native_sso_token: &str) -> bool {
    let Some(token) = token.filter(|t| t.len() == 16) else {
        return false;
    };
    let expected = expected_callback_token(native_sso_token);
    token
        .bytes()
        .zip(expected.bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

/// The login finishes with a custom-scheme redirect carrying the token and the encrypted blob.
pub fn is_callback(url: &str) -> bool {
    url.to_ascii_lowercase().starts_with("fb-viewapp:") || url.contains("frl_login")
}

/// `(token, blob)` from the callback URL's query. Like Swift's
/// `URLComponents.queryItems`, only `%XX` escapes are decoded: a raw `+` in
/// the (base64) blob stays a `+` rather than becoming a space.
pub fn parse_callback(url: &str) -> Option<(Option<String>, Option<String>)> {
    let parsed = url::Url::parse(url).ok()?;
    let mut token = None;
    let mut blob = None;
    for pair in parsed.query().unwrap_or("").split('&') {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        match percent_decode(name).as_str() {
            "token" if token.is_none() => token = Some(percent_decode(value)),
            "blob" if blob.is_none() => blob = Some(percent_decode(value)),
            _ => {}
        }
    }
    Some((token, blob))
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| char::from(b).to_digit(16);
        match (
            bytes[i],
            bytes.get(i + 1).copied().and_then(hex),
            bytes.get(i + 2).copied().and_then(hex),
        ) {
            (b'%', Some(hi), Some(lo)) => {
                out.push((hi * 16 + lo) as u8);
                i += 3;
            }
            (byte, _, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn text(json: &Value, key: &str) -> Option<String> {
    json.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

pub fn parse_tokens(json: &Value) -> Result<SsoTokens, String> {
    match (
        text(json, "native_sso_token"),
        text(json, "native_sso_etoken"),
    ) {
        (Some(native_sso_token), Some(etoken)) => Ok(SsoTokens {
            native_sso_token,
            etoken,
        }),
        _ => Err("meta didn't return sign-in tokens. try again.".into()),
    }
}

pub fn parse_login(json: &Value, device_id: &str) -> Result<MetaSession, String> {
    let token = text(json, "access_token");
    let user = text(json, "user_id").or_else(|| {
        json.get("user_id")
            .and_then(Value::as_u64)
            .map(|id| id.to_string())
    });
    let (Some(access_token), Some(user_id)) = (token, user) else {
        return Err("meta didn't return an account session. try again.".into());
    };
    let mut session = MetaSession::new(access_token, user_id);
    session.device_id = device_id.to_owned();
    session.obtained_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    Ok(session)
}

fn http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("could not start an HTTP client: {error}"))
}

async fn finish(response: reqwest::Result<reqwest::Response>) -> Result<Value, String> {
    let response = response.map_err(|error| format!("couldn't reach meta: {error}"))?;
    let status = response.status().as_u16();
    let json: Option<Value> = response.json().await.ok();
    let json = match json {
        Some(json) if status == 200 => json,
        _ => return Err(format!("meta sign-in failed (http {status}). try again.")),
    };
    if let Some(error) = json.get("error") {
        let code = error
            .get("code")
            .and_then(Value::as_i64)
            .map_or("-".into(), |c| c.to_string());
        let sub = error
            .get("error_subcode")
            .and_then(Value::as_i64)
            .map_or("-".into(), |c| c.to_string());
        return Err(format!(
            "meta sign-in failed (code {code}, subcode {sub}). try again."
        ));
    }
    Ok(json)
}

async fn post_multipart(url: &str, fields: &[(&str, &str)]) -> Result<Value, String> {
    let boundary = format!("kinesis.form.{}", uuid::Uuid::new_v4());
    finish(
        http()?
            .post(url)
            .header(
                reqwest::header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::ORIGIN, "https://auth.meta.com")
            .body(multipart_body(fields, &boundary))
            .send()
            .await,
    )
    .await
}

/// Step 1: the anonymous tokens query.
pub async fn tokens_query() -> Result<SsoTokens, String> {
    let lsd = make_lsd();
    let jaz = jazoest(&lsd);
    parse_tokens(
        &post_multipart(
            TOKENS_QUERY_URL,
            &[
                ("access_token", FRL_CLIENT),
                ("lsd", &lsd),
                ("jazoest", &jaz),
            ],
        )
        .await?,
    )
}

/// Step 3: decrypt the login blob into the FRL access token.
pub async fn decrypt_blob(blob: &str, request_token: &str) -> Result<String, String> {
    let lsd = make_lsd();
    let jaz = jazoest(&lsd);
    let fields = [
        ("blob", blob),
        ("request_token", request_token),
        ("access_token", FRL_CLIENT),
        ("lsd", &lsd),
        ("jazoest", &jaz),
    ];
    text(
        &post_multipart(BLOBS_DECRYPT_URL, &fields).await?,
        "access_token",
    )
    .ok_or_else(|| "meta didn't confirm the sign-in. try again.".into())
}

/// Step 4: exchange the FRL token for the `ar` user session.
pub async fn login(frl_access_token: &str) -> Result<MetaSession, String> {
    let device_id = uuid::Uuid::new_v4().to_string();
    let fields = vec![
        ("frl_access_token", frl_access_token.to_owned()),
        ("logging_session_id", uuid::Uuid::new_v4().to_string()),
        ("format", "json".to_owned()),
        ("device_id", device_id.clone()),
        ("generate_session_cookies", "1".to_owned()),
        ("generate_analytics_claim", "1".to_owned()),
        ("method", "POST".to_owned()),
    ];
    let json = finish(
        http()?
            .post(LOGIN_URL)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::AUTHORIZATION, format!("OAuth {AR_CLIENT}"))
            .body(url_form(&fields))
            .send()
            .await,
    )
    .await?;
    parse_login(&json, &device_id)
}
