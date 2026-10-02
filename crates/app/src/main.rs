mod audio;
mod editor;
mod midi;
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
    /// The shader of a visual was edited.
    ShaderEdited { visual: usize, source: String },
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
    samples: Box<[f32; audio::SAMPLE_COUNT]>,
    analyzer: Analyzer,
    start: Instant,
    last_frame: Instant,
}

impl App {
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

    /// Loads the displayed visual's source into the editor.
    fn show_current_visual(&self) {
        let Some(renderer) = &self.renderer else {
            return;
        };
        let visual = renderer.current_visual();
        let source = renderer.visual_source(visual);
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
            attributes.with_append(true)
        };
        let window = Arc::new(event_loop.create_window(attributes).unwrap());
        self.window = Some(window.clone());

        let size = window.inner_size();
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

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::RendererReady(mut renderer) => {
                if let Some(window) = &self.window {
                    // The canvas may have been resized while we were initializing.
                    let size = window.inner_size();
                    renderer.resize(size.width, size.height);
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
                self.renderer = Some(*renderer);
                self.update_visual_list();
                self.show_current_visual();
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
            UserEvent::ShaderEdited { visual, source } => {
                let Some(renderer) = &mut self.renderer else {
                    return;
                };
                let name = renderer.visual_name(visual);
                if renderer.builtin_source(visual) == Some(source.as_str()) {
                    self.editor.forget_source(name);
                } else {
                    self.editor.save_source(name, &source);
                }
                let result = renderer.set_visual_source(visual, source);
                self.editor.set_error(result.err().as_ref());
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
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
                }
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                // Clamped, so a long pause (e.g. a hidden tab) doesn't make a jump.
                let dt = (now - self.last_frame).as_secs_f32().min(0.1);
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

                if let Some(renderer) = &mut self.renderer {
                    renderer.set_params(self.tuning.params());
                    renderer.set_midi(&midi.notes, &midi.controls);
                    renderer.render(spectrum, time);
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
    let midi = midi::Midi::new().expect("failed to initialize MIDI");
    let app = App {
        proxy: event_loop.create_proxy(),
        editor,
        tuning,
        midi,
        window: None,
        renderer: None,
        analyzer: Analyzer::new(audio.sample_rate(), audio::SAMPLE_COUNT),
        audio,
        samples: Box::new([0.0; audio::SAMPLE_COUNT]),
        start: Instant::now(),
        last_frame: Instant::now(),
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
