//! GPU renderer. Knows nothing about windows or audio: it takes a surface
//! target and a size, and draws a frame.

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    frame: u64,
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
        surface.configure(&device, &config);

        Self {
            surface,
            device,
            queue,
            config,
            frame: 0,
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

    /// `level` is the current loudness, 0..1.
    pub fn render(&mut self, level: f32) {
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

        // Placeholder until the spectrum drives the visuals: slowly cycle
        // the hue, with brightness following the loudness.
        let t = self.frame as f64 / 60.0;
        self.frame += 1;
        let brightness = 0.1 + 0.9 * level as f64;
        let color = wgpu::Color {
            r: brightness * (0.5 + 0.5 * (t * 0.7).sin()),
            g: brightness * (0.5 + 0.5 * (t * 0.7 + 2.1).sin()),
            b: brightness * (0.5 + 0.5 * (t * 0.7 + 4.2).sin()),
            a: 1.0,
        };

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(color),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);

        if suboptimal {
            self.surface.configure(&self.device, &self.config);
        }
    }
}
