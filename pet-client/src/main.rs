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
use shared_protocol::{PetSkin, SharedPetAction};
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::{ElementState, KeyEvent, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{Key, NamedKey},
    window::{Window, WindowId, WindowLevel},
};

#[cfg(target_os = "macos")]
use muda::{ContextMenu, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
#[cfg(target_os = "macos")]
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

const PET_SIZE: f64 = 144.0;

#[derive(Debug)]
enum NetworkEvent {
    Connecting,
    Connected,
    Latency(Duration),
    SharedPetAction(SharedPetAction),
    Disconnected,
}

#[derive(Debug)]
enum NetworkCommand {
    SharedPetAction {
        recipient_id: String,
        action: SharedPetAction,
    },
}

#[cfg(target_os = "macos")]
struct PetMenu {
    menu: Menu,
    pet: MenuItem,
    sunshine: MenuItem,
    blueberry: MenuItem,
    cocoa: MenuItem,
    quality: MenuItem,
    quit: MenuItem,
}

#[cfg(target_os = "macos")]
impl PetMenu {
    fn new(partner_id: Option<&str>) -> Self {
        let menu = Menu::new();
        let owners = MenuItem::new(
            partner_id
                .map(|id| format!("共同小鸡 · 已配对给 {id}"))
                .unwrap_or_else(|| "共同小鸡 · 尚未配对".into()),
            false,
            None,
        );
        let pet = MenuItem::new("轻拍一下", true, None);
        let skins = Submenu::new("换肤", true);
        let sunshine = MenuItem::new("日光小鸡", true, None);
        let blueberry = MenuItem::new("蓝莓小鸡", true, None);
        let cocoa = MenuItem::new("可可小鸡", true, None);
        let chat = MenuItem::new("聊天（界面开发中）", false, None);
        let quality = MenuItem::new("网络：正在连接", false, None);
        let quit = MenuItem::new("退出 Chick Link", true, None);

        skins.append(&sunshine).expect("append sunshine skin");
        skins.append(&blueberry).expect("append blueberry skin");
        skins.append(&cocoa).expect("append cocoa skin");
        menu.append(&owners).expect("append owners label");
        menu.append(&PredefinedMenuItem::separator())
            .expect("append separator");
        menu.append(&pet).expect("append pet action");
        menu.append(&skins).expect("append skins menu");
        menu.append(&chat).expect("append chat item");
        menu.append(&PredefinedMenuItem::separator())
            .expect("append separator");
        menu.append(&quality).expect("append quality label");
        menu.append(&PredefinedMenuItem::separator())
            .expect("append separator");
        menu.append(&quit).expect("append quit action");
        Self {
            menu,
            pet,
            sunshine,
            blueberry,
            cocoa,
            quality,
            quit,
        }
    }

    fn update_quality(&self, label: &str) {
        self.quality.set_text(label);
    }

    fn show(&self, window: &Window, position: winit::dpi::PhysicalPosition<f64>) {
        let Ok(handle) = window.window_handle() else {
            return;
        };
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return;
        };
        let position = muda::dpi::PhysicalPosition {
            x: position.x,
            y: position.y,
        };
        // The window handle is borrowed only while AppKit tracks the native menu.
        unsafe {
            self.menu
                .show_context_menu_for_nsview(handle.ns_view.as_ptr(), Some(position.into()));
        }
    }

    fn action_for(&self, id: &MenuId) -> Option<MenuAction> {
        if id == self.pet.id() {
            Some(MenuAction::Pet)
        } else if id == self.sunshine.id() {
            Some(MenuAction::Skin(PetSkin::Sunshine))
        } else if id == self.blueberry.id() {
            Some(MenuAction::Skin(PetSkin::Blueberry))
        } else if id == self.cocoa.id() {
            Some(MenuAction::Skin(PetSkin::Cocoa))
        } else if id == self.quit.id() {
            Some(MenuAction::Quit)
        } else {
            None
        }
    }
}

#[cfg(target_os = "macos")]
enum MenuAction {
    Pet,
    Skin(PetSkin),
    Quit,
}

struct PetApp {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    started_at: Instant,
    press_started_at: Option<Instant>,
    pet_until: Option<Instant>,
    pet_skin: PetSkin,
    pacer: FramePacer,
    network_events: mpsc::Receiver<NetworkEvent>,
    network_commands: tokio::sync::mpsc::UnboundedSender<NetworkCommand>,
    partner_id: Option<String>,
    latency_ms: Option<u32>,
    #[cfg(target_os = "macos")]
    menu: Option<PetMenu>,
    #[cfg(target_os = "macos")]
    cursor_position: winit::dpi::PhysicalPosition<f64>,
}

impl PetApp {
    fn new(
        network_events: mpsc::Receiver<NetworkEvent>,
        network_commands: tokio::sync::mpsc::UnboundedSender<NetworkCommand>,
        partner_id: Option<String>,
    ) -> Self {
        Self {
            window: None,
            renderer: None,
            started_at: Instant::now(),
            press_started_at: None,
            pet_until: None,
            pet_skin: PetSkin::Sunshine,
            pacer: FramePacer::new(),
            network_events,
            network_commands,
            partner_id,
            latency_ms: None,
            #[cfg(target_os = "macos")]
            menu: None,
            #[cfg(target_os = "macos")]
            cursor_position: winit::dpi::PhysicalPosition::new(0.0, 0.0),
        }
    }

    fn apply_shared_action(&mut self, action: SharedPetAction, share_with_partner: bool) {
        match action {
            SharedPetAction::Pet => {
                self.pet_until = Some(Instant::now() + Duration::from_secs(2));
            }
            SharedPetAction::SetSkin { skin } => self.pet_skin = skin,
        }
        if share_with_partner {
            if let Some(recipient_id) = self.partner_id.clone() {
                let _ = self.network_commands.send(NetworkCommand::SharedPetAction {
                    recipient_id,
                    action,
                });
            }
        }
        self.pacer.activate();
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    fn network_label(&self) -> String {
        match self.latency_ms {
            Some(ms) if ms <= 80 => format!("网络：优秀 · {ms} ms"),
            Some(ms) if ms <= 180 => format!("网络：良好 · {ms} ms"),
            Some(ms) => format!("网络：较弱 · {ms} ms"),
            None => "网络：正在连接".into(),
        }
    }

    #[cfg(target_os = "macos")]
    fn handle_menu_events(&mut self, event_loop: &ActiveEventLoop) {
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            let action = self
                .menu
                .as_ref()
                .and_then(|menu| menu.action_for(&event.id));
            match action {
                Some(MenuAction::Pet) => self.apply_shared_action(SharedPetAction::Pet, true),
                Some(MenuAction::Skin(skin)) => {
                    self.apply_shared_action(SharedPetAction::SetSkin { skin }, true)
                }
                Some(MenuAction::Quit) => event_loop.exit(),
                None => {}
            }
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
        #[cfg(target_os = "macos")]
        {
            self.menu = Some(PetMenu::new(self.partner_id.as_deref()));
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(window) = self.window.clone() else {
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
                    self.apply_shared_action(SharedPetAction::Pet, true);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                #[cfg(target_os = "macos")]
                {
                    self.cursor_position = position;
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Right,
                ..
            } =>
            {
                #[cfg(target_os = "macos")]
                if let Some(menu) = self.menu.as_ref() {
                    menu.show(&window, self.cursor_position);
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
                    if let Err(error) = renderer.render(
                        self.started_at.elapsed().as_secs_f32(),
                        petting,
                        skin_to_uniform(self.pet_skin),
                    ) {
                        eprintln!("render error: {error}");
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(window) = self.window.clone() else {
            return;
        };
        while let Ok(event) = self.network_events.try_recv() {
            match event {
                NetworkEvent::Latency(rtt) => self.latency_ms = Some(rtt.as_millis() as u32),
                NetworkEvent::Disconnected => self.latency_ms = None,
                NetworkEvent::SharedPetAction(action) => self.apply_shared_action(action, false),
                NetworkEvent::Connecting | NetworkEvent::Connected => {}
            }
            #[cfg(target_os = "macos")]
            if let Some(menu) = self.menu.as_ref() {
                menu.update_quality(&self.network_label());
            }
            self.pacer.activate();
        }
        #[cfg(target_os = "macos")]
        self.handle_menu_events(event_loop);
        match self.pacer.next_wake() {
            Some(when) => event_loop.set_control_flow(ControlFlow::WaitUntil(when)),
            None => {
                event_loop.set_control_flow(ControlFlow::Poll);
                window.request_redraw();
            }
        }
    }
}

fn skin_to_uniform(skin: PetSkin) -> f32 {
    match skin {
        PetSkin::Sunshine => 0.0,
        PetSkin::Blueberry => 1.0,
        PetSkin::Cocoa => 2.0,
    }
}

fn main() -> anyhow::Result<()> {
    let config = ClientConfig::default();
    let (events_tx, events_rx) = mpsc::channel();
    let partner_id = config.partner_id.clone();
    let (commands_tx, commands_rx) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || network::run(config, events_tx, commands_rx));
    let event_loop = EventLoop::new()?;
    event_loop.run_app(&mut PetApp::new(events_rx, commands_tx, partner_id))?;
    Ok(())
}

mod network {
    use super::ClientConfig;
    use futures_util::{SinkExt, StreamExt};
    use shared_protocol::{ClientEvent, PROTOCOL_VERSION, PresenceState, ServerEvent};
    use std::sync::mpsc::Sender;
    use std::time::{Duration, Instant};
    use tokio::sync::mpsc::UnboundedReceiver;
    use tokio_tungstenite::tungstenite::Message;

    pub fn run(
        config: ClientConfig,
        events: Sender<super::NetworkEvent>,
        mut commands: UnboundedReceiver<super::NetworkCommand>,
    ) {
        let runtime = tokio::runtime::Runtime::new().expect("network runtime");
        runtime.block_on(async move {
            let relay_url = if config.discover_lan_relay {
                super::lan_discovery::find_relay(Duration::from_secs(1)).unwrap_or(config.relay_url)
            } else {
                config.relay_url
            };
            let _ = events.send(super::NetworkEvent::Connecting);
            let Ok((mut socket, _)) = tokio_tungstenite::connect_async(&relay_url).await else {
                let _ = events.send(super::NetworkEvent::Disconnected);
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
                    let _ = events.send(super::NetworkEvent::Disconnected);
                    return;
                }
            }
            let _ = events.send(super::NetworkEvent::Connected);
            let mut ping_timer = tokio::time::interval(Duration::from_secs(3));
            ping_timer.tick().await;
            let mut ping_sent_at = None;
            loop {
                tokio::select! {
                    _ = ping_timer.tick() => {
                        let Ok(json) = serde_json::to_string(&ClientEvent::Ping) else {
                            continue;
                        };
                        if socket.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                        ping_sent_at = Some(Instant::now());
                    }
                    Some(command) = commands.recv() => {
                        let event = match command {
                            super::NetworkCommand::SharedPetAction { recipient_id, action } => {
                                ClientEvent::SharedPetAction { recipient_id, action }
                            }
                        };
                        let Ok(json) = serde_json::to_string(&event) else {
                            continue;
                        };
                        if socket.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                    }
                    message = socket.next() => {
                        let Some(Ok(message)) = message else {
                            break;
                        };
                        if !message.is_text() {
                            continue;
                        }
                        let Ok(event) = serde_json::from_str::<ServerEvent>(message.to_text().unwrap_or_default()) else {
                            continue;
                        };
                        match event {
                            ServerEvent::Pong => {
                                if let Some(sent_at) = ping_sent_at.take() {
                                    let _ = events.send(super::NetworkEvent::Latency(sent_at.elapsed()));
                                }
                            }
                            ServerEvent::SharedPetAction { action, .. } => {
                                let _ = events.send(super::NetworkEvent::SharedPetAction(action));
                            }
                            _ => {}
                        }
                    }
                }
            }
            let _ = events.send(super::NetworkEvent::Disconnected);
        });
    }
}
