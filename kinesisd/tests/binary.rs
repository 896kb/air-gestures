use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use kinesisd::ipc::{self, Request, Response};
use kinesisd::status::LinkState;

struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

async fn call(socket: &Path, request: Request) -> Response {
    ipc::call(socket, &request).await.unwrap()
}

async fn wait_until(socket: &Path, what: &str, until: impl Fn(&Response) -> bool) -> Response {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(response) = ipc::call(socket, &Request::Status).await
            && until(&response)
        {
            return response;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn the_dry_run_daemon_pairs_connects_and_forgets() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("kinesis.sock");
    let child = Command::new(env!("CARGO_BIN_EXE_kinesisd"))
        .arg("--dry-run")
        .env("KINESIS_SOCKET", &socket)
        .env("XDG_STATE_HOME", dir.path().join("state"))
        .env("XDG_CONFIG_HOME", dir.path().join("config"))
        .env("KINESIS_LOG", "warn")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut daemon = Daemon(child);

    let first = wait_until(&socket, "the unpaired error", |r| {
        r.status.as_ref().is_some_and(|s| s.error.is_some())
    })
    .await;
    assert!(
        first
            .status
            .unwrap()
            .error
            .unwrap()
            .contains("kinesisctl pair")
    );

    let paired = call(&socket, Request::Pair { address: None }).await;
    assert!(paired.ok, "{paired:?}");
    let connected = wait_until(&socket, "the owner connection", |r| {
        r.status
            .as_ref()
            .is_some_and(|s| s.state == LinkState::Connected)
    })
    .await;
    assert_eq!(
        connected.status.unwrap().band.as_deref(),
        Some("Meta Band (simulated)")
    );
    assert!(dir.path().join("state/kinesis-dry-run/owner.key").exists());

    let forgot = call(&socket, Request::Forget).await;
    assert!(forgot.ok);
    assert!(!dir.path().join("state/kinesis-dry-run/owner.key").exists());

    // SIGTERM shuts it down cleanly and removes the socket.
    Command::new("kill")
        .arg(daemon.0.id().to_string())
        .status()
        .unwrap();
    let status = daemon.0.wait().unwrap();
    assert!(status.success(), "{status:?}");
    assert!(!socket.exists());
}
