//! Raw sEMG: recording to JSONL and a live terminal view. Values are unscaled
//! ADC counts (8 channels × 16 samples per batch, 2048 Hz).

use std::collections::VecDeque;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use band_core::events::{EmgBatch, EmgConfig};
use base64::Engine;
use kinesisd::ipc::Request;

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

/// Record frames to `path` (JSONL) until Ctrl-C or `limit`; returns the frame count.
pub async fn record(socket: &Path, path: &Path, limit: Option<Duration>) -> Result<u64, String> {
    let mut file = std::io::BufWriter::new(
        std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?,
    );
    let mut subscription = Subscription::semg(socket).await?;
    client::request(socket, &Request::Semg { on: true }).await?;
    let deadline = limit.map(|limit| tokio::time::Instant::now() + limit);
    let mut frames = 0u64;
    let result = loop {
        let stop = async {
            match deadline {
                Some(deadline) => tokio::time::sleep_until(deadline).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            () = stop => break Ok(()),
            result = tokio::signal::ctrl_c() => break result.map_err(|error| error.to_string()),
            frame = subscription.next_semg() => match frame {
                Some(payload) => {
                    let line = serde_json::json!({
                        "t_ms": now_ms(),
                        "payload": base64::engine::general_purpose::STANDARD.encode(&payload),
                    });
                    if let Err(error) = writeln!(file, "{line}") {
                        break Err(format!("{}: {error}", path.display()));
                    }
                    frames += 1;
                }
                None => break Err("kinesisd went away".to_string()),
            },
        }
    };
    let _ = client::request(socket, &Request::Semg { on: false }).await;
    file.flush()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    result.map(|()| frames)
}

/// The live view (runs until `q`/`Esc`/Ctrl-C).
pub async fn view(socket: &Path) -> Result<(), String> {
    use ratatui::crossterm::event::{self, Event, KeyCode};
    use ratatui::layout::{Constraint, Layout};
    use ratatui::widgets::{Block, Paragraph, Sparkline};

    let mut subscription = Subscription::semg(socket).await?;
    client::request(socket, &Request::Semg { on: true }).await?;
    let mut channels = Channels::new(4096);
    let mut terminal = ratatui::init();
    let started = tokio::time::Instant::now();
    let mut redraw = tokio::time::interval(Duration::from_millis(50));
    let result: Result<(), String> = loop {
        tokio::select! {
            frame = subscription.next_semg() => match frame {
                Some(payload) => { let _ = channels.push(&payload); }
                None => break Err("kinesisd went away".into()),
            },
            _ = redraw.tick() => {
                if event::poll(Duration::ZERO).unwrap_or(false)
                    && let Ok(Event::Key(key)) = event::read()
                    && matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
                {
                    break Ok(());
                }
                let elapsed = started.elapsed().as_secs_f64().max(0.001);
                let draw = terminal.draw(|frame| {
                    let rows = Layout::vertical([Constraint::Length(1); 9]).split(frame.area());
                    let header = format!(
                        "raw sEMG · {:.0} frames/s · {} dropped · q to quit",
                        channels.frames() as f64 / elapsed,
                        channels.dropped()
                    );
                    frame.render_widget(Paragraph::new(header), rows[0]);
                    let width = usize::from(frame.area().width.saturating_sub(6)).max(1);
                    for index in 0..8 {
                        let data: Vec<u64> = channels
                            .channel(index)
                            .iter()
                            .rev()
                            .take(width)
                            .rev()
                            .map(|v| u64::from(*v))
                            .collect();
                        frame.render_widget(
                            Sparkline::default().block(Block::new().title(format!("{index} "))).data(&data),
                            rows[index + 1],
                        );
                    }
                });
                if let Err(error) = draw {
                    break Err(error.to_string());
                }
            }
        }
    };
    ratatui::restore();
    let _ = client::request(socket, &Request::Semg { on: false }).await;
    result
}
