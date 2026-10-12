use std::f32::consts::TAU;

#[derive(Default)]
pub struct Animation {
    last_time: Option<f32>,
    affection: f32,
}

/// Low-frequency, bounded wandering for the desktop pet.
pub struct Wander {
    rng: u64,
    target_x: Option<f64>,
    next_action: std::time::Instant,
    last_motion: std::time::Instant,
    next_move: std::time::Instant,
    next_frame: std::time::Instant,
    speed: f64,
    frame: u8,
}

impl Wander {
    pub fn new(now: std::time::Instant) -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64
            ^ 0x9e37_79b9_7f4a_7c15;
        let mut wander = Self {
            rng: seed.max(1),
            target_x: None,
            next_action: now + std::time::Duration::from_secs(5),
            last_motion: now,
            next_move: now,
            next_frame: now,
            speed: 0.0,
            frame: 0,
        };
        wander.schedule_next(now);
        wander
    }

    pub fn frame(&self) -> Option<u8> {
        self.target_x.map(|_| self.frame)
    }

    /// Advances at most one low-rate walk step and returns a new physical X position if needed.
    pub fn advance(
        &mut self,
        now: std::time::Instant,
        current_x: i32,
        screen_left: i32,
        screen_width: u32,
        window_width: u32,
        scale: f64,
        paused: bool,
    ) -> Option<i32> {
        if paused {
            if self.target_x.take().is_some() {
                self.schedule_next(now);
            }
            return None;
        }

        let margin = (12.0 * scale).round() as i32;
        let min_x = screen_left.saturating_add(margin);
        let max_x = screen_left
            .saturating_add(screen_width.saturating_sub(window_width) as i32)
            .saturating_sub(margin)
            .max(min_x);
        let current_x = current_x.clamp(min_x, max_x);

        if self.target_x.is_none() {
            if now < self.next_action || max_x - min_x < (72.0 * scale) as i32 {
                return None;
            }
            let distance = self.random_between(72.0, 190.0) * scale;
            let direction = if self.next_random() & 1 == 0 {
                -1.0
            } else {
                1.0
            };
            let mut target = (current_x as f64 + direction * distance).round() as i32;
            if target < min_x + (36.0 * scale) as i32 || target > max_x - (36.0 * scale) as i32 {
                target = (current_x as f64 - direction * distance).round() as i32;
            }
            target = target.clamp(min_x, max_x);
            if target == current_x {
                self.schedule_next(now);
                return None;
            }
            self.target_x = Some(target as f64);
            self.speed = self.random_between(52.0, 76.0) * scale;
            self.last_motion = now;
            self.next_move = now;
            self.next_frame = now;
            self.frame = 0;
        }

        if now >= self.next_frame {
            self.frame = (self.frame + 1) % 4;
            self.next_frame = now + std::time::Duration::from_millis(125);
        }
        if now < self.next_move {
            return None;
        }
        self.next_move = now + std::time::Duration::from_millis(33);

        let target = self.target_x?;
        let dt = now.duration_since(self.last_motion).as_secs_f64();
        self.last_motion = now;
        let delta = target - current_x as f64;
        let step = self.speed * dt;
        if delta.abs() <= step.max(1.0) {
            self.target_x = None;
            self.schedule_next(now);
            return Some(target.round() as i32);
        }
        Some((current_x as f64 + delta.signum() * step).round() as i32)
    }

    fn schedule_next(&mut self, now: std::time::Instant) {
        self.next_action =
            now + std::time::Duration::from_secs(self.random_between(7.0, 18.0) as u64);
    }

    fn random_between(&mut self, min: f64, max: f64) -> f64 {
        let unit = (self.next_random() >> 11) as f64 / ((1_u64 << 53) as f64);
        min + (max - min) * unit
    }

    fn next_random(&mut self) -> u64 {
        let mut value = self.rng;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.rng = value;
        value
    }
}

impl Animation {
    /// Evaluate once per frame for both the GPU and layered-window renderers.
    pub fn sample(&mut self, seconds: f32, petting: f32) -> [f32; 4] {
        let dt = self.last_time.map_or(0.0, |last| (seconds - last).max(0.0));
        self.last_time = Some(seconds);
        let target = petting.clamp(0.0, 1.0);
        let response = if target > self.affection { 7.0 } else { 3.5 };
        self.affection += (target - self.affection) * (1.0 - (-response * dt).exp());
        let breath = (seconds * TAU / 3.6).sin();
        let delight = (seconds * TAU / 0.85).sin();
        let lift = self.affection * (0.014 + delight * 0.009);
        let stretch = 1.0 + breath * 0.012 + self.affection * delight * 0.018;
        // Uneven spacing and an occasional double blink avoid a metronomic idle.
        let phase = seconds.rem_euclid(17.0);
        let blink = [2.4, 6.9, 7.25, 12.8]
            .into_iter()
            .map(|center| {
                let distance = ((phase - center) / 0.13_f32).abs().min(1.0);
                distance * distance * (3.0 - 2.0 * distance)
            })
            .fold(1.0_f32, f32::min);
        let eye = (1.0 - self.affection * 0.65) * blink;
        [
            lift,
            stretch,
            eye.max(0.12),
            self.affection * delight * 0.012,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn affection_is_independent_of_refresh_rate() {
        fn run(hz: u32) -> [f32; 4] {
            let mut animation = Animation::default();
            animation.sample(0.0, 0.0);
            let mut pose = [0.0; 4];
            for frame in 1..=hz {
                pose = animation.sample(frame as f32 / hz as f32, 1.0);
            }
            pose
        }
        for (a, b) in run(60).into_iter().zip(run(120)) {
            assert!((a - b).abs() < 0.00001);
        }
    }

    #[test]
    fn input_edges_do_not_snap_and_motion_settles() {
        let mut animation = Animation::default();
        let initial = animation.sample(0.0, 0.0);
        assert_eq!(initial, animation.sample(0.0, 1.0));
        animation.sample(1.0, 1.0);
        let settled = animation.sample(10.0, 0.0);
        assert!(settled[0].abs() < 0.00001);
        assert!(settled.iter().all(|value| value.is_finite()));
    }

    #[test]
    fn wander_stays_inside_screen_and_pauses_cleanly() {
        use std::time::{Duration, Instant};

        let start = Instant::now();
        let mut wander = Wander::new(start);
        let mut x = 500;
        let mut moved = false;
        for tick in 0..4000 {
            let now = start + Duration::from_millis(tick * 10);
            if let Some(next_x) = wander.advance(now, x, 0, 1000, 150, 1.0, false) {
                x = next_x;
                moved = true;
            }
            assert!((12..=838).contains(&x));
        }
        assert!(moved);

        let now = start + Duration::from_secs(50);
        wander.advance(now, x, 0, 1000, 150, 1.0, true);
        assert!(wander.frame().is_none());
    }
}
