#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod animation;
mod chat;
mod chat_window;
mod cloud_client;
mod config;
mod frame_pacer;
mod lan_discovery;
mod outbox;
#[cfg(not(target_os = "windows"))]
mod renderer;
mod self_update;
mod single_instance;
mod sound;
mod startup;
mod update;
mod window_position;
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
use window_position::{PositionRecorder, WindowKind};
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
    OutboxStatus {
        user_id: String,
        pending: usize,
        blocked: usize,
    },
    Cloud(shared_protocol::cloud::CloudResponse),
    Connecting,
    Connected,
    Latency(Duration),
    SharedPetAction {
        sender_id: String,
        action: SharedPetAction,
    },
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
    UnpairPending,
    UnpairRequested {
        requester_id: String,
        requester_display_name: Option<String>,
    },
    UnpairCancelled,
    Unpaired {
        former_partner_id: String,
    },
    ProfileUpdated(String),
    Presence {
        user_id: String,
        state: shared_protocol::PresenceState,
    },
    Error(String),
    Disconnected,
    UpdateChecked(Result<Option<update::Release>, String>),
    UpdateProgress(u64, Option<u64>),
    UpdateInstalled(Result<(), String>),
}

#[derive(Debug, Clone)]
struct EventSender {
    sender: mpsc::Sender<NetworkEvent>,
    wake: winit::event_loop::EventLoopProxy<()>,
}

impl EventSender {
    fn send(&self, event: NetworkEvent) -> Result<(), mpsc::SendError<NetworkEvent>> {
        self.sender.send(event)?;
        let _ = self.wake.send_event(());
        Ok(())
    }
}

#[derive(Debug)]
enum NetworkCommand {
    Cloud {
        request: shared_protocol::cloud::CloudRequest,
        created_at: Instant,
    },
    SharedPetAction {
        recipient_id: String,
        action: SharedPetAction,
        created_at: Instant,
    },
    DirectMessage {
        recipient_id: String,
        body: MessageBody,
    },
    PairWithCode(String),
    RequestUnpair,
    RespondUnpair(bool),
    UpdateProfile(String),
}

// UI and network share one locked queue: neither can overwrite the other's acknowledgement.
fn with_outbox<T>(
    session: &cloud_client::Session,
    action: impl FnOnce(&mut outbox::Outbox) -> std::io::Result<T>,
) -> std::io::Result<T> {
    type Cached = Option<(String, String, outbox::Outbox)>;
    static QUEUE: std::sync::Mutex<Cached> = std::sync::Mutex::new(None);
    let mut cached = QUEUE
        .lock()
        .map_err(|_| std::io::Error::other("待发送队列不可用"))?;
    if !cached
        .as_ref()
        .is_some_and(|(server, user, _)| server == &session.server_url && user == &session.user_id)
    {
        *cached = Some((
            session.server_url.clone(),
            session.user_id.clone(),
            outbox::Outbox::load(&session.server_url, &session.user_id)?,
        ));
    }
    action(&mut cached.as_mut().unwrap().2)
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

    fn set_partner(&self, partner: Option<&str>) {
        self.owners.set_text(
            partner
                .map(|partner| format!("共同小鸡 · 已配对给 {partner}"))
                .unwrap_or_else(|| "共同小鸡 · 尚未配对".into()),
        );
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

    fn set_partner(&self, partner: Option<&str>) {
        self.owners.set_text(
            partner
                .map(|partner| format!("共同小鸡 · 已配对给 {partner}"))
                .unwrap_or_else(|| "共同小鸡 · 尚未配对".into()),
        );
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

    fn set_partner(&self, partner: Option<&str>) {
        self.owners.set_text(
            partner
                .map(|partner| format!("共同小鸡 · 已配对给 {partner}"))
                .unwrap_or_else(|| "共同小鸡 · 尚未配对".into()),
        );
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
    cloud_user: Option<String>,
    sounds: sound::Sounds,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    started_at: Instant,
    press_started_at: Option<Instant>,
    pet_until: Option<Instant>,
    pet_skin: PetSkin,
    wander: animation::Wander,
    dragging: bool,
    pacer: FramePacer,
    network_events: mpsc::Receiver<NetworkEvent>,
    network_commands: tokio::sync::mpsc::UnboundedSender<NetworkCommand>,
    partner_id: Option<String>,
    partner_display_name: Option<String>,
    partner_status: &'static str,
    display_name: String,
    pairing_code: String,
    bubble: Option<(String, Instant)>,
    // Keep this across reconnect attempts so a persistent configuration error is shown once.
    security_warning_bubble: Option<String>,
    update_url: String,
    update_sender: EventSender,
    update_check_at: Instant,
    update_check_started: bool,
    update_check_running: bool,
    update_install_running: bool,
    update_exit_pending: bool,
    update_result: Option<Result<Option<update::Release>, String>>,
    pet_position: PositionRecorder,
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
        update_url: String,
        update_sender: EventSender,
    ) -> Self {
        Self {
            cloud_user: None,
            window: None,
            sounds: sound::Sounds::new(),
            renderer: None,
            started_at: Instant::now(),
            press_started_at: None,
            pet_until: None,
            pet_skin: PetSkin::Sunshine,
            wander: animation::Wander::new(Instant::now()),
            dragging: false,
            pacer: FramePacer::new(),
            network_events,
            network_commands,
            partner_id,
            partner_display_name: None,
            partner_status: "状态未知",
            display_name,
            pairing_code,
            bubble: None,
            security_warning_bubble: None,
            update_url,
            update_sender,
            update_check_at: Instant::now() + Duration::from_secs(30),
            update_check_started: false,
            update_check_running: false,
            update_install_running: false,
            update_exit_pending: false,
            update_result: None,
            pet_position: PositionRecorder::new(WindowKind::Pet),
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
                self.sounds.play(sound::Cue::Pet);
                if !share_with_partner {
                    let name = self.partner_display_name.as_deref().unwrap_or("对方");
                    let feedback = format!("{name} 摸摸了小鸡");
                    if let Some(chat) = self.chat_window.as_mut() {
                        chat.set_status(feedback.clone());
                    }
                    if self
                        .bubble
                        .as_ref()
                        .is_none_or(|(_, until)| *until <= Instant::now())
                    {
                        self.show_bubble(feedback, Duration::from_secs(2));
                    }
                }
            }
            SharedPetAction::SetSkin { skin } => self.pet_skin = skin,
        }
        if share_with_partner && self.network_connected {
            if let Some(recipient_id) = self.partner_id.clone() {
                let _ = self.network_commands.send(NetworkCommand::SharedPetAction {
                    recipient_id,
                    action,
                    created_at: Instant::now(),
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

    fn start_update_check(&mut self) {
        if self.update_check_running {
            return;
        }
        self.update_check_running = true;
        update::mark_checked();
        if let Some(chat) = self.chat_window.as_mut() {
            chat.set_update_checking();
        }
        let url = self.update_url.clone();
        let sender = self.update_sender.clone();
        std::thread::spawn(move || {
            let _ = sender.send(NetworkEvent::UpdateChecked(update::check(&url)));
        });
    }

    fn clear_security_warning(&mut self) {
        let Some(warning) = self.security_warning_bubble.take() else {
            return;
        };
        if self
            .bubble
            .as_ref()
            .is_some_and(|(text, _)| text == &warning)
        {
            self.bubble = None;
            if let Some(window) = self.window.as_ref() {
                window.request_redraw();
            }
        }
        if let Some(chat_window) = self.chat_window.as_mut() {
            chat_window.set_status("连接已恢复");
        }
    }

    fn update_frame_rate(&mut self, window: &Window) {
        self.pacer.set_refresh_rate(
            window
                .current_monitor()
                .and_then(|monitor| monitor.refresh_rate_millihertz()),
        );
    }

    fn update_partner_labels(&self, partner: Option<&str>) {
        #[cfg(target_os = "macos")]
        if let Some(menu) = self.menu.as_ref() {
            menu.set_partner(partner);
        }
        #[cfg(target_os = "windows")]
        {
            if let Some(tray) = self.tray.as_ref() {
                tray.set_partner(partner);
            }
            if let Some(menu) = self.windows_menu.as_ref() {
                menu.set_partner(partner);
            }
        }
    }

    fn update_network_labels(&mut self) {
        let label = self.network_label();
        if let Some(partner) = self.partner_id.as_deref() {
            let name = self.partner_display_name.as_deref().unwrap_or(partner);
            let status = if self.network_connected {
                self.partner_status
            } else {
                "状态未知"
            };
            self.update_partner_labels(Some(&format!("{name} · {status}")));
        }
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
            chat_window.set_partner_status(if self.network_connected {
                self.partner_status
            } else {
                "状态未知（本机连接已断开）"
            });
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
                Ok(mut window) => {
                    if let Some(result) = self.update_result.clone() {
                        window.set_update_result(result);
                    }
                    self.chat_window = Some(window);
                }
                Err(error) => {
                    eprintln!("chat window error: {error:#}");
                    return;
                }
            }
        }
        if let Some(chat_window) = self.chat_window.as_mut() {
            chat_window.set_partner_status(if self.network_connected {
                self.partner_status
            } else {
                "状态未知（本机连接已断开）"
            });
            chat_window.show();
        }
    }

    fn handle_chat_actions(&mut self, actions: Vec<ChatAction>) {
        for action in actions {
            match action {
                ChatAction::CheckForUpdates => self.start_update_check(),
                ChatAction::SetUpdateCheckEnabled(enabled) => {
                    let error = update::set_enabled(enabled).err().map(|e| e.to_string());
                    if let Some(chat) = self.chat_window.as_mut() {
                        chat.finish_update_setting(enabled, error);
                    }
                }
                ChatAction::OpenReleasePage => {
                    if let Some(chat) = self.chat_window.as_ref() {
                        if let Some(url) = chat.update_page_url() {
                            let _ = update::open_page(url);
                        }
                    }
                }
                ChatAction::InstallUpdate => {
                    if self.update_install_running {
                        continue;
                    }
                    let Some(release) = self
                        .update_result
                        .as_ref()
                        .and_then(|r| r.as_ref().ok())
                        .and_then(|r| r.as_ref())
                        .cloned()
                    else {
                        continue;
                    };
                    let can_install = release.asset.as_ref().is_some_and(|asset| {
                        #[cfg(target_os = "macos")]
                        {
                            asset.name.ends_with(".zip")
                        }
                        #[cfg(target_os = "windows")]
                        {
                            asset.name.ends_with(".exe")
                        }
                        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                        {
                            let _ = asset;
                            false
                        }
                    });
                    if !can_install {
                        continue;
                    }
                    if let Err(error) = self_update::preflight() {
                        if let Some(chat) = self.chat_window.as_mut() {
                            chat.set_update_status(error);
                        }
                        let _ = update::open_page(&release.page_url);
                        continue;
                    }
                    self.update_install_running = true;
                    if let Some(chat) = self.chat_window.as_mut() {
                        chat.set_update_status("正在下载更新…".into());
                    }
                    let sender = self.update_sender.clone();
                    let source = self.update_url.clone();
                    std::thread::spawn(move || {
                        let result = update::download_verified(&release, &source, |done, total| {
                            let _ = sender.send(NetworkEvent::UpdateProgress(done, total));
                        })
                        .and_then(|path| {
                            let result = self_update::install(&path);
                            if result.is_err() {
                                if let Some(dir) = path.parent() {
                                    let _ = std::fs::remove_dir_all(dir);
                                }
                            }
                            result
                        });
                        let _ = sender.send(NetworkEvent::UpdateInstalled(result));
                    });
                }
                ChatAction::Cloud(request) => {
                    self.submit_cloud_request(request);
                }
                ChatAction::RetryOutbox => self.edit_outbox(false),
                ChatAction::ClearOutbox => self.edit_outbox(true),
                ChatAction::SetSoundTheme(theme) => {
                    let error = self.sounds.set_theme(theme).err().map(|e| e.to_string());
                    if let Some(chat) = self.chat_window.as_mut() {
                        chat.finish_sound_theme(self.sounds.theme(), error);
                    }
                    self.sounds.play(sound::Cue::Pet);
                }
                ChatAction::SetSoundsEnabled(enabled) => {
                    let error = self
                        .sounds
                        .set_enabled(enabled)
                        .err()
                        .map(|error| error.to_string());
                    if let Some(chat) = self.chat_window.as_mut() {
                        chat.finish_sound_change(self.sounds.enabled(), error);
                    }
                }
                ChatAction::Pair(code) => {
                    let _ = self
                        .network_commands
                        .send(NetworkCommand::PairWithCode(code));
                }
                ChatAction::RequestUnpair => {
                    let _ = self.network_commands.send(NetworkCommand::RequestUnpair);
                }
                ChatAction::RespondUnpair(accept) => {
                    let _ = self
                        .network_commands
                        .send(NetworkCommand::RespondUnpair(accept));
                }
                ChatAction::SetLaunchAtLogin(enabled) => {
                    let error = startup::set_enabled(enabled)
                        .err()
                        .map(|error| error.to_string());
                    if let Some(chat_window) = self.chat_window.as_mut() {
                        chat_window.finish_startup_change(enabled, error);
                    }
                }
                ChatAction::Send(text) => {
                    let Some(recipient_id) = self.partner_id.clone() else {
                        continue;
                    };
                    if self.cloud_user.is_some() {
                        self.submit_cloud_request(
                            shared_protocol::cloud::CloudRequest::SendMessage {
                                client_id: cloud_client::new_message_id(),
                                recipient_id,
                                body: shared_protocol::cloud::CloudBody::Text { text },
                            },
                        );
                        continue;
                    }
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

    fn submit_cloud_request(&mut self, request: shared_protocol::cloud::CloudRequest) {
        if let shared_protocol::cloud::CloudRequest::SendMessage {
            client_id,
            recipient_id,
            body,
        } = request
        {
            let result = cloud_client::load_session()
                .filter(|session| Some(&session.user_id) == self.cloud_user.as_ref())
                .ok_or_else(|| std::io::Error::other("请先登录账号"))
                .and_then(|session| {
                    with_outbox(&session, |queue| {
                        queue.enqueue(client_id, recipient_id, body.clone())?;
                        Ok((queue.len(), queue.blocked_count()))
                    })
                });
            if let Some(chat) = self.chat_window.as_mut() {
                match result {
                    Ok((pending, blocked)) => {
                        chat.set_outbox_status(pending, blocked);
                        chat.set_status("消息已保存在本机待发送，云端确认后显示到历史");
                    }
                    Err(error) => {
                        if let shared_protocol::cloud::CloudBody::Text { text } = body {
                            chat.restore_unsent_text(text);
                        }
                        chat.outbox_error(format!("未能加入待发送队列：{error}"));
                    }
                }
            }
        } else {
            let _ = self.network_commands.send(NetworkCommand::Cloud {
                request,
                created_at: Instant::now(),
            });
        }
    }

    fn edit_outbox(&mut self, clear: bool) {
        let result = cloud_client::load_session()
            .filter(|session| Some(&session.user_id) == self.cloud_user.as_ref())
            .ok_or_else(|| std::io::Error::other("请先登录账号"))
            .and_then(|session| {
                with_outbox(&session, |queue| {
                    if clear {
                        queue.clear()?;
                    } else {
                        queue.retry_all()?;
                    }
                    Ok((queue.len(), queue.blocked_count()))
                })
            });
        if let Some(chat) = self.chat_window.as_mut() {
            match result {
                Ok((pending, blocked)) => chat.set_outbox_status(pending, blocked),
                Err(error) => chat.outbox_error(error.to_string()),
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
        let attributes = event_loop
            .primary_monitor()
            .map(|monitor| {
                attributes
                    .clone()
                    .with_position(initial_pet_position(&monitor))
            })
            .unwrap_or(attributes);
        let attributes =
            window_position::restore(event_loop, WindowKind::Pet, (PET_WIDTH, PET_HEIGHT))
                .map(|position| attributes.clone().with_position(position))
                .unwrap_or(attributes);
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .expect("create desktop pet window"),
        );
        #[cfg(target_os = "windows")]
        enable_windows_transparency(&window);
        self.renderer =
            Some(pollster::block_on(Renderer::new(window.clone())).expect("create GPU renderer"));
        self.update_frame_rate(&window);
        self.window = Some(window.clone());
        #[cfg(target_os = "macos")]
        self_update::clean_successful_backup();
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
            WindowEvent::CloseRequested => {
                self.pet_position.flush();
                event_loop.exit();
            }
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
                self.dragging = true;
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
                self.dragging = false;
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
                self.update_frame_rate(&window);
                window.request_redraw();
            }
            WindowEvent::Moved(position) => {
                self.update_frame_rate(&window);
                if self.dragging {
                    self.pet_position.moved(position);
                }
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
        if self.update_exit_pending {
            self.pet_position.flush();
            event_loop.exit();
            return;
        }
        let Some(window) = self.window.clone() else {
            return;
        };
        if !self.update_check_started && Instant::now() >= self.update_check_at {
            self.update_check_started = true;
            if update::auto_check_due() {
                self.start_update_check();
            }
        }
        self.pet_position.flush_if_due(Instant::now());
        if let (Some(monitor), Ok(position)) = (window.current_monitor(), window.outer_position()) {
            let screen = monitor.size();
            let before = self.wander.frame();
            if let Some(x) = self.wander.advance(
                Instant::now(),
                position.x,
                monitor.position().x,
                screen.width,
                window.outer_size().width,
                window.scale_factor(),
                self.dragging || window.is_visible() == Some(false),
            ) {
                window.set_outer_position(winit::dpi::PhysicalPosition::new(x, position.y));
            }
            if self.wander.frame() != before {
                window.request_redraw();
            }
        }
        while let Ok(event) = self.network_events.try_recv() {
            match event {
                NetworkEvent::OutboxStatus {
                    user_id,
                    pending,
                    blocked,
                } => {
                    if self.cloud_user.as_deref() == Some(user_id.as_str()) {
                        if let Some(chat) = self.chat_window.as_mut() {
                            chat.set_outbox_status(pending, blocked);
                        }
                    }
                }
                NetworkEvent::Cloud(response) => {
                    use shared_protocol::cloud::{CloudBody, CloudResponse};
                    if matches!(&response, CloudResponse::Authenticated { .. }) {
                        self.clear_security_warning();
                    }
                    match &response {
                        CloudResponse::Authenticated { user_id, .. } => {
                            self.cloud_user = Some(user_id.clone())
                        }
                        CloudResponse::LoggedOut => self.cloud_user = None,
                        _ => {}
                    }
                    match &response {
                        CloudResponse::Authenticated { .. } | CloudResponse::LoggedOut => {
                            self.partner_id = None;
                            self.partner_display_name = None;
                            self.partner_status = "状态未知";
                            self.update_partner_labels(None);
                            if matches!(
                                &response,
                                CloudResponse::LoggedOut
                                    | CloudResponse::Authenticated {
                                        recovery_code: Some(_),
                                        ..
                                    }
                            ) {
                                self.open_chat(event_loop);
                            }
                            if let Some(chat) = self.chat_window.as_mut() {
                                chat.set_unpaired();
                            }
                        }
                        CloudResponse::Message { item } => {
                            if self.cloud_user.as_deref() == Some(item.recipient_id.as_str()) {
                                let text = match &item.body {
                                    CloudBody::Text { text } => text.clone(),
                                    CloudBody::Image { .. } => "收到图片表情".into(),
                                };
                                self.show_bubble(text, Duration::from_secs(8));
                                self.sounds.play(sound::Cue::Message);
                            }
                        }
                        _ => {}
                    }
                    if let Some(chat) = self.chat_window.as_mut() {
                        chat.handle_cloud(&response);
                    }
                }
                NetworkEvent::Connecting => {
                    self.partner_status = "状态未知";
                    self.network_connected = false;
                    self.latency_ms = None;
                }
                NetworkEvent::Connected => {
                    self.clear_security_warning();
                    self.network_connected = true;
                    self.latency_ms = None;
                }
                NetworkEvent::Latency(rtt) => {
                    self.network_connected = true;
                    self.latency_ms = Some(rtt.as_millis() as u32);
                }
                NetworkEvent::Disconnected => {
                    self.partner_status = "状态未知";
                    self.network_connected = false;
                    self.latency_ms = None;
                }
                NetworkEvent::SharedPetAction { sender_id, action } => {
                    if self.partner_id.as_deref() == Some(sender_id.as_str())
                        || self.cloud_user.as_deref() == Some(sender_id.as_str())
                    {
                        self.apply_shared_action(action, false);
                    }
                }
                NetworkEvent::Presence { user_id, state } => {
                    if self.partner_id.as_deref() == Some(user_id.as_str()) {
                        let status = match state {
                            shared_protocol::PresenceState::Online => "在线",
                            shared_protocol::PresenceState::Away => "暂离",
                            shared_protocol::PresenceState::Offline => "离线",
                        };
                        let changed = self.partner_status != status;
                        self.partner_status = status;
                        if changed && self.bubble.is_none() {
                            self.show_bubble(format!("对方{status}"), Duration::from_secs(4));
                        }
                    }
                }
                NetworkEvent::DirectMessage {
                    sender_id,
                    sender_display_name,
                    body,
                } => {
                    if let MessageBody::Text { text } = body {
                        self.sounds.play(sound::Cue::Message);
                        let sender_name = sender_display_name.unwrap_or(sender_id);
                        let entry = chat::ChatEntry::new(false, sender_name, text.clone());
                        if let Some(chat_window) = self.chat_window.as_mut() {
                            chat_window.push_message(entry);
                        } else {
                            chat::append_history(&entry);
                        }
                        self.show_bubble(text, Duration::from_secs(8));
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
                    self.update_partner_labels(Some(&partner_label));
                    if is_new_pair {
                        self.partner_status = "状态未知";
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
                NetworkEvent::UnpairPending => {
                    if let Some(chat_window) = self.chat_window.as_mut() {
                        chat_window.set_unpair_pending();
                    }
                    self.show_bubble("解除申请已发出，等待对方确认", Duration::from_secs(8));
                }
                NetworkEvent::UnpairRequested {
                    requester_id,
                    requester_display_name,
                } => {
                    let requester_name = requester_display_name.unwrap_or(requester_id);
                    self.open_chat(event_loop);
                    if let Some(chat_window) = self.chat_window.as_mut() {
                        chat_window.show_unpair_request(requester_name.clone());
                    }
                    self.show_bubble(
                        format!("{requester_name} 请求解除配对，请确认"),
                        Duration::from_secs(12),
                    );
                }
                NetworkEvent::UnpairCancelled => {
                    if let Some(chat_window) = self.chat_window.as_mut() {
                        chat_window.set_unpair_cancelled();
                    }
                    self.show_bubble("解除配对已取消", Duration::from_secs(8));
                }
                NetworkEvent::Unpaired { former_partner_id } => {
                    let former_partner = self
                        .partner_display_name
                        .clone()
                        .unwrap_or(former_partner_id);
                    self.partner_id = None;
                    self.partner_status = "状态未知";
                    self.partner_display_name = None;
                    self.update_partner_labels(None);
                    self.open_chat(event_loop);
                    if let Some(chat_window) = self.chat_window.as_mut() {
                        chat_window.set_unpaired();
                    }
                    let feedback = format!("已与 {former_partner} 解除共同小鸡关系");
                    let entry = chat::ChatEntry::new(false, "系统".into(), feedback.clone());
                    if let Some(chat_window) = self.chat_window.as_mut() {
                        chat_window.push_message(entry);
                    } else {
                        chat::append_history(&entry);
                    }
                    self.show_bubble(feedback, Duration::from_secs(10));
                }
                NetworkEvent::ProfileUpdated(display_name) => {
                    self.partner_display_name = Some(display_name.clone());
                    if let Some(chat_window) = self.chat_window.as_mut() {
                        chat_window.set_partner_display_name(display_name);
                    }
                }
                NetworkEvent::Error(message) => {
                    if (message.contains("RSA 加密") || message.contains("加密握手"))
                        && self.security_warning_bubble.is_none()
                    {
                        self.show_bubble(message.clone(), Duration::from_secs(10));
                        self.security_warning_bubble = Some(message.clone());
                    }
                    if let Some(chat_window) = self.chat_window.as_mut() {
                        chat_window.set_status(message);
                    }
                }
                NetworkEvent::UpdateChecked(result) => {
                    self.update_check_running = false;
                    if let Ok(Some(release)) = &result {
                        update::remember_latest(&release.version);
                        if update::should_notify(&release.version) {
                            self.show_bubble(
                                format!("发现新版本 {}", release.version),
                                Duration::from_secs(8),
                            );
                            update::mark_notified(&release.version);
                        }
                    }
                    if let Some(chat) = self.chat_window.as_mut() {
                        chat.set_update_result(result.clone());
                    }
                    self.update_result = Some(result);
                }
                NetworkEvent::UpdateProgress(done, total) => {
                    if let Some(chat) = self.chat_window.as_mut() {
                        let label = total.map_or_else(
                            || format!("已下载 {} MB", done / 1_000_000),
                            |total| {
                                format!("已下载 {} / {} MB", done / 1_000_000, total / 1_000_000)
                            },
                        );
                        chat.set_update_status(label);
                    }
                }
                NetworkEvent::UpdateInstalled(result) => {
                    self.update_install_running = false;
                    match result {
                        Ok(()) => {
                            self.update_exit_pending = true;
                        }
                        Err(error) => {
                            if let Some(chat) = self.chat_window.as_mut() {
                                chat.set_update_status(format!(
                                    "更新失败：{error}。可打开发布页手动安装"
                                ));
                            }
                        }
                    }
                }
            }
            self.update_network_labels();
        }
        if self
            .bubble
            .as_ref()
            .is_some_and(|(_, until)| *until <= Instant::now())
        {
            self.bubble = None;
            window.request_redraw();
        }
        if let Some(chat_window) = self.chat_window.as_mut() {
            chat_window.process_deferred_actions();
        }
        #[cfg(target_os = "macos")]
        self.handle_menu_events(event_loop);
        #[cfg(target_os = "windows")]
        self.handle_tray_events(event_loop);
        let (redraw, next_wake) = self.pacer.schedule();
        if redraw {
            window.request_redraw();
        }
        let next_wake = self
            .pet_position
            .deadline()
            .map_or(next_wake, |deadline| next_wake.min(deadline));
        let next_wake = self
            .chat_window
            .as_ref()
            .and_then(ChatWindow::position_deadline)
            .map_or(next_wake, |deadline| next_wake.min(deadline));
        let next_wake = if self.update_check_started {
            next_wake
        } else {
            next_wake.min(self.update_check_at)
        };
        event_loop.set_control_flow(ControlFlow::WaitUntil(next_wake));
    }
}

fn initial_pet_position(
    monitor: &winit::monitor::MonitorHandle,
) -> winit::dpi::PhysicalPosition<i32> {
    let origin = monitor.position();
    let screen = monitor.size();
    let scale = monitor.scale_factor();
    let pet_width = (PET_WIDTH * scale).round() as i32;
    let pet_height = (PET_HEIGHT * scale).round() as i32;
    let right_margin = (32.0 * scale).round() as i32;
    let bottom_margin = (88.0 * scale).round() as i32;
    winit::dpi::PhysicalPosition::new(
        (origin.x + screen.width as i32 - pet_width - right_margin).max(origin.x),
        (origin.y + screen.height as i32 - pet_height - bottom_margin).max(origin.y),
    )
}

fn skin_to_uniform(skin: PetSkin) -> f32 {
    match skin {
        PetSkin::Sunshine => 0.0,
        PetSkin::Blueberry => 1.0,
        PetSkin::Cocoa => 2.0,
    }
}

fn main() -> anyhow::Result<()> {
    let _instance_guard = match single_instance::acquire()? {
        Some(guard) => guard,
        None => {
            eprintln!("Chick Link is already running; exiting second instance");
            std::process::exit(10);
        }
    };
    let config = ClientConfig::load()?;
    let (events_tx, events_rx) = mpsc::channel();
    let partner_id = config.partner_id.clone();
    let pairing_code = config.pairing_code.clone();
    let display_name = config.display_name.clone();
    let open_chat_on_start = std::env::var("CHICK_OPEN_CHAT")
        .ok()
        .as_deref()
        .is_some_and(|value| matches!(value, "1" | "true" | "TRUE"))
        || (config.access_token.is_empty() && cloud_client::load_session().is_none());
    let (commands_tx, commands_rx) = tokio::sync::mpsc::unbounded_channel();
    let event_loop = EventLoop::new()?;
    let sender = EventSender {
        sender: events_tx,
        wake: event_loop.create_proxy(),
    };
    let update_url = config.update_url.clone();
    let update_sender = sender.clone();
    std::thread::spawn(move || network::run(config, sender, commands_rx));
    event_loop.run_app(&mut PetApp::new(
        events_rx,
        commands_tx,
        partner_id,
        pairing_code,
        display_name,
        open_chat_on_start,
        update_url,
        update_sender,
    ))?;
    Ok(())
}

mod network {
    use super::ClientConfig;
    use futures_util::{SinkExt, StreamExt};
    use native_tls::Certificate;
    use rsa::{RsaPublicKey, pkcs8::DecodePublicKey, traits::PublicKeyParts};
    use shared_protocol::cloud::{CloudRequest, CloudResponse};
    use shared_protocol::{
        ClientEvent, ClientFrame, PROTOCOL_VERSION, PresenceState, SecureChannel, ServerEvent,
        ServerFrame, encrypt_session_key,
    };
    use std::fs;
    use std::path::Path;
    use std::time::{Duration, Instant};
    use tokio::sync::mpsc::UnboundedReceiver;
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::{Connector, connect_async, connect_async_tls_with_config};
    use url::Url;

    pub fn run(
        config: ClientConfig,
        events: super::EventSender,
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
            let Ok(Ok((mut socket, _))) =
                tokio::time::timeout(Duration::from_secs(10), connect_to_relay(&relay_url, config.tls_ca_cert_path.as_deref())).await
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
                None if relay_url == "wss://pet.narendra.cn:18443/ws" => {
                    match begin_encryption_pem(&mut socket, include_str!("../assets/relay-public.pem")).await {
                        Ok(channel) => Some(channel),
                        Err(_) => { let _ = events.send(super::NetworkEvent::Error("云端加密握手失败".into())); tokio::time::sleep(Duration::from_secs(2)).await; continue; }
                    }
                }
                None => None,
            };
            let mut account = super::cloud_client::load_session().filter(|session| session.server_url == relay_url);
            if secure_channel.is_none() && (!config.access_token.is_empty() || account.is_some()) {
                let _ = events.send(super::NetworkEvent::Error(
                    "此连接没有 RSA 加密，已停止发送令牌；请检查中继地址和公钥配置".into(),
                ));
                let _ = events.send(super::NetworkEvent::Disconnected);
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
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
            let initial = if let Some(session) = &account {
                Some(ClientEvent::Cloud { request: CloudRequest::Resume { token: session.token.clone(), device_id: super::cloud_client::device_id() } })
            } else if !config.access_token.is_empty() { Some(hello) } else { None };
            if let Some(event) = initial {
                if send_client_event(&mut socket, secure_channel.as_ref(), &event).await.is_err() {
                    let _ = events.send(super::NetworkEvent::Disconnected);
                    initialized = false;
                }
            }
            if !initialized {
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
            let mut ping_timer = tokio::time::interval(Duration::from_secs(3));
            ping_timer.tick().await;
            let mut ping_sent_at = None;
            let mut last_pong = Instant::now();
            let mut last_sync = Instant::now();
            let mut last_resume = Instant::now();
            let mut authenticated = false;
            let mut syncing = false;
            let mut load_stickers_after_sync = false;
            let mut outbox_in_flight: Option<(String, Instant)> = None;
            let mut outbox_retry_after = Instant::now();
            loop {
                tokio::select! {
                    _ = ping_timer.tick() => {
                        if last_pong.elapsed() > Duration::from_secs(12) {
                            break;
                        }
                        if send_client_event(&mut socket, secure_channel.as_ref(), &ClientEvent::Ping).await.is_err() {
                            break;
                        }
                        ping_sent_at = Some(Instant::now());
                        if authenticated && secure_channel.is_some() {
                            if let Some(session) = &account {
                                let next = super::with_outbox(session, |queue| {
                                    let _ = events.send(super::NetworkEvent::OutboxStatus { user_id: session.user_id.clone(), pending: queue.len(), blocked: queue.blocked_count() });
                                    Ok(queue.next().map(|entry| (entry.client_id.clone(), entry.recipient_id.clone(), entry.body.clone())))
                                });
                                match next {
                                    Ok(Some((client_id, recipient_id, body))) if Instant::now() >= outbox_retry_after && outbox_in_flight.as_ref().is_none_or(|(_, sent)| sent.elapsed() > Duration::from_secs(15)) => {
                                        let request = CloudRequest::SendMessage { client_id: client_id.clone(), recipient_id, body };
                                        if send_client_event(&mut socket, secure_channel.as_ref(), &ClientEvent::Cloud { request }).await.is_err() { break; }
                                        outbox_in_flight = Some((client_id, Instant::now()));
                                    }
                                    Err(_) => { let _ = events.send(super::NetworkEvent::Error("无法读取待发送队列，请检查磁盘权限；原文件未被覆盖".into())); }
                                    _ => {}
                                }
                            }
                        }
                        if !authenticated && last_resume.elapsed() > Duration::from_secs(6) {
                            if let Some(session) = &account {
                                let request = CloudRequest::Resume { token: session.token.clone(), device_id: super::cloud_client::device_id() };
                                if send_client_event(&mut socket, secure_channel.as_ref(), &ClientEvent::Cloud { request }).await.is_err() { break; }
                            }
                            last_resume = Instant::now();
                        }
                        if authenticated && !syncing && last_sync.elapsed() > Duration::from_secs(30) {
                            if let Some(session) = &account {
                                let request = CloudRequest::Sync { after: super::cloud_client::cursor(session) };
                                if send_client_event(&mut socket, secure_channel.as_ref(), &ClientEvent::Cloud { request }).await.is_err() { break; }
                                syncing = true;
                            }
                            last_sync = Instant::now();
                        }
                    }
                    Some(command) = commands.recv() => {
                        let event = match command {
                            super::NetworkCommand::Cloud { mut request, created_at } => {
                                if created_at.elapsed() > Duration::from_secs(20) {
                                    let _ = events.send(super::NetworkEvent::Cloud(CloudResponse::Error { message: "操作因断线等待过久，已取消；请重试".into() }));
                                    continue;
                                }
                                if secure_channel.is_none() { let _ = events.send(super::NetworkEvent::Error("账号操作需要 RSA 加密连接".into())); continue; }
                                match &mut request {
                                    CloudRequest::Register { device_id, legacy_token, .. } => {
                                        *device_id = super::cloud_client::device_id();
                                        if !config.access_token.is_empty() && account.is_none() { *legacy_token = Some(config.access_token.clone()); }
                                    }
                                    CloudRequest::Login { device_id, .. } | CloudRequest::Recover { device_id, .. } => *device_id = super::cloud_client::device_id(),
                                    _ => {}
                                }
                                ClientEvent::Cloud { request }
                            }
                            super::NetworkCommand::SharedPetAction { recipient_id, action, created_at } => {
                                if matches!(action, shared_protocol::SharedPetAction::Pet) && created_at.elapsed() > Duration::from_secs(1) { continue; }
                                ClientEvent::SharedPetAction { recipient_id, action }
                            }
                            super::NetworkCommand::DirectMessage { recipient_id, body } => {
                                ClientEvent::DirectMessage { recipient_id, body }
                            }
                            super::NetworkCommand::PairWithCode(code) => ClientEvent::PairWithCode { code },
                            super::NetworkCommand::RequestUnpair => ClientEvent::RequestUnpair,
                            super::NetworkCommand::RespondUnpair(accept) => {
                                ClientEvent::RespondUnpair { accept }
                            }
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
                            ServerEvent::Cloud { response } => {
                                let mut duplicate_message = false;
                                let persisted: std::io::Result<()> = match &response {
                                    CloudResponse::Authenticated { user_id, username, token, .. } => {
                                        outbox_in_flight = None;
                                        outbox_retry_after = Instant::now();
                                        let session = super::cloud_client::Session { user_id: user_id.clone(), username: username.clone(), token: token.clone(), server_url: relay_url.clone() };
                                        let result = super::cloud_client::save_session(&session);
                                        account = Some(session);
                                        result
                                    }
                                    CloudResponse::LoggedOut => { account = None; authenticated = false; syncing = false; outbox_in_flight = None; super::cloud_client::clear_session() }
                                    CloudResponse::Message { item } => {
                                        if let Some(session) = &account {
                                            super::cloud_client::append_cloud_history(session, item).and_then(|inserted| {
                                                duplicate_message = !inserted;
                                                if item.sender_id == session.user_id {
                                                    super::with_outbox(session, |queue| queue.ack(&item.client_id))?;
                                                    if outbox_in_flight.as_ref().is_some_and(|(id, _)| id == &item.client_id) { outbox_in_flight = None; }
                                                }
                                                Ok(())
                                            })
                                        } else { Ok(()) }
                                    }
                                    CloudResponse::SendRejected { client_id, retryable, .. } => {
                                        // Only a rejection for this connection's outstanding send can change its retry state.
                                        if outbox_in_flight.as_ref().is_some_and(|(id, _)| id == client_id) {
                                            outbox_in_flight = None;
                                            if *retryable { outbox_retry_after = Instant::now() + Duration::from_secs(60); Ok(()) }
                                            else if let Some(session) = &account { super::with_outbox(session, |queue| queue.block(client_id)) }
                                            else { Ok(()) }
                                        } else { Ok(()) }
                                    }
                                    CloudResponse::History { items, next_cursor, .. } => {
                                        if let Some(session) = &account {
                                            items.iter().try_for_each(|item| super::cloud_client::append_cloud_history(session, item).map(|_| ()))
                                                .and_then(|_| super::cloud_client::save_cursor(session, *next_cursor))
                                        } else { Ok(()) }
                                    }
                                    _ => Ok(()),
                                };
                                if persisted.is_err() {
                                    let _ = events.send(super::NetworkEvent::Error("无法保存本机账号或历史，请检查磁盘权限；同步游标未推进".into()));
                                    syncing = false;
                                }
                                if let CloudResponse::History { next_cursor, has_more, .. } = &response {
                                    syncing = false;
                                    if persisted.is_ok() && *has_more {
                                        let request = CloudRequest::Sync { after: *next_cursor };
                                        if send_client_event(&mut socket, secure_channel.as_ref(), &ClientEvent::Cloud { request }).await.is_err() { break; }
                                        syncing = true;
                                    } else if load_stickers_after_sync {
                                        load_stickers_after_sync = false;
                                        let _ = send_client_event(&mut socket, secure_channel.as_ref(), &ClientEvent::Cloud { request: CloudRequest::ListStickers }).await;
                                    }
                                    last_sync = Instant::now();
                                }
                                if matches!(response, CloudResponse::Error { .. }) { syncing = false; }
                                let response = if duplicate_message {
                                    if let CloudResponse::Message { item } = response {
                                        // Merge retries into the UI without repeating the incoming sound or bubble.
                                        CloudResponse::History { items: vec![item], next_cursor: 0, has_more: false }
                                    } else { response }
                                } else { response };
                                let _ = events.send(super::NetworkEvent::Cloud(response));
                            }
                            ServerEvent::Ready { .. } => {
                                authenticated = true;
                                let _ = events.send(super::NetworkEvent::Connected);
                                for event in [&register_code, &presence] {
                                    let _ = send_client_event(&mut socket, secure_channel.as_ref(), event).await;
                                }
                                if let Some(session) = &account {
                                    let request = CloudRequest::Sync { after: super::cloud_client::cursor(session) };
                                    let _ = send_client_event(&mut socket, secure_channel.as_ref(), &ClientEvent::Cloud { request }).await;
                                    syncing = true;
                                    load_stickers_after_sync = true;
                                }
                            }
                            ServerEvent::Pong => {
                                last_pong = Instant::now();
                                if let Some(sent_at) = ping_sent_at.take() {
                                    let _ = events.send(super::NetworkEvent::Latency(sent_at.elapsed()));
                                }
                            }
                            ServerEvent::SharedPetAction { sender_id, action } => {
                                let _ = events.send(super::NetworkEvent::SharedPetAction { sender_id, action });
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
                            ServerEvent::UnpairPending { .. } => {
                                let _ = events.send(super::NetworkEvent::UnpairPending);
                            }
                            ServerEvent::UnpairRequested { requester_id, requester_display_name } => {
                                let _ = events.send(super::NetworkEvent::UnpairRequested {
                                    requester_id,
                                    requester_display_name,
                                });
                            }
                            ServerEvent::UnpairCancelled { .. } => {
                                let _ = events.send(super::NetworkEvent::UnpairCancelled);
                            }
                            ServerEvent::Unpaired { former_partner_id } => {
                                let _ = events.send(super::NetworkEvent::Unpaired {
                                    former_partner_id,
                                });
                            }
                            ServerEvent::ProfileUpdated { display_name, .. } => {
                                let _ = events.send(super::NetworkEvent::ProfileUpdated(display_name));
                            }
                            ServerEvent::Presence { user_id, state, .. } => {
                                let _ = events.send(super::NetworkEvent::Presence { user_id, state });
                            }
                            ServerEvent::Error { code, message } => {
                                if code == "session_expired" { account = None; let _ = super::cloud_client::clear_session(); let _ = events.send(super::NetworkEvent::Cloud(CloudResponse::LoggedOut)); }
                                let _ = events.send(super::NetworkEvent::Error(message));
                            }
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
        begin_encryption_pem(socket, &pem).await
    }

    async fn begin_encryption_pem(
        socket: &mut RelaySocket,
        pem: &str,
    ) -> anyhow::Result<SecureChannel> {
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
        validate_event_encryption(secure_channel, event)?;
        let json = if let Some(channel) = secure_channel {
            let frame = channel
                .encrypt(event)
                .map_err(|error| anyhow::anyhow!(error))?;
            serde_json::to_string(&ClientFrame::Encrypted { frame })?
        } else {
            serde_json::to_string(event)?
        };
        tokio::time::timeout(
            Duration::from_secs(10),
            socket.send(Message::Text(json.into())),
        )
        .await??;
        Ok(())
    }

    fn event_requires_encryption(event: &ClientEvent) -> bool {
        matches!(event, ClientEvent::Cloud { .. })
            || matches!(event, ClientEvent::Hello { access_token, .. } if !access_token.is_empty())
    }

    fn validate_event_encryption(
        secure_channel: Option<&SecureChannel>,
        event: &ClientEvent,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            secure_channel.is_some() || !event_requires_encryption(event),
            "refusing to send credentials without RSA encryption"
        );
        Ok(())
    }

    #[cfg(test)]
    #[test]
    fn credential_events_require_encryption() {
        let hello = ClientEvent::Hello {
            protocol_version: PROTOCOL_VERSION,
            access_token: "secret".into(),
            device_id: "device".into(),
            display_name: None,
        };
        assert!(validate_event_encryption(None, &hello).is_err());
        let requests = [
            CloudRequest::Register {
                username: "alice".into(),
                password: "secret".into(),
                device_id: "device".into(),
                legacy_token: None,
            },
            CloudRequest::Login {
                username: "alice".into(),
                password: "secret".into(),
                device_id: "device".into(),
            },
            CloudRequest::Recover {
                username: "alice".into(),
                recovery_code: "code".into(),
                new_password: "new secret".into(),
                device_id: "device".into(),
            },
            CloudRequest::Resume {
                token: "session".into(),
                device_id: "device".into(),
            },
        ];
        for request in requests {
            assert!(validate_event_encryption(None, &ClientEvent::Cloud { request }).is_err());
        }
        assert!(validate_event_encryption(None, &ClientEvent::Ping).is_ok());
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
