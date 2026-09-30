//! GPU renderer. Knows nothing about windows or audio: it takes a surface
//! target and a size, and draws a frame from a [`Spectrum`].

use std::sync::Arc;

use analysis::{BAND_COUNT, Spectrum};
use bytemuck::{Pod, Zeroable};

/// Code shared by all visuals. Appended after each visual's code, so errors
/// report line numbers matching what the user wrote (WGSL allows using
/// declarations before they appear).
const COMMON_SHADER: &str = include_str!("shaders/common.wgsl");

/// Built-in visuals: each is a fragment shader defining `fs_main`, selected
/// with [`Renderer::set_visual`].
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
    params: [f32; 4],
    notes: [[f32; 4]; MIDI_COUNT / 4],
    controls: [[f32; 4]; MIDI_COUNT / 4],
}

/// Number of MIDI notes and controllers shaders can read.
pub const MIDI_COUNT: usize = 128;

struct Visual {
    name: &'static str,
    builtin: &'static str,
    /// The source as last edited. It may not compile, in which case
    /// `pipeline` still holds the last version that did.
    source: String,
    pipeline: wgpu::RenderPipeline,
}

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline_layout: wgpu::PipelineLayout,
    visuals: Vec<Visual>,
    visual: usize,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    params: [f32; 4],
    notes: [f32; MIDI_COUNT],
    controls: [f32; MIDI_COUNT],
}

/// Why a shader doesn't compile.
#[derive(Clone, Debug)]
pub struct ShaderError {
    /// Readable message, quoting the offending code.
    pub message: String,
    /// Line of the error in the visual's source, starting at 1, if known.
    pub line: Option<usize>,
}

impl std::fmt::Display for ShaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Checks that a visual's source compiles.
pub fn validate_shader(source: &str) -> Result<(), ShaderError> {
    let full = full_source(source);
    let module = naga::front::wgsl::parse_str(&full).map_err(|e| ShaderError {
        message: e.emit_to_string_with_path(&full, "shader"),
        line: e.location(&full).map(|l| l.line_number as usize),
    })?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .map_err(|e| ShaderError {
        message: e.emit_to_string_with_path(&full, "shader"),
        line: e.location(&full).map(|l| l.line_number as usize),
    })?;
    Ok(())
}

fn full_source(source: &str) -> String {
    format!("{source}\n{COMMON_SHADER}")
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
        // Log GPU errors instead of panicking (the default). Shaders are
        // validated before use, but a browser may still reject one.
        device.on_uncaptured_error(Arc::new(|error| log::error!("GPU error: {error}")));

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

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("visuals"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            ..Default::default()
        });

        let mut renderer = Self {
            surface,
            device,
            queue,
            config,
            pipeline_layout,
            visuals: Vec::new(),
            visual: 0,
            uniform_buffer,
            bind_group,
            params: [0.0; 4],
            notes: [0.0; MIDI_COUNT],
            controls: [0.0; MIDI_COUNT],
        };
        renderer.visuals = VISUALS
            .iter()
            .map(|&(name, builtin)| Visual {
                name,
                builtin,
                source: builtin.to_owned(),
                pipeline: renderer.create_pipeline(name, builtin),
            })
            .collect();
        renderer
    }

    fn create_pipeline(&self, name: &str, source: &str) -> wgpu::RenderPipeline {
        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(name),
                source: wgpu::ShaderSource::Wgsl(full_source(source).into()),
            });
        self.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(name),
                layout: Some(&self.pipeline_layout),
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
                    targets: &[Some(self.config.format.into())],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Switches to the visual at `index`.
    pub fn set_visual(&mut self, index: usize) {
        assert!(index < self.visuals.len());
        self.visual = index;
        log::info!("visual: {}", self.visuals[self.visual].name);
    }

    pub fn visual_count(&self) -> usize {
        self.visuals.len()
    }

    /// Index of the visual being displayed.
    pub fn current_visual(&self) -> usize {
        self.visual
    }

    pub fn visual_name(&self, index: usize) -> &'static str {
        self.visuals[index].name
    }

    /// The visual's source as last edited, which may not compile.
    pub fn visual_source(&self, index: usize) -> &str {
        &self.visuals[index].source
    }

    pub fn builtin_source(&self, index: usize) -> &'static str {
        self.visuals[index].builtin
    }

    /// Values shaders read as `u.params`, e.g. from sliders.
    pub fn set_params(&mut self, params: [f32; 4]) {
        self.params = params;
    }

    /// MIDI values shaders read with `note(n)` and `cc(n)`, 0..1 each.
    pub fn set_midi(&mut self, notes: &[f32; MIDI_COUNT], controls: &[f32; MIDI_COUNT]) {
        self.notes = *notes;
        self.controls = *controls;
    }

    /// Replaces a visual's source. If it doesn't compile, the error is
    /// returned and the visual keeps running its last working version.
    pub fn set_visual_source(&mut self, index: usize, source: String) -> Result<(), ShaderError> {
        let result = validate_shader(&source);
        if result.is_ok() {
            self.visuals[index].pipeline = self.create_pipeline(self.visuals[index].name, &source);
        }
        self.visuals[index].source = source;
        result
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
            params: self.params,
            notes: bytemuck::cast(self.notes),
            controls: bytemuck::cast(self.controls),
        };
        bytemuck::cast_slice_mut::<_, f32>(&mut uniforms.bands).copy_from_slice(&spectrum.bands);
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(self.visuals[self.visual].name),
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
            pass.set_pipeline(&self.visuals[self.visual].pipeline);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_visuals_compile() {
        for (name, source) in VISUALS {
            if let Err(error) = validate_shader(source) {
                panic!("{name} does not compile:\n{error}");
            }
        }
    }

    #[test]
    fn errors_point_at_the_users_line() {
        let source =
            "@fragment\nfn fs_main() -> @location(0) vec4<f32> {\n    return vec4<f32>(oops);\n}\n";
        let error = validate_shader(source).unwrap_err();
        assert!(error.message.contains("oops"), "{error}");
        assert!(error.message.contains("shader:3:"), "{error}");
        assert_eq!(error.line, Some(3));
    }
}
