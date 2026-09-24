//! Running configured commands: detached, through `sh -c`. Actions are often
//! launched apps (e.g. `wofi --show drun`) — never killed for taking a while;
//! a slow one is only logged, and still reaped so it never zombies.

use std::borrow::Cow;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::AsyncReadExt;

pub trait Runner: Send + Sync + 'static {
    /// Start `command` and return immediately.
    fn run(&self, command: String);
}

#[derive(Debug, Clone)]
pub struct ShellRunner {
    /// How long to wait before logging that the action is still running.
    pub timeout: Duration,
}

impl Default for ShellRunner {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(5),
        }
    }
}

/// The last `max` bytes, lossily decoded (a cut multi-byte character just
/// becomes a replacement character).
fn tail(bytes: &[u8], max: usize) -> Cow<'_, str> {
    let start = bytes.len().saturating_sub(max);
    String::from_utf8_lossy(&bytes[start..])
}

impl Runner for ShellRunner {
    fn run(&self, command: String) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            tracing::warn!(%command, "no tokio runtime available: action not started");
            return;
        };
        let timeout = self.timeout;
        handle.spawn(async move {
            let child = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(&command)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                // Its own process group: dropping the runner must never signal it.
                .process_group(0)
                .spawn();
            let mut child = match child {
                Ok(child) => child,
                Err(error) => {
                    tracing::warn!(%command, "could not start action: {error}");
                    return;
                }
            };
            let stderr = child.stderr.take();
            let stderr_task = tokio::spawn(async move {
                let mut buffer = Vec::new();
                if let Some(mut stderr) = stderr {
                    let _ = stderr.read_to_end(&mut buffer).await;
                }
                buffer
            });

            let sleep = tokio::time::sleep(timeout);
            tokio::pin!(sleep);
            let mut timed_out = false;
            let status = loop {
                tokio::select! {
                    result = child.wait() => break result,
                    () = &mut sleep, if !timed_out => {
                        timed_out = true;
                        tracing::debug!(%command, ?timeout, "action still running; leaving it running");
                    }
                }
            };

            let stderr_bytes = stderr_task.await.unwrap_or_default();
            match status {
                Ok(status) if !status.success() => {
                    tracing::warn!(%command, %status, stderr = %tail(&stderr_bytes, 200), "action failed");
                }
                Ok(_) => {}
                Err(error) => tracing::warn!(%command, "action wait failed: {error}"),
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
