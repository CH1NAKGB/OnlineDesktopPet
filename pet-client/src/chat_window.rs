use std::{
    collections::VecDeque,
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

use anyhow::Context;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use egui::{Color32, RichText};
use egui_wgpu::{Renderer as EguiRenderer, ScreenDescriptor};
use sha2::{Digest, Sha256};
use shared_protocol::cloud::{
    CloudBody, CloudMessage, CloudRequest, CloudResponse, CloudSticker, ImageData,
};
#[cfg(target_os = "windows")]
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
#[cfg(target_os = "windows")]
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, SW_RESTORE, SetForegroundWindow, ShowWindow,
};
#[cfg(target_os = "windows")]
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::{
    dpi::LogicalSize,
    event::WindowEvent,
    event_loop::ActiveEventLoop,
    window::{Theme, Window, WindowId},
};

use crate::chat::{self, ChatEntry};
use crate::{
    cloud_client::{self, Session},
    sound::SoundTheme,
};

pub enum ChatAction {
    Pair(String),
    Send(String),
    Rename(String),
    SetLaunchAtLogin(bool),
    SetSoundsEnabled(bool),
    SetSoundTheme(SoundTheme),
    Cloud(CloudRequest),
    RetryOutbox,
    ClearOutbox,
    RequestUnpair,
    RespondUnpair(bool),
}

fn format_chat_time(unix_ms: u64, precise: bool) -> String {
    let Some((year, month, day, hour, minute, second)) = local_datetime(unix_ms) else {
        return "时间未知".to_owned();
    };
    if precise {
        format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
    } else {
        format!("{hour:02}:{minute:02}")
    }
}

#[cfg(not(target_os = "windows"))]
fn local_datetime(unix_ms: u64) -> Option<(i32, u32, u32, u32, u32, u32)> {
    let seconds = libc::time_t::try_from(unix_ms / 1000).ok()?;
    let mut local = std::mem::MaybeUninit::<libc::tm>::uninit();
    // localtime_r uses the machine's configured timezone and is safe for concurrent callers.
    let result = unsafe { libc::localtime_r(&seconds, local.as_mut_ptr()) };
    if result.is_null() {
        return None;
    }
    let local = unsafe { local.assume_init() };
    Some((
        local.tm_year + 1900,
        (local.tm_mon + 1) as u32,
        local.tm_mday as u32,
        local.tm_hour as u32,
        local.tm_min as u32,
        local.tm_sec as u32,
    ))
}

#[cfg(target_os = "windows")]
fn local_datetime(unix_ms: u64) -> Option<(i32, u32, u32, u32, u32, u32)> {
    use windows_sys::Win32::{
        Foundation::{FILETIME, SYSTEMTIME},
        System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime},
    };

    const WINDOWS_EPOCH_OFFSET_100NS: u64 = 116_444_736_000_000_000;
    let ticks = unix_ms
        .checked_mul(10_000)?
        .checked_add(WINDOWS_EPOCH_OFFSET_100NS)?;
    let file_time = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    unsafe {
        if FileTimeToSystemTime(&file_time, &mut utc) == 0
            || SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) == 0
        {
            return None;
        }
    }
    Some((
        local.wYear as i32,
        local.wMonth as u32,
        local.wDay as u32,
        local.wHour as u32,
        local.wMinute as u32,
        local.wSecond as u32,
    ))
}

#[derive(Clone)]
enum UnpairDialog {
    ConfirmLocal,
    Waiting,
    ConfirmRemote(String),
}

pub struct ChatWindow {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    context: egui::Context,
    state: egui_winit::State,
    renderer: EguiRenderer,
    history: Vec<ChatEntry>,
    selected_time: Option<usize>,
    input: String,
    pair_input: String,
    own_code: String,
    partner_id: Option<String>,
    partner_display_name: Option<String>,
    display_name: String,
    edit_display_name: String,
    editing_name: bool,
    startup_enabled: bool,
    sounds_enabled: bool,
    sound_theme: SoundTheme,
    cloud: CloudUi,
    focus_pair_input: bool,
    focus_deadline: Option<Instant>,
    unpair_dialog: Option<UnpairDialog>,
    status: String,
    network_status: String,
    partner_status: String,
    dark_mode: bool,
    pending_size: Option<winit::dpi::PhysicalSize<u32>>,
    resize_deadline: Option<Instant>,
    position: crate::window_position::PositionRecorder,
}

impl ChatWindow {
    pub async fn new(
        event_loop: &ActiveEventLoop,
        own_code: String,
        partner_id: Option<String>,
        display_name: String,
        network_status: String,
    ) -> anyhow::Result<Self> {
        let attributes = Window::default_attributes()
            .with_title("Chick Link")
            .with_inner_size(LogicalSize::new(460.0, 720.0))
            .with_min_inner_size(LogicalSize::new(360.0, 560.0));
        let attributes = crate::window_position::restore(
            event_loop,
            crate::window_position::WindowKind::Chat,
            (460.0, 720.0),
        )
        .map(|position| attributes.clone().with_position(position))
        .unwrap_or(attributes);
        let window = Arc::new(event_loop.create_window(attributes)?);
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let surface = instance.create_surface(window.clone())?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .context("no adapter for chat window")?;
        let mut required_limits = wgpu::Limits::downlevel_defaults();
        required_limits.max_texture_dimension_2d = adapter.limits().max_texture_dimension_2d;
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("chat-ui"),
                    required_features: wgpu::Features::empty(),
                    required_limits,
                    memory_hints: wgpu::MemoryHints::MemoryUsage,
                },
                None,
            )
            .await?;
        let capabilities = surface.get_capabilities(&adapter);
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .unwrap_or(capabilities.formats[0]);
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: wgpu::CompositeAlphaMode::Opaque,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);
        let context = egui::Context::default();
        configure_fonts(&context);
        let dark_mode = matches!(window.theme(), Some(Theme::Dark));
        configure_style(&context, dark_mode);
        let state = egui_winit::State::new(
            context.clone(),
            egui::ViewportId::ROOT,
            window.as_ref(),
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(adapter.limits().max_texture_dimension_2d as usize),
        );
        let renderer = EguiRenderer::new(&device, format, None, 1, false);
        let cloud = CloudUi::new();
        let history = cloud
            .session
            .as_ref()
            .map_or_else(chat::load_history, |session| {
                cloud
                    .items
                    .iter()
                    .map(|item| ChatEntry::from_cloud(&session.user_id, item))
                    .collect()
            });
        Ok(Self {
            window,
            surface,
            device,
            queue,
            config,
            context,
            state,
            renderer,
            history,
            selected_time: None,
            input: String::new(),
            pair_input: String::new(),
            own_code,
            partner_id,
            partner_display_name: None,
            edit_display_name: display_name.clone(),
            display_name,
            editing_name: false,
            startup_enabled: crate::startup::is_enabled(),
            sounds_enabled: crate::config::sounds_enabled(),
            sound_theme: crate::config::sound_theme(),
            cloud,
            focus_pair_input: true,
            focus_deadline: None,
            unpair_dialog: None,
            status: "等待输入对方配对码".into(),
            network_status,
            partner_status: "状态未知".into(),
            dark_mode,
            pending_size: None,
            resize_deadline: None,
            position: crate::window_position::PositionRecorder::new(
                crate::window_position::WindowKind::Chat,
            ),
        })
    }

    pub fn id(&self) -> WindowId {
        self.window.id()
    }

    pub fn show(&mut self) {
        self.reload_cloud();
        self.focus_pair_input = self.partner_id.is_none();
        self.focus_deadline = Some(Instant::now() + Duration::from_millis(90));
        self.window.set_visible(true);
        self.window.focus_window();
        activate_windows_chat(&self.window);
        self.window.request_redraw();
    }

    pub fn set_partner(
        &mut self,
        partner_id: String,
        partner_display_name: Option<String>,
        initiated_by_you: bool,
    ) {
        self.partner_id = Some(partner_id);
        self.partner_display_name = partner_display_name;
        self.focus_pair_input = false;
        self.unpair_dialog = None;
        self.status = if initiated_by_you {
            "已配对成功".into()
        } else {
            "对方发起配对，已自动成功".into()
        };
        self.window.request_redraw();
    }

    pub fn set_unpair_pending(&mut self) {
        self.unpair_dialog = Some(UnpairDialog::Waiting);
        self.status = "解除申请已发出，等待对方确认".into();
        self.window.request_redraw();
    }

    pub fn show_unpair_request(&mut self, requester_name: String) {
        self.unpair_dialog = Some(UnpairDialog::ConfirmRemote(requester_name));
        self.status = "对方请求解除配对，需要你的确认".into();
        self.show();
    }

    pub fn set_unpair_cancelled(&mut self) {
        self.unpair_dialog = None;
        self.status = "解除配对已取消，双方仍在共同养鸡".into();
        self.window.request_redraw();
    }

    pub fn set_unpaired(&mut self) {
        self.partner_id = None;
        self.partner_display_name = None;
        self.unpair_dialog = None;
        self.pair_input.clear();
        self.status = "双方已确认解除，可以重新配对".into();
        self.focus_pair_input = true;
        self.focus_deadline = Some(Instant::now() + Duration::from_millis(90));
        self.window.request_redraw();
    }

    pub fn set_network_status(&mut self, status: String) {
        if self.network_status != status {
            self.network_status = status;
            self.window.request_redraw();
        }
    }

    pub fn set_partner_status(&mut self, status: &str) {
        if self.partner_status != status {
            self.partner_status = status.into();
            self.window.request_redraw();
        }
    }

    pub fn set_partner_display_name(&mut self, display_name: String) {
        self.partner_display_name = Some(display_name);
        self.window.request_redraw();
    }

    pub fn set_display_name(&mut self, display_name: String) {
        self.display_name = display_name.clone();
        self.edit_display_name = display_name;
        self.editing_name = false;
        self.status = "昵称已保存".into();
        self.window.request_redraw();
    }

    pub fn finish_startup_change(&mut self, enabled: bool, error: Option<String>) {
        if let Some(error) = error {
            self.startup_enabled = !enabled;
            self.status = format!("开机自启设置失败：{error}");
        } else {
            self.startup_enabled = enabled;
            self.status = if enabled {
                "已开启开机自启".into()
            } else {
                "已关闭开机自启".into()
            };
        }
        self.window.request_redraw();
    }

    pub fn set_pairing_code(&mut self, code: String) {
        self.own_code = code;
        self.window.request_redraw();
    }

    pub fn finish_sound_change(&mut self, enabled: bool, error: Option<String>) {
        self.sounds_enabled = enabled;
        self.status = error.map_or_else(
            || {
                if enabled {
                    "柔和提示音已开启".into()
                } else {
                    "提示音已静音".into()
                }
            },
            |error| format!("无法保存音效设置：{error}"),
        );
        self.window.request_redraw();
    }

    /// `theme` is the effective (persisted) theme, including after a failed change.
    pub fn finish_sound_theme(&mut self, theme: SoundTheme, error: Option<String>) {
        self.sound_theme = theme;
        self.status = error.map_or_else(
            || format!("提示音：{}", theme.label()),
            |error| format!("无法保存音效设置：{error}"),
        );
        self.window.request_redraw();
    }

    /// Parent persists all responses before forwarding them here, even when no window exists.
    /// Parent also owns Resume/Sync pagination and ListStickers after authentication.
    pub fn handle_cloud(&mut self, response: &CloudResponse) {
        match response {
            CloudResponse::Authenticated {
                user_id,
                username,
                token,
                recovery_code,
            } => {
                let changed = self.cloud.session.as_ref().map(|s| &s.user_id) != Some(user_id);
                if changed {
                    self.cloud.reset_account();
                    self.history.clear();
                    self.input.clear();
                }
                self.cloud.session = Some(Session {
                    user_id: user_id.clone(),
                    username: username.clone(),
                    token: token.clone(),
                    server_url: cloud_client::load_session()
                        .filter(|session| session.user_id == *user_id)
                        .map(|session| session.server_url)
                        .unwrap_or_default(),
                });
                for item in self
                    .cloud
                    .session
                    .as_ref()
                    .map(cloud_client::load_cloud_history)
                    .unwrap_or_default()
                {
                    cloud_client::merge_message(user_id, &mut self.cloud.items, &item);
                }
                if let Some(code) = recovery_code {
                    let fingerprint = format!("{:x}", Sha256::digest(code.as_bytes()));
                    if self.cloud.seen_recovery.as_ref() != Some(&fingerprint) {
                        self.cloud.recovery_notice = Some(code.clone());
                        self.cloud.seen_recovery = Some(fingerprint);
                        self.cloud.recovery_saved = false;
                    }
                }
                self.cloud.clear_credentials();
                self.cloud.status = "云账号已登录".into();
                self.cloud.auth_pending = false;
                self.cloud.auth_deadline = None;
                self.rebuild_cloud_history();
            }
            CloudResponse::LoggedOut => {
                self.cloud.reset_account();
                self.history.clear();
                self.input.clear();
                self.cloud.status = "已退出云账号".into();
            }
            CloudResponse::History { items, .. } => {
                if let Some(session) = &self.cloud.session {
                    for item in items {
                        cloud_client::merge_message(&session.user_id, &mut self.cloud.items, item);
                    }
                    self.rebuild_cloud_history();
                }
            }
            CloudResponse::Message { item } => {
                self.apply_cloud_message(item);
            }
            CloudResponse::SendRejected {
                message, retryable, ..
            } => {
                self.cloud.send_rejected(message, *retryable);
                self.status = self.cloud.outbox_message.clone();
            }
            CloudResponse::Stickers { items } if self.cloud.session.is_some() => {
                self.cloud.stickers = items.clone();
                self.cloud.status = "表情库已同步".into();
            }
            CloudResponse::StickerSaved { item } if self.cloud.session.is_some() => {
                self.cloud.stickers.retain(|old| old.id != item.id);
                self.cloud.stickers.push(item.clone());
                self.cloud.status = "已保存到云表情库".into();
            }
            CloudResponse::Error { message } => {
                self.cloud.status = message.clone();
                self.cloud.auth_pending = false;
                self.cloud.auth_deadline = None;
                self.cloud.clear_credentials();
            }
            _ => {}
        }
        self.window.request_redraw();
    }

    /// Counts come from the parent's outbox; pending includes blocked messages.
    pub fn set_outbox_status(&mut self, pending: usize, blocked: usize) {
        self.cloud.set_outbox_status(pending, blocked);
        self.window.request_redraw();
    }

    pub fn outbox_error(&mut self, message: String) {
        self.cloud.outbox_message = message.clone();
        self.status = message;
        self.window.request_redraw();
    }

    pub fn restore_unsent_text(&mut self, text: String) {
        append_unsent_text(&mut self.input, &text);
        self.window.request_redraw();
    }

    fn rebuild_cloud_history(&mut self) {
        if let Some(session) = &self.cloud.session {
            self.history = self
                .cloud
                .items
                .iter()
                .map(|item| ChatEntry::from_cloud(&session.user_id, item))
                .collect();
        }
    }

    /// UI-only merge; the parent must persist the message independently.
    pub fn apply_cloud_message(&mut self, item: &CloudMessage) {
        if let Some(session) = &self.cloud.session {
            cloud_client::merge_message(&session.user_id, &mut self.cloud.items, item);
            self.rebuild_cloud_history();
            self.window.request_redraw();
        }
    }

    fn reload_cloud(&mut self) {
        let session = cloud_client::load_session();
        let changed =
            self.cloud.session.as_ref().map(|s| &s.user_id) != session.as_ref().map(|s| &s.user_id);
        if changed {
            self.cloud.reset_account();
            self.history.clear();
            self.input.clear();
        }
        if let Some(session) = session {
            for item in cloud_client::load_cloud_history(&session) {
                cloud_client::merge_message(&session.user_id, &mut self.cloud.items, &item);
            }
            self.cloud.session = Some(session);
            self.rebuild_cloud_history();
        }
    }

    pub fn set_status(&mut self, status: impl Into<String>) {
        self.status = status.into();
        self.window.request_redraw();
    }

    pub fn push_message(&mut self, entry: ChatEntry) {
        if self.cloud.session.is_some() || entry.account_id.is_some() {
            return;
        }
        chat::append_history(&entry);
        self.history.push(entry);
        self.window.request_redraw();
    }

    pub fn handle_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        event: &WindowEvent,
    ) -> anyhow::Result<Vec<ChatAction>> {
        let response = self.state.on_window_event(&self.window, event);
        if response.repaint {
            self.window.request_redraw();
        }
        match event {
            WindowEvent::CloseRequested => {
                self.position.flush();
                if self.cloud.recovery_notice.is_some() {
                    self.cloud.status = "请先保存恢复码并确认".into();
                    self.window.request_redraw();
                } else {
                    self.cloud.clear_credentials();
                    self.window.set_visible(false);
                }
            }
            WindowEvent::DroppedFile(path) => {
                self.cloud.account_open = true;
                self.cloud.start_import(path.clone(), self.window.clone());
                self.window.request_redraw();
            }
            WindowEvent::Resized(size) if size.width > 0 && size.height > 0 => {
                self.pending_size = Some(*size);
                self.resize_deadline = Some(Instant::now() + Duration::from_millis(40));
            }
            WindowEvent::Moved(position) => self.position.moved(*position),
            WindowEvent::ThemeChanged(theme) => {
                self.dark_mode = matches!(theme, Theme::Dark);
                configure_style(&self.context, self.dark_mode);
                self.window.request_redraw();
            }
            WindowEvent::Focused(true) if self.partner_id.is_none() => {
                self.focus_pair_input = true;
                self.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                if self
                    .resize_deadline
                    .is_some_and(|deadline| Instant::now() < deadline)
                {
                    return Ok(Vec::new());
                }
                self.apply_pending_resize();
                return self.render();
            }
            _ => {}
        }
        Ok(Vec::new())
    }

    pub fn process_deferred_actions(&mut self) {
        let now = Instant::now();
        self.position.flush_if_due(now);
        if self.cloud.expire_auth(now) {
            self.window.request_redraw();
        }
        if self.resize_deadline.is_some_and(|deadline| now >= deadline) {
            self.window.request_redraw();
        }
        if self.focus_deadline.is_some_and(|deadline| now >= deadline) {
            self.focus_deadline = None;
            self.focus_pair_input = self.partner_id.is_none();
            self.window.focus_window();
            activate_windows_chat(&self.window);
            self.window.request_redraw();
        }
    }

    pub fn position_deadline(&self) -> Option<Instant> {
        self.position.deadline()
    }

    fn apply_pending_resize(&mut self) {
        let Some(size) = self.pending_size.take() else {
            return;
        };
        self.resize_deadline = None;
        self.config.width = size.width.max(1);
        self.config.height = size.height.max(1);
        self.surface.configure(&self.device, &self.config);
    }

    fn render(&mut self) -> anyhow::Result<Vec<ChatAction>> {
        self.cloud.expire_auth(Instant::now());
        self.cloud.poll_import();
        let raw_input = self.state.take_egui_input(&self.window);
        let mut actions = Vec::new();
        let partner_id = self.partner_id.clone();
        let colors = Palette::new(self.dark_mode);
        let context = self.context.clone();
        let output = context.run(raw_input, |ctx| {
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::default()
                        .fill(colors.background)
                        .inner_margin(16.0),
                )
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        let label = self
                            .cloud
                            .session
                            .as_ref()
                            .map_or("云账号".to_owned(), |session| {
                                format!("云账号 · {}", session.username)
                            });
                        if ui.button(label).clicked() {
                            self.cloud.account_open = !self.cloud.account_open;
                        }
                        if self.cloud.session.is_some() && ui.small_button("表情库").clicked() {
                            self.cloud.account_open = true;
                        }
                    });
                    if self.cloud.items.len() == cloud_client::MAX_HISTORY_ITEMS {
                        ui.small("显示最近 100 条消息，更早记录保存在本机");
                    }
                    if partner_id.is_none() {
                        egui::ScrollArea::vertical()
                            .id_salt("pairing-page")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.vertical_centered(|ui| {
                                    ui.add_space(12.0);
                                    ui.label(
                                        RichText::new("CHICK LINK")
                                            .size(12.0)
                                            .strong()
                                            .color(colors.accent),
                                    );
                                    ui.add_space(6.0);
                                    ui.label(
                                        RichText::new("一起养一只小鸡")
                                            .size(25.0)
                                            .strong()
                                            .color(colors.text),
                                    );
                                    ui.add_space(5.0);
                                    ui.label(
                                        RichText::new("交换配对码后，两台电脑就会连接起来")
                                            .size(13.0)
                                            .color(colors.muted),
                                    );
                                    ui.add_space(6.0);
                                    ui.label(
                                        RichText::new(&self.network_status)
                                            .size(12.0)
                                            .color(colors.accent),
                                    );
                                    ui.add_space(16.0);

                                    let card_width = ui.available_width().min(380.0);
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(card_width, 0.0),
                                        egui::Layout::top_down(egui::Align::Center),
                                        |ui| {
                                            egui::Frame::default()
                                                .fill(colors.card)
                                                .stroke(egui::Stroke::new(1.0_f32, colors.border))
                                                .corner_radius(14.0)
                                                .inner_margin(20.0)
                                                .show(ui, |ui| {
                                                    let content_width = ui.available_width();
                                                    ui.set_width(content_width);
                                                    ui.label(
                                                        RichText::new("我的昵称")
                                                            .size(13.0)
                                                            .color(colors.muted),
                                                    );
                                                    ui.add_space(7.0);
                                                    let name_response = ui.add_sized(
                                                        [content_width, 42.0],
                                                        egui::TextEdit::singleline(
                                                            &mut self.edit_display_name,
                                                        )
                                                        .hint_text("给自己起个名字")
                                                        .horizontal_align(egui::Align::Center)
                                                        .vertical_align(egui::Align::Center),
                                                    );
                                                    let normalized_name =
                                                        shared_protocol::normalize_display_name(
                                                            &self.edit_display_name,
                                                        );
                                                    if name_response.changed() {
                                                        self.status = "昵称修改后需要保存".into();
                                                    }
                                                    ui.add_space(7.0);
                                                    if ui
                                                        .add_enabled(
                                                            normalized_name.as_deref().is_some_and(
                                                                |name| name != self.display_name,
                                                            ),
                                                            rounded_button("保存昵称").min_size(
                                                                egui::vec2(content_width, 42.0),
                                                            ),
                                                        )
                                                        .clicked()
                                                    {
                                                        actions.push(ChatAction::Rename(
                                                            normalized_name.unwrap(),
                                                        ));
                                                    }
                                                    ui.add_space(10.0);
                                                    ui.separator();
                                                    ui.add_space(10.0);
                                                    ui.label(
                                                        RichText::new("我的配对码")
                                                            .size(13.0)
                                                            .color(colors.muted),
                                                    );
                                                    ui.add_space(8.0);
                                                    ui.label(
                                                        RichText::new(spaced_code(&self.own_code))
                                                            .monospace()
                                                            .size(27.0)
                                                            .strong()
                                                            .color(colors.text),
                                                    );
                                                    ui.add_space(10.0);
                                                    if ui
                                                        .add_sized(
                                                            [content_width, 42.0],
                                                            rounded_button("复制配对码"),
                                                        )
                                                        .clicked()
                                                    {
                                                        ui.ctx().copy_text(self.own_code.clone());
                                                        self.status = "配对码已复制".into();
                                                    }
                                                    ui.add_space(10.0);
                                                    ui.separator();
                                                    ui.add_space(10.0);
                                                    ui.with_layout(
                                                        egui::Layout::left_to_right(
                                                            egui::Align::Center,
                                                        ),
                                                        |ui| {
                                                            ui.label(
                                                                RichText::new("输入对方配对码")
                                                                    .size(13.0)
                                                                    .color(colors.muted),
                                                            );
                                                        },
                                                    );
                                                    ui.add_space(7.0);
                                                    let response = ui.add_sized(
                                                        [content_width, 42.0],
                                                        egui::TextEdit::singleline(
                                                            &mut self.pair_input,
                                                        )
                                                        .font(egui::TextStyle::Monospace)
                                                        .hint_text("7KQ3M9XZ")
                                                        .horizontal_align(egui::Align::Center)
                                                        .vertical_align(egui::Align::Center),
                                                    );
                                                    if self.focus_pair_input {
                                                        response.request_focus();
                                                        self.focus_pair_input = false;
                                                    }
                                                    self.pair_input.make_ascii_uppercase();
                                                    let valid =
                                                        shared_protocol::is_valid_pairing_code(
                                                            self.pair_input.trim(),
                                                        );
                                                    ui.add_space(10.0);
                                                    let submit = ui
                                                        .add_enabled(
                                                            valid,
                                                            rounded_button(
                                                                RichText::new("开始共同养育")
                                                                    .strong()
                                                                    .color(colors.on_accent),
                                                            )
                                                            .fill(colors.accent)
                                                            .min_size(egui::vec2(
                                                                content_width,
                                                                42.0,
                                                            )),
                                                        )
                                                        .clicked()
                                                        || (valid
                                                            && response.lost_focus()
                                                            && ui.input(|i| {
                                                                i.key_pressed(egui::Key::Enter)
                                                            }));
                                                    if submit {
                                                        actions.push(ChatAction::Pair(
                                                            self.pair_input.trim().to_owned(),
                                                        ));
                                                        self.status = "正在连接对方...".into();
                                                    }
                                                });
                                        },
                                    );
                                    ui.add_space(14.0);
                                    let startup_changed = ui
                                        .checkbox(
                                            &mut self.startup_enabled,
                                            "开机时自动启动 Chick Link",
                                        )
                                        .changed();
                                    if startup_changed {
                                        actions.push(ChatAction::SetLaunchAtLogin(
                                            self.startup_enabled,
                                        ));
                                        self.status = "正在更新开机自启设置...".into();
                                    }
                                    ui.add_space(8.0);
                                    sound_controls(
                                        ui,
                                        &mut self.sounds_enabled,
                                        self.sound_theme,
                                        &mut actions,
                                    );
                                    ui.label(
                                        RichText::new(&self.status)
                                            .size(12.0)
                                            .color(status_color(&self.status, colors)),
                                    );
                                });
                            });
                        return;
                    }

                    ui.horizontal(|ui| {
                        let heading_width = (ui.available_width() - 190.0).max(100.0);
                        ui.allocate_ui_with_layout(
                            egui::vec2(heading_width, 52.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.label(
                                    RichText::new("共同小鸡")
                                        .size(20.0)
                                        .strong()
                                        .color(colors.text),
                                );
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(
                                            self.partner_display_name.as_deref().unwrap_or_else(
                                                || partner_id.as_deref().unwrap_or("对方"),
                                            ),
                                        )
                                        .size(12.0)
                                        .color(colors.muted),
                                    )
                                    .truncate(),
                                );
                            },
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add(
                                    rounded_button("解除配对")
                                        .fill(colors.danger_bg)
                                        .stroke(egui::Stroke::new(1.0_f32, colors.danger)),
                                )
                                .clicked()
                            {
                                self.unpair_dialog = Some(UnpairDialog::ConfirmLocal);
                            }
                            if ui.add(rounded_button("设置")).clicked() {
                                self.editing_name = !self.editing_name;
                            }
                        });
                    });
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!("对方：{}", self.partner_status))
                            .size(13.0)
                            .color(if self.partner_status == "在线" {
                                colors.text
                            } else {
                                colors.muted
                            }),
                    );
                    egui::Frame::default()
                        .fill(colors.status_bg)
                        .corner_radius(12.0)
                        .inner_margin(egui::Margin::symmetric(10, 5))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(
                                    self.network_status
                                        .strip_prefix("网络：")
                                        .unwrap_or(&self.network_status),
                                )
                                .size(12.0)
                                .color(colors.accent),
                            );
                        });
                    if self.editing_name {
                        ui.add_space(10.0);
                        egui::Frame::default()
                            .fill(colors.card)
                            .stroke(egui::Stroke::new(1.0_f32, colors.border))
                            .corner_radius(10.0)
                            .inner_margin(10.0)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let width = (ui.available_width() - 82.0).max(120.0);
                                    ui.add_sized(
                                        [width, 36.0],
                                        egui::TextEdit::singleline(&mut self.edit_display_name)
                                            .vertical_align(egui::Align::Center),
                                    );
                                    let normalized = shared_protocol::normalize_display_name(
                                        &self.edit_display_name,
                                    );
                                    if ui
                                        .add_enabled(
                                            normalized
                                                .as_deref()
                                                .is_some_and(|name| name != self.display_name),
                                            rounded_button("保存").min_size(egui::vec2(70.0, 36.0)),
                                        )
                                        .clicked()
                                    {
                                        actions.push(ChatAction::Rename(normalized.unwrap()));
                                    }
                                });
                                ui.add_space(8.0);
                                ui.separator();
                                ui.add_space(6.0);
                                let startup_changed = ui
                                    .checkbox(
                                        &mut self.startup_enabled,
                                        "开机时自动启动 Chick Link",
                                    )
                                    .changed();
                                if startup_changed {
                                    actions
                                        .push(ChatAction::SetLaunchAtLogin(self.startup_enabled));
                                    self.status = "正在更新开机自启设置...".into();
                                }
                                sound_controls(
                                    ui,
                                    &mut self.sounds_enabled,
                                    self.sound_theme,
                                    &mut actions,
                                );
                            });
                    }
                    ui.add_space(7.0);
                    ui.label(
                        RichText::new(&self.status)
                            .size(12.0)
                            .color(status_color(&self.status, colors)),
                    );
                    ui.add_space(12.0);
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .id_salt("chat-history")
                        .stick_to_bottom(true)
                        .auto_shrink([false, false])
                        .max_height((ui.available_height() - 194.0).max(32.0))
                        .show(ui, |ui| {
                            ui.add_space(10.0);
                            if self.history.is_empty() {
                                ui.vertical_centered(|ui| {
                                    ui.add_space(24.0);
                                    ui.label(
                                        RichText::new("你们的小窝，已经连接")
                                            .strong()
                                            .color(colors.text),
                                    );
                                    ui.label(
                                        RichText::new("发句问候，让对方的小鸡捎个口信")
                                            .color(colors.muted),
                                    );
                                });
                            }
                            for (index, entry) in self.history.iter().enumerate() {
                                let same_minute_as_previous = index > 0
                                    && self.history[index - 1].unix_ms / 60_000
                                        == entry.unix_ms / 60_000;
                                if !same_minute_as_previous {
                                    ui.vertical_centered(|ui| {
                                        ui.label(
                                            RichText::new(format_chat_time(entry.unix_ms, false))
                                                .size(11.0)
                                                .color(colors.muted),
                                        );
                                    });
                                    ui.add_space(4.0);
                                }
                                let fill = if entry.outgoing {
                                    colors.outgoing
                                } else {
                                    colors.incoming
                                };
                                let bubble = ui.with_layout(
                                    if entry.outgoing {
                                        egui::Layout::right_to_left(egui::Align::TOP)
                                    } else {
                                        egui::Layout::left_to_right(egui::Align::TOP)
                                    },
                                    |ui| {
                                        egui::Frame::default()
                                            .fill(fill)
                                            .corner_radius(11.0)
                                            .inner_margin(egui::Margin::symmetric(12, 9))
                                            .show(ui, |ui| {
                                                ui.set_max_width(
                                                    (ui.available_width() * 0.78).min(290.0),
                                                );
                                                if let Some(CloudMessage {
                                                    body: CloudBody::Image { image },
                                                    ..
                                                }) = &entry.cloud_message
                                                {
                                                    self.cloud
                                                        .textures
                                                        .show(ui, image, 220.0, false);
                                                } else {
                                                    ui.label(
                                                        RichText::new(&entry.text)
                                                            .size(15.0)
                                                            .color(colors.text),
                                                    );
                                                }
                                            });
                                    },
                                );
                                let response = ui.interact(
                                    bubble.response.rect,
                                    ui.id().with(("message-time", index)),
                                    egui::Sense::click(),
                                );
                                if response.clicked() {
                                    self.selected_time =
                                        (self.selected_time != Some(index)).then_some(index);
                                }
                                if self.selected_time == Some(index) {
                                    ui.with_layout(
                                        if entry.outgoing {
                                            egui::Layout::right_to_left(egui::Align::TOP)
                                        } else {
                                            egui::Layout::left_to_right(egui::Align::TOP)
                                        },
                                        |ui| {
                                            ui.label(
                                                RichText::new(format_chat_time(
                                                    entry.unix_ms,
                                                    true,
                                                ))
                                                .size(11.0)
                                                .color(colors.muted),
                                            );
                                        },
                                    );
                                }
                                ui.add_space(7.0);
                            }
                        });
                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(7.0);
                    ui.horizontal_wrapped(|ui| {
                        for quick in ["想你啦", "摸摸小鸡", "忙完找你", "晚安"] {
                            if ui.add(rounded_button(quick)).clicked() {
                                actions.push(ChatAction::Send(quick.into()));
                            }
                        }
                    });
                    ui.add_space(7.0);
                    ui.horizontal(|ui| {
                        let send_width = 68.0;
                        let editor_width = (ui.available_width() - send_width - 10.0).max(160.0);
                        let response = ui.add_sized(
                            [editor_width, 54.0],
                            egui::TextEdit::multiline(&mut self.input).hint_text("输入消息..."),
                        );
                        let submit = ui
                            .add_sized(
                                [send_width, 54.0],
                                rounded_button(
                                    RichText::new("发送").strong().color(colors.on_accent),
                                )
                                .fill(colors.accent),
                            )
                            .clicked()
                            || (response.has_focus()
                                && ui.input(|i| {
                                    i.key_pressed(egui::Key::Enter) && !i.modifiers.shift
                                }));
                        if submit {
                            let text = self
                                .input
                                .trim()
                                .chars()
                                .take(shared_protocol::MAX_TEXT_CHARS)
                                .collect::<String>();
                            if !text.is_empty() {
                                actions.push(ChatAction::Send(text));
                                self.input.clear();
                            }
                        }
                    });
                    ui.label(
                        RichText::new("Enter 发送 · Shift + Enter 换行")
                            .size(11.0)
                            .color(colors.muted),
                    );
                });

            self.cloud
                .show_account(ctx, partner_id.as_deref(), &self.window, &mut actions);

            if let Some(dialog) = self.unpair_dialog.clone() {
                egui::Window::new("解除配对")
                    .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                    .collapsible(false)
                    .resizable(false)
                    .fixed_size(egui::vec2(320.0, 210.0))
                    .frame(
                        egui::Frame::window(&ctx.style())
                            .fill(colors.card)
                            .stroke(egui::Stroke::new(1.0_f32, colors.border))
                            .corner_radius(14.0)
                            .inner_margin(18.0),
                    )
                    .show(ctx, |ui| match dialog {
                        UnpairDialog::ConfirmLocal => {
                            ui.label(
                                RichText::new("要申请解除共同小鸡关系吗？")
                                    .size(16.0)
                                    .strong()
                                    .color(colors.text),
                            );
                            ui.add_space(7.0);
                            ui.label(
                                RichText::new("你确认后不会立即解除，还需要对方再次确认。")
                                    .size(13.0)
                                    .color(colors.muted),
                            );
                            ui.add_space(16.0);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .add_sized(
                                            [108.0, 36.0],
                                            rounded_button("确认申请")
                                                .fill(colors.danger_bg)
                                                .stroke(egui::Stroke::new(1.0_f32, colors.danger)),
                                        )
                                        .clicked()
                                    {
                                        actions.push(ChatAction::RequestUnpair);
                                        self.unpair_dialog = Some(UnpairDialog::Waiting);
                                        self.status = "正在提交解除申请...".into();
                                    }
                                    if ui.add_sized([82.0, 36.0], rounded_button("取消")).clicked()
                                    {
                                        self.unpair_dialog = None;
                                    }
                                },
                            );
                        }
                        UnpairDialog::Waiting => {
                            ui.label(
                                RichText::new("等待对方确认")
                                    .size(16.0)
                                    .strong()
                                    .color(colors.text),
                            );
                            ui.add_space(7.0);
                            ui.label(
                                RichText::new("当前配对仍然有效；只有对方同意后才会解除。")
                                    .size(13.0)
                                    .color(colors.muted),
                            );
                            ui.add_space(16.0);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.add_sized([82.0, 36.0], rounded_button("收起")).clicked()
                                    {
                                        self.unpair_dialog = None;
                                    }
                                },
                            );
                        }
                        UnpairDialog::ConfirmRemote(requester_name) => {
                            ui.label(
                                RichText::new(format!("{requester_name} 请求解除配对"))
                                    .size(16.0)
                                    .strong()
                                    .color(colors.text),
                            );
                            ui.add_space(7.0);
                            ui.label(
                                RichText::new("请再次确认。拒绝后双方会继续共同养鸡。")
                                    .size(13.0)
                                    .color(colors.muted),
                            );
                            ui.add_space(16.0);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .add_sized(
                                            [108.0, 36.0],
                                            rounded_button("同意解除")
                                                .fill(colors.danger_bg)
                                                .stroke(egui::Stroke::new(1.0_f32, colors.danger)),
                                        )
                                        .clicked()
                                    {
                                        actions.push(ChatAction::RespondUnpair(true));
                                        self.unpair_dialog = Some(UnpairDialog::Waiting);
                                        self.status = "双方已确认，正在解除...".into();
                                    }
                                    if ui
                                        .add_sized([96.0, 36.0], rounded_button("暂不解除"))
                                        .clicked()
                                    {
                                        actions.push(ChatAction::RespondUnpair(false));
                                        self.unpair_dialog = None;
                                        self.status = "已拒绝解除申请".into();
                                    }
                                },
                            );
                        }
                    });
            }
        });
        self.state
            .handle_platform_output(&self.window, output.platform_output);
        let paint_jobs = self
            .context
            .tessellate(output.shapes, output.pixels_per_point);
        for (id, delta) in &output.textures_delta.set {
            self.renderer
                .update_texture(&self.device, &self.queue, *id, delta);
        }
        let frame = self.surface.get_current_texture()?;
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("chat-ui-frame"),
            });
        let descriptor = ScreenDescriptor {
            size_in_pixels: [self.config.width, self.config.height],
            pixels_per_point: output.pixels_per_point,
        };
        let extra = self.renderer.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            &paint_jobs,
            &descriptor,
        );
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("chat-ui-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: colors.background.r() as f64 / 255.0,
                            g: colors.background.g() as f64 / 255.0,
                            b: colors.background.b() as f64 / 255.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            self.renderer
                .render(&mut pass.forget_lifetime(), &paint_jobs, &descriptor);
        }
        self.queue
            .submit(extra.into_iter().chain([encoder.finish()]));
        frame.present();
        for id in &output.textures_delta.free {
            self.renderer.free_texture(id);
        }
        Ok(actions)
    }
}

#[cfg(target_os = "windows")]
fn activate_windows_chat(window: &Window) {
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let hwnd = handle.hwnd.get() as _;
    unsafe {
        ShowWindow(hwnd, SW_RESTORE);
        BringWindowToTop(hwnd);
        SetForegroundWindow(hwnd);
        SetFocus(hwnd);
    }
}

#[cfg(not(target_os = "windows"))]
fn activate_windows_chat(_window: &Window) {}

#[cfg(test)]
mod theme_tests {
    use super::*;

    #[test]
    fn theme_changes_keep_custom_control_geometry() {
        let context = egui::Context::default();
        for dark in [false, true, false] {
            configure_style(&context, dark);
            let _ = context.run(egui::RawInput::default(), |ctx| {
                assert_eq!(ctx.style().visuals.dark_mode, dark);
                let widgets = &ctx.style().visuals.widgets;
                for widget in [&widgets.inactive, &widgets.hovered, &widgets.active] {
                    assert_eq!(widget.corner_radius, egui::CornerRadius::same(10));
                    assert_eq!(widget.expansion, 0.0);
                }
            });
        }
    }

    #[test]
    fn interactive_labels_keep_readable_contrast_in_both_themes() {
        fn luminance(color: Color32) -> f32 {
            let linear = |channel: u8| {
                let value = f32::from(channel) / 255.0;
                if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
        }
        let context = egui::Context::default();
        for dark in [false, true, false] {
            configure_style(&context, dark);
            let colors = Palette::new(dark);
            let style = context.style();
            let widgets = &style.visuals.widgets;
            for widget in [
                &widgets.inactive,
                &widgets.hovered,
                &widgets.active,
                &widgets.open,
            ] {
                assert_eq!(widget.fg_stroke.color, colors.text);
                for background in [colors.background, colors.card, widget.bg_fill] {
                    let foreground = luminance(widget.fg_stroke.color);
                    let background = luminance(background);
                    let contrast =
                        (foreground.max(background) + 0.05) / (foreground.min(background) + 0.05);
                    assert!(contrast >= 4.5, "dark={dark}, contrast={contrast}");
                }
            }
        }
    }
}

fn rounded_button(text: impl Into<egui::WidgetText>) -> egui::Button<'static> {
    egui::Button::new(text).corner_radius(egui::CornerRadius::same(10))
}

fn sound_controls(
    ui: &mut egui::Ui,
    enabled: &mut bool,
    theme: SoundTheme,
    actions: &mut Vec<ChatAction>,
) {
    if ui.checkbox(enabled, "提示音 · 拍拍和新消息").changed() {
        actions.push(ChatAction::SetSoundsEnabled(*enabled));
    }
    ui.horizontal_wrapped(|ui| {
        for choice in SoundTheme::ALL {
            if ui
                .selectable_label(theme == choice, choice.label())
                .clicked()
            {
                actions.push(ChatAction::SetSoundTheme(choice));
            }
        }
    });
}

fn append_unsent_text(input: &mut String, text: &str) {
    if text.is_empty() {
        return;
    }
    if !input.is_empty() && !input.ends_with('\n') {
        input.push('\n');
    }
    input.push_str(text);
}

#[derive(Default)]
struct CloudUi {
    session: Option<Session>,
    items: Vec<CloudMessage>,
    stickers: Vec<CloudSticker>,
    textures: ImageTextures,
    account_open: bool,
    username: String,
    password: String,
    recovery_input: String,
    new_password: String,
    auth_pending: bool,
    auth_deadline: Option<Instant>,
    recovery_notice: Option<String>,
    seen_recovery: Option<String>,
    recovery_saved: bool,
    image_path: String,
    sticker_name: String,
    imported: Option<ImageData>,
    import_worker: Option<mpsc::Receiver<Result<ImageData, String>>>,
    outbox_pending: usize,
    outbox_blocked: usize,
    outbox_message: String,
    outbox_clear_confirm: bool,
    status: String,
}

impl CloudUi {
    fn new() -> Self {
        let session = cloud_client::load_session();
        let items = session
            .as_ref()
            .map_or_else(Vec::new, cloud_client::load_cloud_history);
        Self {
            session,
            items,
            ..Self::default()
        }
    }

    fn clear_credentials(&mut self) {
        self.password.clear();
        self.new_password.clear();
        self.recovery_input.clear();
    }

    fn reset_account(&mut self) {
        self.session = None;
        self.items.clear();
        self.stickers.clear();
        self.textures.0.clear();
        self.imported = None;
        // A worker for an old account may finish, but its result is discarded.
        self.import_worker = None;
        self.image_path.clear();
        self.sticker_name.clear();
        self.username.clear();
        self.clear_credentials();
        self.recovery_notice = None;
        self.seen_recovery = None;
        self.recovery_saved = false;
        self.auth_pending = false;
        self.auth_deadline = None;
        self.set_outbox_status(0, 0);
        self.status.clear();
    }

    fn set_outbox_status(&mut self, pending: usize, blocked: usize) {
        self.outbox_pending = pending;
        self.outbox_blocked = blocked;
        if pending == 0 {
            self.outbox_clear_confirm = false;
            self.outbox_message.clear();
        }
    }

    fn send_rejected(&mut self, message: &str, retryable: bool) {
        self.outbox_message = if retryable {
            format!("消息已保留在待发送队列，将自动重试：{message}")
        } else {
            format!("消息已保留，待手动重试：{message}")
        };
    }

    fn show_outbox(&mut self, ui: &mut egui::Ui, actions: &mut Vec<ChatAction>) {
        ui.label(format!("待发送：{} 条", self.outbox_pending));
        if self.outbox_blocked > 0 {
            ui.label(format!("其中 {} 条待手动重试", self.outbox_blocked));
        }
        if !self.outbox_message.is_empty() {
            ui.label(&self.outbox_message);
        }
        if self.outbox_clear_confirm {
            ui.label(format!(
                "确定清空 {} 条待发送消息？清空后无法恢复，也不能撤回已经提交给服务器的消息。",
                self.outbox_pending
            ));
            ui.horizontal_wrapped(|ui| {
                if ui.button("取消清空").clicked() {
                    self.outbox_clear_confirm = false;
                }
                if ui
                    .add_enabled(self.outbox_pending > 0, egui::Button::new("确认清空"))
                    .clicked()
                {
                    self.outbox_clear_confirm = false;
                    actions.push(ChatAction::ClearOutbox);
                }
            });
        } else {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(self.outbox_pending > 0, egui::Button::new("重试待发送"))
                    .clicked()
                {
                    actions.push(ChatAction::RetryOutbox);
                }
                if ui
                    .add_enabled(self.outbox_pending > 0, egui::Button::new("清空待发送"))
                    .clicked()
                {
                    self.outbox_clear_confirm = true;
                }
            });
        }
    }

    fn start_import(&mut self, path: PathBuf, window: Arc<Window>) {
        if self.session.is_none() {
            self.status = "请先登录云账号再导入图片".into();
            return;
        }
        if self.import_worker.is_some() {
            return;
        }
        self.imported = None;
        let (sender, receiver) = mpsc::sync_channel(1);
        self.import_worker = Some(receiver);
        self.status = "正在检查图片...".into();
        std::thread::spawn(move || {
            let result = import_image(&path);
            let _ = sender.send(result);
            window.request_redraw();
        });
    }

    fn poll_import(&mut self) {
        let Some(receiver) = &self.import_worker else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok(image)) => {
                self.imported = Some(image);
                self.status = "图片已就绪，可发送或保存为云表情".into();
                self.import_worker = None;
            }
            Ok(Err(error)) => {
                self.status = error;
                self.import_worker = None;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.status = "图片导入失败，请重试".into();
                self.import_worker = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }

    fn show_account(
        &mut self,
        ctx: &egui::Context,
        recipient: Option<&str>,
        window: &Arc<Window>,
        actions: &mut Vec<ChatAction>,
    ) {
        if self.account_open {
            let mut open = true;
            egui::Window::new("云账号与表情")
                .id(egui::Id::new("cloud-account"))
                .open(&mut open)
                .default_width(310.0)
                .resizable(true)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height((ctx.screen_rect().height() - 140.0).max(180.0))
                        .show(ui, |ui| {
                            ui.colored_label(
                                ui.visuals().warn_fg_color,
                                "云端可读取消息；本机消息、待发送内容和登录状态目前保存在未加密文件中。",
                            );
                            ui.separator();
                            if let Some(session) = &self.session {
                                ui.label(format!("已登录：{}", session.username));
                                ui.horizontal(|ui| {
                                    if ui
                                        .add_enabled(
                                            !self.auth_pending && self.recovery_notice.is_none(),
                                            egui::Button::new("退出账号"),
                                        )
                                        .clicked()
                                    {
                                        actions.push(ChatAction::Cloud(CloudRequest::Logout));
                                        self.begin_auth();
                                        self.status = "正在退出云账号...".into();
                                    }
                                    if ui.button("同步表情库").clicked() {
                                        actions.push(ChatAction::Cloud(CloudRequest::ListStickers));
                                    }
                                });
                                ui.separator();
                                self.show_outbox(ui, actions);
                                ui.separator();
                                ui.label("静态 PNG / JPEG · 最大 256 KiB · 每边不超过 1024 像素");
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.image_path)
                                        .hint_text("图片路径，或把文件拖到窗口"),
                                );
                                if ui
                                    .add_enabled(
                                        self.import_worker.is_none()
                                            && !self.image_path.trim().is_empty(),
                                        egui::Button::new("导入图片"),
                                    )
                                    .clicked()
                                {
                                    self.start_import(
                                        PathBuf::from(self.image_path.trim()),
                                        window.clone(),
                                    );
                                }
                                if let Some(image) = &self.imported {
                                    self.textures.show(ui, image, 120.0, false);
                                    ui.add(
                                        egui::TextEdit::singleline(&mut self.sticker_name)
                                            .hint_text("表情名称"),
                                    );
                                    ui.horizontal_wrapped(|ui| {
                                        if ui
                                            .add_enabled(
                                                !self.sticker_name.trim().is_empty(),
                                                egui::Button::new("保存到云表情库"),
                                            )
                                            .clicked()
                                        {
                                            actions.push(ChatAction::Cloud(
                                                CloudRequest::PutSticker {
                                                    name: self.sticker_name.trim().to_owned(),
                                                    image: image.clone(),
                                                },
                                            ));
                                        }
                                        if ui
                                            .add_enabled(
                                                recipient.is_some(),
                                                egui::Button::new("发送图片"),
                                            )
                                            .clicked()
                                        {
                                            if let Some(recipient) = recipient {
                                                actions.push(image_message(recipient, image));
                                            }
                                        }
                                    });
                                }
                                ui.separator();
                                ui.label(if recipient.is_some() {
                                    "点击表情发送"
                                } else {
                                    "配对后即可发送表情"
                                });
                                if self.stickers.is_empty() {
                                    ui.label("表情库为空，可导入图片或同步表情库");
                                }
                                for sticker in &self.stickers {
                                    ui.horizontal(|ui| {
                                        let response = self.textures.show(
                                            ui,
                                            &sticker.image,
                                            64.0,
                                            recipient.is_some(),
                                        );
                                        ui.label(&sticker.name);
                                        if response.clicked() {
                                            if let Some(recipient) = recipient {
                                                actions
                                                    .push(image_message(recipient, &sticker.image));
                                            }
                                        }
                                    });
                                }
                            } else {
                                ui.label("登录后同步消息与表情；恢复码用于找回账号。");
                                ui.add_enabled_ui(!self.auth_pending, |ui| {
                                    ui.add(
                                        egui::TextEdit::singleline(&mut self.username)
                                            .hint_text("用户名"),
                                    );
                                    ui.add(
                                        egui::TextEdit::singleline(&mut self.password)
                                            .password(true)
                                            .hint_text("密码"),
                                    );
                                    let ready = !self.username.trim().is_empty()
                                        && !self.password.is_empty();
                                    ui.horizontal(|ui| {
                                        if ui
                                            .add_enabled(ready, egui::Button::new("登录"))
                                            .clicked()
                                        {
                                            actions.push(ChatAction::Cloud(CloudRequest::Login {
                                                username: self.username.trim().to_owned(),
                                                password: std::mem::take(&mut self.password),
                                                device_id: String::new(),
                                            }));
                                            self.begin_auth();
                                        }
                                        if ui
                                            .add_enabled(
                                                ready && !self.auth_pending,
                                                egui::Button::new("注册"),
                                            )
                                            .clicked()
                                        {
                                            actions.push(ChatAction::Cloud(
                                                CloudRequest::Register {
                                                    username: self.username.trim().to_owned(),
                                                    password: std::mem::take(&mut self.password),
                                                    device_id: String::new(),
                                                    legacy_token: None,
                                                },
                                            ));
                                            self.begin_auth();
                                        }
                                    });
                                    egui::CollapsingHeader::new("使用恢复码重置密码").show(
                                        ui,
                                        |ui| {
                                            ui.add(
                                                egui::TextEdit::singleline(
                                                    &mut self.recovery_input,
                                                )
                                                .password(true)
                                                .hint_text("恢复码"),
                                            );
                                            ui.add(
                                                egui::TextEdit::singleline(&mut self.new_password)
                                                    .password(true)
                                                    .hint_text("新密码"),
                                            );
                                            if ui
                                                .add_enabled(
                                                    !self.username.trim().is_empty()
                                                        && !self.recovery_input.trim().is_empty()
                                                        && !self.new_password.is_empty(),
                                                    egui::Button::new("恢复账号"),
                                                )
                                                .clicked()
                                            {
                                                actions.push(ChatAction::Cloud(
                                                    CloudRequest::Recover {
                                                        username: self.username.trim().to_owned(),
                                                        recovery_code: std::mem::take(
                                                            &mut self.recovery_input,
                                                        ),
                                                        new_password: std::mem::take(
                                                            &mut self.new_password,
                                                        ),
                                                        device_id: String::new(),
                                                    },
                                                ));
                                                self.begin_auth();
                                            }
                                        },
                                    );
                                });
                            }
                            ui.separator();
                            ui.label(&self.status);
                        });
                });
            self.account_open = open;
            if !open {
                self.outbox_clear_confirm = false;
                self.clear_credentials();
            }
        }
        if let Some(code) = &self.recovery_notice {
            let mut dismiss = false;
            egui::Modal::new(egui::Id::new("cloud-recovery-code")).show(ctx, |ui| {
                ui.set_max_width(300.0);
                ui.heading("请保存恢复码");
                ui.label("恢复码只在此显示。忘记密码时需要它，请保存到安全位置。");
                ui.label(RichText::new(code).monospace());
                if ui.button("复制恢复码").clicked() {
                    ui.ctx().copy_text(code.clone());
                }
                ui.checkbox(&mut self.recovery_saved, "我已将恢复码保存到安全位置");
                if ui
                    .add_enabled(self.recovery_saved, egui::Button::new("确认保存并关闭"))
                    .clicked()
                {
                    dismiss = true;
                }
            });
            if dismiss {
                self.recovery_notice = None;
            }
        }
    }

    fn begin_auth(&mut self) {
        self.clear_credentials();
        self.auth_pending = true;
        self.auth_deadline = Some(Instant::now() + Duration::from_secs(20));
        self.status = "正在连接云账号...".into();
    }

    fn expire_auth(&mut self, now: Instant) -> bool {
        if self.auth_deadline.is_some_and(|deadline| now >= deadline) {
            self.auth_pending = false;
            self.auth_deadline = None;
            self.clear_credentials();
            self.status = "云账号请求超时，请检查网络后重试".into();
            return true;
        }
        false
    }
}

fn image_message(recipient: &str, image: &ImageData) -> ChatAction {
    ChatAction::Cloud(CloudRequest::SendMessage {
        client_id: cloud_client::new_message_id(),
        recipient_id: recipient.to_owned(),
        body: CloudBody::Image {
            image: image.clone(),
        },
    })
}

const MAX_IMAGE_BYTES: usize = 256 * 1024;
const MAX_IMAGE_SIDE: u32 = 1024;
const MAX_TEXTURES: usize = 32;

fn import_image(path: &Path) -> Result<ImageData, String> {
    // Reject large/nonregular files before opening; still cap reads if a file grows.
    let metadata = fs::metadata(path).map_err(|_| "无法读取图片文件")?;
    if !metadata.is_file() {
        return Err("请选择普通图片文件".into());
    }
    if metadata.len() > MAX_IMAGE_BYTES as u64 {
        return Err("图片超过 256 KiB".into());
    }
    let file = fs::File::open(path).map_err(|_| "无法打开图片文件")?;
    let mut bytes = Vec::new();
    file.take(MAX_IMAGE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取图片文件")?;
    let (decoded, mime) = decode_still(&bytes)?;
    Ok(ImageData {
        mime: mime.into(),
        data_base64: STANDARD.encode(&bytes),
        width: decoded.width(),
        height: decoded.height(),
    })
}

fn decode_still(bytes: &[u8]) -> Result<(image::DynamicImage, &'static str), String> {
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err("图片超过 256 KiB".into());
    }
    let format = image::guess_format(bytes).map_err(|_| "仅支持静态 PNG 或 JPEG 图片")?;
    let mime = match format {
        image::ImageFormat::Png => {
            // Reject APNG even when its first frame would decode as an ordinary PNG.
            let mut offset = 8usize;
            while offset + 12 <= bytes.len() {
                let length =
                    u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
                if &bytes[offset + 4..offset + 8] == b"acTL" {
                    return Err("不支持动画 PNG，请选择静态图片".into());
                }
                offset = offset
                    .checked_add(length)
                    .and_then(|value| value.checked_add(12))
                    .ok_or("PNG 文件无效")?;
            }
            "image/png"
        }
        image::ImageFormat::Jpeg => "image/jpeg",
        _ => return Err("仅支持静态 PNG 或 JPEG 图片".into()),
    };
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    limits.max_alloc = Some(16 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|_| "图片无效或超过 1024 像素限制")?;
    if decoded.width() == 0
        || decoded.height() == 0
        || decoded.width() > MAX_IMAGE_SIDE
        || decoded.height() > MAX_IMAGE_SIDE
    {
        return Err("图片尺寸必须为 1 至 1024 像素".into());
    }
    Ok((decoded, mime))
}

fn decode_payload(image: &ImageData) -> Result<egui::ColorImage, String> {
    if image.data_base64.len() > MAX_IMAGE_BYTES.div_ceil(3) * 4 {
        return Err("图片超过大小限制".into());
    }
    let bytes = STANDARD
        .decode(&image.data_base64)
        .map_err(|_| "图片编码无效")?;
    let (decoded, mime) = decode_still(&bytes)?;
    if mime != image.mime || decoded.width() != image.width || decoded.height() != image.height {
        return Err("图片尺寸或格式与声明不一致".into());
    }
    let thumbnail = decoded.thumbnail(256, 256).to_rgba8();
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [thumbnail.width() as usize, thumbnail.height() as usize],
        thumbnail.as_raw(),
    ))
}

#[cfg(test)]
mod cloud_ui_tests {
    use super::*;

    fn outbox_frame(
        context: &egui::Context,
        cloud: &mut CloudUi,
        actions: &mut Vec<ChatAction>,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(420.0, 300.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| cloud.show_outbox(ui, actions));
            },
        )
    }

    fn click_outbox(
        context: &egui::Context,
        cloud: &mut CloudUi,
        actions: &mut Vec<ChatAction>,
        label: &str,
    ) {
        let output = outbox_frame(context, cloud, actions, Vec::new());
        let position = output
            .shapes
            .iter()
            .find_map(|shape| {
                if let egui::epaint::Shape::Text(text) = &shape.shape {
                    if text.galley.text() == label {
                        return Some(text.pos + text.galley.size() / 2.0);
                    }
                }
                None
            })
            .expect("outbox button must be visible");
        for pressed in [true, false] {
            outbox_frame(
                context,
                cloud,
                actions,
                vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }

    #[test]
    fn outbox_buttons_require_pending_messages_and_explicit_clear_confirmation() {
        let context = egui::Context::default();
        let mut cloud = CloudUi::default();
        let mut actions = Vec::new();
        assert_eq!((cloud.outbox_pending, cloud.outbox_blocked), (0, 0));
        click_outbox(&context, &mut cloud, &mut actions, "重试待发送");
        click_outbox(&context, &mut cloud, &mut actions, "清空待发送");
        assert!(actions.is_empty());
        assert!(!cloud.outbox_clear_confirm);

        cloud.set_outbox_status(3, 2);
        click_outbox(&context, &mut cloud, &mut actions, "重试待发送");
        assert!(matches!(actions.as_slice(), [ChatAction::RetryOutbox]));
        actions.clear();
        click_outbox(&context, &mut cloud, &mut actions, "清空待发送");
        assert!(actions.is_empty());
        assert!(cloud.outbox_clear_confirm);
        click_outbox(&context, &mut cloud, &mut actions, "取消清空");
        assert!(actions.is_empty());
        assert!(!cloud.outbox_clear_confirm);

        click_outbox(&context, &mut cloud, &mut actions, "清空待发送");
        click_outbox(&context, &mut cloud, &mut actions, "确认清空");
        assert!(matches!(actions.as_slice(), [ChatAction::ClearOutbox]));
        assert!(!cloud.outbox_clear_confirm);
        // Only the parent may change counts after actually clearing the outbox.
        assert_eq!((cloud.outbox_pending, cloud.outbox_blocked), (3, 2));
    }

    #[test]
    fn outbox_rejections_preserve_counts_and_authentication_state() {
        let mut cloud = CloudUi::default();
        cloud.set_outbox_status(2, 1);
        cloud.auth_pending = true;
        cloud.password = "draft password".into();
        cloud.send_rejected("服务繁忙", true);
        assert!(cloud.outbox_message.contains("自动重试"));
        assert!(cloud.outbox_message.contains("服务繁忙"));
        cloud.send_rejected("配对关系已变更", false);
        assert!(cloud.outbox_message.contains("待手动重试"));
        assert!(cloud.outbox_message.contains("配对关系已变更"));
        assert_eq!((cloud.outbox_pending, cloud.outbox_blocked), (2, 1));
        assert!(cloud.auth_pending);
        assert_eq!(cloud.password, "draft password");
        cloud.outbox_clear_confirm = true;
        cloud.set_outbox_status(0, 0);
        assert!(!cloud.outbox_clear_confirm);
        assert!(cloud.outbox_message.is_empty());
    }

    #[test]
    fn unsent_text_restoration_preserves_new_drafts() {
        for (draft, text, expected) in [
            ("", "未发送内容", "未发送内容"),
            ("新草稿", "未发送内容", "新草稿\n未发送内容"),
            ("新草稿\n", "未发送内容", "新草稿\n未发送内容"),
            ("新草稿", "", "新草稿"),
        ] {
            let mut input = draft.to_owned();
            append_unsent_text(&mut input, text);
            assert_eq!(input, expected);
        }
    }

    fn encoded_image(width: u32, height: u32, format: image::ImageFormat) -> Vec<u8> {
        let image = image::DynamicImage::new_rgb8(width, height);
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, format).unwrap();
        bytes.into_inner()
    }

    #[test]
    fn auth_timeout_unlocks_controls_and_clears_all_credentials() {
        let mut cloud = CloudUi::default();
        cloud.password = "password".into();
        cloud.recovery_input = "recovery".into();
        cloud.new_password = "new password".into();
        cloud.begin_auth();
        let deadline = cloud.auth_deadline.unwrap();
        assert!(
            cloud.password.is_empty()
                && cloud.new_password.is_empty()
                && cloud.recovery_input.is_empty()
        );
        assert!(!cloud.expire_auth(deadline - Duration::from_millis(1)));
        assert!(cloud.auth_pending);
        assert!(cloud.expire_auth(deadline));
        assert!(!cloud.auth_pending);
        assert!(cloud.auth_deadline.is_none());
        assert!(!cloud.expire_auth(deadline + Duration::from_secs(1)));
    }

    #[test]
    fn account_reset_discards_history_secrets_media_and_late_imports() {
        let mut cloud = CloudUi::default();
        cloud.session = Some(Session {
            user_id: "alice".into(),
            username: "Alice".into(),
            token: "secret".into(),
            server_url: "wss://example.test/ws".into(),
        });
        cloud.items.push(CloudMessage {
            seq: 1,
            client_id: "client".into(),
            sender_id: "alice".into(),
            recipient_id: "bob".into(),
            body: CloudBody::Text {
                text: "private".into(),
            },
            unix_ms: 1,
        });
        let image = ImageData {
            mime: "image/png".into(),
            data_base64: String::new(),
            width: 1,
            height: 1,
        };
        cloud.stickers.push(CloudSticker {
            id: "sticker".into(),
            name: "Private".into(),
            image: image.clone(),
        });
        cloud.imported = Some(image.clone());
        cloud.recovery_notice = Some("recovery secret".into());
        cloud.password = "password".into();
        cloud.textures.0.push_back(([0; 32], None));
        let (sender, receiver) = mpsc::sync_channel(1);
        cloud.import_worker = Some(receiver);
        cloud.set_outbox_status(2, 1);
        cloud.outbox_clear_confirm = true;
        cloud.send_rejected("暂时不可发送", false);
        cloud.reset_account();
        assert!(sender.send(Ok(image)).is_err());
        cloud.poll_import();
        assert!(cloud.session.is_none());
        assert!(cloud.items.is_empty() && cloud.stickers.is_empty() && cloud.textures.0.is_empty());
        assert!(cloud.imported.is_none() && cloud.recovery_notice.is_none());
        assert!(cloud.password.is_empty());
        assert_eq!((cloud.outbox_pending, cloud.outbox_blocked), (0, 0));
        assert!(!cloud.outbox_clear_confirm);
        assert!(cloud.outbox_message.is_empty());
    }

    #[test]
    fn png_and_jpeg_decode_and_thumbnail_with_declared_metadata() {
        for (format, mime) in [
            (image::ImageFormat::Png, "image/png"),
            (image::ImageFormat::Jpeg, "image/jpeg"),
        ] {
            let bytes = encoded_image(512, 256, format);
            let mut payload = ImageData {
                mime: mime.into(),
                data_base64: STANDARD.encode(bytes),
                width: 512,
                height: 256,
            };
            assert_eq!(decode_payload(&payload).unwrap().size, [256, 128]);
            payload.width = 513;
            assert!(decode_payload(&payload).is_err());
        }
    }

    #[test]
    fn rejects_oversized_animated_malformed_and_unsupported_images() {
        assert!(decode_still(&vec![0; MAX_IMAGE_BYTES + 1]).is_err());
        assert!(decode_still(&encoded_image(1025, 1, image::ImageFormat::Png)).is_err());
        assert!(decode_still(b"GIF89a\x01\x00\x01\x00").is_err());
        assert!(decode_still(b"not an image").is_err());
        let mut animation = b"\x89PNG\r\n\x1a\n".to_vec();
        animation.extend_from_slice(b"\x00\x00\x00\x00acTL\x00\x00\x00\x00");
        assert!(decode_still(&animation).unwrap_err().contains("动画"));
        let payload = ImageData {
            mime: "image/png".into(),
            data_base64: "A".repeat(MAX_IMAGE_BYTES.div_ceil(3) * 4 + 1),
            width: 1,
            height: 1,
        };
        assert!(decode_payload(&payload).is_err());
    }
}

#[derive(Default)]
struct ImageTextures(VecDeque<([u8; 32], Option<egui::TextureHandle>)>);

impl ImageTextures {
    fn show(
        &mut self,
        ui: &mut egui::Ui,
        image: &ImageData,
        edge: f32,
        clickable: bool,
    ) -> egui::Response {
        let width = image.width.clamp(1, MAX_IMAGE_SIDE) as f32;
        let height = image.height.clamp(1, MAX_IMAGE_SIDE) as f32;
        let scale = (edge.min(ui.available_width()) / width.max(height)).min(1.0);
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(width * scale, height * scale),
            if clickable {
                egui::Sense::click()
            } else {
                egui::Sense::hover()
            },
        );
        if !ui.is_rect_visible(rect) {
            return response;
        }
        let mut hash = Sha256::new();
        hash.update(image.mime.as_bytes());
        hash.update(image.width.to_le_bytes());
        hash.update(image.height.to_le_bytes());
        hash.update(image.data_base64.as_bytes());
        let key: [u8; 32] = hash.finalize().into();
        let cached = if let Some(index) = self.0.iter().position(|entry| entry.0 == key) {
            self.0.remove(index).unwrap()
        } else {
            let texture = decode_payload(image).ok().map(|pixels| {
                ui.ctx()
                    .load_texture("cloud-image", pixels, egui::TextureOptions::LINEAR)
            });
            (key, texture)
        };
        if let Some(texture) = &cached.1 {
            ui.painter().image(
                texture.id(),
                rect,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        } else {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "图片无效",
                egui::FontId::proportional(12.0),
                ui.visuals().text_color(),
            );
        }
        self.0.push_back(cached);
        if self.0.len() > MAX_TEXTURES {
            self.0.pop_front();
        }
        response
    }
}

fn configure_style(context: &egui::Context, dark_mode: bool) {
    context.set_theme(if dark_mode {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    });
    let colors = Palette::new(dark_mode);
    let mut style = (*context.style()).clone();
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(14.0, 8.0);
    style.spacing.interact_size = egui::vec2(36.0, 36.0);
    style
        .text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Monospace, egui::FontId::monospace(18.0));
    style.visuals = if dark_mode {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    style.visuals.window_fill = colors.background;
    style.visuals.panel_fill = colors.background;
    style.visuals.extreme_bg_color = colors.input;
    style.visuals.widgets.inactive.bg_fill = colors.button;
    style.visuals.widgets.inactive.weak_bg_fill = colors.button;
    style.visuals.widgets.inactive.fg_stroke.color = colors.text;
    style.visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0_f32, colors.border);
    // Checkbox labels share these foregrounds but sit outside the filled indicator.
    // Reserve on_accent text for buttons that explicitly paint an accent background.
    for widget in [
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.open,
    ] {
        widget.fg_stroke.color = colors.text;
        widget.bg_fill = colors.status_bg;
        widget.weak_bg_fill = colors.status_bg;
        widget.bg_stroke = egui::Stroke::new(1.0_f32, colors.accent_hover);
    }
    style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, colors.border);
    style.visuals.selection.bg_fill = colors.accent.linear_multiply(0.65);
    style.visuals.widgets.noninteractive.fg_stroke.color = colors.text;
    style.visuals.window_corner_radius = egui::CornerRadius::same(14);
    style.visuals.menu_corner_radius = egui::CornerRadius::same(10);
    let widget_corner_radius = egui::CornerRadius::same(10);
    style.visuals.widgets.noninteractive.corner_radius = widget_corner_radius;
    style.visuals.widgets.inactive.corner_radius = widget_corner_radius;
    style.visuals.widgets.hovered.corner_radius = widget_corner_radius;
    style.visuals.widgets.active.corner_radius = widget_corner_radius;
    style.visuals.widgets.open.corner_radius = widget_corner_radius;
    // Keep focus and hover outlines on the same geometry as the resting control.
    for widget in [
        &mut style.visuals.widgets.noninteractive,
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.open,
    ] {
        widget.expansion = 0.0;
        widget.bg_stroke.width = 1.0;
    }
    context.set_style(style);
}

#[derive(Clone, Copy)]
struct Palette {
    background: Color32,
    card: Color32,
    input: Color32,
    button: Color32,
    border: Color32,
    text: Color32,
    muted: Color32,
    accent: Color32,
    on_accent: Color32,
    accent_hover: Color32,
    danger: Color32,
    danger_bg: Color32,
    status_bg: Color32,
    outgoing: Color32,
    incoming: Color32,
}

impl Palette {
    fn new(dark: bool) -> Self {
        if dark {
            Self {
                background: Color32::from_rgb(25, 28, 26),
                card: Color32::from_rgb(35, 39, 36),
                input: Color32::from_rgb(28, 32, 29),
                button: Color32::from_rgb(47, 53, 48),
                border: Color32::from_rgb(63, 70, 64),
                text: Color32::from_rgb(238, 237, 230),
                muted: Color32::from_rgb(158, 166, 158),
                accent: Color32::from_rgb(232, 177, 65),
                on_accent: Color32::from_rgb(38, 31, 16),
                accent_hover: Color32::from_rgb(244, 193, 83),
                danger: Color32::from_rgb(226, 116, 101),
                danger_bg: Color32::from_rgb(77, 43, 39),
                status_bg: Color32::from_rgb(35, 53, 41),
                outgoing: Color32::from_rgb(55, 79, 60),
                incoming: Color32::from_rgb(48, 51, 47),
            }
        } else {
            Self {
                background: Color32::from_rgb(246, 244, 237),
                card: Color32::from_rgb(255, 253, 247),
                input: Color32::from_rgb(250, 249, 244),
                button: Color32::from_rgb(237, 234, 224),
                border: Color32::from_rgb(219, 215, 202),
                text: Color32::from_rgb(43, 48, 44),
                muted: Color32::from_rgb(111, 119, 112),
                accent: Color32::from_rgb(190, 125, 24),
                on_accent: Color32::from_rgb(30, 25, 14),
                accent_hover: Color32::from_rgb(218, 151, 38),
                danger: Color32::from_rgb(184, 72, 61),
                danger_bg: Color32::from_rgb(249, 229, 224),
                status_bg: Color32::from_rgb(226, 240, 228),
                outgoing: Color32::from_rgb(214, 237, 211),
                incoming: Color32::from_rgb(238, 234, 224),
            }
        }
    }
}

fn spaced_code(code: &str) -> String {
    code.chars()
        .map(|character| character.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

fn status_color(status: &str, colors: Palette) -> Color32 {
    if status.contains("不存在")
        || status.contains("失败")
        || status.contains("invalid")
        || status.contains("already")
    {
        Color32::from_rgb(211, 85, 74)
    } else {
        colors.muted
    }
}

fn configure_fonts(context: &egui::Context) {
    #[cfg(target_os = "windows")]
    let candidates = [r"C:\Windows\Fonts\msyh.ttc", r"C:\Windows\Fonts\simhei.ttf"];
    #[cfg(target_os = "macos")]
    let candidates = [
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/STHeiti Medium.ttc",
    ];
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let candidates = ["/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", ""];
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "pairing-code".into(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../assets/fonts/JetBrainsMonoSlashedNL-Medium.otf"
        ))),
    );
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .insert(0, "pairing-code".into());
    if let Some(bytes) = candidates.iter().find_map(|path| fs::read(path).ok()) {
        fonts.font_data.insert(
            "chinese".into(),
            Arc::new(egui::FontData::from_owned(bytes)),
        );
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "chinese".into());
    }
    context.set_fonts(fonts);
}
