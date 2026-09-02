use std::{fs, sync::Arc};

use anyhow::Context;
use winit::{dpi::PhysicalSize, window::Window};

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    max_surface_dimension: u32,
    pipeline: wgpu::RenderPipeline,
    time_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    bubble_context: egui::Context,
    bubble_renderer: egui_wgpu::Renderer,
    pixels_per_point: f32,
}

impl Renderer {
    pub async fn new(window: Arc<Window>) -> anyhow::Result<Self> {
        let size = window.inner_size();
        let pixels_per_point = window.scale_factor() as f32;
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let surface = instance.create_surface(window)?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
            })
            .await
            .context("no compatible GPU adapter")?;
        let mut required_limits = wgpu::Limits::downlevel_defaults();
        required_limits.max_texture_dimension_2d = adapter.limits().max_texture_dimension_2d;
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("pet-renderer"),
                    required_features: wgpu::Features::empty(),
                    required_limits,
                    memory_hints: wgpu::MemoryHints::MemoryUsage,
                },
                None,
            )
            .await?;
        // Remote and software adapters can reject oversized reported window dimensions.
        let max_surface_dimension = device.limits().max_texture_dimension_2d;
        let capabilities = surface.get_capabilities(&adapter);
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .unwrap_or(capabilities.formats[0]);
        let alpha_mode = capabilities
            .alpha_modes
            .iter()
            .copied()
            .find(|mode| {
                matches!(
                    mode,
                    wgpu::CompositeAlphaMode::PreMultiplied
                        | wgpu::CompositeAlphaMode::PostMultiplied
                )
            })
            .unwrap_or(capabilities.alpha_modes[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: Self::surface_dimension(size.width, max_surface_dimension),
            height: Self::surface_dimension(size.height, max_surface_dimension),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);
        let time_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("animation-time"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("animation-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("animation-bind-group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: time_buffer.as_entire_binding(),
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("original-chicken-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("chicken.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pet-pipeline-layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pet-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let bubble_context = egui::Context::default();
        configure_bubble_font(&bubble_context);
        let bubble_renderer = egui_wgpu::Renderer::new(&device, format, None, 1, false);
        Ok(Self {
            surface,
            device,
            queue,
            config,
            max_surface_dimension,
            pipeline,
            time_buffer,
            bind_group,
            bubble_context,
            bubble_renderer,
            pixels_per_point,
        })
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        self.config.width = Self::surface_dimension(size.width, self.max_surface_dimension);
        self.config.height = Self::surface_dimension(size.height, self.max_surface_dimension);
        self.surface.configure(&self.device, &self.config);
    }

    pub fn set_scale_factor(&mut self, scale_factor: f64) {
        self.pixels_per_point = scale_factor.max(1.0) as f32;
    }

    fn surface_dimension(requested: u32, maximum: u32) -> u32 {
        requested.max(1).min(maximum.max(1))
    }

    pub fn render(
        &mut self,
        seconds: f32,
        petting: f32,
        skin: f32,
        bubble: Option<&str>,
    ) -> Result<(), wgpu::SurfaceError> {
        self.queue.write_buffer(
            &self.time_buffer,
            0,
            bytemuck::cast_slice(&[seconds, petting, skin, 0.0]),
        );
        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pet-frame"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pet-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            let pet_size = self.config.width.min(self.config.height);
            pass.set_viewport(
                0.0,
                (self.config.height - pet_size) as f32,
                pet_size as f32,
                pet_size as f32,
                0.0,
                1.0,
            );
            pass.draw(0..3, 0..1);
        }
        let mut command_buffers = Vec::new();
        if let Some(text) = bubble {
            let screen_points = egui::vec2(
                self.config.width as f32 / self.pixels_per_point,
                self.config.height as f32 / self.pixels_per_point,
            );
            let raw_input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, screen_points)),
                ..Default::default()
            };
            let output = self.bubble_context.run(raw_input, |context| {
                egui::Area::new(egui::Id::new("incoming-bubble"))
                    .fixed_pos(egui::pos2(4.0, 4.0))
                    .show(context, |ui| {
                        egui::Frame::default()
                            .fill(egui::Color32::from_rgba_premultiplied(255, 247, 220, 245))
                            .stroke(egui::Stroke::new(
                                1.0_f32,
                                egui::Color32::from_rgba_premultiplied(128, 104, 65, 115),
                            ))
                            .corner_radius(10.0)
                            .inner_margin(10.0)
                            .show(ui, |ui| {
                                ui.set_width(screen_points.x - 28.0);
                                ui.label(
                                    egui::RichText::new(text)
                                        .size(16.0)
                                        .strong()
                                        .color(egui::Color32::from_rgb(55, 47, 36)),
                                );
                            });
                    });
            });
            let jobs = self
                .bubble_context
                .tessellate(output.shapes, output.pixels_per_point);
            for (id, delta) in &output.textures_delta.set {
                self.bubble_renderer
                    .update_texture(&self.device, &self.queue, *id, delta);
            }
            let descriptor = egui_wgpu::ScreenDescriptor {
                size_in_pixels: [self.config.width, self.config.height],
                pixels_per_point: self.pixels_per_point,
            };
            command_buffers.extend(self.bubble_renderer.update_buffers(
                &self.device,
                &self.queue,
                &mut encoder,
                &jobs,
                &descriptor,
            ));
            {
                let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("bubble-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    occlusion_query_set: None,
                    timestamp_writes: None,
                });
                self.bubble_renderer
                    .render(&mut pass.forget_lifetime(), &jobs, &descriptor);
            }
            for id in &output.textures_delta.free {
                self.bubble_renderer.free_texture(id);
            }
        }
        self.queue
            .submit(command_buffers.into_iter().chain([encoder.finish()]));
        output.present();
        Ok(())
    }
}

fn configure_bubble_font(context: &egui::Context) {
    let Some(bytes) = [
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/STHeiti Medium.ttc",
    ]
    .iter()
    .find_map(|path| fs::read(path).ok()) else {
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
