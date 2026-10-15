//! Persist physical window coordinates with the scale at which they were saved.

use std::{
    collections::VecDeque,
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use winit::{dpi::PhysicalPosition, event_loop::ActiveEventLoop};

const SAVE_DELAY: Duration = Duration::from_millis(350);
const WANDER_EVENT_TTL: Duration = Duration::from_secs(1);

#[derive(Clone, Copy)]
pub enum WindowKind {
    Pet,
    Chat,
}

impl WindowKind {
    fn filename(self) -> &'static str {
        match self {
            Self::Pet => "pet-window-position.txt",
            Self::Chat => "chat-window-position.txt",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SavedPosition {
    physical: PhysicalPosition<i32>,
    scale: f64,
}

#[derive(Clone, Copy)]
struct MonitorGeometry {
    origin: PhysicalPosition<i32>,
    size: (i32, i32),
    scale: f64,
}

impl MonitorGeometry {
    fn contains_logical(self, point: (f64, f64)) -> bool {
        let left = self.origin.x as f64 / self.scale;
        let top = self.origin.y as f64 / self.scale;
        point.0 >= left
            && point.0 < left + self.size.0 as f64 / self.scale
            && point.1 >= top
            && point.1 < top + self.size.1 as f64 / self.scale
    }
}

pub struct PositionRecorder {
    kind: WindowKind,
    root: Option<PathBuf>,
    pending: Option<(SavedPosition, Instant)>,
    wander_moves: VecDeque<(PhysicalPosition<i32>, Instant)>,
}

impl PositionRecorder {
    pub fn new(kind: WindowKind) -> Self {
        Self::with_root(kind, crate::config::app_data_root())
    }

    fn with_root(kind: WindowKind, root: Option<PathBuf>) -> Self {
        Self {
            kind,
            root,
            pending: None,
            wander_moves: VecDeque::new(),
        }
    }

    /// A Moved event caused by our own wandering must not replace the user's chosen position.
    pub fn note_wander_move(&mut self, position: PhysicalPosition<i32>) {
        self.wander_moves
            .push_back((position, Instant::now() + WANDER_EVENT_TTL));
        while self.wander_moves.len() > 32 {
            self.wander_moves.pop_front();
        }
    }

    pub fn moved(&mut self, position: PhysicalPosition<i32>, scale: f64) {
        let now = Instant::now();
        while self
            .wander_moves
            .front()
            .is_some_and(|(_, deadline)| *deadline < now)
        {
            self.wander_moves.pop_front();
        }
        if let Some(index) = self.wander_moves.iter().position(|(p, _)| *p == position) {
            self.wander_moves.drain(..=index);
            return;
        }
        self.wander_moves.clear();
        if valid_scale(scale) {
            self.pending = Some((
                SavedPosition {
                    physical: position,
                    scale,
                },
                now + SAVE_DELAY,
            ));
        }
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.pending.map(|(_, deadline)| deadline)
    }

    pub fn flush_if_due(&mut self, now: Instant) {
        if self.pending.is_some_and(|(_, deadline)| deadline <= now) {
            self.flush();
        }
    }

    pub fn flush(&mut self) {
        if let (Some(root), Some((position, _))) = (self.root.as_ref(), self.pending.take()) {
            if let Err(error) = save_at(root, self.kind, position) {
                eprintln!("window position save failed: {error}");
            }
        }
    }
}

impl Drop for PositionRecorder {
    fn drop(&mut self) {
        self.flush();
    }
}

fn valid_scale(scale: f64) -> bool {
    scale.is_finite() && scale > 0.0 && scale <= 16.0
}

fn parse(text: &str) -> Option<SavedPosition> {
    let mut parts = text.trim().split(',');
    let x = parts.next()?.parse().ok()?;
    let y = parts.next()?.parse().ok()?;
    let scale = parts.next().map_or(Some(1.0), |s| s.parse().ok())?;
    if parts.next().is_some() || !valid_scale(scale) {
        return None;
    }
    Some(SavedPosition {
        physical: PhysicalPosition::new(x, y),
        scale,
    })
}

fn read_at(root: &Path, kind: WindowKind) -> Option<SavedPosition> {
    parse(&fs::read_to_string(root.join(kind.filename())).ok()?)
}

pub fn restore(
    event_loop: &ActiveEventLoop,
    kind: WindowKind,
    logical_size: (f64, f64),
) -> Option<PhysicalPosition<i32>> {
    let saved = read_at(crate::config::app_data_root()?.as_path(), kind)?;
    let geometry = |monitor: &winit::monitor::MonitorHandle| MonitorGeometry {
        origin: monitor.position(),
        size: (monitor.size().width as i32, monitor.size().height as i32),
        scale: monitor.scale_factor(),
    };
    let monitors: Vec<_> = event_loop
        .available_monitors()
        .map(|m| geometry(&m))
        .collect();
    let primary = event_loop.primary_monitor().map(|m| geometry(&m));
    restore_position(saved, &monitors, primary, logical_size)
}

fn restore_position(
    saved: SavedPosition,
    monitors: &[MonitorGeometry],
    primary: Option<MonitorGeometry>,
    logical_size: (f64, f64),
) -> Option<PhysicalPosition<i32>> {
    let logical = (
        saved.physical.x as f64 / saved.scale,
        saved.physical.y as f64 / saved.scale,
    );
    let monitor = monitors
        .iter()
        .copied()
        .find(|m| valid_scale(m.scale) && m.contains_logical(logical))
        .or(primary)
        .or_else(|| monitors.first().copied())?;
    if !valid_scale(monitor.scale) {
        return None;
    }
    let physical = PhysicalPosition::new(
        (logical.0 * monitor.scale).round() as i32,
        (logical.1 * monitor.scale).round() as i32,
    );
    let width = (logical_size.0 * monitor.scale).round() as i32;
    let height = (logical_size.1 * monitor.scale).round() as i32;
    Some(clamp(
        physical,
        monitor.origin,
        monitor.size,
        (width, height),
    ))
}

fn clamp(
    saved: PhysicalPosition<i32>,
    origin: PhysicalPosition<i32>,
    screen: (i32, i32),
    window: (i32, i32),
) -> PhysicalPosition<i32> {
    let max_x = origin
        .x
        .saturating_add(screen.0.saturating_sub(window.0).max(0));
    let max_y = origin
        .y
        .saturating_add(screen.1.saturating_sub(window.1).max(0));
    PhysicalPosition::new(
        saved.x.clamp(origin.x, max_x),
        saved.y.clamp(origin.y, max_y),
    )
}

fn save_at(root: &Path, kind: WindowKind, position: SavedPosition) -> io::Result<()> {
    fs::create_dir_all(root)?;
    fs::write(
        root.join(kind.filename()),
        format!(
            "{},{},{}\n",
            position.physical.x, position.physical.y, position.scale
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_old_and_scaled_positions() {
        assert_eq!(
            parse("-400,120\n"),
            Some(SavedPosition {
                physical: PhysicalPosition::new(-400, 120),
                scale: 1.0
            })
        );
        assert_eq!(
            parse("100,200,2\n"),
            Some(SavedPosition {
                physical: PhysicalPosition::new(100, 200),
                scale: 2.0
            })
        );
        assert_eq!(parse("10,abc"), None);
        assert_eq!(parse("10,20,0"), None);
        assert_eq!(parse("10,20,NaN"), None);
        assert_eq!(parse("10,20,2,extra"), None);
    }

    #[test]
    fn mixed_scale_and_changed_scale_select_logical_monitor() {
        let primary = MonitorGeometry {
            origin: PhysicalPosition::new(0, 0),
            size: (2880, 1800),
            scale: 2.0,
        };
        let external = MonitorGeometry {
            origin: PhysicalPosition::new(1440, 0),
            size: (1920, 1080),
            scale: 1.0,
        };
        let saved = SavedPosition {
            physical: PhysicalPosition::new(2000, 100),
            scale: 1.0,
        };
        assert_eq!(
            restore_position(saved, &[primary, external], Some(primary), (144.0, 220.0)),
            Some(saved.physical)
        );
        let external_scaled = MonitorGeometry {
            origin: PhysicalPosition::new(2880, 0),
            size: (3840, 2160),
            scale: 2.0,
        };
        assert_eq!(
            restore_position(
                saved,
                &[primary, external_scaled],
                Some(primary),
                (144.0, 220.0)
            ),
            Some(PhysicalPosition::new(4000, 200))
        );
    }

    #[test]
    fn disconnected_monitor_clamps_to_visible_primary() {
        let primary = MonitorGeometry {
            origin: PhysicalPosition::new(0, 0),
            size: (1920, 1080),
            scale: 2.0,
        };
        let saved = SavedPosition {
            physical: PhysicalPosition::new(2200, 150),
            scale: 1.0,
        };
        assert_eq!(
            restore_position(saved, &[primary], Some(primary), (144.0, 220.0)),
            Some(PhysicalPosition::new(1632, 300))
        );
    }

    #[test]
    fn wander_moves_are_ignored_but_system_moves_recorded() {
        let root =
            std::env::temp_dir().join(format!("chicklink-position-{:x}", rand::random::<u64>()));
        let mut recorder = PositionRecorder::with_root(WindowKind::Pet, Some(root.clone()));
        recorder.note_wander_move(PhysicalPosition::new(100, 200));
        recorder.moved(PhysicalPosition::new(100, 200), 2.0);
        assert!(recorder.pending.is_none());
        recorder.moved(PhysicalPosition::new(400, 300), 2.0);
        assert!(recorder.pending.is_some());
        recorder.flush();
        assert_eq!(
            read_at(&root, WindowKind::Pet),
            Some(SavedPosition {
                physical: PhysicalPosition::new(400, 300),
                scale: 2.0
            })
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn drop_flushes_last_pending_move() {
        let root = std::env::temp_dir().join(format!("chicklink-drop-{:x}", rand::random::<u64>()));
        {
            let mut recorder = PositionRecorder::with_root(WindowKind::Chat, Some(root.clone()));
            recorder.moved(PhysicalPosition::new(41, 82), 1.5);
        }
        assert_eq!(
            read_at(&root, WindowKind::Chat),
            Some(SavedPosition {
                physical: PhysicalPosition::new(41, 82),
                scale: 1.5
            })
        );
        fs::remove_dir_all(root).unwrap();
    }
}
