//! The control socket: newline-delimited JSON requests and responses.

use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::config::DialTarget;
use crate::status::Status;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HandChoice {
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum Request {
    Status,
    /// A status line now and on every change.
    Subscribe,
    /// One line per raw sEMG frame (turn streaming on with `semg`).
    SubscribeSemg,
    Connect,
    Disconnect,
    Pause,
    Resume,
    TogglePause,
    Dial {
        target: DialTarget,
    },
    CycleDial,
    Hand {
        hand: HandChoice,
    },
    Semg {
        on: bool,
    },
    /// Claim a band in pairing mode (the first one found unless `address` is given).
    Pair {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        address: Option<String>,
    },
    /// Forget the band: keys, record, Meta session and the Bluetooth bond.
    Forget,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// One raw sEMG payload, base64 (only on `subscribe-semg` streams).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semg: Option<String>,
}

impl Response {
    pub fn ok(status: Status) -> Self {
        Self {
            ok: true,
            status: Some(status),
            ..Self::default()
        }
    }

    pub fn message(status: Status, message: impl Into<String>) -> Self {
        Self {
            ok: true,
            status: Some(status),
            message: Some(message.into()),
            ..Self::default()
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: Some(message.into()),
            ..Self::default()
        }
    }
}

/// A request plus where to send its response.
pub type Call = (Request, oneshot::Sender<Response>);

#[derive(Clone)]
pub struct Handles {
    pub calls: mpsc::Sender<Call>,
    pub status: watch::Receiver<Status>,
    pub semg: broadcast::Sender<Vec<u8>>,
}

/// `$KINESIS_SOCKET`, else `$XDG_RUNTIME_DIR/kinesis.sock`
/// (`kinesis-dry-run.sock` for a dry run), else under `/tmp`.
pub fn socket_path(dry_run: bool) -> PathBuf {
    if let Some(path) = std::env::var_os("KINESIS_SOCKET") {
        return PathBuf::from(path);
    }
    let name = if dry_run {
        "kinesis-dry-run.sock"
    } else {
        "kinesis.sock"
    };
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) => PathBuf::from(dir).join(name),
        None => {
            let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
            PathBuf::from(format!("/tmp/{user}-{name}"))
        }
    }
}

/// Bind the socket (0600). A live socket means another daemon runs; a stale file is replaced.
pub fn bind(path: &Path) -> io::Result<UnixListener> {
    if path.exists() {
        if std::os::unix::net::UnixStream::connect(path).is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "kinesisd is already running",
            ));
        }
        std::fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

pub async fn serve(listener: UnixListener, handles: Handles) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let handles = handles.clone();
                tokio::spawn(async move {
                    if let Err(error) = connection(stream, handles).await {
                        tracing::debug!("control client: {error}");
                    }
                });
            }
            Err(error) => {
                tracing::warn!("control socket accept failed: {error}");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

async fn write_line<W: AsyncWrite + Unpin>(writer: &mut W, response: &Response) -> io::Result<()> {
    let mut line = serde_json::to_vec(response).map_err(io::Error::other)?;
    line.push(b'\n');
    writer.write_all(&line).await
}

async fn connection(stream: UnixStream, handles: Handles) -> io::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let request = match serde_json::from_str::<Request>(&line) {
            Ok(request) => request,
            Err(error) => {
                write_line(
                    &mut write,
                    &Response::error(format!("bad request: {error}")),
                )
                .await?;
                continue;
            }
        };
        match request {
            Request::Subscribe => {
                let mut status = handles.status.clone();
                loop {
                    let current = status.borrow_and_update().clone();
                    write_line(&mut write, &Response::ok(current)).await?;
                    if status.changed().await.is_err() {
                        return Ok(());
                    }
                }
            }
            Request::SubscribeSemg => {
                let mut frames = handles.semg.subscribe();
                loop {
                    match frames.recv().await {
                        Ok(frame) => {
                            let line = Response {
                                ok: true,
                                semg: Some(B64.encode(frame)),
                                ..Response::default()
                            };
                            write_line(&mut write, &line).await?;
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => return Ok(()),
                    }
                }
            }
            request => {
                let (reply, response) = oneshot::channel();
                if handles.calls.send((request, reply)).await.is_err() {
                    return Ok(());
                }
                let response = response
                    .await
                    .unwrap_or_else(|_| Response::error("kinesisd is shutting down"));
                write_line(&mut write, &response).await?;
            }
        }
    }
    Ok(())
}

/// One request, one response (for kinesisctl and tests).
pub async fn call(path: &Path, request: &Request) -> io::Result<Response> {
    let stream = UnixStream::connect(path).await?;
    let (read, mut write) = stream.into_split();
    let mut line = serde_json::to_vec(request).map_err(io::Error::other)?;
    line.push(b'\n');
    write.write_all(&line).await?;
    let mut lines = BufReader::new(read).lines();
    let answer = lines.next_line().await?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "kinesisd closed the connection",
        )
    })?;
    serde_json::from_str(&answer).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}
