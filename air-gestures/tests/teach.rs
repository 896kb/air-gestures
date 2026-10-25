mod support;

use air_gestures::teach::{self, Guide};
use air_gesturesd::status::LinkState;
use std::sync::{Arc, Mutex};

use air_gesturesd::transport::SIM_ADDRESS;
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
    fn ready(&mut self, _prompt: &str) -> bool {
        true
    }
}

fn guide() -> (Recorded, mpsc::UnboundedReceiver<String>) {
    let (notified, cues) = mpsc::unbounded_channel();
    (Recorded { said: Vec::new(), notified, roll: None }, cues)
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
