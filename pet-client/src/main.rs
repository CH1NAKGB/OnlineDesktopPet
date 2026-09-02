#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod chat;
mod chat_window;
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

use chat_window::{ChatAction, ChatWindow};
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

const PET_WIDTH: f64 = 144.0;
const PET_HEIGHT: f64 = 220.0;

#[derive(Debug)]
enum NetworkEvent {
    Connecting,
    Connected,
    Latency(Duration),
    SharedPetAction(SharedPetAction),
    DirectMessage {
        sender_id: String,
        sender_display_name: Option<String>,
        body: MessageBody,
    },
    PairingCodeRegistered(String),
    Paired {
        partner_id: String,
        partner_display_name: Option<String>,
        initiated_by_you: bool,
    },
    ProfileUpdated(String),
    Error(String),
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
    UpdateProfile(String),
}

#[cfg(target_os = "macos")]
struct PetMenu {
    menu: Menu,
    owners: MenuItem,
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
            owners,
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

    fn update_partner(&self, partner: &str) {
        self.owners
            .set_text(format!("共同小鸡 · 已配对给 {partner}"));
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
    owners: TrayMenuItem,
    chat: TrayMenuItem,
    show_pet: TrayMenuItem,
    quality: TrayMenuItem,
    quit: TrayMenuItem,
}

#[cfg(target_os = "windows")]
impl WindowsTray {
    fn new(partner_id: Option<&str>) -> Self {
        let menu = TrayMenu::new();
        let owners = TrayMenuItem::new(
            partner_id
                .map(|id| format!("共同小鸡 · 已配对给 {id}"))
                .unwrap_or_else(|| "共同小鸡 · 尚未配对".into()),
            false,
            None,
        );
        let chat = TrayMenuItem::new("配对 / 聊天", true, None);
        let show_pet = TrayMenuItem::new("显示小鸡", true, None);
        let quality = TrayMenuItem::new("网络：正在连接", false, None);
        let quit = TrayMenuItem::new("退出 Chick Link", true, None);
        menu.append(&owners).expect("append tray owners");
        menu.append(&TrayPredefinedMenuItem::separator())
            .expect("append tray separator");
        menu.append(&chat).expect("append tray chat");
        menu.append(&show_pet).expect("append tray show pet");
        menu.append(&TrayPredefinedMenuItem::separator())
            .expect("append tray separator");
        menu.append(&quality).expect("append tray quality");
        menu.append(&TrayPredefinedMenuItem::separator())
            .expect("append tray separator");
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
            owners,
            chat,
            show_pet,
            quality,
            quit,
        }
    }

    fn update_quality(&self, label: &str) {
        self.quality.set_text(label);
    }

    fn update_partner(&self, partner: &str) {
        self.owners
            .set_text(format!("共同小鸡 · 已配对给 {partner}"));
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
    owners: TrayMenuItem,
    pet: TrayMenuItem,
    sunshine: TrayMenuItem,
    blueberry: TrayMenuItem,
    cocoa: TrayMenuItem,
    chat: TrayMenuItem,
    quality: TrayMenuItem,
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
        let quality = TrayMenuItem::new("网络：正在连接", false, None);
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
        menu.append(&quality).expect("append quality label");
        menu.append(&TrayPredefinedMenuItem::separator())
            .expect("append separator");
        menu.append(&quit).expect("append quit action");
        if let Some(hwnd) = windows_hwnd(window) {
            unsafe { menu.attach_menu_subclass_for_hwnd(hwnd) };
        }
        Self {
            menu,
            owners,
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

    fn update_partner(&self, partner: &str) {
        self.owners
            .set_text(format!("共同小鸡 · 已配对给 {partner}"));
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
    partner_display_name: Option<String>,
    display_name: String,
    pairing_code: String,
    bubble: Option<(String, Instant)>,
    chat_window: Option<ChatWindow>,
    open_chat_on_start: bool,
    latency_ms: Option<u32>,
    network_connected: bool,
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
        display_name: String,
        open_chat_on_start: bool,
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
            partner_display_name: None,
            display_name,
            pairing_code,
            bubble: None,
            chat_window: None,
            open_chat_on_start,
            latency_ms: None,
            network_connected: false,
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

    fn network_label(&self) -> String {
        match self.latency_ms {
            Some(ms) if ms <= 80 => format!("网络：优秀 · {ms} ms"),
            Some(ms) if ms <= 180 => format!("网络：良好 · {ms} ms"),
            Some(ms) => format!("网络：较弱 · {ms} ms"),
            None if self.network_connected => "网络：已连接 · 测量中".into(),
            None => "网络：正在连接".into(),
        }
    }

    fn show_bubble(&mut self, text: impl Into<String>, duration: Duration) {
        self.bubble = Some((text.into(), Instant::now() + duration));
        self.pacer.activate();
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    fn update_partner_labels(&self, partner: &str) {
        #[cfg(target_os = "macos")]
        if let Some(menu) = self.menu.as_ref() {
            menu.update_partner(partner);
        }
        #[cfg(target_os = "windows")]
        {
            if let Some(tray) = self.tray.as_ref() {
                tray.update_partner(partner);
            }
            if let Some(menu) = self.windows_menu.as_ref() {
                menu.update_partner(partner);
            }
        }
    }

    fn update_network_labels(&mut self) {
        let label = self.network_label();
        #[cfg(target_os = "macos")]
        if let Some(menu) = self.menu.as_ref() {
            menu.update_quality(&label);
        }
        #[cfg(target_os = "windows")]
        {
            if let Some(tray) = self.tray.as_ref() {
                tray.update_quality(&label);
            }
            if let Some(menu) = self.windows_menu.as_ref() {
                menu.update_quality(&label);
            }
        }
        if let Some(chat_window) = self.chat_window.as_mut() {
            chat_window.set_network_status(label);
        }
    }

    fn open_chat(&mut self, event_loop: &ActiveEventLoop) {
        if self.chat_window.is_none() {
            match pollster::block_on(ChatWindow::new(
                event_loop,
                self.pairing_code.clone(),
                self.partner_id.clone(),
                self.display_name.clone(),
                self.network_label(),
            )) {
                Ok(window) => self.chat_window = Some(window),
                Err(error) => {
                    eprintln!("chat window error: {error:#}");
                    return;
                }
            }
        }
        if let Some(chat_window) = self.chat_window.as_ref() {
            chat_window.show();
        }
    }

    fn handle_chat_actions(&mut self, actions: Vec<ChatAction>) {
        for action in actions {
            match action {
                ChatAction::Pair(code) => {
                    let _ = self
                        .network_commands
                        .send(NetworkCommand::PairWithCode(code));
                }
                ChatAction::Send(text) => {
                    let Some(recipient_id) = self.partner_id.clone() else {
                        continue;
                    };
                    let entry = chat::ChatEntry::new(true, "我".into(), text.clone());
                    if let Some(chat_window) = self.chat_window.as_mut() {
                        chat_window.push_message(entry);
                    }
                    let _ = self.network_commands.send(NetworkCommand::DirectMessage {
                        recipient_id,
                        body: MessageBody::Text { text },
                    });
                }
                ChatAction::Rename(name) => {
                    let Some(name) = config::save_display_name(&name) else {
                        continue;
                    };
                    self.display_name = name.clone();
                    if let Some(chat_window) = self.chat_window.as_mut() {
                        chat_window.set_display_name(name.clone());
                    }
                    let _ = self
                        .network_commands
                        .send(NetworkCommand::UpdateProfile(name));
                }
            }
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
                Some(MenuAction::Chat) => self.open_chat(event_loop),
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
                Some(TrayAction::Chat) => self.open_chat(event_loop),
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
            .with_inner_size(LogicalSize::new(PET_WIDTH, PET_HEIGHT))
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
        if self.open_chat_on_start {
            self.open_chat_on_start = false;
            self.open_chat(event_loop);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self
            .chat_window
            .as_ref()
            .is_some_and(|chat| chat.id() == id)
        {
            let actions = self
                .chat_window
                .as_mut()
                .and_then(|chat| {
                    chat.handle_event(event_loop, &event)
                        .map_err(|error| eprintln!("chat render error: {error:#}"))
                        .ok()
                })
                .unwrap_or_default();
            self.handle_chat_actions(actions);
            return;
        }
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
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.set_scale_factor(scale_factor);
                }
                window.request_redraw();
            }
            WindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size);
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(renderer) = self.renderer.as_mut() {
                    let petting =
                        self.pet_until.is_some_and(|until| until > Instant::now()) as u32 as f32;
                    if let Err(error) = renderer.render(
                        self.started_at.elapsed().as_secs_f32(),
                        petting,
                        skin_to_uniform(self.pet_skin),
                        self.bubble
                            .as_ref()
                            .filter(|(_, until)| *until > Instant::now())
                            .map(|(text, _)| text.as_str()),
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
                NetworkEvent::Connecting => {
                    self.network_connected = false;
                    self.latency_ms = None;
                }
                NetworkEvent::Connected => {
                    self.network_connected = true;
                    self.latency_ms = None;
                }
                NetworkEvent::Latency(rtt) => {
                    self.network_connected = true;
                    self.latency_ms = Some(rtt.as_millis() as u32);
                }
                NetworkEvent::Disconnected => {
                    self.network_connected = false;
                    self.latency_ms = None;
                }
                NetworkEvent::SharedPetAction(action) => self.apply_shared_action(action, false),
                NetworkEvent::DirectMessage {
                    sender_id,
                    sender_display_name,
                    body,
                } => {
                    if let MessageBody::Text { text } = body {
                        let sender_name = sender_display_name.unwrap_or(sender_id);
                        let entry = chat::ChatEntry::new(false, sender_name.clone(), text.clone());
                        if let Some(chat_window) = self.chat_window.as_mut() {
                            chat_window.push_message(entry);
                        } else {
                            chat::append_history(&entry);
                        }
                        self.show_bubble(format!("{sender_name}: {text}"), Duration::from_secs(8));
                    }
                }
                NetworkEvent::PairingCodeRegistered(code) => {
                    self.pairing_code = code.clone();
                    if let Some(chat_window) = self.chat_window.as_mut() {
                        chat_window.set_pairing_code(code);
                    }
                }
                NetworkEvent::Paired {
                    partner_id,
                    partner_display_name,
                    initiated_by_you,
                } => {
                    let is_new_pair = self.partner_id.as_deref() != Some(partner_id.as_str());
                    let partner_label = partner_display_name
                        .clone()
                        .unwrap_or_else(|| partner_id.clone());
                    self.partner_id = Some(partner_id.clone());
                    self.partner_display_name = partner_display_name.clone();
                    self.update_partner_labels(&partner_label);
                    if is_new_pair {
                        self.open_chat(event_loop);
                    }
                    if let Some(chat_window) = self.chat_window.as_mut() {
                        chat_window.set_partner(partner_id, partner_display_name, initiated_by_you);
                    }
                    if is_new_pair {
                        let feedback = if initiated_by_you {
                            format!("配对成功！已和 {partner_label} 共养小鸡")
                        } else {
                            format!("{partner_label} 发起了配对，已自动成功")
                        };
                        let entry = chat::ChatEntry::new(false, "系统".into(), feedback.clone());
                        if let Some(chat_window) = self.chat_window.as_mut() {
                            chat_window.push_message(entry);
                        } else {
                            chat::append_history(&entry);
                        }
                        self.show_bubble(feedback, Duration::from_secs(10));
                    }
                }
                NetworkEvent::ProfileUpdated(display_name) => {
                    self.partner_display_name = Some(display_name.clone());
                    if let Some(chat_window) = self.chat_window.as_mut() {
                        chat_window.set_partner_display_name(display_name);
                    }
                }
                NetworkEvent::Error(message) => {
                    if let Some(chat_window) = self.chat_window.as_mut() {
                        chat_window.set_status(message);
                    }
                }
            }
            self.update_network_labels();
            self.pacer.activate();
        }
        if self
            .bubble
            .as_ref()
            .is_some_and(|(_, until)| *until <= Instant::now())
        {
            self.bubble = None;
            window.request_redraw();
        }
        if let Some(chat_window) = self.chat_window.as_ref() {
            chat_window.request_resize_redraw_if_due();
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
    let display_name = config.display_name.clone();
    let open_chat_on_start = std::env::var("CHICK_OPEN_CHAT")
        .ok()
        .as_deref()
        .is_some_and(|value| matches!(value, "1" | "true" | "TRUE"));
    let (commands_tx, commands_rx) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || network::run(config, events_tx, commands_rx));
    let event_loop = EventLoop::new()?;
    event_loop.run_app(&mut PetApp::new(
        events_rx,
        commands_tx,
        partner_id,
        pairing_code,
        display_name,
        open_chat_on_start,
    ))?;
    Ok(())
}

mod network {
    use super::ClientConfig;
    use futures_util::{SinkExt, StreamExt};
    use native_tls::Certificate;
    use rsa::{RsaPublicKey, pkcs8::DecodePublicKey, traits::PublicKeyParts};
    use shared_protocol::{
        ClientEvent, ClientFrame, PROTOCOL_VERSION, PresenceState, SecureChannel, ServerEvent,
        ServerFrame, encrypt_session_key,
    };
    use std::fs;
    use std::path::Path;
    use std::sync::mpsc::Sender;
    use std::time::{Duration, Instant};
    use tokio::sync::mpsc::UnboundedReceiver;
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::{Connector, connect_async, connect_async_tls_with_config};
    use url::Url;

    pub fn run(
        config: ClientConfig,
        events: Sender<super::NetworkEvent>,
        mut commands: UnboundedReceiver<super::NetworkCommand>,
    ) {
        let runtime = tokio::runtime::Runtime::new().expect("network runtime");
        runtime.block_on(async move {
          loop {
            let relay_url = if config.discover_lan_relay {
                super::lan_discovery::find_relay(Duration::from_secs(1))
                    .unwrap_or_else(|| config.relay_url.clone())
            } else {
                config.relay_url.clone()
            };
            let _ = events.send(super::NetworkEvent::Connecting);
            let Ok((mut socket, _)) =
                connect_to_relay(&relay_url, config.tls_ca_cert_path.as_deref()).await
            else {
                let _ = events.send(super::NetworkEvent::Disconnected);
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            };
            let secure_channel = match config.server_rsa_public_key_path.as_deref() {
                Some(path) => match begin_application_encryption(&mut socket, path).await {
                    Ok(channel) => Some(channel),
                    Err(error) => {
                        let _ = events.send(super::NetworkEvent::Error(format!(
                            "无法建立加密连接：{error}"
                        )));
                        let _ = events.send(super::NetworkEvent::Disconnected);
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        continue;
                    }
                },
                None => None,
            };
            let hello = ClientEvent::Hello {
                protocol_version: PROTOCOL_VERSION,
                access_token: config.access_token.clone(),
                device_id: config.device_id.clone(),
                display_name: Some(config.display_name.clone()),
            };
            let register_code = ClientEvent::RegisterPairingCode {
                code: config.pairing_code.clone(),
            };
            let presence = ClientEvent::Presence {
                state: PresenceState::Online,
                activity: None,
            };
            let mut initialized = true;
            for event in [hello, register_code, presence] {
                if send_client_event(&mut socket, secure_channel.as_ref(), &event).await.is_err() {
                    let _ = events.send(super::NetworkEvent::Disconnected);
                    initialized = false;
                    break;
                }
            }
            if !initialized {
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
            let _ = events.send(super::NetworkEvent::Connected);
            let mut ping_timer = tokio::time::interval(Duration::from_secs(3));
            ping_timer.tick().await;
            let mut ping_sent_at = None;
            loop {
                tokio::select! {
                    _ = ping_timer.tick() => {
                        if send_client_event(&mut socket, secure_channel.as_ref(), &ClientEvent::Ping).await.is_err() {
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
                            super::NetworkCommand::UpdateProfile(display_name) => {
                                ClientEvent::UpdateProfile { display_name }
                            }
                        };
                        if send_client_event(&mut socket, secure_channel.as_ref(), &event).await.is_err() {
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
                        let event = if let Some(channel) = &secure_channel {
                            let Ok(ServerFrame::Encrypted { frame }) = serde_json::from_str(message.to_text().unwrap_or_default()) else {
                                continue;
                            };
                            channel.decrypt(&frame)
                        } else {
                            serde_json::from_str::<ServerEvent>(message.to_text().unwrap_or_default())
                                .map_err(|error| error.to_string())
                        };
                        let Ok(event) = event else {
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
                            ServerEvent::DirectMessage { sender_id, sender_display_name, body } => {
                                let _ = events.send(super::NetworkEvent::DirectMessage { sender_id, sender_display_name, body });
                            }
                            ServerEvent::PairingCodeRegistered { code } => {
                                let _ = events.send(super::NetworkEvent::PairingCodeRegistered(code));
                            }
                            ServerEvent::Paired { partner_id, partner_display_name, initiated_by_you } => {
                                let _ = events.send(super::NetworkEvent::Paired {
                                    partner_id,
                                    partner_display_name,
                                    initiated_by_you,
                                });
                            }
                            ServerEvent::ProfileUpdated { display_name, .. } => {
                                let _ = events.send(super::NetworkEvent::ProfileUpdated(display_name));
                            }
                            ServerEvent::Error { message, .. } => {
                                let _ = events.send(super::NetworkEvent::Error(message));
                            }
                            _ => {}
                        }
                    }
                }
            }
            let _ = events.send(super::NetworkEvent::Disconnected);
            tokio::time::sleep(Duration::from_secs(2)).await;
          }
        });
    }

    type RelaySocket = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    async fn begin_application_encryption(
        socket: &mut RelaySocket,
        public_key_path: &Path,
    ) -> anyhow::Result<SecureChannel> {
        let pem = fs::read_to_string(public_key_path)?;
        let public_key = RsaPublicKey::from_public_key_pem(&pem)?;
        if public_key.n().bits() < 2048 {
            anyhow::bail!("服务端 RSA 公钥必须至少为 2048 位");
        }
        let (channel, session_key) = SecureChannel::generate();
        let encrypted_session_key = encrypt_session_key(&public_key, &session_key)
            .map_err(|error| anyhow::anyhow!(error))?;
        let handshake = ClientFrame::SecureHandshake {
            encrypted_session_key,
        };
        socket
            .send(Message::Text(serde_json::to_string(&handshake)?.into()))
            .await?;
        Ok(channel)
    }

    async fn send_client_event(
        socket: &mut RelaySocket,
        secure_channel: Option<&SecureChannel>,
        event: &ClientEvent,
    ) -> anyhow::Result<()> {
        let json = if let Some(channel) = secure_channel {
            let frame = channel
                .encrypt(event)
                .map_err(|error| anyhow::anyhow!(error))?;
            serde_json::to_string(&ClientFrame::Encrypted { frame })?
        } else {
            serde_json::to_string(event)?
        };
        socket.send(Message::Text(json.into())).await?;
        Ok(())
    }

    async fn connect_to_relay(
        relay_url: &str,
        tls_ca_cert_path: Option<&Path>,
    ) -> anyhow::Result<(
        RelaySocket,
        tokio_tungstenite::tungstenite::handshake::client::Response,
    )> {
        if Url::parse(relay_url)?.scheme() != "wss" {
            return Ok(connect_async(relay_url).await?);
        }

        let connector = tls_ca_cert_path
            .map(|path| -> anyhow::Result<Connector> {
                let pem = fs::read(path)?;
                let mut builder = native_tls::TlsConnector::builder();
                for certificate in Certificate::stack_from_pem(&pem)? {
                    builder.add_root_certificate(certificate);
                }
                Ok(Connector::NativeTls(builder.build()?))
            })
            .transpose()?;

        Ok(connect_async_tls_with_config(relay_url, None, false, connector).await?)
    }
}
