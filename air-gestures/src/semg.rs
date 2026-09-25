//! Raw sEMG: recording to JSONL and a live terminal view. Values are unscaled
//! ADC counts (8 channels × 16 samples per batch, 2048 Hz).

use std::collections::VecDeque;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use air_gesturesd::ipc::Request;
use air_gesturesd::status::LinkState;
use band_core::events::{EmgBatch, EmgConfig};
use base64::Engine;

use crate::client::{self, Subscription};

fn config() -> EmgConfig {
    EmgConfig {
        sample_rate: 2048,
        channels: 8,
        adc_bits: 16,
        samples_per_batch: 16,
        encoding: 0,
    }
}

/// Rolling per-channel sample buffers plus dropped-batch accounting.
pub struct Channels {
    capacity: usize,
    data: Vec<VecDeque<u16>>,
    last_sequence: Option<u64>,
    dropped: u64,
    frames: u64,
}

impl Channels {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            data: vec![VecDeque::with_capacity(capacity); 8],
            last_sequence: None,
            dropped: 0,
            frames: 0,
        }
    }

    pub fn push(&mut self, payload: &[u8]) -> Result<(), String> {
        let batch = EmgBatch::parse(payload, &config()).map_err(|error| error.to_string())?;
        if let Some(last) = self.last_sequence
            && batch.sequence > last + 1
        {
            self.dropped += batch.sequence - last - 1;
        }
        self.last_sequence = Some(batch.sequence);
        self.frames += 1;
        for (index, value) in batch.values.iter().enumerate() {
            let channel = &mut self.data[index % 8];
            if channel.len() == self.capacity {
                channel.pop_front();
            }
            channel.push_back(*value);
        }
        Ok(())
    }

    pub fn channel(&self, index: usize) -> Vec<u16> {
        self.data[index].iter().copied().collect()
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Record frames to `path` until Ctrl-C or `limit`; returns the frame count.
///
/// `path` is written as JSONL, one object per raw sEMG frame:
/// `{"t_ms": <host receive time, unix ms>, "payload": "<base64>"}`, where
/// `payload` is the raw sEMG batch protobuf as the band sent it (field 1: the
/// batch sequence number, field 2: the band's own `timestamp_us`, field 3:
/// 256 bytes = 16 samples × 8 channels, little-endian `u16`, interleaved by
/// sample then channel — see [`EmgBatch::parse`]).
pub async fn record(socket: &Path, path: &Path, limit: Option<Duration>) -> Result<u64, String> {
    let status = client::status(socket).await?;
    if status.state != LinkState::Connected {
        return Err("connect the band first".into());
    }
    let mut subscription = Subscription::semg(socket).await?;
    // Only turn sEMG on (and only then create the output file) once we know
    // the daemon accepted it: an error here must leave nothing behind.
    client::request(socket, &Request::Semg { on: true }).await?;
    let outcome = record_frames(path, limit, &mut subscription).await;
    let _ = client::request(socket, &Request::Semg { on: false }).await;
    outcome
}

async fn record_frames(
    path: &Path,
    limit: Option<Duration>,
    subscription: &mut Subscription,
) -> Result<u64, String> {
    let mut file = std::io::BufWriter::new(
        std::fs::File::create(path).map_err(|error| format!("{}: {error}", path.display()))?,
    );
    let deadline = limit.map(|limit| tokio::time::Instant::now() + limit);
    // Created once and pinned so the same `ctrl_c` registration is polled
    // (and, once fired, kept satisfied) across every loop iteration.
    let mut ctrl_c = Box::pin(tokio::signal::ctrl_c());
    let mut frames = 0u64;
    loop {
        let stop = async {
            match deadline {
                Some(deadline) => tokio::time::sleep_until(deadline).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            () = stop => break Ok(frames),
            result = &mut ctrl_c => break result.map(|()| frames).map_err(|error| error.to_string()),
            frame = subscription.next_semg() => match frame {
                Some(payload) => {
                    let line = serde_json::json!({
                        "t_ms": now_ms(),
                        "payload": base64::engine::general_purpose::STANDARD.encode(&payload),
                    });
                    if let Err(error) = writeln!(file, "{line}") {
                        break Err(format!(
                            "{}: {error} ({frames} frames written)",
                            path.display()
                        ));
                    }
                    frames += 1;
                }
                None => break Err(format!("air-gesturesd went away ({frames} frames written)")),
            },
        }
    }
    .and_then(|frames| {
        file.flush()
            .map(|()| frames)
            .map_err(|error| format!("{}: {error} ({frames} frames written)", path.display()))
    })
}

/// Decimate `samples` to at most `width` points, each the peak-to-peak
/// (max − min) of its chunk. This throws away the ADC's DC offset and shows
/// where the signal is actually active, rather than just its raw level.
pub fn decimate(samples: &[u16], width: usize) -> Vec<u64> {
    if width == 0 {
        return Vec::new();
    }
    if samples.len() < 2 {
        return vec![0; width];
    }
    // At least 2 samples per chunk: a lone sample's "peak-to-peak" is always
    // zero, which would flatten the whole display whenever there are fewer
    // samples than columns (e.g. the first couple of seconds after `view`
    // starts, before the rolling buffer has filled up).
    let chunk = samples.len().div_ceil(width).max(2);
    samples
        .chunks(chunk)
        .map(|chunk| {
            let min = chunk.iter().copied().min().unwrap_or(0);
            let max = chunk.iter().copied().max().unwrap_or(0);
            u64::from(max - min)
        })
        .collect()
}

/// Render one frame: a one-line header, then 8 channel rows (a short label
/// plus that channel's sparkline), each getting an equal share of the rest of
/// the screen. A `Sparkline` needs at least one full line for its bars, so
/// each channel's label and sparkline sit side by side rather than a label
/// consuming its channel's only line (as a `Block` title would).
pub fn draw(frame: &mut ratatui::Frame, channels: &Channels, header: &str) {
    use ratatui::layout::{Constraint, Layout};
    use ratatui::widgets::{Paragraph, Sparkline};

    let mut constraints = vec![Constraint::Length(1)];
    constraints.extend(std::iter::repeat_n(Constraint::Ratio(1, 8), 8));
    let rows = Layout::vertical(constraints).split(frame.area());
    frame.render_widget(Paragraph::new(header.to_string()), rows[0]);
    for index in 0..8 {
        let columns =
            Layout::horizontal([Constraint::Length(3), Constraint::Min(1)]).split(rows[index + 1]);
        frame.render_widget(Paragraph::new(format!("{index:>2}")), columns[0]);
        let width = usize::from(columns[1].width).max(1);
        let data = decimate(&channels.channel(index), width);
        frame.render_widget(Sparkline::default().data(&data), columns[1]);
    }
}

/// The live view (runs until `q`/`Esc`/Ctrl-C quits it).
pub async fn view(socket: &Path) -> Result<(), String> {
    use ratatui::crossterm::event::{self, Event, KeyCode, KeyModifiers};

    let status = client::status(socket).await?;
    if status.state != LinkState::Connected {
        return Err("connect the band first".into());
    }
    let mut subscription = Subscription::semg(socket).await?;
    client::request(socket, &Request::Semg { on: true }).await?;
    let mut channels = Channels::new(4096);
    let mut terminal = match ratatui::try_init() {
        Ok(terminal) => terminal,
        Err(error) => {
            let _ = client::request(socket, &Request::Semg { on: false }).await;
            return Err(format!("could not use this terminal: {error}"));
        }
    };
    let started = tokio::time::Instant::now();
    let mut redraw = tokio::time::interval(Duration::from_millis(50));
    let result: Result<(), String> = loop {
        tokio::select! {
            frame = subscription.next_semg() => match frame {
                Some(payload) => { let _ = channels.push(&payload); }
                None => break Err("air-gesturesd went away".into()),
            },
            _ = redraw.tick() => {
                if event::poll(Duration::ZERO).unwrap_or(false)
                    && let Ok(Event::Key(key)) = event::read()
                    && (matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
                        || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)))
                {
                    break Ok(());
                }
                let elapsed = started.elapsed().as_secs_f64().max(0.001);
                let header = format!(
                    "raw sEMG · {:.0} frames/s · {} dropped · q to quit",
                    channels.frames() as f64 / elapsed,
                    channels.dropped()
                );
                if let Err(error) = terminal.draw(|frame| draw(frame, &channels, &header)) {
                    break Err(error.to_string());
                }
            }
        }
    };
    ratatui::restore();
    let _ = client::request(socket, &Request::Semg { on: false }).await;
    result
}
