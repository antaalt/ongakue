//! Browser audio through the Web Audio API, from a file or the microphone.
//!
//! Graph:
//! - file: AudioBufferSourceNode -> AnalyserNode, and -> speakers
//! - microphone: MediaStreamAudioSourceNode -> AnalyserNode only (playing it
//!   back would cause feedback)
//!
//! The analyser taps what is actually being played or heard, so the visuals
//! stay in sync with the sound.
//!
//! The controls (`#source`, `#file-controls`, `#file`, `#play`, `#status`)
//! are defined in `index.html`.

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AnalyserNode, AudioBuffer, AudioBufferSourceNode, AudioContext, AudioContextState,
    AudioScheduledSourceNode, Document, HtmlButtonElement, HtmlElement, HtmlInputElement,
    HtmlSelectElement, MediaStream, MediaStreamAudioSourceNode, MediaStreamConstraints,
    MediaStreamTrack,
};

/// Number of samples returned by [`Backend::latest_samples`].
pub const SAMPLE_COUNT: usize = 2048;

pub struct Backend {
    analyser: AnalyserNode,
    sample_rate: f32,
}

struct Microphone {
    stream: MediaStream,
    node: MediaStreamAudioSourceNode,
}

struct State {
    ctx: AudioContext,
    analyser: AnalyserNode,
    buffer: Option<AudioBuffer>,
    /// Name of the decoded file, to show again when switching back to it.
    file_name: Option<String>,
    /// The node currently playing (or paused), if any. A source node can only
    /// be started once, so a new one is created each time playback restarts.
    source: Option<AudioBufferSourceNode>,
    microphone: Option<Microphone>,
    play_button: HtmlButtonElement,
    file_controls: HtmlElement,
    status: HtmlElement,
}

impl Backend {
    pub fn new() -> Result<Self, JsValue> {
        let document = web_sys::window().unwrap().document().unwrap();

        // Created suspended; browsers only let it start from a user gesture.
        let ctx = AudioContext::new()?;
        let analyser = ctx.create_analyser()?;
        analyser.set_fft_size(SAMPLE_COUNT as u32);

        let state = Rc::new(RefCell::new(State {
            ctx,
            analyser: analyser.clone(),
            buffer: None,
            file_name: None,
            source: None,
            microphone: None,
            play_button: element(&document, "play")?,
            file_controls: element(&document, "file-controls")?,
            status: element(&document, "status")?,
        }));

        let file_input: HtmlInputElement = element(&document, "file")?;
        fn load_file_from_input(input: &HtmlInputElement, state: Rc<RefCell<State>>) {
            if let Some(file) = input.files().and_then(|files| files.get(0)) {
                log::info!("Loading file {}", file.name());
                wasm_bindgen_futures::spawn_local(load_file(state.clone(), file));
            }
        }
        load_file_from_input(&file_input, state.clone());
        let on_change = {
            let state = state.clone();
            let input = file_input.clone();
            Closure::<dyn FnMut()>::new(move || {
                load_file_from_input(&input, state.clone());
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

        let source_select: HtmlSelectElement = element(&document, "source")?;
        let on_source = {
            let (state, select) = (state.clone(), source_select.clone());
            Closure::<dyn FnMut()>::new(move || {
                if select.value() == "microphone" {
                    wasm_bindgen_futures::spawn_local(use_microphone(state.clone()));
                } else {
                    state.borrow_mut().use_file();
                }
            })
        };
        source_select.set_onchange(Some(on_source.as_ref().unchecked_ref()));
        on_source.forget();

        let sample_rate = state.borrow().ctx.sample_rate();
        Ok(Self {
            analyser,
            sample_rate,
        })
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
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
        s.file_name = None;
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
            s.file_name = Some(file.name());
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
        source.connect_with_audio_node(&s.ctx.destination())?;

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

async fn use_microphone(state: Rc<RefCell<State>>) {
    let ctx = {
        let mut s = state.borrow_mut();
        s.stop_source();
        s.play_button.set_text_content(Some("Play"));
        s.file_controls.set_hidden(true);
        s.status
            .set_text_content(Some("Waiting for the microphone…"));
        // Resume right away, while the choice still counts as a user gesture.
        let _ = s.ctx.resume();
        s.ctx.clone()
    };

    let stream = async {
        let devices = web_sys::window().unwrap().navigator().media_devices()?;
        let stream =
            JsFuture::from(devices.get_user_media_with_constraints(&constraints())?).await?;
        Ok::<MediaStream, JsValue>(stream.unchecked_into())
    }
    .await;

    let mut s = state.borrow_mut();
    // The source may have been switched back to the file while waiting.
    if !s.file_controls.hidden() {
        if let Ok(stream) = stream {
            stop_tracks(&stream);
        }
        return;
    }
    let microphone = stream.and_then(|stream| {
        let node = ctx.create_media_stream_source(&stream)?;
        node.connect_with_audio_node(&s.analyser)?;
        Ok(Microphone { stream, node })
    });
    match microphone {
        Ok(microphone) => {
            s.microphone = Some(microphone);
            let _ = ctx.resume();
            s.status
                .set_text_content(Some("Listening to the microphone"));
        }
        Err(e) => {
            log::warn!("microphone unavailable: {e:?}");
            s.status
                .set_text_content(Some("Microphone unavailable or not allowed"));
        }
    }
}

/// Raw sound: the processing meant for calls (echo cancellation, noise
/// suppression, automatic gain) would distort the music.
fn constraints() -> MediaStreamConstraints {
    let audio = js_sys::Object::new();
    for setting in ["echoCancellation", "noiseSuppression", "autoGainControl"] {
        let _ = js_sys::Reflect::set(&audio, &setting.into(), &false.into());
    }
    let constraints = MediaStreamConstraints::new();
    constraints.set_audio(&audio);
    constraints
}

fn stop_tracks(stream: &MediaStream) {
    for track in stream.get_tracks() {
        track.unchecked_into::<MediaStreamTrack>().stop();
    }
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

    fn use_file(&mut self) {
        if let Some(microphone) = self.microphone.take() {
            microphone.node.disconnect().ok();
            // Also turns off the browser's "recording" indicator.
            stop_tracks(&microphone.stream);
        }
        self.file_controls.set_hidden(false);
        let status = self.file_name.as_deref().unwrap_or("Choose an audio file");
        self.status.set_text_content(Some(status));
    }
}

fn element<T: JsCast>(document: &Document, id: &str) -> Result<T, JsValue> {
    document
        .get_element_by_id(id)
        .ok_or_else(|| JsValue::from_str(&format!("missing #{id} element")))?
        .dyn_into()
        .map_err(|_| JsValue::from_str(&format!("#{id} has the wrong element type")))
}
