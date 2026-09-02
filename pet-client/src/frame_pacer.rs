use std::time::{Duration, Instant};

/// Separates animation time from wake-up cadence so an idle pet does not waste GPU work.
pub struct FramePacer {
    active_until: Instant,
    next_frame_at: Instant,
    active_interval: Duration,
    idle_interval: Duration,
}

impl FramePacer {
    pub fn new() -> Self {
        Self {
            active_until: Instant::now(),
            next_frame_at: Instant::now(),
            active_interval: Duration::from_nanos(16_666_667),
            idle_interval: Duration::from_millis(50),
        }
    }

    pub fn set_refresh_rate(&mut self, refresh_rate_millihertz: Option<u32>) {
        let refresh_rate = refresh_rate_millihertz
            .unwrap_or(60_000)
            .clamp(30_000, 240_000);
        self.active_interval = Duration::from_nanos(1_000_000_000_000 / refresh_rate as u64);
    }

    pub fn activate(&mut self) {
        let now = Instant::now();
        self.active_until = now + Duration::from_secs(3);
        self.next_frame_at = now;
    }

    pub fn schedule(&mut self) -> (bool, Instant) {
        let now = Instant::now();
        if now >= self.next_frame_at {
            let interval = if now < self.active_until {
                self.active_interval
            } else {
                self.idle_interval
            };
            self.next_frame_at = now + interval;
            (true, self.next_frame_at)
        } else {
            (false, self.next_frame_at)
        }
    }
}
