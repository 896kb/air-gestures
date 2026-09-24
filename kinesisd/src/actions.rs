//! Running configured commands: detached, through `sh -c`, killed after a
//! timeout so a stuck command can never back up the band connection.

use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub trait Runner: Send + Sync + 'static {
    /// Start `command` and return immediately.
    fn run(&self, command: String);
}

#[derive(Debug, Clone)]
pub struct ShellRunner {
    pub timeout: Duration,
}

impl Default for ShellRunner {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(5),
        }
    }
}

impl Runner for ShellRunner {
    fn run(&self, command: String) {
        let timeout = self.timeout;
        tokio::spawn(async move {
            let child = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(&command)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn();
            let mut child = match child {
                Ok(child) => child,
                Err(error) => {
                    tracing::warn!(%command, "could not start action: {error}");
                    return;
                }
            };
            match tokio::time::timeout(timeout, child.wait()).await {
                Ok(Ok(status)) if !status.success() => {
                    tracing::warn!(%command, %status, "action failed")
                }
                Ok(Err(error)) => tracing::warn!(%command, "action wait failed: {error}"),
                Err(_) => {
                    let _ = child.kill().await;
                    tracing::warn!(%command, "action timed out and was killed");
                }
                Ok(Ok(_)) => {}
            }
        });
    }
}

/// Records commands instead of running them (tests, `--dry-run` logs).
#[derive(Debug, Clone, Default)]
pub struct RecordingRunner(Arc<Mutex<Vec<String>>>);

impl RecordingRunner {
    pub fn commands(&self) -> Vec<String> {
        self.0.lock().expect("runner lock").clone()
    }
}

impl Runner for RecordingRunner {
    fn run(&self, command: String) {
        self.0.lock().expect("runner lock").push(command);
    }
}
