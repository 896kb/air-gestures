//! Driving a pairing through kinesisd and reporting its progress.

use std::path::Path;
use std::time::Duration;

use kinesisd::ipc::Request;
use kinesisd::status::LinkState;

use crate::client::{self, Subscription};

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Paired { band: String },
    Failed { message: String },
}

/// The daemon asks for a Meta sign-in with this wording (kinesisd `start_pairing`).
pub fn needs_sign_in(error: &str) -> bool {
    error.to_ascii_lowercase().contains("sign in to meta")
}

pub async fn start_pairing(socket: &Path, address: Option<String>) -> Result<(), String> {
    client::request(socket, &Request::Pair { address })
        .await
        .map(|_| ())
}

/// Follow the status until the pairing ends; on success also wait for the owner connection.
pub async fn follow_pairing(
    socket: &Path,
    mut on_stage: impl FnMut(&str),
) -> Result<Outcome, String> {
    let mut subscription = Subscription::status(socket).await?;
    let mut last_stage = String::new();
    let outcome = loop {
        let status = tokio::time::timeout(Duration::from_secs(240), subscription.next_status())
            .await
            .map_err(|_| "the pairing took too long".to_string())?
            .ok_or_else(|| "kinesisd went away during the pairing".to_string())?;
        if status.state == LinkState::Pairing {
            if let Some(stage) = &status.pairing
                && *stage != last_stage
            {
                last_stage = stage.clone();
                on_stage(stage);
            }
            continue;
        }
        if let Some(result) = &status.last_pair {
            break if result.ok {
                Outcome::Paired {
                    band: status.band.clone().unwrap_or_else(|| "the band".into()),
                }
            } else {
                Outcome::Failed {
                    message: result.message.clone(),
                }
            };
        }
    };
    if let Outcome::Paired { .. } = &outcome {
        on_stage("connecting as the new owner");
        let connected = tokio::time::timeout(Duration::from_secs(60), async {
            while let Some(status) = subscription.next_status().await {
                if status.state == LinkState::Connected {
                    return true;
                }
            }
            false
        })
        .await
        .unwrap_or(false);
        if !connected {
            return Ok(Outcome::Failed {
                message: "the band was claimed but didn't connect yet; kinesisd keeps retrying (see `kinesisctl status`)".into(),
            });
        }
    }
    Ok(outcome)
}
