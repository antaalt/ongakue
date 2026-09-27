mod audio;

use std::sync::Arc;

use render::Renderer;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowId};

/// Sent back to the event loop once the (async) GPU setup is done.
enum UserEvent {
    RendererReady(Renderer),
}

struct App {
    proxy: EventLoopProxy<UserEvent>,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    audio: audio::Backend,
    samples: Box<[f32; audio::SAMPLE_COUNT]>,
    /// Smoothed loudness, 0..1. Temporary until the spectrum analysis lands.
    level: f32,
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
            let _ = proxy.send_event(UserEvent::RendererReady(init.await));
        });
        #[cfg(not(target_arch = "wasm32"))]
        let _ = proxy.send_event(UserEvent::RendererReady(pollster::block_on(init)));
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
                self.renderer = Some(renderer);
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
                self.audio.latest_samples(&mut self.samples);
                let rms = (self.samples.iter().map(|s| s * s).sum::<f32>()
                    / self.samples.len() as f32)
                    .sqrt();
                // Fast attack, slow decay.
                self.level = (rms * 4.0).min(1.0).max(self.level * 0.92);

                if let Some(renderer) = &mut self.renderer {
                    renderer.render(self.level);
                }
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
    let app = App {
        proxy: event_loop.create_proxy(),
        window: None,
        renderer: None,
        audio: audio::Backend::new().expect("failed to initialize audio"),
        samples: Box::new([0.0; audio::SAMPLE_COUNT]),
        level: 0.0,
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
