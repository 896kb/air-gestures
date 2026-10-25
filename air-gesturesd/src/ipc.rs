//! The control socket: newline-delimited JSON requests and responses.

use std::io;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde::{Deserialize, Serialize};
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::config::DialTarget;
use crate::status::Status;
use crate::store::StoredPalm;
use crate::teach::Step;

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
    /// Switch profiles: "next", "previous" or a profile's name.
    Profile {
        to: String,
    },
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
    /// Forget the band: keys, band record and the Bluetooth bond (the Meta
    /// sign-in is kept; `air-gestures signout` removes it).
    Forget,
    /// Teaching: the band's roll for `seconds`, for the palm calibration (answers `teach: rolls`).
    PalmRolls { seconds: f64 },
    /// Teaching drags: the next pinch that moves, within `seconds` (answers `teach: drag`).
    TeachDrag { seconds: f64 },
    /// Teaching a drawing: one drawing, opened now, within `seconds` (answers `teach: strokes`).
    DrawTry { seconds: f64 },
    /// Store (`null`: forget) the palm calibration for the paired band and the hand in use.
    SetPalm {
        #[serde(default)]
        palm: Option<StoredPalm>,
    },
    /// Store (`null`: forget) the taught drags for the paired band and the hand in use.
    SetDrags {
        #[serde(default)]
        drags: Option<TaughtDrags>,
    },
    /// The palm position the band reads now (answers `teach: palm` at once).
    Palm,
}

impl Request {
    /// A teaching step (and how long it may wait, within the step's limits): answered when
    /// the band has done it, at its deadline, or never if the client hangs up first (which
    /// ends the step).
    pub fn teach_step(&self) -> Option<(Step, f64)> {
        let (step, seconds) = match self {
            Request::PalmRolls { seconds } => (Step::Rolls, *seconds),
            Request::TeachDrag { seconds } => (Step::Drag, *seconds),
            Request::DrawTry { seconds } => (Step::Draw, *seconds),
            Request::Status
            | Request::Subscribe
            | Request::SubscribeSemg
            | Request::Connect
            | Request::Disconnect
            | Request::Pause
            | Request::Resume
            | Request::TogglePause
            | Request::Dial { .. }
            | Request::CycleDial
            | Request::Profile { .. }
            | Request::Hand { .. }
            | Request::Semg { .. }
            | Request::Pair { .. }
            | Request::Forget
            | Request::SetPalm { .. }
            | Request::SetDrags { .. }
            | Request::Palm => return None,
        };
        let (shortest, longest) = step.limits();
        // `clamp` keeps a NaN: that's no time at all.
        let seconds = if seconds.is_nan() { shortest } else { seconds.clamp(shortest, longest) };
        Some((step, seconds))
    }
}

/// Taught drag samples: the turn, then the six drags in `Drag::ALL` order (forearm frame).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaughtDrags {
    pub turn: [f64; 3],
    pub drags: [[f64; 3]; 6],
}

/// What a teaching step brought back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Taught {
    /// The readable rolls (degrees) of `samples` orientation samples.
    Rolls { rolls: Vec<f64>, samples: usize },
    /// One pinch's direction (forearm frame, unit); `finger` is "index" or "middle".
    Drag { finger: String, direction: [f64; 3] },
    /// One drawing's strokes of [x, y] points (degrees).
    Strokes { strokes: Vec<Vec<[f64; 2]>> },
    /// The palm position now: "down", "sideways" or "up" (`Palm::key`).
    Palm { palm: String },
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
    /// A stable, machine-readable tag for `error` (e.g. [`CODE_NEEDS_SIGN_IN`]),
    /// so a client can react without matching on the human-readable text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// What a teaching step (or `palm`) brought back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub teach: Option<Taught>,
}

/// `error`'s code when a `Pair` request needs a Meta sign-in first.
pub const CODE_NEEDS_SIGN_IN: &str = "needs_sign_in";
/// `error`'s code when a teaching try didn't work (nothing drawn, no pinch in time): try again.
pub const CODE_TRY_AGAIN: &str = "try_again";

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

    pub fn taught(status: Status, taught: Taught) -> Self {
        Self { ok: true, status: Some(status), teach: Some(taught), ..Self::default() }
    }

    /// An `error` carrying a stable `code` a client can match on.
    pub fn error_code(message: impl Into<String>, code: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: Some(message.into()),
            code: Some(code.into()),
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

/// The longest request line accepted (without its newline).
const MAX_LINE: usize = 64 * 1024;

/// `$AIR_GESTURES_SOCKET`, else `$XDG_RUNTIME_DIR/air-gestures.sock`
/// (`air-gestures-dry-run.sock` for a dry run), else in a private
/// `/tmp/air-gestures-<uid>/` directory (created and checked by `bind`).
pub fn socket_path(dry_run: bool) -> PathBuf {
    if let Some(path) = std::env::var_os("AIR_GESTURES_SOCKET") {
        return PathBuf::from(path);
    }
    let name = if dry_run {
        "air-gestures-dry-run.sock"
    } else {
        "air-gestures.sock"
    };
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) => PathBuf::from(dir).join(name),
        None => fallback_dir().join(name),
    }
}

/// Our uid: the owner of `/proc/self` is the process's effective uid.
fn uid() -> io::Result<u32> {
    std::fs::metadata("/proc/self").map(|metadata| metadata.uid())
}

/// The socket directory when there is no `$XDG_RUNTIME_DIR`.
fn fallback_dir() -> PathBuf {
    match uid() {
        Ok(uid) => PathBuf::from(format!("/tmp/air-gestures-{uid}")),
        // Unknown uid: a name `bind` never trusts as the private directory.
        Err(_) => PathBuf::from("/tmp/air-gestures-unknown"),
    }
}

/// Create `dir` (0700) if needed and make sure it is a real directory that
/// only we can use: `/tmp` is shared, so someone else may have made it first.
fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Err(error) if error.kind() != io::ErrorKind::AlreadyExists => return Err(error),
        _ => {}
    }
    let metadata = std::fs::symlink_metadata(dir)?;
    if !metadata.file_type().is_dir() || metadata.uid() != uid()? || metadata.mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "unsafe socket directory (not a private directory of ours): {}",
                dir.display()
            ),
        ));
    }
    Ok(())
}

/// Bind the socket (0600). A live socket means another daemon runs; a stale
/// socket is replaced; anything else at the path is left alone.
pub fn bind(path: &Path) -> io::Result<UnixListener> {
    if let Some(parent) = path.parent()
        && parent == fallback_dir()
    {
        ensure_private_dir(parent)?;
    }
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.file_type().is_socket() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("not a socket: {}", path.display()),
                ));
            }
            if std::os::unix::net::UnixStream::connect(path).is_ok() {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    "air-gesturesd is already running",
                ));
            }
            std::fs::remove_file(path)?;
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
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

enum Line {
    Text(String),
    TooLong,
    End,
}

/// One request line, at most `MAX_LINE` bytes.
async fn read_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    buffer: &mut Vec<u8>,
) -> io::Result<Line> {
    buffer.clear();
    let read = (&mut *reader)
        .take(MAX_LINE as u64 + 1)
        .read_until(b'\n', buffer)
        .await?;
    if read == 0 {
        return Ok(Line::End);
    }
    if buffer.last() == Some(&b'\n') {
        buffer.pop();
    } else if buffer.len() > MAX_LINE {
        return Ok(Line::TooLong);
    }
    Ok(Line::Text(String::from_utf8_lossy(buffer).into_owned()))
}

/// Resolves once the client hangs up (or its side fails); anything it sends
/// meanwhile is ignored. Cancel-safe.
async fn hung_up<R: AsyncBufRead + Unpin>(reader: &mut R) {
    loop {
        let length = match reader.fill_buf().await {
            Ok([]) | Err(_) => return,
            Ok(bytes) => bytes.len(),
        };
        reader.consume(length);
    }
}

async fn connection(stream: UnixStream, handles: Handles) -> io::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);
    let mut buffer = Vec::new();
    loop {
        let line = match read_line(&mut reader, &mut buffer).await? {
            Line::Text(line) => line,
            Line::End => return Ok(()),
            Line::TooLong => {
                write_line(&mut write, &Response::error("request too long")).await?;
                return Ok(());
            }
        };
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
                    tokio::select! {
                        changed = status.changed() => if changed.is_err() {
                            return Ok(());
                        },
                        () = hung_up(&mut reader) => return Ok(()),
                    }
                }
            }
            Request::SubscribeSemg => {
                let mut frames = handles.semg.subscribe();
                loop {
                    let frame = tokio::select! {
                        frame = frames.recv() => frame,
                        () = hung_up(&mut reader) => return Ok(()),
                    };
                    match frame {
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
                let waits = request.teach_step().is_some();
                let (reply, response) = oneshot::channel();
                if handles.calls.send((request, reply)).await.is_err() {
                    return Ok(());
                }
                let response = if waits {
                    // A teaching step waits for the band; a client that hangs up meanwhile
                    // drops `response`, and the daemon ends the step. `hung_up` reads (and
                    // discards) whatever the client sends during the step, so a request
                    // pipelined behind it is lost; and a client that only shuts down its
                    // writing half counts as hung up too (its EOF is all we can see).
                    tokio::select! {
                        response = response => response,
                        () = hung_up(&mut reader) => return Ok(()),
                    }
                } else {
                    response.await
                };
                let response = response.unwrap_or_else(|_| Response::error("air-gesturesd is shutting down"));
                write_line(&mut write, &response).await?;
            }
        }
    }
}

/// One request, one response (for air-gestures and tests).
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
            "air-gesturesd closed the connection",
        )
    })?;
    serde_json::from_str(&answer).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}
