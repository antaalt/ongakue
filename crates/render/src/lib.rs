//! GPU renderer. Knows nothing about windows or audio: it takes a surface
//! target and a size, and draws a frame from a [`Spectrum`].

use analysis::{BAND_COUNT, Spectrum};
use bytemuck::{Pod, Zeroable};

/// Code shared by all visuals, prepended to each of them.
const COMMON_SHADER: &str = include_str!("shaders/common.wgsl");

/// Each visual is a fragment shader, cycled through with [`Renderer::next_visual`].
const VISUALS: &[(&str, &str)] = &[
    ("radial", include_str!("shaders/radial.wgsl")),
    ("bars", include_str!("shaders/bars.wgsl")),
];

/// Mirrors `Uniforms` in `common.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    bands: [[f32; 4]; BAND_COUNT / 4],
    resolution: [f32; 2],
    time: f32,
    beat: f32,
    bass: f32,
    mid: f32,
    treble: f32,
    _padding: f32,
}

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipelines: Vec<wgpu::RenderPipeline>,
    visual: usize,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl Renderer {
    pub async fn new(
        target: impl Into<wgpu::SurfaceTarget<'static>>,
        width: u32,
        height: u32,
    ) -> Self {
        #[cfg(target_arch = "wasm32")]
        let instance = wgpu::util::new_instance_with_webgpu_detection(
            wgpu::InstanceDescriptor::new_without_display_handle(),
        )
        .await;
        #[cfg(not(target_arch = "wasm32"))]
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());

        let surface = instance
            .create_surface(target)
            .expect("failed to create surface");

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .expect("no suitable GPU adapter");
        log::info!("using adapter: {:?}", adapter.get_info());

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: None,
                // WebGL2 is the lowest common denominator we target.
                required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                    .using_resolution(adapter.limits()),
                ..Default::default()
            })
            .await
            .expect("failed to create device");

        let mut config = surface
            .get_default_config(&adapter, width.max(1), height.max(1))
            .expect("surface not supported by adapter");
        config.present_mode = wgpu::PresentMode::AutoVsync;
        // Browsers only offer non-sRGB surfaces; use the same natively so the
        // shader colors look identical everywhere.
        config.format = config.format.remove_srgb_suffix();
        surface.configure(&device, &config);

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniforms"),
            size: size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("uniforms"),
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
            label: Some("uniforms"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("visuals"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            ..Default::default()
        });
        let pipelines = VISUALS
            .iter()
            .map(|(name, source)| {
                let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(name),
                    source: wgpu::ShaderSource::Wgsl(format!("{COMMON_SHADER}\n{source}").into()),
                });
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(name),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs_main"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("fs_main"),
                        compilation_options: Default::default(),
                        targets: &[Some(config.format.into())],
                    }),
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    multiview_mask: None,
                    cache: None,
                })
            })
            .collect();

        Self {
            surface,
            device,
            queue,
            config,
            pipelines,
            visual: 0,
            uniform_buffer,
            bind_group,
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Switches to the next visual, wrapping around.
    pub fn next_visual(&mut self) {
        self.visual = (self.visual + 1) % self.pipelines.len();
        log::info!("visual: {}", VISUALS[self.visual].0);
    }

    /// `time` is in seconds since start, and drives the animations.
    pub fn render(&mut self, spectrum: &Spectrum, time: f32) {
        let (frame, suboptimal) = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => (frame, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => (frame, true),
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            // Timeout, occluded window, validation error: skip this frame.
            _ => return,
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut uniforms = Uniforms {
            bands: [[0.0; 4]; BAND_COUNT / 4],
            resolution: [self.config.width as f32, self.config.height as f32],
            time,
            beat: spectrum.beat,
            bass: spectrum.bass,
            mid: spectrum.mid,
            treble: spectrum.treble,
            _padding: 0.0,
        };
        bytemuck::cast_slice_mut::<_, f32>(&mut uniforms.bands).copy_from_slice(&spectrum.bands);
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(VISUALS[self.visual].0),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipelines[self.visual]);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);

        if suboptimal {
            self.surface.configure(&self.device, &self.config);
        }
    }
}
