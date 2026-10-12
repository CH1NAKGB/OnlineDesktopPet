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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SoundTheme {
    #[default]
    Soft,
    Chirp,
    Bell,
}

impl SoundTheme {
    pub const ALL: [Self; 3] = [Self::Soft, Self::Chirp, Self::Bell];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Soft => "Soft",
            Self::Chirp => "Chirp",
            Self::Bell => "Bell",
        }
    }

    pub(crate) const fn setting(self) -> &'static str {
        match self {
            Self::Soft => "soft",
            Self::Chirp => "chirp",
            Self::Bell => "bell",
        }
    }

    pub(crate) fn from_setting(value: &str) -> Self {
        match value.trim() {
            "chirp" => Self::Chirp,
            "bell" => Self::Bell,
            _ => Self::Soft,
        }
    }
}

pub struct Sounds {
    enabled: Arc<AtomicBool>,
    theme: SoundTheme,
    sender: mpsc::SyncSender<(Cue, SoundTheme, Instant)>,
    last_pet: Option<Instant>,
    last_message: Option<Instant>,
}

impl Sounds {
    pub fn new() -> Self {
        let enabled = Arc::new(AtomicBool::new(crate::config::sounds_enabled()));
        let theme = crate::config::sound_theme();
        let (sender, receiver) = mpsc::sync_channel::<(Cue, SoundTheme, Instant)>(2);
        let worker_enabled = enabled.clone();
        std::thread::spawn(move || {
            while let Ok((cue, theme, queued)) = receiver.recv() {
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
                    samples(cue, theme),
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
            theme,
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

    pub fn theme(&self) -> SoundTheme {
        self.theme
    }

    /// Persist first so a failed save leaves the current theme unchanged.
    pub fn set_theme(&mut self, theme: SoundTheme) -> std::io::Result<()> {
        crate::config::save_sound_theme(theme)?;
        self.theme = theme;
        Ok(())
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
        if self.sender.try_send((cue, self.theme, now)).is_ok() {
            *last = Some(now);
        }
    }
}

const SAMPLE_RATE: u32 = 24_000;

fn samples(cue: Cue, theme: SoundTheme) -> Vec<f32> {
    let duration = match cue {
        Cue::Pet => 0.22,
        Cue::Message => 0.42,
    };
    (0..(duration * SAMPLE_RATE as f32) as usize)
        .map(|i| {
            let time = i as f32 / SAMPLE_RATE as f32;
            match (theme, cue) {
                (SoundTheme::Soft, Cue::Pet) => note(time, 0.0, 0.22, 360.0, 0.085),
                (SoundTheme::Soft, Cue::Message) => {
                    note(time, 0.0, 0.26, 523.25, 0.055) + note(time, 0.14, 0.28, 659.25, 0.045)
                }
                (SoundTheme::Chirp, Cue::Pet) => chirp(time, 0.0, 0.22, 440.0, 660.0, 0.065),
                (SoundTheme::Chirp, Cue::Message) => {
                    chirp(time, 0.0, 0.19, 520.0, 740.0, 0.05)
                        + chirp(time, 0.21, 0.21, 620.0, 880.0, 0.045)
                }
                (SoundTheme::Bell, Cue::Pet) => bell(time, 0.0, 0.22, 480.0, 0.065),
                (SoundTheme::Bell, Cue::Message) => {
                    bell(time, 0.0, 0.30, 600.0, 0.045) + bell(time, 0.12, 0.30, 800.0, 0.035)
                }
            }
        })
        .collect()
}

fn note(time: f32, start: f32, duration: f32, frequency: f32, volume: f32) -> f32 {
    let t = time - start;
    volume
        * envelope(t, duration)
        * ((TAU * frequency * t).sin() + 0.12 * (TAU * frequency * 2.0 * t).sin())
}

fn chirp(time: f32, start: f32, duration: f32, from: f32, to: f32, volume: f32) -> f32 {
    let t = time - start;
    // Integrate the pitch sweep to keep the oscillator's phase continuous.
    let phase = TAU * (from * t + (to - from) * t * t / (2.0 * duration));
    volume * envelope(t, duration) * phase.sin()
}

fn bell(time: f32, start: f32, duration: f32, frequency: f32, volume: f32) -> f32 {
    let t = time - start;
    let phase = TAU * frequency * t;
    volume
        * envelope(t, duration)
        * (phase.sin()
            + 0.18 * (-8.0 * t).exp() * (2.76 * phase).sin()
            + 0.07 * (-12.0 * t).exp() * (4.07 * phase).sin())
}

fn envelope(t: f32, duration: f32) -> f32 {
    if t <= 0.0 || t >= duration {
        return 0.0;
    }
    let attack = (t / 0.025).min(1.0);
    let release = ((duration - t) / 0.10).min(1.0);
    attack
        * attack
        * (3.0 - 2.0 * attack)
        * release
        * release
        * (3.0 - 2.0 * release)
        * (-4.0 * t).exp()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires a real output device and plays audible preview tones"]
    fn preview_all_themes_on_audio_device() {
        let (_stream, handle) = rodio::OutputStream::try_default().expect("audio output device");
        let sink = rodio::Sink::try_new(&handle).unwrap();
        for (theme, cue) in SoundTheme::ALL
            .into_iter()
            .flat_map(|theme| [Cue::Pet, Cue::Message].map(|cue| (theme, cue)))
        {
            sink.append(rodio::buffer::SamplesBuffer::new(
                1,
                SAMPLE_RATE,
                samples(cue, theme),
            ));
            sink.sleep_until_end();
            std::thread::sleep(Duration::from_millis(350));
        }
    }

    #[test]
    fn cues_are_short_quiet_and_have_no_hard_edges() {
        for (theme, cue) in SoundTheme::ALL
            .into_iter()
            .flat_map(|theme| [Cue::Pet, Cue::Message].map(|cue| (theme, cue)))
        {
            let data = samples(cue, theme);
            assert!(data.len() < SAMPLE_RATE as usize / 2);
            assert_eq!(data[0], 0.0);
            assert!(data.last().unwrap().abs() < 0.0001);
            assert!(
                data.iter()
                    .all(|sample| sample.is_finite() && sample.abs() < 0.12)
            );
            assert!(data.windows(2).all(|pair| (pair[1] - pair[0]).abs() < 0.02));
            let mean_square =
                data.iter().map(|sample| sample * sample).sum::<f32>() / data.len() as f32;
            assert!(mean_square > 0.00001, "{theme:?} must be audible");
        }
    }

    #[test]
    fn themes_and_cues_are_distinct() {
        assert_eq!(SoundTheme::default(), SoundTheme::Soft);
        assert_eq!(
            SoundTheme::ALL.map(SoundTheme::label),
            ["Soft", "Chirp", "Bell"]
        );
        for (index, theme) in SoundTheme::ALL.into_iter().enumerate() {
            let pet = samples(Cue::Pet, theme);
            let message = samples(Cue::Message, theme);
            assert_ne!(pet, message[..pet.len()]);
            for other in &SoundTheme::ALL[index + 1..] {
                for cue in [Cue::Pet, Cue::Message] {
                    let difference = samples(cue, theme)
                        .iter()
                        .zip(samples(cue, *other))
                        .map(|(a, b)| (a - b).powi(2))
                        .sum::<f32>();
                    assert!(
                        difference > 0.1,
                        "{theme:?} and {other:?} sound too similar"
                    );
                }
            }
        }
    }

    #[test]
    fn playback_queues_selected_theme_and_preserves_mute_and_cooldowns() {
        for theme in SoundTheme::ALL {
            let (sender, receiver) = mpsc::sync_channel(2);
            let mut sounds = Sounds {
                enabled: Arc::new(AtomicBool::new(false)),
                theme,
                sender,
                last_pet: None,
                last_message: None,
            };
            assert_eq!(sounds.theme(), theme);
            sounds.play(Cue::Pet);
            assert!(receiver.try_recv().is_err());
            assert!(sounds.last_pet.is_none());
            sounds.enabled.store(true, Ordering::Relaxed);
            for cue in [Cue::Pet, Cue::Message] {
                sounds.play(cue);
                let (queued_cue, queued_theme, _) = receiver.try_recv().unwrap();
                assert_eq!(
                    std::mem::discriminant(&queued_cue),
                    std::mem::discriminant(&cue)
                );
                assert_eq!(queued_theme, theme);
                sounds.play(cue);
                assert!(receiver.try_recv().is_err());
            }
        }
    }
}
