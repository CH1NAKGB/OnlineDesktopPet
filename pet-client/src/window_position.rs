//! Persist physical window coordinates without storing monitor identities.

use std::{
    fs, io,
    path::Path,
    time::{Duration, Instant},
};

use winit::{dpi::PhysicalPosition, event_loop::ActiveEventLoop};

const SAVE_DELAY: Duration = Duration::from_millis(350);

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

pub struct PositionRecorder {
    kind: WindowKind,
    pending: Option<(PhysicalPosition<i32>, Instant)>,
}

impl PositionRecorder {
    pub fn new(kind: WindowKind) -> Self {
        Self {
            kind,
            pending: None,
        }
    }

    pub fn moved(&mut self, position: PhysicalPosition<i32>) {
        self.pending = Some((position, Instant::now() + SAVE_DELAY));
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
        if let Some((position, _)) = self.pending.take() {
            if let Err(error) = save(self.kind, position) {
                eprintln!("window position save failed: {error}");
            }
        }
    }
}

fn path(kind: WindowKind) -> Option<std::path::PathBuf> {
    Some(crate::config::app_data_root()?.join(kind.filename()))
}

fn parse(text: &str) -> Option<PhysicalPosition<i32>> {
    let (x, y) = text.trim().split_once(',')?;
    Some(PhysicalPosition::new(x.parse().ok()?, y.parse().ok()?))
}

fn read_at(root: &Path, kind: WindowKind) -> Option<PhysicalPosition<i32>> {
    parse(&fs::read_to_string(root.join(kind.filename())).ok()?)
}

pub fn restore(
    event_loop: &ActiveEventLoop,
    kind: WindowKind,
    logical_size: (f64, f64),
) -> Option<PhysicalPosition<i32>> {
    let saved = read_at(crate::config::app_data_root()?.as_path(), kind)?;
    let monitors: Vec<_> = event_loop.available_monitors().collect();
    let monitor = monitors
        .iter()
        .find(|monitor| {
            let origin = monitor.position();
            let size = monitor.size();
            saved.x >= origin.x
                && saved.x < origin.x.saturating_add(size.width as i32)
                && saved.y >= origin.y
                && saved.y < origin.y.saturating_add(size.height as i32)
        })
        .cloned()
        .or_else(|| event_loop.primary_monitor())
        .or_else(|| monitors.into_iter().next())?;
    let size = monitor.size();
    let scale = monitor.scale_factor();
    let width = (logical_size.0 * scale).round() as i32;
    let height = (logical_size.1 * scale).round() as i32;
    Some(clamp(
        saved,
        monitor.position(),
        (size.width as i32, size.height as i32),
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

fn save(kind: WindowKind, position: PhysicalPosition<i32>) -> io::Result<()> {
    let path = path(kind).ok_or_else(|| io::Error::other("无法确定设置目录"))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, format!("{},{}\n", position.x, position.y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_physical_position_and_rejects_invalid_values() {
        assert_eq!(parse("-400,120\n"), Some(PhysicalPosition::new(-400, 120)));
        assert_eq!(parse("10,abc"), None);
        assert_eq!(parse("10,20,30"), None);
    }

    #[test]
    fn disconnected_monitor_clamps_to_visible_primary() {
        assert_eq!(
            clamp(
                PhysicalPosition::new(2200, 150),
                PhysicalPosition::new(0, 0),
                (1920, 1080),
                (288, 440)
            ),
            PhysicalPosition::new(1632, 150),
        );
        assert_eq!(
            clamp(
                PhysicalPosition::new(-200, -300),
                PhysicalPosition::new(0, 0),
                (1920, 1080),
                (288, 440)
            ),
            PhysicalPosition::new(0, 0),
        );
    }
}
