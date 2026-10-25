//! Talking to air-gesturesd over its control socket (newline-delimited JSON).

use std::path::Path;
use std::time::Duration;

use air_gesturesd::ipc::{Request, Response};
use air_gesturesd::status::Status;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

const CALL_TIMEOUT: Duration = Duration::from_secs(10);

/// A request refused (or otherwise failed) over the control socket.
///
/// Carries the daemon's `code`, if any (e.g. [`air_gesturesd::ipc::CODE_NEEDS_SIGN_IN`]),
/// alongside the human-readable `message`, so a caller can react to *why* a
/// request failed without matching on that text.
#[derive(Debug, Clone, PartialEq)]
pub struct RequestError {
    pub message: String,
    pub code: Option<String>,
}

impl std::fmt::Display for RequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for RequestError {}

impl From<String> for RequestError {
    fn from(message: String) -> Self {
        Self {
            message,
            code: None,
        }
    }
}

impl From<RequestError> for String {
    fn from(error: RequestError) -> Self {
        error.message
    }
}

fn not_running(path: &Path, error: std::io::Error) -> String {
    format!(
        "air-gesturesd isn't running ({}: {error}). start it with `systemctl --user start air-gesturesd`",
        path.display()
    )
}

async fn connect(path: &Path) -> Result<UnixStream, String> {
    UnixStream::connect(path)
        .await
        .map_err(|error| not_running(path, error))
}

async fn send(write: &mut OwnedWriteHalf, request: &Request) -> Result<(), String> {
    let mut line = serde_json::to_vec(request).map_err(|error| error.to_string())?;
    line.push(b'\n');
    write
        .write_all(&line)
        .await
        .map_err(|error| format!("writing to air-gesturesd failed: {error}"))
}

/// One request, one response. `Err` carries the daemon's error text (and,
/// where the daemon sent one, its stable [`RequestError::code`]) or reports a
/// transport problem (never carries a code).
pub async fn request(path: &Path, request: &Request) -> Result<Response, RequestError> {
    request_within(path, request, CALL_TIMEOUT).await
}

/// Like [`request`], waiting up to `timeout` for the answer (teaching steps wait for the band).
pub async fn request_within(path: &Path, request: &Request, timeout: Duration) -> Result<Response, RequestError> {
    let stream = connect(path).await?;
    let (read, mut write) = stream.into_split();
    send(&mut write, request).await?;
    let mut lines = BufReader::new(read).lines();
    let line = tokio::time::timeout(timeout, lines.next_line())
        .await
        .map_err(|_| "air-gesturesd didn't answer in time".to_string())?
        .map_err(|error| format!("reading from air-gesturesd failed: {error}"))?
        .ok_or_else(|| "air-gesturesd closed the connection".to_string())?;
    let response: Response = serde_json::from_str(&line)
        .map_err(|error| format!("unexpected answer from air-gesturesd: {error}"))?;
    if response.ok {
        Ok(response)
    } else {
        Err(RequestError {
            message: response
                .error
                .unwrap_or_else(|| "air-gesturesd refused the request".into()),
            code: response.code,
        })
    }
}

pub async fn status(path: &Path) -> Result<Status, String> {
    request(path, &Request::Status)
        .await?
        .status
        .ok_or_else(|| "air-gesturesd sent no status".to_string())
}

/// A long-lived `subscribe` / `subscribe-semg` connection. The write half is
/// kept open: the daemon treats EOF from the client as a hang-up.
pub struct Subscription {
    lines: Lines<BufReader<OwnedReadHalf>>,
    _write: OwnedWriteHalf,
}

impl Subscription {
    async fn open(path: &Path, request: &Request) -> Result<Self, String> {
        let stream = connect(path).await?;
        let (read, mut write) = stream.into_split();
        send(&mut write, request).await?;
        Ok(Self {
            lines: BufReader::new(read).lines(),
            _write: write,
        })
    }

    pub async fn status(path: &Path) -> Result<Self, String> {
        Self::open(path, &Request::Subscribe).await
    }

    pub async fn semg(path: &Path) -> Result<Self, String> {
        Self::open(path, &Request::SubscribeSemg).await
    }

    /// `None` on EOF, a broken line, or a rejected subscribe (`ok: false`):
    /// none of those ever produce a status or sEMG frame, so a caller waiting
    /// on one must not be left hanging.
    async fn next_response(&mut self) -> Option<Response> {
        loop {
            let line = self.lines.next_line().await.ok()??;
            if let Ok(response) = serde_json::from_str::<Response>(&line) {
                return if response.ok { Some(response) } else { None };
            }
        }
    }

    /// The next status line; `None` when the daemon goes away.
    pub async fn next_status(&mut self) -> Option<Status> {
        loop {
            let response = self.next_response().await?;
            if let Some(status) = response.status {
                return Some(status);
            }
        }
    }

    /// The next raw sEMG payload (base64-decoded); `None` when the daemon goes away.
    pub async fn next_semg(&mut self) -> Option<Vec<u8>> {
        use base64::Engine;
        loop {
            let response = self.next_response().await?;
            if let Some(frame) = response.semg
                && let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(frame)
            {
                return Some(bytes);
            }
        }
    }
}
