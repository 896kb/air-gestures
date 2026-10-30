mod support;

use std::time::Duration;

use air_gestures::teach::{self, Guide};
use air_gesturesd::status::LinkState;
use std::sync::{Arc, Mutex};

use air_gesturesd::transport::{SIM_ADDRESS, SimTransport};
use band_core::events::Hand;
use band_core::responder::SimMode;
use tokio::sync::mpsc;

/// Records what the flow says; each notification goes to the test (the cue to act). With
/// `roll`, the "Palm down" / "Palm up" notifications also turn the simulated wrist (before
/// the flow asks the daemon for rolls, so no sample of the other position gets in).
struct Recorded {
    said: Vec<String>,
    notified: mpsc::UnboundedSender<String>,
    roll: Option<Arc<Mutex<f64>>>,
    /// Drop the band's link at the post-save check.
    close_on_check: Option<SimTransport>,
}

impl Guide for Recorded {
    fn say(&mut self, text: &str) {
        self.said.push(text.to_owned());
    }
    fn notify(&mut self, title: &str, body: &str) {
        if let Some(roll) = &self.roll {
            match title {
                "Palm down" => *roll.lock().unwrap() = -140.0,
                "Palm up" => *roll.lock().unwrap() = -10.0,
                _ => {}
            }
        }
        let _ = self.notified.send(format!("{title}: {body}"));
    }
    fn ready(&mut self, prompt: &str) -> bool {
        if let (true, Some(transport)) = (prompt.starts_with("Check"), &self.close_on_check) {
            transport.inject(air_gesturesd::transport::SimInject::Close);
            std::thread::sleep(Duration::from_millis(500));
        }
        true
    }
}

fn guide() -> (Recorded, mpsc::UnboundedReceiver<String>) {
    let (notified, cues) = mpsc::unbounded_channel();
    (Recorded { said: Vec::new(), notified, roll: None, close_on_check: None }, cues)
}

#[tokio::test]
async fn calibrating_saves_palm_down_and_up_and_confirms_palm_up() {
    let daemon = support::start(SimMode::Enrolled, true).await;
    daemon.wait("the hand", |s| s.state == LinkState::Connected && s.hand.as_deref() == Some("right")).await;
    let roll = Arc::new(Mutex::new(-140.0));
    let feeder = tokio::spawn(support::feed_roll(daemon.transport.clone(), roll.clone()));
    let (mut guide, _cues) = guide();
    guide.roll = Some(roll);
    let socket = daemon.socket.clone();
    let flow = tokio::spawn(async move { teach::calibrate(&socket, &mut guide).await.map(|c| (c, guide.said)) });
    let (calibration, said) = flow.await.unwrap().unwrap();
    feeder.abort();
    assert!((calibration.down_roll + 140.0).abs() < 0.5 && (calibration.up_roll + 10.0).abs() < 0.5, "{calibration:?}");
    assert!(said.iter().any(|s| s.contains("palm up now")), "{said:?}");
    let stored = daemon.store.load_calibration(SIM_ADDRESS, Hand::Right).unwrap();
    assert_eq!(stored.palm.map(|p| p.calibration()), Some(calibration));
}

#[tokio::test]
async fn calibrating_without_orientation_says_so() {
    let daemon = support::start(SimMode::Enrolled, true).await;
    daemon.wait("the hand", |s| s.state == LinkState::Connected && s.hand.is_some()).await;
    let (mut guide, _cues) = guide();
    let error = teach::calibrate(&daemon.socket, &mut guide).await.unwrap_err();
    assert!(error.contains("no orientation"), "{error}");
}

#[tokio::test]
async fn calibrating_needs_a_connected_band() {
    let daemon = support::start(SimMode::Enrolled, false).await;
    let (mut guide, _cues) = guide();
    let error = teach::calibrate(&daemon.socket, &mut guide).await.unwrap_err();
    assert!(error.contains("connect the band first"), "{error}");
}

use band_core::controls::Drag;
use band_core::controls::pinch::TaughtError;

#[test]
fn a_refused_set_of_drags_names_the_moves_to_teach_again() {
    assert_eq!(teach::redo(TaughtError::NoTurn), [0]);
    assert_eq!(teach::redo(TaughtError::NoDrag(Drag::Toward)), [3]);
    assert_eq!(teach::redo(TaughtError::LikeTheTurn(Drag::Up)), [5]);
    assert_eq!(teach::redo(TaughtError::TooAlike(Drag::Left, Drag::Right)), [1, 2]);
    assert_eq!(teach::redo(TaughtError::NoRoll), [0, 1, 2, 3, 4, 5, 6]);
    assert_eq!(teach::MOVES[0].0, "turn");
    assert_eq!(teach::MOVES.map(|m| m.0)[1..], Drag::ALL.map(air_gesturesd::teach::drag_name));
}

#[tokio::test]
async fn one_drag_sample_comes_back_from_the_band() {
    let daemon = support::start(SimMode::Enrolled, true).await;
    daemon.wait("the hand", |s| s.state == LinkState::Connected && s.hand.is_some()).await;
    let (mut guide, mut cues) = guide();
    let socket = daemon.socket.clone();
    let sample = tokio::spawn(async move { teach::drag_sample(&socket, &mut guide, 1, 30.0).await });
    cues.recv().await.expect("the cue");
    tokio::time::sleep(Duration::from_millis(300)).await;
    support::pinch_moving(&daemon.transport, 600, 50).await;
    let teach::Try::Got(direction) = sample.await.unwrap().unwrap() else { panic!("no sample") };
    let along: f64 = (0..3).map(|i| direction[i] * support::away()[i]).sum();
    assert!(along > 0.99, "the drag away it was made along: {direction:?}");
}

use air_gesturesd::drawings;
use band_core::controls::draw::Drawing;

#[tokio::test]
async fn teaching_a_drawing_saves_three_alike_tries() {
    let daemon = support::start(SimMode::Enrolled, true).await;
    daemon.wait("the hand", |s| s.state == LinkState::Connected && s.hand.is_some()).await;
    let (mut guide, mut cues) = guide();
    let (socket, dir) = (daemon.socket.clone(), daemon.drawings());
    let teach = tokio::spawn(async move { teach::teach_drawing(&socket, &dir, "line", &mut guide).await.map(|()| guide.said) });
    for _ in 0..3 {
        cues.recv().await.expect("a try");
        tokio::time::sleep(Duration::from_millis(300)).await;
        support::pinch_moving(&daemon.transport, 600, 40).await;
    }
    let said = teach.await.unwrap().unwrap();
    assert!(said.iter().any(|s| s.contains("[drawings]")), "{said:?}");
    let (saved, problems) = drawings::load(&daemon.drawings());
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].name, "line");
    assert_eq!(saved[0].tries.len(), 3);
}

#[tokio::test]
async fn a_taken_or_bad_name_is_refused_before_drawing() {
    let dir = tempfile::tempdir().unwrap();
    let line = vec![vec![[0.0, 0.0], [0.0, 10.0]]];
    drawings::save(dir.path(), &Drawing { name: "z".into(), tries: vec![line] }).unwrap();
    let nowhere = dir.path().join("no.sock");
    let (mut guide, _cues) = guide();
    let taken = teach::teach_drawing(&nowhere, dir.path(), "z", &mut guide).await.unwrap_err();
    assert!(taken.contains("already taught") && taken.contains("air-gestures draw delete z"), "{taken}");
    let bad = teach::teach_drawing(&nowhere, dir.path(), "it's", &mut guide).await.unwrap_err();
    assert!(bad.contains("it's"), "{bad}");
}

#[test]
fn drawings_are_listed_and_deleted() {
    let dir = tempfile::tempdir().unwrap();
    assert!(teach::list_drawings(dir.path()).unwrap().contains("no drawings yet"));
    let line = vec![vec![[0.0, 0.0], [0.0, 10.0]]];
    drawings::save(dir.path(), &Drawing { name: "z".into(), tries: vec![line.clone(), line] }).unwrap();
    assert_eq!(teach::list_drawings(dir.path()).unwrap(), "z (2 tries)");
    teach::delete_drawing(dir.path(), "z").unwrap();
    assert!(teach::delete_drawing(dir.path(), "z").unwrap_err().contains("no drawing \"z\""));
}

#[tokio::test]
async fn a_client_that_hangs_up_ends_its_teaching_step() {
    use tokio::io::AsyncWriteExt;
    let daemon = support::start(SimMode::Enrolled, true).await;
    daemon.wait("the hand", |s| s.state == LinkState::Connected && s.hand.is_some()).await;
    let mut stream = tokio::net::UnixStream::connect(&daemon.socket).await.unwrap();
    stream.write_all(b"{\"cmd\":\"teach-drag\",\"seconds\":30.0}\n").await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    drop(stream);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let request = air_gesturesd::ipc::Request::TeachDrag { seconds: 1.0 };
    let error = air_gestures::client::request_within(&daemon.socket, &request, Duration::from_secs(5)).await.unwrap_err();
    assert!(error.message.contains("no pinch"), "the first step ended: {error}");
}

use air_gesturesd::ipc::Request;

/// Waits for the next cue, then does `act` for it.
async fn on_cue(cues: &mut mpsc::UnboundedReceiver<String>) -> String {
    let cue = tokio::time::timeout(Duration::from_secs(20), cues.recv()).await.expect("a cue in time").expect("a cue");
    tokio::time::sleep(Duration::from_millis(300)).await;
    cue
}

#[tokio::test]
async fn a_killed_flow_doesnt_block_the_next_one() {
    let daemon = support::start(SimMode::Enrolled, true).await;
    daemon.wait("the hand", |s| s.state == LinkState::Connected && s.hand.is_some()).await;
    let (mut guide, mut cues) = guide();
    let socket = daemon.socket.clone();
    let flow = tokio::spawn(async move { teach::drag_sample(&socket, &mut guide, 0, 30.0).await });
    cues.recv().await.expect("the cue");
    tokio::time::sleep(Duration::from_millis(200)).await;
    flow.abort();
    let _ = flow.await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let request = Request::TeachDrag { seconds: 1.0 };
    let error = air_gestures::client::request_within(&daemon.socket, &request, Duration::from_secs(5)).await.unwrap_err();
    assert!(error.message.contains("no pinch"), "accepted, then missed: {error}");
}

#[tokio::test]
async fn a_failed_drawing_try_is_drawn_again() {
    let daemon = support::start(SimMode::Enrolled, true).await;
    daemon.wait("the hand", |s| s.state == LinkState::Connected && s.hand.is_some()).await;
    let (mut guide, mut cues) = guide();
    let (socket, dir) = (daemon.socket.clone(), daemon.drawings());
    let teach = tokio::spawn(async move { teach::teach_drawing_within(&socket, &dir, "line", &mut guide, 3.0).await.map(|()| guide.said) });
    // Good, nothing (the try runs out), good; then the missed one again.
    for act in [true, false, true, true] {
        on_cue(&mut cues).await;
        if act {
            support::pinch_moving(&daemon.transport, 600, 40).await;
        }
    }
    let said = teach.await.unwrap().unwrap();
    assert!(said.iter().any(|s| s.contains("didn't end within 3 s")), "{said:?}");
    let (saved, _) = drawings::load(&daemon.drawings());
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].tries.len(), 3);
}

#[tokio::test]
async fn a_too_small_try_is_drawn_again() {
    let daemon = support::start(SimMode::Enrolled, true).await;
    daemon.wait("the hand", |s| s.state == LinkState::Connected && s.hand.is_some()).await;
    let (mut guide, mut cues) = guide();
    let (socket, dir) = (daemon.socket.clone(), daemon.drawings());
    let teach = tokio::spawn(async move { teach::teach_drawing_within(&socket, &dir, "line", &mut guide, 5.0).await.map(|()| guide.said) });
    for raw in [600, 50, 600, 600] {
        on_cue(&mut cues).await;
        support::pinch_moving(&daemon.transport, raw, 40).await;
    }
    let said = teach.await.unwrap().unwrap();
    assert!(said.iter().any(|s| s.contains("too small") || s.contains("nothing was drawn")), "{said:?}");
    assert_eq!(drawings::load(&daemon.drawings()).0[0].tries.len(), 3);
}

#[tokio::test]
async fn a_missed_drag_is_taught_again() {
    let daemon = support::start(SimMode::Enrolled, true).await;
    daemon.wait("the hand", |s| s.state == LinkState::Connected && s.hand.as_deref() == Some("right")).await;
    let (mut guide, mut cues) = guide();
    let socket = daemon.socket.clone();
    let teach = tokio::spawn(async move { teach::teach_drags_within(&socket, &mut guide, &[], 3.0).await.map(|()| guide.said) });
    // The turn, then six clearly different drags; the second drag is missed once.
    let turn = [350, 350, 350];
    let drags = [[600, 0, 0], [-600, 0, 0], [0, 600, 0], [0, -600, 0], [0, 0, 600], [0, 0, -600]];
    let mut moves: Vec<Option<[i16; 3]>> = std::iter::once(Some(turn)).chain(drags.iter().copied().map(Some)).collect();
    moves.insert(2, None);
    moves.push(Some(drags[1]));
    for axes in moves {
        on_cue(&mut cues).await;
        if let Some(axes) = axes {
            support::pinch_moving3(&daemon.transport, axes, 50).await;
        }
    }
    let said = teach.await.unwrap().unwrap();
    assert!(said.iter().any(|s| s.contains("no pinch that moved")), "{said:?}");
    assert!(daemon.store.load_calibration(SIM_ADDRESS, Hand::Right).unwrap().drags.is_some(), "{said:?}");
}

#[tokio::test]
async fn skipped_drags_are_not_asked_for_and_are_saved_as_skipped() {
    let daemon = support::start(SimMode::Enrolled, true).await;
    daemon.wait("the hand", |s| s.state == LinkState::Connected && s.hand.as_deref() == Some("right")).await;
    let (mut guide, mut cues) = guide();
    let socket = daemon.socket.clone();
    let skip = [Drag::Up, Drag::Down];
    let teach = tokio::spawn(async move { teach::teach_drags_within(&socket, &mut guide, &skip, 3.0).await.map(|()| guide.said) });
    // The turn, then the four drags not skipped.
    let moves = [[350, 350, 350], [600, 0, 0], [-600, 0, 0], [0, 600, 0], [0, -600, 0]];
    for axes in moves {
        on_cue(&mut cues).await;
        support::pinch_moving3(&daemon.transport, axes, 50).await;
    }
    let said = teach.await.unwrap().unwrap();
    assert!(said.iter().any(|s| s.starts_with("Skipped: up, down")), "{said:?}");
    assert!(!said.iter().any(|s| s.contains(" up: pinch") || s.contains(" down: pinch")), "{said:?}");
    let stored = daemon.store.load_calibration(SIM_ADDRESS, Hand::Right).unwrap().drags.unwrap();
    assert_eq!(stored.drags.map(|d| d.is_some()), [true, true, true, true, false, false]);
}

#[test]
fn drags_are_named_as_taught() {
    assert_eq!(teach::drag_named("toward"), Some(Drag::Toward));
    assert_eq!(teach::drag_named("down"), Some(Drag::Down));
    assert_eq!(teach::drag_named("turn"), None);
}

#[tokio::test]
async fn a_busy_daemon_ends_the_flow_and_changes_nothing() {
    let daemon = support::start(SimMode::Enrolled, true).await;
    daemon.wait("the hand", |s| s.state == LinkState::Connected && s.hand.is_some()).await;
    let socket = daemon.socket.clone();
    let other = tokio::spawn(async move {
        air_gestures::client::request_within(&socket, &Request::TeachDrag { seconds: 30.0 }, Duration::from_secs(40)).await
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (mut guide, _cues) = guide();
    let error = teach::teach_drags(&daemon.socket, &mut guide, &[]).await.unwrap_err();
    assert!(error.contains("another teaching step") && error.contains("nothing was changed"), "{error}");
    other.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_held_palm_that_moved_is_held_again() {
    let daemon = support::start(SimMode::Enrolled, true).await;
    daemon.wait("the hand", |s| s.state == LinkState::Connected && s.hand.as_deref() == Some("right")).await;
    // (roll, wobble): the first palm-down hold wobbles by ±30°.
    let roll = Arc::new(Mutex::new((-140.0, false)));
    let feeder = tokio::spawn(feed_wobbly(daemon.transport.clone(), roll.clone()));
    struct Wobbly {
        roll: Arc<Mutex<(f64, bool)>>,
        downs: usize,
        said: Vec<String>,
    }
    impl Guide for Wobbly {
        fn say(&mut self, text: &str) {
            self.said.push(text.to_owned());
        }
        fn notify(&mut self, title: &str, _body: &str) {
            match title {
                "Palm down" => {
                    self.downs += 1;
                    *self.roll.lock().unwrap() = (-140.0, self.downs == 1);
                }
                "Palm up" => *self.roll.lock().unwrap() = (-10.0, false),
                _ => {}
            }
        }
        fn ready(&mut self, _prompt: &str) -> bool {
            true
        }
    }
    let mut guide = Wobbly { roll, downs: 0, said: Vec::new() };
    let calibration = teach::calibrate(&daemon.socket, &mut guide).await.unwrap();
    feeder.abort();
    assert!(guide.said.iter().any(|s| s.contains("hold still")), "{:?}", guide.said);
    assert_eq!(guide.downs, 2, "palm down again, palm up kept");
    assert!((calibration.down_roll + 140.0).abs() < 0.5, "{calibration:?}");
}

async fn feed_wobbly(transport: SimTransport, roll: Arc<Mutex<(f64, bool)>>) {
    use air_gesturesd::transport::SimInject;
    let mut flip = 1.0;
    loop {
        let (degrees, wobble) = *roll.lock().unwrap();
        flip = -flip;
        let degrees = if wobble { degrees + 30.0 * flip } else { degrees };
        transport.inject(SimInject::Orientation(support::rolled(degrees)));
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_check_that_cant_run_still_keeps_the_calibration() {
    let daemon = support::start(SimMode::Enrolled, true).await;
    daemon.wait("the hand", |s| s.state == LinkState::Connected && s.hand.as_deref() == Some("right")).await;
    let roll = Arc::new(Mutex::new(-140.0));
    let feeder = tokio::spawn(support::feed_roll(daemon.transport.clone(), roll.clone()));
    let (mut guide, _cues) = guide();
    guide.roll = Some(roll);
    guide.close_on_check = Some(daemon.transport.clone());
    let result = teach::calibrate(&daemon.socket, &mut guide).await;
    feeder.abort();
    assert!(result.is_ok(), "{result:?}");
    assert!(guide.said.iter().any(|s| s.starts_with("Saved, but the check couldn't run")), "{:?}", guide.said);
    assert!(daemon.store.load_calibration(SIM_ADDRESS, Hand::Right).unwrap().palm.is_some());
}

#[tokio::test]
async fn resetting_says_what_was_forgotten() {
    let daemon = support::start(SimMode::Enrolled, true).await;
    daemon.wait("the hand", |s| s.state == LinkState::Connected && s.hand.as_deref() == Some("right")).await;
    let palm = air_gesturesd::store::StoredPalm { down_roll: -140.0, up_roll: -10.0 };
    air_gestures::client::request(&daemon.socket, &Request::SetPalm { palm: Some(palm) }).await.unwrap();
    assert_eq!(teach::reset_palm(&daemon.socket).await.unwrap(), "palm calibration forgotten for your right hand");
    assert_eq!(teach::reset_drags(&daemon.socket).await.unwrap(), "taught drags forgotten for your right hand");
    assert!(daemon.store.load_calibration(SIM_ADDRESS, Hand::Right).unwrap().is_empty());
}

#[tokio::test]
async fn a_corrupt_template_counts_as_taken() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("z.json"), "{").unwrap();
    let (mut guide, _cues) = guide();
    let error = teach::teach_drawing(&dir.path().join("no.sock"), dir.path(), "z", &mut guide).await.unwrap_err();
    assert!(error.contains("already taught"), "{error}");
}

#[test]
fn the_terminal_doesnt_wait_for_the_notification() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let stub = dir.path().join("slow-notify");
    std::fs::write(&stub, "#!/bin/sh\nsleep 5\n").unwrap();
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut terminal = teach::Terminal { notify_command: stub.display().to_string() };
    let started = std::time::Instant::now();
    terminal.notify("Draw", "now");
    assert!(started.elapsed() < Duration::from_secs(1), "{:?}", started.elapsed());
}

#[test]
fn the_saved_hint_says_to_put_draw_on_a_gesture() {
    use air_gesturesd::config::Config;
    assert!(teach::draw_hint(&Config::default()).unwrap().contains("[band] draw"));
    assert_eq!(teach::draw_hint(&Config::parse("[band]\ndraw = \"swipe_up\"\n").unwrap()), None);
}

#[test]
fn a_drag_like_the_turn_twice_teaches_the_turn_again() {
    assert_eq!(teach::redo_round(TaughtError::LikeTheTurn(Drag::Up), None), [5]);
    assert_eq!(teach::redo_round(TaughtError::LikeTheTurn(Drag::Up), Some(TaughtError::LikeTheTurn(Drag::Left))), [5, 0]);
    assert_eq!(teach::redo_round(TaughtError::NoTurn, Some(TaughtError::LikeTheTurn(Drag::Left))), [0]);
}
