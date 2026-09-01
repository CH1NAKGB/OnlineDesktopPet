#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod config;
mod frame_pacer;
mod lan_discovery;
mod renderer;

use std::{
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

use config::ClientConfig;
use frame_pacer::FramePacer;
use renderer::Renderer;
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::{ElementState, KeyEvent, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{Key, NamedKey},
    window::{Window, WindowId, WindowLevel},
};

const PET_SIZE: f64 = 144.0;

struct PetApp {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    started_at: Instant,
    press_started_at: Option<Instant>,
    pet_until: Option<Instant>,
    pacer: FramePacer,
    _network_events: mpsc::Receiver<String>,
}

impl PetApp {
    fn new(network_events: mpsc::Receiver<String>) -> Self {
        Self {
            window: None,
            renderer: None,
            started_at: Instant::now(),
            press_started_at: None,
            pet_until: None,
            pacer: FramePacer::new(),
            _network_events: network_events,
        }
    }
}

impl ApplicationHandler for PetApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("Chick Link")
            .with_inner_size(LogicalSize::new(PET_SIZE, PET_SIZE))
            .with_transparent(true)
            .with_decorations(false)
            .with_resizable(false)
            .with_window_level(WindowLevel::AlwaysOnTop);
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .expect("create desktop pet window"),
        );
        self.renderer =
            Some(pollster::block_on(Renderer::new(window.clone())).expect("create GPU renderer"));
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key: Key::Named(NamedKey::Escape),
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => event_loop.exit(),
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                // Let macOS/Windows own the drag so the window stays locked to the pointer.
                let _ = window.drag_window();
                self.press_started_at = Some(Instant::now());
                self.pacer.activate();
                window.request_redraw();
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                if self
                    .press_started_at
                    .take()
                    .is_some_and(|pressed_at| pressed_at.elapsed() < Duration::from_millis(180))
                {
                    self.pet_until = Some(Instant::now() + Duration::from_secs(2));
                    self.pacer.activate();
                    window.request_redraw();
                }
            }
            WindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size);
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(renderer) = self.renderer.as_ref() {
                    let petting =
                        self.pet_until.is_some_and(|until| until > Instant::now()) as u32 as f32;
                    if let Err(error) =
                        renderer.render(self.started_at.elapsed().as_secs_f32(), petting)
                    {
                        eprintln!("render error: {error}");
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        while self._network_events.try_recv().is_ok() {
            self.pacer.activate();
        }
        match self.pacer.next_wake() {
            Some(when) => event_loop.set_control_flow(ControlFlow::WaitUntil(when)),
            None => {
                event_loop.set_control_flow(ControlFlow::Poll);
                window.request_redraw();
            }
        }
    }
}

fn main() -> anyhow::Result<()> {
    let config = ClientConfig::default();
    let (events_tx, events_rx) = mpsc::channel();
    std::thread::spawn(move || network::run(config, events_tx));
    let event_loop = EventLoop::new()?;
    event_loop.run_app(&mut PetApp::new(events_rx))?;
    Ok(())
}

mod network {
    use super::ClientConfig;
    use futures_util::{SinkExt, StreamExt};
    use shared_protocol::{ClientEvent, PROTOCOL_VERSION, PresenceState};
    use std::sync::mpsc::Sender;
    use std::time::Duration;
    use tokio_tungstenite::tungstenite::Message;

    pub fn run(config: ClientConfig, events: Sender<String>) {
        let runtime = tokio::runtime::Runtime::new().expect("network runtime");
        runtime.block_on(async move {
            let relay_url = if config.discover_lan_relay {
                super::lan_discovery::find_relay(Duration::from_secs(1)).unwrap_or(config.relay_url)
            } else {
                config.relay_url
            };
            let _ = events.send(format!("connecting to {relay_url}"));
            let Ok((mut socket, _)) = tokio_tungstenite::connect_async(&relay_url).await else {
                return;
            };
            let hello = ClientEvent::Hello {
                protocol_version: PROTOCOL_VERSION,
                access_token: config.access_token,
                device_id: config.device_id,
            };
            let presence = ClientEvent::Presence {
                state: PresenceState::Online,
                activity: None,
            };
            for event in [hello, presence] {
                let Ok(json) = serde_json::to_string(&event) else {
                    continue;
                };
                if socket.send(Message::Text(json.into())).await.is_err() {
                    return;
                }
            }
            while let Some(Ok(message)) = socket.next().await {
                if message.is_text() {
                    let _ = events.send("peer activity".into());
                }
            }
        });
    }
}
