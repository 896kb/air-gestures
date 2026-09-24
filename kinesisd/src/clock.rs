use tokio::time::Instant;

/// Monotonic seconds since the daemon started. Built on tokio's clock so
/// tests can pause and advance it.
#[derive(Debug, Clone, Copy)]
pub struct Clock {
    start: Instant,
}

impl Clock {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }

    pub fn now(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}
