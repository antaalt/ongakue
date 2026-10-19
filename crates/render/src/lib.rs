//! GPU renderer. Knows nothing about windows or audio: it takes a surface
//! target and a size, and draws a frame from a [`Spectrum`].

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use analysis::{BAND_COUNT, Spectrum};
use bytemuck::{Pod, Zeroable};

mod guard;
pub use guard::MAX_ITERATIONS;

/// Code shared by all visuals. Appended after each visual's code, so errors
/// report line numbers matching what the user wrote (WGSL allows using
/// declarations before they appear).
const COMMON_SHADER: &str = include_str!("shaders/common.wgsl");

/// The same for GLSL visuals: the prelude goes before the visual's code (GLSL
/// needs declarations first), and `main`, calling its `mainImage`, after.
const GLSL_PRELUDE: &str = include_str!("shaders/common.glsl");
const GLSL_MAIN: &str = include_str!("shaders/main.glsl");

/// Built-in visuals: each is a fragment shader defining `fs_main`, selected
/// with [`Renderer::set_visual`].
const VISUALS: &[(&str, &str)] = &[
    ("radial", include_str!("shaders/radial.wgsl")),
    ("bars", include_str!("shaders/bars.wgsl")),
    // Shows the value of every input, to check what shaders receive.
    ("debug", include_str!("shaders/debug.wgsl")),
    // A GLSL visual, written like on Shadertoy.
    ("glsl", include_str!("shaders/example.glsl")),
];

/// Starting point for new visuals. Also drawn by a new visual whose code
/// doesn't compile yet, since it has no working version of its own.
pub const TEMPLATE_SHADER: &str = include_str!("shaders/template.wgsl");

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
    name: String,
    /// The original source of a built-in visual; `None` for visuals added
    /// with [`Renderer::add_visual`].
    builtin: Option<&'static str>,
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
    /// Set when the GPU device is lost, e.g. reset by the system because a
    /// shader ran too long.
    lost: Arc<AtomicBool>,
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

/// The language a visual is written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderLanguage {
    /// Defines `fs_main`.
    Wgsl,
    /// Defines `mainImage`, like on Shadertoy.
    Glsl,
}

/// Tells the language of a visual's code: GLSL visuals define
/// `void mainImage(…)`. Comments don't count, so mentioning `mainImage` in
/// one doesn't make WGSL code GLSL.
pub fn shader_language(source: &str) -> ShaderLanguage {
    let mut code = String::with_capacity(source.len());
    let mut rest = source;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("//") {
            rest = after.find('\n').map_or("", |end| &after[end..]);
        } else if let Some(after) = rest.strip_prefix("/*") {
            rest = after.find("*/").map_or("", |end| &after[end + 2..]);
            code.push(' ');
        } else {
            let c = rest.chars().next().unwrap();
            code.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    // Any spacing between `void` and the name.
    let words: Vec<&str> = code.split_whitespace().collect();
    let defines_main_image = words
        .windows(2)
        .any(|pair| pair[0] == "void" && pair[1].starts_with("mainImage"));
    if defines_main_image {
        ShaderLanguage::Glsl
    } else {
        ShaderLanguage::Wgsl
    }
}

/// Checks that a visual's source compiles.
pub fn validate_shader(source: &str) -> Result<(), ShaderError> {
    compile(source).map(|_| ())
}

/// A visual's code, ready for the GPU: WGSL defining the fragment shader
/// `fs_main` (GLSL is translated), with its loops guarded (see `guard.rs`).
fn compile(source: &str) -> Result<String, ShaderError> {
    match shader_language(source) {
        ShaderLanguage::Wgsl => {
            let full = format!("{source}\n{COMMON_SHADER}");
            let module = naga::front::wgsl::parse_str(&full).map_err(|e| ShaderError {
                message: e.emit_to_string_with_path(&full, "shader"),
                line: e.location(&full).map(|l| l.line_number as usize),
            })?;
            validate(&module).map_err(|e| ShaderError {
                message: e.emit_to_string_with_path(&full, "shader"),
                line: e.location(&full).map(|l| l.line_number as usize),
            })?;
            guarded_wgsl(module)
        }
        ShaderLanguage::Glsl => compile_glsl(source),
    }
}

/// The module as WGSL, with its loops guarded. Only for modules that are
/// valid: the errors are the user's code's, before guarding.
fn guarded_wgsl(mut module: naga::Module) -> Result<String, ShaderError> {
    guard::guard_loops(&mut module);
    // A guard can leave a loop at a different iteration in each pixel, so
    // derivatives (`dpdx`, `fwidth`…) after a loop are, strictly, no longer
    // in uniform control flow. Accept them, as without the guards.
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all() - naga::valid::ValidationFlags::CONTROL_FLOW_UNIFORMITY,
        naga::valid::Capabilities::empty(),
    )
    .validate(&module);
    let internal = |error: String| ShaderError {
        message: format!("error: internal, while guarding loops: {error}"),
        line: None,
    };
    let info = info.map_err(|e| internal(e.into_inner().to_string()))?;
    let wgsl =
        naga::back::wgsl::write_string(&module, &info, naga::back::wgsl::WriterFlags::empty())
            .map_err(|e| internal(e.to_string()))?;
    Ok(format!("diagnostic(off, derivative_uniformity);\n{wgsl}"))
}

type ValidationError = Box<naga::WithSpan<naga::valid::ValidationError>>;

fn validate(module: &naga::Module) -> Result<naga::valid::ModuleInfo, ValidationError> {
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(module)
    .map_err(Box::new)
}

/// Translates a GLSL visual to WGSL, with errors pointing at the visual's own
/// lines (not the prelude's).
fn compile_glsl(source: &str) -> Result<String, ShaderError> {
    let prelude = GLSL_PRELUDE.trim_end();
    let full = format!("{prelude}\n{source}\n{GLSL_MAIN}");
    // Lines before the visual's first one.
    let offset = prelude.lines().count();
    let user_lines = source.lines().count().max(1);
    let error = |kind: String, location: Option<naga::SourceLocation>| {
        let line = location
            .map(|l| l.line_number as usize)
            .filter(|&line| line > offset && line <= offset + user_lines)
            .map(|line| (line - offset, location.unwrap().line_position as usize));
        ShaderError {
            message: located_message(&kind, source, line),
            line: line.map(|(line, _)| line),
        }
    };

    let options = naga::front::glsl::Options::from(naga::ShaderStage::Fragment);
    let module = naga::front::glsl::Frontend::default()
        .parse(&options, &full)
        .map_err(|errors| {
            let first = &errors.errors[0];
            let mut kind = first.kind.to_string();
            if errors.errors.len() > 1 {
                kind += &format!(" (and {} more)", errors.errors.len() - 1);
            }
            error(kind, first.location(&full))
        })?;
    validate(&module).map_err(|e| {
        // The details are in the chain of causes.
        let mut kind = e.as_inner().to_string();
        let mut cause = std::error::Error::source(e.as_inner());
        while let Some(inner) = cause {
            kind += &format!(": {inner}");
            cause = inner.source();
        }
        error(kind, e.location(&full))
    })?;
    // The translation's entry point is GLSL's `main`.
    let mut module = module;
    for entry_point in &mut module.entry_points {
        entry_point.name = "fs_main".to_owned();
    }
    Ok(format!("{}\n{COMMON_VERTEX}", guarded_wgsl(module)?))
}

/// The vertex shader, for translated GLSL visuals: the WGSL one from
/// common.wgsl, which uses no uniforms.
const COMMON_VERTEX: &str = "@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}";

/// An error message quoting the visual's line, like naga's for WGSL.
fn located_message(kind: &str, source: &str, location: Option<(usize, usize)>) -> String {
    let Some((line, column)) = location else {
        return format!("error: {kind}");
    };
    let code = source.lines().nth(line - 1).unwrap_or("");
    let margin = " ".repeat(line.to_string().len());
    format!(
        "error: {kind}\n{margin} ┌─ shader:{line}:{column}\n{margin} │\n{line} │ {code}\n{margin} │ {}^",
        " ".repeat(column.saturating_sub(1))
    )
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
        // A shader running too long (an endless loop…) makes the system reset
        // the GPU: the device is lost, and the app must make a new renderer.
        let lost = Arc::new(AtomicBool::new(false));
        device.set_device_lost_callback({
            let lost = lost.clone();
            move |reason, message| {
                log::error!("GPU device lost ({reason:?}): {message}");
                lost.store(true, Ordering::Relaxed);
            }
        });

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
            lost,
        };
        renderer.visuals = VISUALS
            .iter()
            .map(|&(name, builtin)| Visual {
                name: name.to_owned(),
                builtin: Some(builtin),
                source: builtin.to_owned(),
                pipeline: renderer.create_pipeline(name, builtin),
            })
            .collect();
        renderer
    }

    /// Only called with sources that compile (see [`validate_shader`]).
    fn create_pipeline(&self, name: &str, source: &str) -> wgpu::RenderPipeline {
        let compiled = compile(source).unwrap_or_else(|_| compile(TEMPLATE_SHADER).unwrap());
        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(name),
                source: wgpu::ShaderSource::Wgsl(compiled.into()),
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

    pub fn visual_name(&self, index: usize) -> &str {
        &self.visuals[index].name
    }

    /// The visual's source as last edited, which may not compile.
    pub fn visual_source(&self, index: usize) -> &str {
        &self.visuals[index].source
    }

    /// The original source of a built-in visual, `None` for added ones.
    pub fn builtin_source(&self, index: usize) -> Option<&'static str> {
        self.visuals[index].builtin
    }

    /// Adds a visual after the existing ones and returns its index. If the
    /// source doesn't compile, the error is returned too, and the visual draws
    /// the template until its source is fixed.
    pub fn add_visual(&mut self, name: String, source: String) -> (usize, Result<(), ShaderError>) {
        let result = validate_shader(&source);
        let working = if result.is_ok() {
            &source
        } else {
            TEMPLATE_SHADER
        };
        let pipeline = self.create_pipeline(&name, working);
        self.visuals.push(Visual {
            name,
            builtin: None,
            source,
            pipeline,
        });
        (self.visuals.len() - 1, result)
    }

    /// Removes a visual added with [`Renderer::add_visual`]; built-in ones
    /// stay. Returns whether it was removed. The displayed visual stays the
    /// same, unless it's the one removed: then the next one is shown.
    pub fn remove_visual(&mut self, index: usize) -> bool {
        if self.visuals[index].builtin.is_some() {
            return false;
        }
        self.visuals.remove(index);
        if self.visual > index || self.visual == self.visuals.len() {
            self.visual -= 1;
        }
        true
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

    /// Whether the GPU device was lost. The renderer can't draw anymore: make
    /// a new one.
    pub fn is_lost(&self) -> bool {
        self.lost.load(Ordering::Relaxed)
    }

    /// Loses the GPU device on purpose, as a GPU reset would: to test the
    /// recovery without hanging the GPU.
    pub fn lose_device(&self) {
        self.device.destroy();
    }

    /// Keeps a visual's source, but runs a safe version instead: the original
    /// of a built-in visual, the template for the others. For a visual whose
    /// code hung the GPU.
    pub fn suspend_visual(&mut self, index: usize) {
        let visual = &self.visuals[index];
        let safe = visual.builtin.unwrap_or(TEMPLATE_SHADER);
        self.visuals[index].pipeline = self.create_pipeline(&visual.name, safe);
    }

    /// Replaces a visual's source without running it: the visual keeps
    /// running its current version until [`Renderer::set_visual_source`].
    pub fn set_visual_text(&mut self, index: usize, source: String) {
        self.visuals[index].source = source;
    }

    /// Replaces a visual's source. If it doesn't compile, the error is
    /// returned and the visual keeps running its last working version.
    pub fn set_visual_source(&mut self, index: usize, source: String) -> Result<(), ShaderError> {
        let result = validate_shader(&source);
        if result.is_ok() {
            self.visuals[index].pipeline = self.create_pipeline(&self.visuals[index].name, &source);
        }
        self.visuals[index].source = source;
        result
    }

    /// `time` is in seconds since start, and drives the animations.
    pub fn render(&mut self, spectrum: &Spectrum, time: f32) {
        if self.is_lost() {
            return;
        }
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
                label: Some(&self.visuals[self.visual].name),
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
        for (name, source) in VISUALS.iter().chain([&("template", TEMPLATE_SHADER)]) {
            if let Err(error) = validate_shader(source) {
                panic!("{name} does not compile:\n{error}");
            }
        }
    }

    #[test]
    fn detects_the_language() {
        assert_eq!(shader_language(TEMPLATE_SHADER), ShaderLanguage::Wgsl);
        assert_eq!(
            shader_language("void mainImage(out vec4 c, in vec2 p) {}"),
            ShaderLanguage::Glsl
        );
    }

    #[test]
    fn shadertoy_style_glsl_compiles() {
        // Typical Shadertoy code: defines, helper functions, iTime and iResolution.
        let source = "#define SPEED 2.0
float wave(vec2 p) {
    return 0.5 + 0.5 * sin(10.0 * p.x + SPEED * iTime);
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;
    vec3 col = 0.5 + 0.5 * cos(iTime + uv.xyx + vec3(0, 2, 4));
    fragColor = vec4(col * wave(uv) * (band(3) + note(60) + cc(21) + u.bass), 1.0);
}
";
        if let Err(error) = validate_shader(source) {
            panic!("{error}");
        }
    }

    #[test]
    fn glsl_translates_to_valid_wgsl() {
        let wgsl = compile(include_str!("shaders/example.glsl")).unwrap();
        let module = naga::front::wgsl::parse_str(&wgsl)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&wgsl)));
        validate(&module).unwrap();
        // The entry points the pipeline uses.
        let entries: Vec<_> = module
            .entry_points
            .iter()
            .map(|e| e.name.as_str())
            .collect();
        assert!(
            entries.contains(&"fs_main") && entries.contains(&"vs_main"),
            "{entries:?}"
        );
    }

    #[test]
    fn glsl_errors_point_at_the_users_line() {
        let source = "void mainImage(out vec4 fragColor, in vec2 fragCoord) {\n    float x = 1.0;\n    fragColor = vec4(oops, 0.0, 0.0, 1.0);\n}\n";
        let error = validate_shader(source).unwrap_err();
        assert_eq!(error.line, Some(3), "{error}");
        assert!(error.message.contains("shader:3:"), "{error}");
        assert!(error.message.contains("fragColor = vec4(oops"), "{error}");
    }

    #[test]
    fn detection_needs_a_main_image_definition() {
        // Mentioned in comments only: still WGSL.
        let commented =
            "// Shadertoy's mainImage\n/* void mainImage */\n@fragment\nfn fs_main() {}";
        assert_eq!(shader_language(commented), ShaderLanguage::Wgsl);
        // Any spacing, and the definition on the next line, still count.
        let glsl = "void\n  mainImage (out vec4 fragColor, in vec2 fragCoord) {}";
        assert_eq!(shader_language(glsl), ShaderLanguage::Glsl);
        // Called but not defined: not GLSL.
        assert_eq!(
            shader_language("x = mainImage(c, p);"),
            ShaderLanguage::Wgsl
        );
    }

    /// Compiles and checks the WGSL given to the GPU.
    fn compiled_module(source: &str) -> (String, naga::Module) {
        let wgsl = compile(source).unwrap_or_else(|e| panic!("{e}"));
        let module = naga::front::wgsl::parse_str(&wgsl)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&wgsl)));
        validate(&module).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&wgsl)));
        (wgsl, module)
    }

    /// Counts the loops of a block, and the ones starting with the guard.
    fn loops(block: &naga::Block) -> (usize, usize) {
        let mut found = (0, 0);
        for statement in block.iter() {
            let (all, guarded) = match statement {
                naga::Statement::Block(inner) => loops(inner),
                naga::Statement::If { accept, reject, .. } => {
                    let (a, b) = (loops(accept), loops(reject));
                    (a.0 + b.0, a.1 + b.1)
                }
                naga::Statement::Switch { cases, .. } => cases
                    .iter()
                    .map(|case| loops(&case.body))
                    .fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1)),
                naga::Statement::Loop { body, .. } => {
                    let inner = loops(body);
                    // The guard: count, store, then `if … { break; }`, before
                    // the loop's own code.
                    let guarded = matches!(
                        body.iter().find(|s| matches!(s, naga::Statement::If { .. })),
                        Some(naga::Statement::If { accept, .. })
                            if matches!(accept.first(), Some(naga::Statement::Break))
                    ) && body
                        .iter()
                        .any(|s| matches!(s, naga::Statement::Store { .. }));
                    (inner.0 + 1, inner.1 + guarded as usize)
                }
                _ => (0, 0),
            };
            found = (found.0 + all, found.1 + guarded);
        }
        found
    }

    fn guarded_loops(module: &naga::Module) -> (usize, usize) {
        module
            .functions
            .iter()
            .map(|(_, f)| &f.body)
            .chain(module.entry_points.iter().map(|e| &e.function.body))
            .map(loops)
            .fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1))
    }

    #[test]
    fn every_loop_is_guarded() {
        let source = "fn helper() -> f32 {
    var x = 0.0;
    while x < 10.0 { x += 1.0; }
    return x;
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    var total = 0.0;
    for (var i = 0; i < 10; i++) {
        for (var j = 0; j < 10; j++) {
            if total > 5.0 { loop { total -= 1.0; if total < 0.0 { break; } } }
            total += helper();
        }
    }
    // Endless: stopped by the guard.
    loop {}
    return vec4<f32>(fwidth(total), 0.0, 0.0, 1.0);
}
";
        let (wgsl, module) = compiled_module(source);
        assert_eq!(guarded_loops(&module), (5, 5), "{wgsl}");
        assert!(wgsl.contains(&format!("{}u", MAX_ITERATIONS)), "{wgsl}");
    }

    #[test]
    fn glsl_loops_are_guarded() {
        let source = "void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    float t = 0.0;
    for (int i = 0; i < 64; i++) { t += 0.1; }
    while (true) { t += 1.0; }
    do { t -= 1.0; } while (t > 0.0);
    fragColor = vec4(t, 0.0, 0.0, 1.0);
}
";
        let (wgsl, module) = compiled_module(source);
        let (all, guarded) = guarded_loops(&module);
        assert!(
            all >= 3 && guarded == all,
            "{all} loops, {guarded} guarded:\n{wgsl}"
        );
    }

    #[test]
    fn builtin_visuals_still_compile_guarded() {
        for (name, source) in VISUALS.iter().chain([&("template", TEMPLATE_SHADER)]) {
            let (_, module) = compiled_module(source);
            let (all, guarded) = guarded_loops(&module);
            assert_eq!(all, guarded, "{name}");
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
