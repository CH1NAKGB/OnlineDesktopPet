use std::{fs, sync::Arc};

use anyhow::Context;
use egui::{Color32, RichText};
use egui_wgpu::{Renderer as EguiRenderer, ScreenDescriptor};
use winit::{
    dpi::LogicalSize,
    event::WindowEvent,
    event_loop::ActiveEventLoop,
    window::{Window, WindowId},
};

use crate::chat::{self, ChatEntry};

pub enum ChatAction {
    Pair(String),
    Send(String),
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
    status: String,
}

impl ChatWindow {
    pub async fn new(
        event_loop: &ActiveEventLoop,
        own_code: String,
        partner_id: Option<String>,
    ) -> anyhow::Result<Self> {
        let attributes = Window::default_attributes()
            .with_title("Chick Link Chat")
            .with_inner_size(LogicalSize::new(430.0, 640.0))
            .with_min_inner_size(LogicalSize::new(360.0, 500.0));
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
        configure_style(&context);
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
            status: "Ready".into(),
        })
    }

    pub fn id(&self) -> WindowId {
        self.window.id()
    }

    pub fn show(&self) {
        self.window.set_visible(true);
        self.window.focus_window();
        self.window.request_redraw();
    }

    pub fn set_partner(&mut self, partner_id: String) {
        self.partner_id = Some(partner_id);
        self.status = "Paired and online".into();
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
                self.config.width = size.width;
                self.config.height = size.height;
                self.surface.configure(&self.device, &self.config);
            }
            WindowEvent::RedrawRequested => return self.render(),
            _ => {}
        }
        Ok(Vec::new())
    }

    fn render(&mut self) -> anyhow::Result<Vec<ChatAction>> {
        let raw_input = self.state.take_egui_input(&self.window);
        let mut actions = Vec::new();
        let partner_id = self.partner_id.clone();
        let output = self.context.run(raw_input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.heading(
                    RichText::new("Chick Link")
                        .size(25.0)
                        .color(Color32::from_rgb(42, 55, 48)),
                );
                ui.label(RichText::new(&self.status).color(Color32::from_rgb(74, 119, 91)));
                ui.add_space(10.0);
                if partner_id.is_none() {
                    ui.group(|ui| {
                        ui.label("我的配对码");
                        ui.label(
                            RichText::new(&self.own_code)
                                .monospace()
                                .size(28.0)
                                .strong(),
                        );
                        if ui.button("复制配对码").clicked() {
                            ui.ctx().copy_text(self.own_code.clone());
                        }
                    });
                    ui.add_space(16.0);
                    ui.label("输入对方的 8 位配对码");
                    let response = ui.text_edit_singleline(&mut self.pair_input);
                    self.pair_input.make_ascii_uppercase();
                    let valid = shared_protocol::is_valid_pairing_code(self.pair_input.trim());
                    if ui
                        .add_enabled(valid, egui::Button::new("建立共同小鸡"))
                        .clicked()
                        || (valid
                            && response.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                    {
                        actions.push(ChatAction::Pair(self.pair_input.trim().to_owned()));
                        self.status = "Pairing...".into();
                    }
                    return;
                }

                ui.horizontal(|ui| {
                    ui.label("共同小鸡");
                    ui.strong(partner_id.as_deref().unwrap_or_default());
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .max_height(ui.available_height() - 150.0)
                    .show(ui, |ui| {
                        for entry in &self.history {
                            let fill = if entry.outgoing {
                                Color32::from_rgb(205, 240, 190)
                            } else {
                                Color32::from_rgb(243, 236, 215)
                            };
                            ui.with_layout(
                                if entry.outgoing {
                                    egui::Layout::right_to_left(egui::Align::TOP)
                                } else {
                                    egui::Layout::left_to_right(egui::Align::TOP)
                                },
                                |ui| {
                                    egui::Frame::default().fill(fill).inner_margin(10.0).show(
                                        ui,
                                        |ui| {
                                            ui.set_max_width(300.0);
                                            ui.label(&entry.text);
                                        },
                                    );
                                },
                            );
                            ui.add_space(5.0);
                        }
                    });
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    for quick in ["想你啦", "摸摸小鸡", "忙完找你", "晚安"] {
                        if ui.small_button(quick).clicked() {
                            actions.push(ChatAction::Send(quick.into()));
                        }
                    }
                });
                ui.horizontal(|ui| {
                    let response = ui.add_sized(
                        [ui.available_width() - 72.0, 52.0],
                        egui::TextEdit::multiline(&mut self.input).hint_text("输入消息..."),
                    );
                    let submit = ui
                        .add_sized([64.0, 52.0], egui::Button::new("发送"))
                        .clicked()
                        || (response.has_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift));
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
            });
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
                            r: 0.96,
                            g: 0.94,
                            b: 0.88,
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

fn configure_style(context: &egui::Context) {
    let mut style = (*context.style()).clone();
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.visuals.window_fill = Color32::from_rgb(247, 241, 224);
    style.visuals.panel_fill = Color32::from_rgb(247, 241, 224);
    style.visuals.widgets.active.bg_fill = Color32::from_rgb(232, 171, 61);
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(242, 191, 88);
    context.set_style(style);
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
    let Some(bytes) = candidates.iter().find_map(|path| fs::read(path).ok()) else {
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "chinese".into(),
        Arc::new(egui::FontData::from_owned(bytes)),
    );
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "chinese".into());
    context.set_fonts(fonts);
}
