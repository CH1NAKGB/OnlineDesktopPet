use std::f32::consts::TAU;

#[derive(Default)]
pub struct Animation {
    last_time: Option<f32>,
    affection: f32,
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
}
