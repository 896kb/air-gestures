mod support;

use std::time::Duration;

use band_core::responder::SimMode;
use kinesisctl::semg::{Channels, decimate, draw, record, view};
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

#[tokio::test]
async fn record_refuses_to_start_when_the_band_isnt_connected() {
    let daemon = support::start(SimMode::Legacy, false).await;
    daemon
        .wait("the unpaired error", |s| s.error.is_some())
        .await;
    let file = daemon.dir.path().join("rec.jsonl");
    let error = record(&daemon.socket, &file, Some(Duration::from_millis(50)))
        .await
        .unwrap_err();
    assert!(error.contains("connect the band first"), "{error}");
    assert!(!file.exists(), "no file should be created on refusal");
}

#[tokio::test]
async fn view_refuses_to_start_when_the_band_isnt_connected() {
    let daemon = support::start(SimMode::Legacy, false).await;
    daemon
        .wait("the unpaired error", |s| s.error.is_some())
        .await;
    // Never reaches `ratatui::try_init` (the connected check comes first),
    // so this is safe to run without a real terminal.
    let error = view(&daemon.socket).await.unwrap_err();
    assert!(error.contains("connect the band first"), "{error}");
}

#[test]
fn decimation_removes_the_dc_offset_and_shows_where_a_burst_is() {
    let flat = vec![2000u16; 100];
    assert!(
        decimate(&flat, 10).iter().all(|&v| v == 0),
        "a flat signal has no peak-to-peak activity anywhere"
    );

    let mut burst = vec![2000u16; 100];
    for (i, sample) in burst[40..50].iter_mut().enumerate() {
        // Oscillating, not just a different constant level: a *constant*
        // burst has zero peak-to-peak too, same as the flat baseline: it's
        // the swing that decimate is meant to surface.
        *sample = if i % 2 == 0 { 2000 } else { 3000 };
    }
    // 100 samples over a width of 10 makes exactly 10 chunks of 10 samples
    // each; chunk 4 (samples 40..50) is exactly the burst.
    let decimated = decimate(&burst, 10);
    assert_eq!(decimated.len(), 10);
    for (index, value) in decimated.iter().enumerate() {
        if index == 4 {
            assert!(*value > 0, "the burst's chunk should show activity");
        } else {
            assert_eq!(*value, 0, "chunk {index} outside the burst should be flat");
        }
    }
}

#[test]
fn view_draws_a_sparkline_in_every_channels_row() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::{Constraint, Layout, Rect};

    let mut channels = Channels::new(64);
    for i in 0..20u16 {
        let samples: Vec<u8> = (0..128u16)
            .map(|j| i.wrapping_mul(37).wrapping_add(j.wrapping_mul(13)) % 4000)
            .flat_map(u16::to_le_bytes)
            .collect();
        let payload = [
            band_core::proto::field_int(1, u64::from(i)),
            band_core::proto::field_int(2, 1_000),
            band_core::proto::field_bytes(3, &samples),
        ]
        .concat();
        channels.push(&payload).unwrap();
    }

    let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
    terminal
        .draw(|frame| draw(frame, &channels, "raw sEMG"))
        .unwrap();
    let buffer = terminal.backend().buffer();

    // The same row split `draw` uses, to check each channel's own area (not
    // just some arbitrary terminal line, which a layout tweak could shift).
    let mut constraints = vec![Constraint::Length(1)];
    constraints.extend(std::iter::repeat_n(Constraint::Ratio(1, 8), 8));
    let rows = Layout::vertical(constraints).split(Rect::new(0, 0, 80, 20));

    let bars = "▁▂▃▄▅▆▇█";
    for index in 0..8usize {
        let row = rows[index + 1];
        let has_bar = (row.left()..row.right()).any(|x| {
            (row.top()..row.bottom()).any(|y| {
                buffer
                    .cell((x, y))
                    .is_some_and(|cell| bars.contains(cell.symbol()))
            })
        });
        assert!(has_bar, "channel {index}'s row has no sparkline glyphs");
    }
}
