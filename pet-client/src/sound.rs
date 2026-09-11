use std::{
    f32::consts::TAU,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
pub enum Cue {
    Pet,
    Message,
}

pub struct Sounds {
    enabled: Arc<AtomicBool>,
    sender: mpsc::SyncSender<(Cue, Instant)>,
    last_pet: Option<Instant>,
    last_message: Option<Instant>,
}

impl Sounds {
    pub fn new() -> Self {
        let enabled = Arc::new(AtomicBool::new(crate::config::sounds_enabled()));
        let (sender, receiver) = mpsc::sync_channel::<(Cue, Instant)>(2);
        let worker_enabled = enabled.clone();
        std::thread::spawn(move || {
            while let Ok((cue, queued)) = receiver.recv() {
                if !worker_enabled.load(Ordering::Relaxed)
                    || queued.elapsed() > Duration::from_millis(700)
                {
                    continue;
                }
                // Open only while playing: no permanent audio callback when the pet is idle.
                let Ok((_stream, handle)) = rodio::OutputStream::try_default() else {
                    continue;
                };
                let Ok(sink) = rodio::Sink::try_new(&handle) else {
                    continue;
                };
                sink.append(rodio::buffer::SamplesBuffer::new(
                    1,
                    SAMPLE_RATE,
                    samples(cue),
                ));
                while !sink.empty() {
                    if !worker_enabled.load(Ordering::Relaxed) {
                        sink.stop();
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(15));
                }
            }
        });
        Self {
            enabled,
            sender,
            last_pet: None,
            last_message: None,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) -> std::io::Result<()> {
        crate::config::save_sounds_enabled(enabled)?;
        self.enabled.store(enabled, Ordering::Relaxed);
        Ok(())
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn play(&mut self, cue: Cue) {
        if !self.enabled() {
            return;
        }
        let (last, cooldown) = match cue {
            Cue::Pet => (&mut self.last_pet, Duration::from_millis(400)),
            Cue::Message => (&mut self.last_message, Duration::from_millis(1000)),
        };
        let now = Instant::now();
        if last.is_some_and(|last| now.duration_since(last) < cooldown) {
            return;
        }
        if self.sender.try_send((cue, now)).is_ok() {
            *last = Some(now);
        }
    }
}

const SAMPLE_RATE: u32 = 24_000;

fn samples(cue: Cue) -> Vec<f32> {
    let duration = match cue {
        Cue::Pet => 0.22,
        Cue::Message => 0.42,
    };
    (0..(duration * SAMPLE_RATE as f32) as usize)
        .map(|i| {
            let time = i as f32 / SAMPLE_RATE as f32;
            match cue {
                Cue::Pet => note(time, 0.0, 0.22, 360.0, 0.085),
                Cue::Message => {
                    note(time, 0.0, 0.26, 523.25, 0.055) + note(time, 0.14, 0.28, 659.25, 0.045)
                }
            }
        })
        .collect()
}

fn note(time: f32, start: f32, duration: f32, frequency: f32, volume: f32) -> f32 {
    let t = time - start;
    if t <= 0.0 || t >= duration {
        return 0.0;
    }
    let attack = (t / 0.025).min(1.0);
    let release = ((duration - t) / 0.10).min(1.0);
    let envelope = attack
        * attack
        * (3.0 - 2.0 * attack)
        * release
        * release
        * (3.0 - 2.0 * release)
        * (-4.0 * t).exp();
    volume * envelope * ((TAU * frequency * t).sin() + 0.12 * (TAU * frequency * 2.0 * t).sin())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires a real output device and plays audible preview tones"]
    fn preview_both_cues_on_audio_device() {
        let (_stream, handle) = rodio::OutputStream::try_default().expect("audio output device");
        let sink = rodio::Sink::try_new(&handle).unwrap();
        for cue in [Cue::Pet, Cue::Message] {
            sink.append(rodio::buffer::SamplesBuffer::new(
                1,
                SAMPLE_RATE,
                samples(cue),
            ));
            sink.sleep_until_end();
            std::thread::sleep(Duration::from_millis(350));
        }
    }

    #[test]
    fn cues_are_short_quiet_and_have_no_hard_edges() {
        for cue in [Cue::Pet, Cue::Message] {
            let data = samples(cue);
            assert!(data.len() < SAMPLE_RATE as usize / 2);
            assert_eq!(data[0], 0.0);
            assert!(data.last().unwrap().abs() < 0.0001);
            assert!(
                data.iter()
                    .all(|sample| sample.is_finite() && sample.abs() < 0.12)
            );
            assert!(data.windows(2).all(|pair| (pair[1] - pair[0]).abs() < 0.02));
        }
    }
}
