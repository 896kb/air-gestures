//! Driving a pairing through air-gesturesd and reporting its progress.

use std::path::Path;
use std::time::Duration;

use air_gesturesd::ipc::Request;
use air_gesturesd::status::{LinkState, Status};

use crate::client::{self, RequestError, Subscription};

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Paired {
        band: String,
    },
    /// Claimed, but not connected within the wait: air-gesturesd keeps retrying.
    PairedNotConnected {
        band: String,
    },
    Failed {
        message: String,
    },
    /// The pairing failed because the Meta session is missing or expired
    /// (`PairResult::needs_sign_in`): sign in and pair again.
    NeedsSignIn,
}

pub async fn start_pairing(socket: &Path, address: Option<String>) -> Result<(), RequestError> {
    client::request(socket, &Request::Pair { address })
        .await
        .map(|_| ())
}

fn outcome_of(status: &Status) -> Option<Outcome> {
    let result = status.last_pair.as_ref()?;
    Some(if result.needs_sign_in {
        Outcome::NeedsSignIn
    } else if result.ok {
        Outcome::Paired {
            band: status.band.clone().unwrap_or_else(|| "the band".into()),
        }
    } else {
        Outcome::Failed {
            message: result.message.clone(),
        }
    })
}

/// Follow the status until the pairing ends; on success also wait for the owner connection.
pub async fn follow_pairing(
    socket: &Path,
    mut on_stage: impl FnMut(&str),
) -> Result<Outcome, String> {
    let mut subscription = Subscription::status(socket).await?;
    let mut last_stage = String::new();
    let (status, outcome) = loop {
        let status = tokio::time::timeout(Duration::from_secs(240), subscription.next_status())
            .await
            .map_err(|_| "the pairing took too long".to_string())?
            .ok_or_else(|| "air-gesturesd went away during the pairing".to_string())?;
        if status.state == LinkState::Pairing {
            if let Some(stage) = &status.pairing
                && *stage != last_stage
            {
                last_stage = stage.clone();
                on_stage(stage);
            }
            continue;
        }
        if let Some(outcome) = outcome_of(&status) {
            break (status, outcome);
        }
    };
    if let Outcome::Paired { band } = &outcome {
        // The status that ended the loop may already show the settling
        // reconnect as done (a fast, e.g. simulated, pairing easily beats a
        // fresh subscriber to the punch): nothing left to wait for then.
        if status.state == LinkState::Connected {
            return Ok(Outcome::Paired { band: band.clone() });
        }
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
            return Ok(Outcome::PairedNotConnected { band: band.clone() });
        }
    }
    Ok(outcome)
}
