mod audio;
mod editor;
mod midi;
#[cfg(target_arch = "wasm32")]
mod panel;
mod piano;
mod stats;
mod tuning;

use std::sync::Arc;

use analysis::Analyzer;
use render::Renderer;
use web_time::Instant;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowId};

/// Events sent to the event loop from outside of it.
// The editor, which sends the visual and shader events, only exists on the web.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
enum UserEvent {
    /// The (async) GPU setup is done.
    RendererReady(Box<Renderer>),
    /// A visual was picked in the editor.
    VisualSelected(usize),
    /// A visual was created or imported in the editor.
    VisualAdded { name: String, source: String },
    /// A visual added earlier was deleted.
    VisualDeleted(usize),
    /// The shader of a visual was edited. It runs if `compile`, otherwise
    /// it's only saved and checked for errors.
    ShaderEdited {
        visual: usize,
        source: String,
        compile: bool,
    },
    /// Restore a visual's built-in shader.
    ShaderReset { visual: usize },
}

struct App {
    proxy: EventLoopProxy<UserEvent>,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    audio: audio::Backend,
    editor: editor::Editor,
    tuning: tuning::Tuning,
    midi: midi::Midi,
    piano: piano::Piano,
    samples: Box<[f32; audio::SAMPLE_COUNT]>,
    analyzer: Analyzer,
    stats: stats::Stats,
    meter: stats::FrameMeter,
    /// The resolution scale the renderer was last sized with.
    scale: f32,
    start: Instant,
    last_frame: Instant,
    /// The visual that was showing when the GPU was reset, to run safely in
    /// the next renderer.
    suspended: Option<usize>,
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static SIMULATE_GPU_RESET: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Loses the GPU device at the next frame, as a GPU reset would, to test the
/// recovery: `wasmBindings.simulate_gpu_reset()` in the browser's console.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn simulate_gpu_reset() {
    SIMULATE_GPU_RESET.set(true);
}

impl App {
    /// Sizes the renderer to the window, at the resolution scale.
    fn resize_renderer(&mut self) {
        self.scale = self.stats.scale();
        if let (Some(window), Some(renderer)) = (&self.window, &mut self.renderer) {
            let size = window.inner_size();
            let (width, height) = stats::scaled(size.width, size.height, self.scale);
            renderer.resize(width, height);
        }
    }

    /// Starts the GPU setup, which sends [`UserEvent::RendererReady`] when done.
    fn create_renderer(&self) {
        let Some(window) = self.window.clone() else {
            return;
        };
        let size = window.inner_size();
        // On the web, the canvas: the renderer picks the GPU for WebGL with it.
        #[cfg(target_arch = "wasm32")]
        let window = {
            use winit::platform::web::WindowExtWebSys;
            render::SurfaceTarget::Canvas(window.canvas().expect("the window has a canvas"))
        };
        let init = Renderer::new(window, size.width, size.height);
        let proxy = self.proxy.clone();

        // Adapter/device requests are async: block natively, spawn on the web.
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(async move {
            let _ = proxy.send_event(UserEvent::RendererReady(Box::new(init.await)));
        });
        #[cfg(not(target_arch = "wasm32"))]
        let _ = proxy.send_event(UserEvent::RendererReady(Box::new(pollster::block_on(init))));
    }

    /// After a GPU reset, most likely caused by a shader running too long:
    /// makes a new renderer, in which the visual that was showing runs a safe
    /// version, and turns auto compile off so the code isn't run again before
    /// it's fixed.
    fn recover_from_gpu_reset(&mut self) {
        let Some(renderer) = self.renderer.take() else {
            return;
        };
        let visual = renderer.current_visual();
        let name = renderer.visual_name(visual).to_owned();
        drop(renderer);
        log::warn!("GPU reset while showing \"{name}\": recreating the renderer");
        self.suspended = Some(visual);
        self.editor.set_auto_compile(false);
        self.editor.notify(&format!(
            "The GPU was reset, probably because \"{name}\" took too long to draw \
             (an endless loop?). It runs a safe version until you press Compile, \
             and auto compile is off."
        ));
        self.create_renderer();
    }

    /// Shows the visuals in the editor's picker, and saves the list of the
    /// ones the user added (listed after the built-in ones).
    fn update_visual_list(&self) {
        let Some(renderer) = &self.renderer else {
            return;
        };
        let names: Vec<_> = (0..renderer.visual_count())
            .map(|visual| renderer.visual_name(visual))
            .collect();
        let builtin_count = (0..renderer.visual_count())
            .filter(|&visual| renderer.builtin_source(visual).is_some())
            .count();
        self.editor.set_visuals(&names, builtin_count);
        self.editor.save_visuals(&names[builtin_count..]);
    }

    /// Loads the displayed visual's source into the editor, and remembers it
    /// for the next visit.
    fn show_current_visual(&self) {
        let Some(renderer) = &self.renderer else {
            return;
        };
        let visual = renderer.current_visual();
        let source = renderer.visual_source(visual);
        self.editor
            .save_current_visual(renderer.visual_name(visual));
        self.editor.show(visual, source);
        self.editor
            .set_error(render::validate_shader(source).err().as_ref());
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attributes = Window::default_attributes().with_title("Ongakue");
        #[cfg(target_arch = "wasm32")]
        let attributes = {
            use winit::platform::web::WindowAttributesExtWebSys;
            // Let the browser handle keys and clicks on the canvas too (F11
            // for fullscreen, F5, the context menu…): winit blocks them by
            // default. The app reads the keyboard with its own listener (see
            // piano/web.rs), and the page doesn't scroll.
            attributes.with_append(true).with_prevent_default(false)
        };
        let window = Arc::new(event_loop.create_window(attributes).unwrap());
        self.window = Some(window);
        self.create_renderer();
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::RendererReady(mut renderer) => {
                if let Some(window) = &self.window {
                    // The canvas may have been resized while we were initializing.
                    let size = window.inner_size();
                    self.scale = self.stats.scale();
                    let (width, height) = stats::scaled(size.width, size.height, self.scale);
                    renderer.resize(width, height);
                    window.request_redraw();
                }
                // Restore the shaders edited in previous sessions, then the
                // visuals added.
                for visual in 0..renderer.visual_count() {
                    if let Some(source) = self.editor.saved_source(renderer.visual_name(visual)) {
                        let _ = renderer.set_visual_source(visual, source);
                    }
                }
                for name in self.editor.saved_visuals() {
                    let source = self
                        .editor
                        .saved_source(&name)
                        .unwrap_or_else(|| render::TEMPLATE_SHADER.to_owned());
                    // Errors show when the visual is opened in the editor.
                    let _ = renderer.add_visual(name, source);
                }
                // Back to the visual shown last time.
                let last = self.editor.saved_current_visual();
                if let Some(visual) = (0..renderer.visual_count())
                    .find(|&visual| Some(renderer.visual_name(visual)) == last.as_deref())
                {
                    renderer.set_visual(visual);
                }
                // After a GPU reset: back to the visual that was showing, but
                // safely.
                if let Some(visual) = self.suspended.take()
                    && visual < renderer.visual_count()
                {
                    renderer.set_visual(visual);
                    renderer.suspend_visual(visual);
                }
                self.renderer = Some(*renderer);
                self.update_visual_list();
                self.show_current_visual();
                // The page was opened with a shared link.
                if let Some((name, source)) = self.editor.take_shared_visual() {
                    let _ = self
                        .proxy
                        .send_event(UserEvent::VisualAdded { name, source });
                }
            }
            UserEvent::VisualSelected(visual) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.set_visual(visual);
                }
                self.show_current_visual();
            }
            UserEvent::VisualAdded { name, source } => {
                let Some(renderer) = &mut self.renderer else {
                    return;
                };
                self.editor.save_source(&name, &source);
                let (visual, _) = renderer.add_visual(name, source);
                renderer.set_visual(visual);
                self.update_visual_list();
                self.show_current_visual();
            }
            UserEvent::VisualDeleted(visual) => {
                let Some(renderer) = &mut self.renderer else {
                    return;
                };
                let name = renderer.visual_name(visual).to_owned();
                if renderer.remove_visual(visual) {
                    self.editor.forget_source(&name);
                    self.update_visual_list();
                    self.show_current_visual();
                }
            }
            UserEvent::ShaderEdited {
                visual,
                source,
                compile,
            } => {
                let Some(renderer) = &mut self.renderer else {
                    return;
                };
                let name = renderer.visual_name(visual);
                if renderer.builtin_source(visual) == Some(source.as_str()) {
                    self.editor.forget_source(name);
                } else {
                    self.editor.save_source(name, &source);
                }
                if compile {
                    let result = renderer.set_visual_source(visual, source);
                    self.editor.set_error(result.err().as_ref());
                } else {
                    self.editor
                        .set_error(render::validate_shader(&source).err().as_ref());
                    renderer.set_visual_text(visual, source);
                }
            }
            UserEvent::ShaderReset { visual } => {
                let Some(renderer) = &mut self.renderer else {
                    return;
                };
                let Some(builtin) = renderer.builtin_source(visual) else {
                    return;
                };
                self.editor.forget_source(renderer.visual_name(visual));
                let _ = renderer.set_visual_source(visual, builtin.to_owned());
                self.show_current_visual();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        for message in self.piano.window_event(&event) {
            self.midi.handle(&message);
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(_) => self.resize_renderer(),
            WindowEvent::RedrawRequested => {
                #[cfg(target_arch = "wasm32")]
                if SIMULATE_GPU_RESET.take()
                    && let Some(renderer) = &self.renderer
                {
                    renderer.lose_device();
                }
                if self.renderer.as_ref().is_some_and(Renderer::is_lost) {
                    self.recover_from_gpu_reset();
                }
                // The resolution slider moved.
                if self.stats.scale() != self.scale {
                    self.resize_renderer();
                }
                let now = Instant::now();
                let interval = (now - self.last_frame).as_secs_f32();
                // Clamped, so a long pause (e.g. a hidden tab) doesn't make a jump.
                let dt = interval.min(0.1);
                self.last_frame = now;

                let midi = self.midi.frame(dt);
                self.tuning.apply_midi(&midi.controls, midi.moved);

                self.audio.latest_samples(&mut self.samples);
                self.analyzer.settings = self.tuning.settings();
                self.analyzer.process(&self.samples[..], dt);
                // Held MIDI notes light their frequency's band too.
                let spectrum = self.analyzer.add_notes(&midi.notes);
                let beat = spectrum.beat == 1.0;
                let time = (now - self.start).as_secs_f32();
                let analyzed = Instant::now();

                if let Some(renderer) = &mut self.renderer {
                    renderer.set_params(self.tuning.params());
                    renderer.set_midi(&midi.notes, &midi.controls);
                    renderer.render(spectrum, time);
                }
                let drawn = Instant::now();
                let gpu = self.renderer.as_mut().and_then(Renderer::take_gpu_time);
                let report = self.meter.record(
                    interval,
                    (analyzed - now).as_secs_f32(),
                    (drawn - analyzed).as_secs_f32(),
                    gpu,
                );
                if let (Some(report), Some(renderer), Some(window)) =
                    (report, &self.renderer, &self.window)
                {
                    let (width, height) = renderer.size();
                    let target = stats::Target {
                        width,
                        height,
                        scale: self.scale,
                        gpu: renderer.gpu_name(),
                        backend: renderer.backend_name(),
                        measures_gpu_time: renderer.measures_gpu_time(),
                    };
                    self.stats.show(&report, &target, window);
                }
                self.tuning.record(
                    time,
                    self.analyzer.beat_flux(),
                    self.analyzer.beat_threshold(),
                    beat,
                );
                // On the web this schedules the next requestAnimationFrame.
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }
}

fn main() {
    #[cfg(target_arch = "wasm32")]
    {
        std::panic::set_hook(Box::new(console_error_panic_hook::hook));
        console_log::init_with_level(log::Level::Info).unwrap();
    }
    #[cfg(not(target_arch = "wasm32"))]
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let event_loop = EventLoop::<UserEvent>::with_user_event().build().unwrap();
    let audio = audio::Backend::new().expect("failed to initialize audio");
    let editor =
        editor::Editor::new(event_loop.create_proxy()).expect("failed to initialize editor");
    let tuning = tuning::Tuning::new().expect("failed to initialize the tuning panel");
    let stats = stats::Stats::new().expect("failed to initialize the stats");
    let midi = midi::Midi::new().expect("failed to initialize MIDI");
    // The keyboard piano's notes go to MIDI and, on the web, to the synth.
    #[cfg(target_arch = "wasm32")]
    let piano = {
        let (sink, synth) = (midi.sink(), audio.synth());
        piano::Piano::new(move |message| {
            sink.handle(message);
            synth.handle(message);
        })
        .expect("failed to initialize the keyboard piano")
    };
    #[cfg(not(target_arch = "wasm32"))]
    let piano = piano::Piano::new();
    let app = App {
        proxy: event_loop.create_proxy(),
        editor,
        tuning,
        midi,
        piano,
        window: None,
        renderer: None,
        analyzer: Analyzer::new(audio.sample_rate(), audio::SAMPLE_COUNT),
        audio,
        samples: Box::new([0.0; audio::SAMPLE_COUNT]),
        stats,
        meter: stats::FrameMeter::default(),
        scale: 1.0,
        start: Instant::now(),
        last_frame: Instant::now(),
        suspended: None,
    };

    #[cfg(target_arch = "wasm32")]
    {
        use winit::platform::web::EventLoopExtWebSys;
        event_loop.spawn_app(app);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut app = app;
        event_loop.run_app(&mut app).unwrap();
    }
}
