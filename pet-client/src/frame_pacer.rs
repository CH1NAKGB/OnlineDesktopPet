use std::time::{Duration, Instant};

/// Separates animation time from wake-up cadence so an idle pet does not waste GPU work.
pub struct FramePacer {
    active_until: Instant,
    idle_interval: Duration,
}

impl FramePacer {
    pub fn new() -> Self {
        Self {
            active_until: Instant::now(),
            idle_interval: Duration::from_millis(50),
        }
    }
    pub fn activate(&mut self) {
        self.active_until = Instant::now() + Duration::from_secs(3);
    }
    pub fn next_wake(&self) -> Option<Instant> {
        if Instant::now() < self.active_until {
            None
        } else {
            Some(Instant::now() + self.idle_interval)
        }
    }
}
