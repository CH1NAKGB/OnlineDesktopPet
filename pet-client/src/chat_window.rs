use std::{
    fs,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Context;
use egui::{Color32, RichText};
use egui_wgpu::{Renderer as EguiRenderer, ScreenDescriptor};
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

pub enum ChatAction {
    Pair(String),
    Send(String),
    Rename(String),
    SetLaunchAtLogin(bool),
    RequestUnpair,
    RespondUnpair(bool),
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
    input: String,
    pair_input: String,
    own_code: String,
    partner_id: Option<String>,
    partner_display_name: Option<String>,
    display_name: String,
    edit_display_name: String,
    editing_name: bool,
    startup_enabled: bool,
    focus_pair_input: bool,
    focus_deadline: Option<Instant>,
    unpair_dialog: Option<UnpairDialog>,
    status: String,
    network_status: String,
    dark_mode: bool,
    pending_size: Option<winit::dpi::PhysicalSize<u32>>,
    resize_deadline: Option<Instant>,
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
        Ok(Self {
            window,
            surface,
            device,
            queue,
            config,
            context,
            state,
            renderer,
            history: chat::load_history(),
            input: String::new(),
            pair_input: String::new(),
            own_code,
            partner_id,
            partner_display_name: None,
            edit_display_name: display_name.clone(),
            display_name,
            editing_name: false,
            startup_enabled: crate::startup::is_enabled(),
            focus_pair_input: true,
            focus_deadline: None,
            unpair_dialog: None,
            status: "等待输入对方配对码".into(),
            network_status,
            dark_mode,
            pending_size: None,
            resize_deadline: None,
        })
    }

    pub fn id(&self) -> WindowId {
        self.window.id()
    }

    pub fn show(&mut self) {
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

    pub fn set_status(&mut self, status: impl Into<String>) {
        self.status = status.into();
        self.window.request_redraw();
    }

    pub fn push_message(&mut self, entry: ChatEntry) {
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
            WindowEvent::CloseRequested => self.window.set_visible(false),
            WindowEvent::Resized(size) if size.width > 0 && size.height > 0 => {
                self.pending_size = Some(*size);
                self.resize_deadline = Some(Instant::now() + Duration::from_millis(40));
            }
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
        let raw_input = self.state.take_egui_input(&self.window);
        let mut actions = Vec::new();
        let partner_id = self.partner_id.clone();
        let colors = Palette::new(self.dark_mode);
        let output = self.context.run(raw_input, |ctx| {
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::default()
                        .fill(colors.background)
                        .inner_margin(16.0),
                )
                .show(ctx, |ui| {
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
                            for entry in &self.history {
                                let fill = if entry.outgoing {
                                    colors.outgoing
                                } else {
                                    colors.incoming
                                };
                                ui.with_layout(
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
                                                ui.label(
                                                    RichText::new(&entry.text)
                                                        .size(15.0)
                                                        .color(colors.text),
                                                );
                                            });
                                    },
                                );
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
}

fn rounded_button(text: impl Into<egui::WidgetText>) -> egui::Button<'static> {
    egui::Button::new(text).corner_radius(egui::CornerRadius::same(10))
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
    style.visuals.widgets.hovered.fg_stroke.color = colors.on_accent;
    style.visuals.widgets.active.fg_stroke.color = colors.on_accent;
    style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, colors.border);
    style.visuals.widgets.active.bg_fill = colors.accent;
    style.visuals.widgets.active.weak_bg_fill = colors.accent;
    style.visuals.widgets.hovered.bg_fill = colors.accent_hover;
    style.visuals.widgets.hovered.weak_bg_fill = colors.accent_hover;
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
