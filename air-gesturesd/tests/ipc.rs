use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use air_gesturesd::config::DialTarget;
use air_gesturesd::ipc::{self, Call, HandChoice, Handles, Request, Response};
use air_gesturesd::status::{LinkState, Status};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{broadcast, mpsc, watch};

/// A socket server whose "daemon" answers every call with the current status.
async fn serve(
    dir: &std::path::Path,
) -> (
    std::path::PathBuf,
    watch::Sender<Status>,
    broadcast::Sender<Vec<u8>>,
) {
    let path = dir.join("air-gestures.sock");
    let listener = ipc::bind(&path).unwrap();
    let (status_tx, status) = watch::channel(Status::default());
    let (semg, _) = broadcast::channel(16);
    let (calls, mut call_rx) = mpsc::channel::<Call>(8);
    let answering = status.clone();
    tokio::spawn(async move {
        while let Some((request, reply)) = call_rx.recv().await {
            let response = match request {
                Request::Pair {
                    address: Some(address),
                } => Response::error(format!("no band at {address}")),
                _ => Response::ok(answering.borrow().clone()),
            };
            let _ = reply.send(response);
        }
    });
    tokio::spawn(ipc::serve(
        listener,
        Handles {
            calls,
            status,
            semg: semg.clone(),
        },
    ));
    (path, status_tx, semg)
}

#[test]
fn requests_use_kebab_case_commands() {
    let parse = |json: &str| serde_json::from_str::<Request>(json).unwrap();
    assert_eq!(parse(r#"{"cmd":"status"}"#), Request::Status);
    assert_eq!(parse(r#"{"cmd":"toggle-pause"}"#), Request::TogglePause);
    assert_eq!(
        parse(r#"{"cmd":"dial","target":"brightness"}"#),
        Request::Dial {
            target: DialTarget::Brightness
        }
    );
    assert_eq!(
        parse(r#"{"cmd":"hand","hand":"left"}"#),
        Request::Hand {
            hand: HandChoice::Left
        }
    );
    assert_eq!(parse(r#"{"cmd":"pair"}"#), Request::Pair { address: None });
    assert_eq!(
        parse(r#"{"cmd":"semg","on":true}"#),
        Request::Semg { on: true }
    );
    assert_eq!(
        serde_json::to_string(&Request::SubscribeSemg).unwrap(),
        r#"{"cmd":"subscribe-semg"}"#
    );
}

#[tokio::test]
async fn a_call_gets_one_response() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _status, _semg) = serve(dir.path()).await;
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let response = ipc::call(&path, &Request::Status).await.unwrap();
    assert!(response.ok);
    assert_eq!(response.status.unwrap().state, LinkState::Idle);
    let refused = ipc::call(
        &path,
        &Request::Pair {
            address: Some("x".into()),
        },
    )
    .await
    .unwrap();
    assert!(!refused.ok && refused.error.unwrap().contains("no band at x"));
}

#[tokio::test]
async fn a_bad_line_gets_an_error_and_the_connection_stays_usable() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _status, _semg) = serve(dir.path()).await;
    let stream = UnixStream::connect(&path).await.unwrap();
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    write
        .write_all(b"not json\n{\"cmd\":\"status\"}\n")
        .await
        .unwrap();
    let first: Response = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert!(!first.ok && first.error.unwrap().contains("bad request"));
    let second: Response =
        serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert!(second.ok);
}

#[tokio::test]
async fn subscribers_get_the_status_now_and_on_every_change() {
    let dir = tempfile::tempdir().unwrap();
    let (path, status, _semg) = serve(dir.path()).await;
    let stream = UnixStream::connect(&path).await.unwrap();
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    write.write_all(b"{\"cmd\":\"subscribe\"}\n").await.unwrap();
    let now: Response = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(now.status.unwrap().state, LinkState::Idle);
    status
        .send(Status {
            state: LinkState::Connected,
            ..Status::default()
        })
        .unwrap();
    let next = tokio::time::timeout(Duration::from_secs(5), lines.next_line())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let next: Response = serde_json::from_str(&next).unwrap();
    assert_eq!(next.status.unwrap().state, LinkState::Connected);
}

#[tokio::test]
async fn semg_subscribers_get_base64_frames() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _status, semg) = serve(dir.path()).await;
    let stream = UnixStream::connect(&path).await.unwrap();
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    write
        .write_all(b"{\"cmd\":\"subscribe-semg\"}\n")
        .await
        .unwrap();
    // Give the server a moment to subscribe before sending.
    tokio::time::sleep(Duration::from_millis(100)).await;
    semg.send(vec![1, 2, 3]).unwrap();
    let line = tokio::time::timeout(Duration::from_secs(5), lines.next_line())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let frame: Response = serde_json::from_str(&line).unwrap();
    assert_eq!(frame.semg.as_deref(), Some("AQID"));
}

#[tokio::test]
async fn a_second_daemon_cannot_take_a_live_socket_but_a_stale_one_is_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _status, _semg) = serve(dir.path()).await;
    assert_eq!(
        ipc::bind(&path).unwrap_err().kind(),
        std::io::ErrorKind::AddrInUse
    );
    let stale = dir.path().join("stale.sock");
    drop(std::os::unix::net::UnixListener::bind(&stale).unwrap());
    assert!(stale.exists());
    ipc::bind(&stale).unwrap();
}

#[tokio::test]
async fn a_subscriber_that_hangs_up_is_let_go() {
    let dir = tempfile::tempdir().unwrap();
    let (path, status, _semg) = serve(dir.path()).await;
    let baseline = status.receiver_count();
    let stream = UnixStream::connect(&path).await.unwrap();
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    write.write_all(b"{\"cmd\":\"subscribe\"}\n").await.unwrap();
    lines.next_line().await.unwrap().unwrap();
    assert!(status.receiver_count() > baseline);
    drop(lines);
    drop(write);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        status.receiver_count(),
        baseline,
        "the connection task still holds the status"
    );
}

#[tokio::test]
async fn an_overlong_request_is_refused_and_the_connection_closed() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _status, _semg) = serve(dir.path()).await;
    let stream = UnixStream::connect(&path).await.unwrap();
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    let mut request = vec![b'x'; 70 * 1024];
    request.push(b'\n');
    // The server may close before reading all of it.
    let _ = write.write_all(&request).await;
    let line = tokio::time::timeout(Duration::from_secs(5), lines.next_line())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let refused: Response = serde_json::from_str(&line).unwrap();
    assert!(!refused.ok && refused.error.unwrap().contains("request too long"));
    let end = tokio::time::timeout(Duration::from_secs(5), lines.next_line())
        .await
        .unwrap();
    assert!(matches!(end, Ok(None) | Err(_)), "the connection is closed");
}

#[test]
fn bind_refuses_to_replace_something_that_is_not_a_socket() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("air-gestures.sock");
    std::fs::write(&path, b"precious").unwrap();
    let error = ipc::bind(&path).unwrap_err();
    assert!(error.to_string().contains("not a socket"), "{error}");
    assert_eq!(std::fs::read(&path).unwrap(), b"precious");
}
