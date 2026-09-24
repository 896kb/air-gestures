use std::time::Duration;

use kinesisd::actions::{RecordingRunner, Runner, ShellRunner};

async fn appears(path: &std::path::Path, within: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + within;
    while tokio::time::Instant::now() < deadline {
        if path.exists() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    path.exists()
}

#[tokio::test]
async fn commands_run_through_the_shell() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("ran");
    ShellRunner::default().run(format!("true && touch '{}'", marker.display()));
    assert!(appears(&marker, Duration::from_secs(3)).await);
}

#[tokio::test]
async fn a_stuck_command_is_killed_at_the_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("late");
    ShellRunner {
        timeout: Duration::from_millis(100),
    }
    .run(format!("sleep 1; touch '{}'", marker.display()));
    assert!(!appears(&marker, Duration::from_millis(1600)).await);
}

#[test]
fn the_recording_runner_keeps_every_command() {
    let runner = RecordingRunner::default();
    runner.run("a".into());
    runner.run("b".into());
    assert_eq!(runner.commands(), vec!["a".to_string(), "b".to_string()]);
}
