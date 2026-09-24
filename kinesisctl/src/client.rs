//! Talking to kinesisd over its control socket (newline-delimited JSON).

use std::path::Path;
use std::time::Duration;

use kinesisd::ipc::{Request, Response};
use kinesisd::status::Status;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

const CALL_TIMEOUT: Duration = Duration::from_secs(10);

fn not_running(path: &Path, error: std::io::Error) -> String {
    format!(
        "kinesisd isn't running ({}: {error}). start it with `systemctl --user start kinesisd`",
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
        .map_err(|error| format!("writing to kinesisd failed: {error}"))
}

/// One request, one response. `Err` carries the daemon's error text (or a transport problem).
pub async fn request(path: &Path, request: &Request) -> Result<Response, String> {
    let stream = connect(path).await?;
    let (read, mut write) = stream.into_split();
    send(&mut write, request).await?;
    let mut lines = BufReader::new(read).lines();
    let line = tokio::time::timeout(CALL_TIMEOUT, lines.next_line())
        .await
        .map_err(|_| "kinesisd didn't answer in time".to_string())?
        .map_err(|error| format!("reading from kinesisd failed: {error}"))?
        .ok_or_else(|| "kinesisd closed the connection".to_string())?;
    let response: Response = serde_json::from_str(&line)
        .map_err(|error| format!("unexpected answer from kinesisd: {error}"))?;
    if response.ok {
        Ok(response)
    } else {
        Err(response
            .error
            .unwrap_or_else(|| "kinesisd refused the request".into()))
    }
}

pub async fn status(path: &Path) -> Result<Status, String> {
    request(path, &Request::Status)
        .await?
        .status
        .ok_or_else(|| "kinesisd sent no status".to_string())
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

    async fn next_response(&mut self) -> Option<Response> {
        loop {
            let line = self.lines.next_line().await.ok()??;
            if let Ok(response) = serde_json::from_str::<Response>(&line) {
                return Some(response);
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
