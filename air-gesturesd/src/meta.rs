//! The ownership HTTP exchanges against Meta's hardware graph (kinesis
//! `MetaPairClient.swift`). Everything is a form-encoded POST; receipts are
//! signed strings and travel verbatim. The app credential below is Meta's own
//! public hardware client id, as used by kinesis.

use std::future::Future;
use std::time::Duration;

use band_core::ceremony::{CeremonyPair, CeremonyPairRequest};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const HW_CLIENT: &str = "HW|1312539125771114|98588f106d5d542adbf590619ca071fe";
pub const UNIVERSE: &str = "ar";
const HOST: &str = "https://graph.facebook-hardware.com";

fn universe() -> String {
    UNIVERSE.into()
}

/// The signed-in account session (written by `air-gestures` after sign-in).
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct MetaSession {
    pub access_token: String,
    pub user_id: String,
    #[serde(default)]
    pub device_id: String,
    #[serde(default = "universe")]
    pub universe: String,
    /// Unix seconds.
    #[serde(default)]
    pub obtained_at: u64,
}

impl MetaSession {
    pub fn new(access_token: impl Into<String>, user_id: impl Into<String>) -> Self {
        Self {
            access_token: access_token.into(),
            user_id: user_id.into(),
            device_id: String::new(),
            universe: universe(),
            obtained_at: 0,
        }
    }
}

impl std::fmt::Debug for MetaSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetaSession")
            .field("user_id", &self.user_id)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub signature: Vec<u8>,
    pub receipt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Final {
    pub signature: Vec<u8>,
    pub receipt: String,
    pub device_public_key: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairError {
    /// The account session is expired or revoked: sign in again.
    SessionInvalid(String),
    Failed(String),
}

impl std::fmt::Display for PairError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PairError::SessionInvalid(message) | PairError::Failed(message) => f.write_str(message),
        }
    }
}

pub fn additional_data(nonce: &[u8], app_public_key: &[u8], secondary_cert_b64: &str) -> String {
    format!(
        "{{\"device_nonce\":\"{}\",\"app_pubkey\":\"{}\",\"secondary_cert\":\"{}\"}}",
        B64.encode(nonce),
        B64.encode(app_public_key),
        secondary_cert_b64
    )
}

pub fn pair_request_fields(
    request: &CeremonyPairRequest,
    session: &MetaSession,
) -> Vec<(&'static str, String)> {
    vec![
        ("access_token", HW_CLIENT.into()),
        ("user_access_token", session.access_token.clone()),
        ("user_token_universe", UNIVERSE.into()),
        ("pair_protocol_version", "3".into()),
        (
            "device_cert",
            B64.encode(&request.identity.device_certificate),
        ),
        ("serial_number", request.identity.serial.clone()),
        (
            "additional_data",
            additional_data(
                &request.nonce,
                &request.app_public_key,
                &B64.encode(&request.identity.secondary_certificate),
            ),
        ),
    ]
}

pub fn pair_fields(pair: &CeremonyPair, session: &MetaSession) -> Vec<(&'static str, String)> {
    vec![
        ("access_token", HW_CLIENT.into()),
        ("user_access_token", session.access_token.clone()),
        ("user_token_universe", UNIVERSE.into()),
        ("pair_protocol_version", "3".into()),
        ("device_pending_ownership_receipt", pair.receipt.clone()),
        (
            "device_pending_ownership_receipt_signature",
            B64.encode(&pair.signature),
        ),
    ]
}

fn percent_encode(text: &str) -> String {
    text.bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

/// `application/x-www-form-urlencoded` with every reserved character escaped
/// (a space is `%20`, a plus `%2B`), matching kinesis `MetaAuth.urlForm`.
pub fn url_form(fields: &[(&str, String)]) -> String {
    fields
        .iter()
        .map(|(name, value)| format!("{}={}", percent_encode(name), percent_encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

fn decode_signature(json: &Value) -> Result<Vec<u8>, PairError> {
    B64.decode(
        json.get("receipt_signature")
            .and_then(Value::as_str)
            .unwrap_or(""),
    )
    .map_err(|_| {
        PairError::Failed(
            "the band claim service returned an unreadable signature. try again.".into(),
        )
    })
}

pub fn parse_pending(json: &Value) -> Result<Pending, PairError> {
    let receipt = json
        .get("pending_ownership_receipt")
        .and_then(Value::as_str)
        .filter(|r| !r.is_empty());
    let Some(receipt) = receipt else {
        return Err(PairError::Failed(
            "the band claim service didn't return a receipt. try again.".into(),
        ));
    };
    Ok(Pending {
        signature: decode_signature(json)?,
        receipt: receipt.to_owned(),
    })
}

fn device_key(json: &Value) -> Option<Vec<u8>> {
    let key = json
        .get("additional_data")?
        .get("device_ec_public_key")?
        .as_str()?;
    B64.decode(key).ok().filter(|key| !key.is_empty())
}

pub fn parse_final(json: &Value) -> Result<Final, PairError> {
    let receipt = json
        .get("final_ownership_receipt")
        .and_then(Value::as_str)
        .filter(|r| !r.is_empty());
    let Some(receipt) = receipt else {
        return Err(PairError::Failed(
            "the band claim service didn't return a final receipt. try again.".into(),
        ));
    };
    let device_public_key = device_key(json).or_else(|| {
        serde_json::from_str::<Value>(receipt)
            .ok()
            .and_then(|inner| device_key(&inner))
    });
    Ok(Final {
        signature: decode_signature(json)?,
        receipt: receipt.to_owned(),
        device_public_key,
    })
}

/// An auth HTTP status, or the graph's expired-token error code 190.
pub fn is_session_failure(status: u16, error: Option<&Value>) -> bool {
    status == 401
        || status == 403
        || error.and_then(|e| e.get("code")).and_then(Value::as_i64) == Some(190)
}

pub trait PairClient: Send + Sync + 'static {
    fn pair_request(
        &self,
        request: &CeremonyPairRequest,
    ) -> impl Future<Output = Result<Pending, PairError>> + Send;
    fn pair(&self, pair: &CeremonyPair) -> impl Future<Output = Result<Final, PairError>> + Send;
}

pub struct HttpPairClient {
    http: reqwest::Client,
    session: MetaSession,
}

impl HttpPairClient {
    pub fn new(session: MetaSession) -> Result<Self, PairError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|error| PairError::Failed(format!("could not set up HTTPS: {error}")))?;
        Ok(Self { http, session })
    }

    async fn execute(
        &self,
        route: &str,
        fields: Vec<(&'static str, String)>,
    ) -> Result<Value, PairError> {
        let response = self
            .http
            .post(format!("{HOST}/{route}"))
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .header(reqwest::header::ACCEPT, "application/json")
            .body(url_form(&fields))
            .send()
            .await
            .map_err(|error| {
                PairError::Failed(format!(
                    "the band claim service couldn't be reached: {error}"
                ))
            })?;
        let status = response.status().as_u16();
        let json: Option<Value> = response.json().await.ok();
        if is_session_failure(status, json.as_ref().and_then(|j| j.get("error"))) {
            return Err(PairError::SessionInvalid(
                "your meta session expired. sign in to claim the band again.".into(),
            ));
        }
        let json = match json {
            Some(json) if status == 200 => json,
            _ => {
                return Err(PairError::Failed(format!(
                    "the band claim service failed (http {status}). try again."
                )));
            }
        };
        if let Some(error) = json.get("error") {
            let code = error
                .get("code")
                .and_then(Value::as_i64)
                .map_or("-".to_string(), |code| code.to_string());
            return Err(PairError::Failed(format!(
                "the band claim service rejected the request (code {code}). try again."
            )));
        }
        Ok(json)
    }
}

impl PairClient for HttpPairClient {
    async fn pair_request(&self, request: &CeremonyPairRequest) -> Result<Pending, PairError> {
        parse_pending(
            &self
                .execute("pair_request", pair_request_fields(request, &self.session))
                .await?,
        )
    }

    async fn pair(&self, pair: &CeremonyPair) -> Result<Final, PairError> {
        parse_final(
            &self
                .execute("pair", pair_fields(pair, &self.session))
                .await?,
        )
    }
}

/// Answers both steps without a network (tests and `--dry-run`).
pub struct FakePairClient {
    device_point: Option<Vec<u8>>,
    fail: Option<PairError>,
}

impl PairClient for FakePairClient {
    async fn pair_request(&self, _request: &CeremonyPairRequest) -> Result<Pending, PairError> {
        if let Some(error) = &self.fail {
            return Err(error.clone());
        }
        Ok(Pending {
            signature: vec![0x30, 0x44, 1],
            receipt: r#"{"receipt_type":"ServerPendingOwnershipReceipt"}"#.into(),
        })
    }

    async fn pair(&self, _pair: &CeremonyPair) -> Result<Final, PairError> {
        Ok(Final {
            signature: vec![0x30, 0x44, 2],
            receipt: r#"{"receipt_type":"ServerFinalOwnershipReceipt"}"#.into(),
            device_public_key: self.device_point.clone(),
        })
    }
}

/// Builds the client for one pairing from the saved Meta session.
pub trait ClientFactory: Send + Sync + 'static {
    type Client: PairClient;
    /// `Err(None)` means there is no session: sign in first.
    fn client(&self, session: Option<MetaSession>) -> Result<Self::Client, Option<String>>;
}

pub struct HttpFactory;

impl ClientFactory for HttpFactory {
    type Client = HttpPairClient;
    fn client(&self, session: Option<MetaSession>) -> Result<HttpPairClient, Option<String>> {
        HttpPairClient::new(session.ok_or(None)?).map_err(|error| Some(error.to_string()))
    }
}

pub struct FakeFactory {
    pub device_point: Option<Vec<u8>>,
    pub fail: Option<PairError>,
}

impl ClientFactory for FakeFactory {
    type Client = FakePairClient;
    fn client(&self, _session: Option<MetaSession>) -> Result<FakePairClient, Option<String>> {
        Ok(FakePairClient {
            device_point: self.device_point.clone(),
            fail: self.fail.clone(),
        })
    }
}
