//! Browser audio through the Web Audio API.
//!
//! Graph: AudioBufferSourceNode -> AnalyserNode -> speakers. The analyser taps
//! what is actually being played, so the visuals stay in sync with the sound.
//!
//! The controls (`#file`, `#play`, `#status`) are defined in `index.html`.

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AnalyserNode, AudioBuffer, AudioBufferSourceNode, AudioContext, AudioContextState,
    AudioScheduledSourceNode, Document, HtmlButtonElement, HtmlElement, HtmlInputElement,
};

/// Number of samples returned by [`Backend::latest_samples`].
pub const SAMPLE_COUNT: usize = 2048;

pub struct Backend {
    analyser: AnalyserNode,
}

struct State {
    ctx: AudioContext,
    analyser: AnalyserNode,
    buffer: Option<AudioBuffer>,
    /// The node currently playing (or paused), if any. A source node can only
    /// be started once, so a new one is created each time playback restarts.
    source: Option<AudioBufferSourceNode>,
    play_button: HtmlButtonElement,
    status: HtmlElement,
}

impl Backend {
    pub fn new() -> Result<Self, JsValue> {
        let document = web_sys::window().unwrap().document().unwrap();

        // Created suspended; browsers only let it start from a user gesture.
        let ctx = AudioContext::new()?;
        let analyser = ctx.create_analyser()?;
        analyser.set_fft_size(SAMPLE_COUNT as u32);
        analyser.connect_with_audio_node(&ctx.destination())?;

        let state = Rc::new(RefCell::new(State {
            ctx,
            analyser: analyser.clone(),
            buffer: None,
            source: None,
            play_button: element(&document, "play")?,
            status: element(&document, "status")?,
        }));

        let file_input: HtmlInputElement = element(&document, "file")?;
        let on_change = {
            let state = state.clone();
            let input = file_input.clone();
            Closure::<dyn FnMut()>::new(move || {
                if let Some(file) = input.files().and_then(|files| files.get(0)) {
                    wasm_bindgen_futures::spawn_local(load_file(state.clone(), file));
                }
            })
        };
        file_input.set_onchange(Some(on_change.as_ref().unchecked_ref()));
        // The listeners live as long as the page.
        on_change.forget();

        let on_click = {
            let state = state.clone();
            Closure::<dyn FnMut()>::new(move || {
                if let Err(e) = toggle_playback(&state) {
                    log::error!("playback error: {e:?}");
                }
            })
        };
        state
            .borrow()
            .play_button
            .set_onclick(Some(on_click.as_ref().unchecked_ref()));
        on_click.forget();

        Ok(Self { analyser })
    }

    /// Copies the most recently played samples (mono, -1..1) into `out`.
    pub fn latest_samples(&mut self, out: &mut [f32; SAMPLE_COUNT]) {
        self.analyser.get_float_time_domain_data(out);
    }
}

async fn load_file(state: Rc<RefCell<State>>, file: web_sys::File) {
    let ctx = {
        let mut s = state.borrow_mut();
        s.stop_source();
        s.buffer = None;
        s.play_button.set_disabled(true);
        s.play_button.set_text_content(Some("Play"));
        s.status.set_text_content(Some("Decoding…"));
        s.ctx.clone()
    };

    let decoded = async {
        let bytes = JsFuture::from(file.array_buffer()).await?;
        let buffer = JsFuture::from(ctx.decode_audio_data(&bytes.into())?).await?;
        Ok::<AudioBuffer, JsValue>(buffer.into())
    }
    .await;

    let mut s = state.borrow_mut();
    match decoded {
        Ok(buffer) => {
            s.buffer = Some(buffer);
            s.play_button.set_disabled(false);
            s.status.set_text_content(Some(&file.name()));
        }
        Err(e) => {
            log::error!("could not decode {}: {e:?}", file.name());
            s.status
                .set_text_content(Some("Could not decode this file"));
        }
    }
}

fn toggle_playback(state: &Rc<RefCell<State>>) -> Result<(), JsValue> {
    let mut s = state.borrow_mut();
    let Some(buffer) = s.buffer.clone() else {
        return Ok(());
    };

    if s.source.is_none() {
        let source = s.ctx.create_buffer_source()?;
        source.set_buffer(Some(&buffer));
        source.connect_with_audio_node(&s.analyser)?;

        let on_ended = {
            let state = state.clone();
            Closure::once_into_js(move || {
                let mut s = state.borrow_mut();
                s.source = None;
                s.play_button.set_text_content(Some("Play"));
            })
        };
        let scheduled: &AudioScheduledSourceNode = &source;
        scheduled.set_onended(Some(on_ended.unchecked_ref()));
        scheduled.start()?;
        s.source = Some(source);
        let _ = s.ctx.resume()?;
        s.play_button.set_text_content(Some("Pause"));
    } else if s.ctx.state() == AudioContextState::Running {
        let _ = s.ctx.suspend()?;
        s.play_button.set_text_content(Some("Play"));
    } else {
        let _ = s.ctx.resume()?;
        s.play_button.set_text_content(Some("Pause"));
    }
    Ok(())
}

impl State {
    fn stop_source(&mut self) {
        if let Some(source) = self.source.take() {
            let scheduled: &AudioScheduledSourceNode = &source;
            // Detach the handler so the old node doesn't reset the new state.
            scheduled.set_onended(None);
            let _ = scheduled.stop();
        }
    }
}

fn element<T: JsCast>(document: &Document, id: &str) -> Result<T, JsValue> {
    document
        .get_element_by_id(id)
        .ok_or_else(|| JsValue::from_str(&format!("missing #{id} element")))?
        .dyn_into()
        .map_err(|_| JsValue::from_str(&format!("#{id} has the wrong element type")))
}
