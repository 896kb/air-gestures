use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use kinesisd::ipc::{self, Request, Response};
use kinesisd::status::LinkState;

struct Daemon {
    child: Child,
    log: PathBuf,
}

impl Daemon {
    /// The last lines the daemon wrote to stderr, for failure messages.
    fn log_tail(&self) -> String {
        let text = std::fs::read_to_string(&self.log).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        let tail = lines[lines.len().saturating_sub(20)..].join("\n");
        format!("\n--- kinesisd stderr (tail) ---\n{tail}")
    }

    /// Wait up to `timeout` for the daemon to exit; kill it and panic if it doesn't.
    fn wait(&mut self, timeout: Duration) -> ExitStatus {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                panic!("kinesisd didn't exit within {timeout:?}{}", self.log_tail());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn call(daemon: &Daemon, socket: &Path, request: Request) -> Response {
    match ipc::call(socket, &request).await {
        Ok(response) => response,
        Err(error) => panic!("{request:?} failed: {error}{}", daemon.log_tail()),
    }
}

async fn wait_until(
    daemon: &Daemon,
    socket: &Path,
    what: &str,
    until: impl Fn(&Response) -> bool,
) -> Response {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(response) = ipc::call(socket, &Request::Status).await
            && until(&response)
        {
            return response;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}{}",
            daemon.log_tail()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn the_dry_run_daemon_pairs_connects_and_forgets() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("kinesis.sock");
    let log = dir.path().join("kinesisd.log");
    let child = Command::new(env!("CARGO_BIN_EXE_kinesisd"))
        .arg("--dry-run")
        .env("KINESIS_SOCKET", &socket)
        .env("XDG_STATE_HOME", dir.path().join("state"))
        .env("XDG_CONFIG_HOME", dir.path().join("config"))
        .env("KINESIS_LOG", "info")
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(&log).unwrap())
        .spawn()
        .unwrap();
    let mut daemon = Daemon { child, log };

    let first = wait_until(&daemon, &socket, "the unpaired error", |r| {
        r.status.as_ref().is_some_and(|s| s.error.is_some())
    })
    .await;
    let error = first.status.unwrap().error.unwrap();
    assert!(
        error.contains("kinesisctl pair"),
        "{error}{}",
        daemon.log_tail()
    );

    let paired = call(&daemon, &socket, Request::Pair { address: None }).await;
    assert!(paired.ok, "{paired:?}{}", daemon.log_tail());
    let connected = wait_until(&daemon, &socket, "the owner connection", |r| {
        r.status
            .as_ref()
            .is_some_and(|s| s.state == LinkState::Connected)
    })
    .await;
    assert_eq!(
        connected.status.unwrap().band.as_deref(),
        Some("Meta Band (simulated)"),
        "{}",
        daemon.log_tail()
    );
    let owner_key = dir.path().join("state/kinesis-dry-run/owner.key");
    assert!(owner_key.exists(), "no owner key{}", daemon.log_tail());

    let forgot = call(&daemon, &socket, Request::Forget).await;
    assert!(forgot.ok, "{forgot:?}{}", daemon.log_tail());
    assert!(!owner_key.exists(), "owner key kept{}", daemon.log_tail());

    // SIGTERM shuts it down cleanly and removes the socket.
    Command::new("kill")
        .arg(daemon.child.id().to_string())
        .status()
        .unwrap();
    let status = daemon.wait(Duration::from_secs(10));
    assert!(status.success(), "{status:?}{}", daemon.log_tail());
    assert!(!socket.exists(), "socket left behind{}", daemon.log_tail());
}
