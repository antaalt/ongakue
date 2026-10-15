//! Browser audio through the Web Audio API, from a file or the microphone,
//! plus a small synthesizer for the computer-keyboard piano.
//!
//! Graph:
//! - file: AudioBufferSourceNode -> AnalyserNode, and -> speakers
//! - microphone: MediaStreamAudioSourceNode -> AnalyserNode only (playing it
//!   back would cause feedback)
//! - synth: one voice per note -> AnalyserNode, and -> speakers
//!
//! Pausing a file stops its source and remembers the position, rather than
//! suspending the whole audio context, so the synth keeps playing.
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
    AnalyserNode, AudioBuffer, AudioBufferSourceNode, AudioContext, AudioScheduledSourceNode,
    BiquadFilterType, Document, GainNode, HtmlButtonElement, HtmlElement, HtmlInputElement,
    HtmlSelectElement, MediaStream, MediaStreamAudioSourceNode, MediaStreamConstraints,
    MediaStreamTrack, OscillatorNode, OscillatorType,
};

/// Number of samples returned by [`Backend::latest_samples`].
pub const SAMPLE_COUNT: usize = 2048;

pub struct Backend {
    analyser: AnalyserNode,
    sample_rate: f32,
    synth: Rc<Synth>,
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
    /// The node currently playing, if any. A source node can only be started
    /// once, so a new one is created each time playback starts or resumes.
    source: Option<AudioBufferSourceNode>,
    /// Where playback resumes in the file, in seconds.
    position: f64,
    /// Audio context time when the current source started playing.
    started_at: f64,
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
            position: 0.0,
            started_at: 0.0,
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
        let synth = Rc::new(Synth::new(&state.borrow().ctx, &analyser)?);
        Ok(Self {
            analyser,
            sample_rate,
            synth,
        })
    }

    /// The synthesizer playing the computer-keyboard piano's notes.
    pub fn synth(&self) -> Rc<Synth> {
        self.synth.clone()
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
        s.position = 0.0;
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
    if s.source.is_some() {
        s.pause();
        return Ok(());
    }

    let source = s.ctx.create_buffer_source()?;
    source.set_buffer(Some(&buffer));
    source.connect_with_audio_node(&s.analyser)?;
    source.connect_with_audio_node(&s.ctx.destination())?;

    let on_ended = {
        let state = state.clone();
        Closure::once_into_js(move || {
            // Played to the end: the next play starts over.
            let mut s = state.borrow_mut();
            s.source = None;
            s.position = 0.0;
            s.play_button.set_text_content(Some("Play"));
        })
    };
    let scheduled: &AudioScheduledSourceNode = &source;
    scheduled.set_onended(Some(on_ended.unchecked_ref()));
    if s.position >= buffer.duration() {
        s.position = 0.0;
    }
    source.start_with_when_and_grain_offset(0.0, s.position)?;
    s.started_at = s.ctx.current_time();
    s.source = Some(source);
    let _ = s.ctx.resume()?;
    s.play_button.set_text_content(Some("Pause"));
    Ok(())
}

async fn use_microphone(state: Rc<RefCell<State>>) {
    let ctx = {
        let mut s = state.borrow_mut();
        s.pause();
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
    /// Stops the file, remembering where, to resume from there.
    fn pause(&mut self) {
        if self.source.is_some() {
            self.position += self.ctx.current_time() - self.started_at;
            self.stop_source();
        }
        self.play_button.set_text_content(Some("Play"));
    }

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

/// A small synthesizer: a filtered sawtooth per note, with a short fade in
/// and out so notes don't click. Its sound goes to the speakers and to the
/// analyser, like a file's.
pub struct Synth {
    ctx: AudioContext,
    output: GainNode,
    /// The notes playing, with their oscillator and volume.
    voices: RefCell<Vec<(u8, OscillatorNode, GainNode)>>,
}

impl Synth {
    fn new(ctx: &AudioContext, analyser: &AnalyserNode) -> Result<Self, JsValue> {
        let output = ctx.create_gain()?;
        // Leaves room for chords without clipping.
        output.gain().set_value(0.25);
        output.connect_with_audio_node(analyser)?;
        output.connect_with_audio_node(&ctx.destination())?;
        Ok(Self {
            ctx: ctx.clone(),
            output,
            voices: RefCell::new(Vec::new()),
        })
    }

    /// Plays a MIDI note-on or note-off message; ignores the others.
    pub fn handle(&self, message: &[u8]) {
        let &[status, note, velocity, ..] = message else {
            return;
        };
        let result = match status & 0xF0 {
            0x90 if velocity > 0 => self.note_on(note, f32::from(velocity) / 127.0),
            0x80 | 0x90 => self.note_off(note),
            _ => Ok(()),
        };
        if let Err(error) = result {
            log::warn!("synth error: {error:?}");
        }
    }

    fn note_on(&self, note: u8, velocity: f32) -> Result<(), JsValue> {
        self.note_off(note)?;
        // A key press counts as a user gesture: the context may start.
        let _ = self.ctx.resume();
        let now = self.ctx.current_time();

        let oscillator = self.ctx.create_oscillator()?;
        oscillator.set_type(OscillatorType::Sawtooth);
        oscillator
            .frequency()
            .set_value(440.0 * 2f32.powf((f32::from(note) - 69.0) / 12.0));
        let filter = self.ctx.create_biquad_filter()?;
        filter.set_type(BiquadFilterType::Lowpass);
        filter.frequency().set_value(3000.0);
        let gain = self.ctx.create_gain()?;
        let volume = gain.gain();
        volume.set_value_at_time(0.0, now)?;
        volume.linear_ramp_to_value_at_time(velocity, now + 0.005)?;
        // Then settles a little lower while held.
        volume.set_target_at_time(velocity * 0.6, now + 0.005, 0.3)?;

        oscillator.connect_with_audio_node(&filter)?;
        filter.connect_with_audio_node(&gain)?;
        gain.connect_with_audio_node(&self.output)?;
        let scheduled: &AudioScheduledSourceNode = &oscillator;
        scheduled.start()?;
        self.voices.borrow_mut().push((note, oscillator, gain));
        Ok(())
    }

    fn note_off(&self, note: u8) -> Result<(), JsValue> {
        let now = self.ctx.current_time();
        let mut voices = self.voices.borrow_mut();
        let (released, playing): (Vec<_>, Vec<_>) =
            voices.drain(..).partition(|(voice, _, _)| *voice == note);
        *voices = playing;
        for (_, oscillator, gain) in released {
            // Fades out in about a quarter of a second.
            gain.gain().set_target_at_time(0.0, now, 0.06)?;
            let scheduled: &AudioScheduledSourceNode = &oscillator;
            scheduled.stop_with_when(now + 0.4)?;
        }
        Ok(())
    }
}

fn element<T: JsCast>(document: &Document, id: &str) -> Result<T, JsValue> {
    document
        .get_element_by_id(id)
        .ok_or_else(|| JsValue::from_str(&format!("missing #{id} element")))?
        .dyn_into()
        .map_err(|_| JsValue::from_str(&format!("#{id} has the wrong element type")))
}
