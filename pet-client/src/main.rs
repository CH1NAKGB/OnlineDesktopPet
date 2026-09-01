#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod chat;
mod config;
mod frame_pacer;
mod lan_discovery;
#[cfg(not(target_os = "windows"))]
mod renderer;
#[cfg(target_os = "windows")]
mod windows_renderer;

use std::{
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

use config::ClientConfig;
use frame_pacer::FramePacer;
#[cfg(not(target_os = "windows"))]
use renderer::Renderer;
use shared_protocol::{MessageBody, PetSkin, SharedPetAction};
#[cfg(target_os = "windows")]
use windows_renderer::Renderer;
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
#[cfg(target_os = "windows")]
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{
        ContextMenu as TrayContextMenu, Menu as TrayMenu, MenuEvent as TrayMenuEvent,
        MenuId as TrayMenuId, MenuItem as TrayMenuItem,
        PredefinedMenuItem as TrayPredefinedMenuItem, Submenu as TraySubmenu,
    },
};
#[cfg(target_os = "windows")]
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetWindowLongW, SetWindowLongW, WS_EX_LAYERED,
};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

const PET_SIZE: f64 = 144.0;

#[derive(Debug)]
enum NetworkEvent {
    Connecting,
    Connected,
    Latency(Duration),
    SharedPetAction(SharedPetAction),
    DirectMessage {
        sender_id: String,
        body: MessageBody,
    },
    PairingCodeRegistered(String),
    Paired(String),
    Disconnected,
}

#[derive(Debug)]
enum NetworkCommand {
    SharedPetAction {
        recipient_id: String,
        action: SharedPetAction,
    },
    DirectMessage {
        recipient_id: String,
        body: MessageBody,
    },
    PairWithCode(String),
}

#[cfg(target_os = "macos")]
struct PetMenu {
    menu: Menu,
    pet: MenuItem,
    sunshine: MenuItem,
    blueberry: MenuItem,
    cocoa: MenuItem,
    chat: MenuItem,
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
        let chat = MenuItem::new("配对 / 聊天", true, None);
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
            chat,
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
        } else if id == self.chat.id() {
            Some(MenuAction::Chat)
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
    Chat,
    Quit,
}

#[cfg(target_os = "windows")]
struct WindowsTray {
    _tray: TrayIcon,
    chat: TrayMenuItem,
    show_pet: TrayMenuItem,
    quit: TrayMenuItem,
}

#[cfg(target_os = "windows")]
impl WindowsTray {
    fn new(_partner_id: Option<&str>) -> Self {
        let menu = TrayMenu::new();
        let chat = TrayMenuItem::new("配对 / 聊天", true, None);
        let show_pet = TrayMenuItem::new("显示小鸡", true, None);
        let quit = TrayMenuItem::new("退出 Chick Link", true, None);
        menu.append(&chat).expect("append tray chat");
        menu.append(&show_pet).expect("append tray show pet");
        menu.append(&quit).expect("append tray quit");
        let icon = Icon::from_rgba(tray_icon_rgba(), 32, 32).expect("create tray icon");
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Chick Link")
            .with_icon(icon)
            .build()
            .expect("create Windows tray icon");
        Self {
            _tray: tray,
            chat,
            show_pet,
            quit,
        }
    }

    fn action_for(&self, id: &TrayMenuId) -> Option<TrayAction> {
        if id == self.chat.id() {
            Some(TrayAction::Chat)
        } else if id == self.show_pet.id() {
            Some(TrayAction::ShowPet)
        } else if id == self.quit.id() {
            Some(TrayAction::Quit)
        } else {
            None
        }
    }
}

#[cfg(target_os = "windows")]
enum TrayAction {
    Pet,
    Skin(PetSkin),
    Chat,
    ShowPet,
    Quit,
}

#[cfg(target_os = "windows")]
struct WindowsPetMenu {
    menu: TrayMenu,
    pet: TrayMenuItem,
    sunshine: TrayMenuItem,
    blueberry: TrayMenuItem,
    cocoa: TrayMenuItem,
    chat: TrayMenuItem,
    quit: TrayMenuItem,
}

#[cfg(target_os = "windows")]
impl WindowsPetMenu {
    fn new(window: &Window, partner_id: Option<&str>) -> Self {
        let menu = TrayMenu::new();
        let owners = TrayMenuItem::new(
            partner_id
                .map(|id| format!("共同小鸡 · 已配对给 {id}"))
                .unwrap_or_else(|| "共同小鸡 · 尚未配对".into()),
            false,
            None,
        );
        let pet = TrayMenuItem::new("轻拍一下", true, None);
        let skins = TraySubmenu::new("换肤", true);
        let sunshine = TrayMenuItem::new("日光小鸡", true, None);
        let blueberry = TrayMenuItem::new("蓝莓小鸡", true, None);
        let cocoa = TrayMenuItem::new("可可小鸡", true, None);
        let chat = TrayMenuItem::new("配对 / 聊天", true, None);
        let quit = TrayMenuItem::new("退出 Chick Link", true, None);
        skins.append(&sunshine).expect("append sunshine skin");
        skins.append(&blueberry).expect("append blueberry skin");
        skins.append(&cocoa).expect("append cocoa skin");
        menu.append(&owners).expect("append owners label");
        menu.append(&TrayPredefinedMenuItem::separator())
            .expect("append separator");
        menu.append(&pet).expect("append pet action");
        menu.append(&skins).expect("append skins menu");
        menu.append(&chat).expect("append chat action");
        menu.append(&TrayPredefinedMenuItem::separator())
            .expect("append separator");
        menu.append(&quit).expect("append quit action");
        if let Some(hwnd) = windows_hwnd(window) {
            unsafe { menu.attach_menu_subclass_for_hwnd(hwnd) };
        }
        Self {
            menu,
            pet,
            sunshine,
            blueberry,
            cocoa,
            chat,
            quit,
        }
    }

    fn show(&self, window: &Window, position: winit::dpi::PhysicalPosition<f64>) {
        let Some(hwnd) = windows_hwnd(window) else {
            return;
        };
        let position = tray_icon::dpi::PhysicalPosition {
            x: position.x,
            y: position.y,
        };
        unsafe {
            self.menu
                .show_context_menu_for_hwnd(hwnd, Some(position.into()));
        }
    }

    fn action_for(&self, id: &TrayMenuId) -> Option<TrayAction> {
        if id == self.pet.id() {
            Some(TrayAction::Pet)
        } else if id == self.sunshine.id() {
            Some(TrayAction::Skin(PetSkin::Sunshine))
        } else if id == self.blueberry.id() {
            Some(TrayAction::Skin(PetSkin::Blueberry))
        } else if id == self.cocoa.id() {
            Some(TrayAction::Skin(PetSkin::Cocoa))
        } else if id == self.chat.id() {
            Some(TrayAction::Chat)
        } else if id == self.quit.id() {
            Some(TrayAction::Quit)
        } else {
            None
        }
    }
}

#[cfg(target_os = "windows")]
fn windows_hwnd(window: &Window) -> Option<isize> {
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return None;
    };
    Some(handle.hwnd.get())
}

#[cfg(target_os = "windows")]
fn enable_windows_transparency(window: &Window) {
    let Some(hwnd) = windows_hwnd(window) else {
        return;
    };
    unsafe {
        let style = GetWindowLongW(hwnd as _, GWL_EXSTYLE);
        SetWindowLongW(hwnd as _, GWL_EXSTYLE, style | WS_EX_LAYERED as i32);
    }
}

#[cfg(target_os = "windows")]
fn tray_icon_rgba() -> Vec<u8> {
    let mut pixels = vec![0; 32 * 32 * 4];
    for y in 0..32 {
        for x in 0..32 {
            let dx = x as f32 - 15.5;
            let dy = y as f32 - 16.0;
            let body = dx * dx / 150.0 + dy * dy / 120.0 <= 1.0;
            let index = (y * 32 + x) * 4;
            if body {
                pixels[index..index + 4].copy_from_slice(&[255, 194, 45, 255]);
            }
        }
    }
    pixels
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
    pairing_code: String,
    bubble: Option<(String, Instant)>,
    latency_ms: Option<u32>,
    #[cfg(target_os = "macos")]
    menu: Option<PetMenu>,
    #[cfg(target_os = "windows")]
    tray: Option<WindowsTray>,
    #[cfg(target_os = "windows")]
    windows_menu: Option<WindowsPetMenu>,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    cursor_position: winit::dpi::PhysicalPosition<f64>,
}

impl PetApp {
    fn new(
        network_events: mpsc::Receiver<NetworkEvent>,
        network_commands: tokio::sync::mpsc::UnboundedSender<NetworkCommand>,
        partner_id: Option<String>,
        pairing_code: String,
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
            pairing_code,
            bubble: None,
            latency_ms: None,
            #[cfg(target_os = "macos")]
            menu: None,
            #[cfg(target_os = "windows")]
            tray: None,
            #[cfg(target_os = "windows")]
            windows_menu: None,
            #[cfg(any(target_os = "macos", target_os = "windows"))]
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

    #[cfg(target_os = "macos")]
    fn network_label(&self) -> String {
        match self.latency_ms {
            Some(ms) if ms <= 80 => format!("网络：优秀 · {ms} ms"),
            Some(ms) if ms <= 180 => format!("网络：良好 · {ms} ms"),
            Some(ms) => format!("网络：较弱 · {ms} ms"),
            None => "网络：正在连接".into(),
        }
    }

    fn open_chat(&self) {
        let sender = self.network_commands.clone();
        let partner_id = self.partner_id.clone();
        let pairing_code = self.pairing_code.clone();
        // Native dialogs are modal. Run them away from Winit so animation and menus stay responsive.
        std::thread::spawn(move || {
            if let Some(recipient_id) = partner_id {
                if let Some(text) = chat::compose(Some(&recipient_id)) {
                    let _ = sender.send(NetworkCommand::DirectMessage {
                        recipient_id,
                        body: MessageBody::Text { text },
                    });
                }
            } else if let Some(code) = chat::prompt_pairing_code(&pairing_code) {
                let _ = sender.send(NetworkCommand::PairWithCode(code));
            }
        });
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
                Some(MenuAction::Chat) => self.open_chat(),
                Some(MenuAction::Quit) => event_loop.exit(),
                None => {}
            }
        }
    }

    #[cfg(target_os = "windows")]
    fn handle_tray_events(&mut self, event_loop: &ActiveEventLoop) {
        while let Ok(event) = TrayMenuEvent::receiver().try_recv() {
            let action = self
                .tray
                .as_ref()
                .and_then(|tray| tray.action_for(&event.id))
                .or_else(|| {
                    self.windows_menu
                        .as_ref()
                        .and_then(|menu| menu.action_for(&event.id))
                });
            match action {
                Some(TrayAction::Pet) => self.apply_shared_action(SharedPetAction::Pet, true),
                Some(TrayAction::Skin(skin)) => {
                    self.apply_shared_action(SharedPetAction::SetSkin { skin }, true)
                }
                Some(TrayAction::Chat) => self.open_chat(),
                Some(TrayAction::ShowPet) => {
                    if let Some(window) = self.window.as_ref() {
                        window.set_visible(true);
                        window.focus_window();
                    }
                }
                Some(TrayAction::Quit) => event_loop.exit(),
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
        #[cfg(target_os = "windows")]
        let attributes = {
            use winit::platform::windows::WindowAttributesExtWindows;
            attributes.with_skip_taskbar(true)
        };
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .expect("create desktop pet window"),
        );
        #[cfg(target_os = "windows")]
        enable_windows_transparency(&window);
        self.renderer =
            Some(pollster::block_on(Renderer::new(window.clone())).expect("create GPU renderer"));
        self.window = Some(window.clone());
        #[cfg(target_os = "macos")]
        {
            self.menu = Some(PetMenu::new(self.partner_id.as_deref()));
        }
        #[cfg(target_os = "windows")]
        {
            self.tray = Some(WindowsTray::new(self.partner_id.as_deref()));
            self.windows_menu = Some(WindowsPetMenu::new(&window, self.partner_id.as_deref()));
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
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            WindowEvent::CursorMoved { position, .. } => self.cursor_position = position,
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Right,
                ..
            } => {
                #[cfg(target_os = "macos")]
                if let Some(menu) = self.menu.as_ref() {
                    menu.show(&window, self.cursor_position);
                }
                #[cfg(target_os = "windows")]
                if let Some(menu) = self.windows_menu.as_ref() {
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
                NetworkEvent::DirectMessage { sender_id, body } => {
                    if let MessageBody::Text { text } = body {
                        self.bubble = Some((
                            format!("{sender_id}: {text}"),
                            Instant::now() + Duration::from_secs(8),
                        ));
                    }
                }
                NetworkEvent::PairingCodeRegistered(code) => self.pairing_code = code,
                NetworkEvent::Paired(partner_id) => self.partner_id = Some(partner_id),
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
        #[cfg(target_os = "windows")]
        self.handle_tray_events(event_loop);
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
    let pairing_code = config.pairing_code.clone();
    let (commands_tx, commands_rx) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || network::run(config, events_tx, commands_rx));
    let event_loop = EventLoop::new()?;
    event_loop.run_app(&mut PetApp::new(
        events_rx,
        commands_tx,
        partner_id,
        pairing_code,
    ))?;
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
            let register_code = ClientEvent::RegisterPairingCode {
                code: config.pairing_code,
            };
            let presence = ClientEvent::Presence {
                state: PresenceState::Online,
                activity: None,
            };
            for event in [hello, register_code, presence] {
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
                            super::NetworkCommand::DirectMessage { recipient_id, body } => {
                                ClientEvent::DirectMessage { recipient_id, body }
                            }
                            super::NetworkCommand::PairWithCode(code) => ClientEvent::PairWithCode { code },
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
                            ServerEvent::DirectMessage { sender_id, body } => {
                                let _ = events.send(super::NetworkEvent::DirectMessage { sender_id, body });
                            }
                            ServerEvent::PairingCodeRegistered { code } => {
                                let _ = events.send(super::NetworkEvent::PairingCodeRegistered(code));
                            }
                            ServerEvent::Paired { partner_id } => {
                                let _ = events.send(super::NetworkEvent::Paired(partner_id));
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
