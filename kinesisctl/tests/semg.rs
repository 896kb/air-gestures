mod support;

use std::time::Duration;

use band_core::responder::SimMode;
use kinesisctl::semg::{Channels, record};
use kinesisd::status::LinkState;
use kinesisd::transport::SimInject;

#[test]
fn batches_decode_into_eight_channels_and_gaps_are_counted() {
    let mut channels = Channels::new(64);
    let samples: Vec<u8> = (0..128u16).flat_map(|i| (1000 + i).to_le_bytes()).collect();
    let payload = [
        band_core::proto::field_int(1, 7),
        band_core::proto::field_int(2, 1_000),
        band_core::proto::field_bytes(3, &samples),
    ]
    .concat();
    channels.push(&payload).unwrap();
    assert_eq!(channels.channel(0)[..2], [1000, 1008]);
    assert_eq!(channels.channel(7)[0], 1007);
    let next = [
        band_core::proto::field_int(1, 10),
        band_core::proto::field_int(2, 2_000),
        band_core::proto::field_bytes(3, &samples),
    ]
    .concat();
    channels.push(&next).unwrap();
    assert_eq!(channels.dropped(), 2, "sequence 8 and 9 never arrived");
    assert!(channels.push(&[1, 2, 3]).is_err());
}

#[tokio::test]
async fn record_writes_one_line_per_frame() {
    let daemon = support::start(SimMode::Enrolled, true).await;
    daemon
        .wait("connected", |s| s.state == LinkState::Connected)
        .await;
    let file = daemon.dir.path().join("rec.jsonl");
    let socket = daemon.socket.clone();
    let path = file.clone();
    let recording =
        tokio::spawn(async move { record(&socket, &path, Some(Duration::from_secs(3))).await });
    daemon.wait("sEMG on", |s| s.semg).await;
    for _ in 0..5 {
        daemon.transport.inject(SimInject::RawEmg);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let frames = recording.await.unwrap().unwrap();
    assert_eq!(frames, 5);
    let text = std::fs::read_to_string(&file).unwrap();
    assert_eq!(text.lines().count(), 5);
    let first: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert!(first["t_ms"].as_u64().unwrap() > 0 && first["payload"].as_str().unwrap().len() > 300);
    daemon.wait("sEMG off after recording", |s| !s.semg).await;
}
