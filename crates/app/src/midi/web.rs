//! MIDI input through the Web MIDI API (Chrome, Edge, Firefox; not Safari).
//! Access is requested when the `#midi` button is clicked, since browsers ask
//! for permission. Every connected input is listened to, including devices
//! plugged in later.

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{HtmlButtonElement, MidiAccess, MidiInput, MidiMessageEvent};

use super::state::{MidiFrame, MidiState};

pub struct Midi {
    state: Rc<RefCell<MidiState>>,
}

impl Midi {
    pub fn new() -> Result<Self, JsValue> {
        let document = web_sys::window().unwrap().document().unwrap();
        let button: HtmlButtonElement = document
            .get_element_by_id("midi")
            .ok_or("missing #midi element")?
            .dyn_into()?;
        let state = Rc::new(RefCell::new(MidiState::default()));

        let on_click = {
            let (state, button) = (state.clone(), button.clone());
            Closure::<dyn FnMut()>::new(move || {
                wasm_bindgen_futures::spawn_local(connect(state.clone(), button.clone()));
            })
        };
        button.set_onclick(Some(on_click.as_ref().unchecked_ref()));
        // The listeners live as long as the page.
        on_click.forget();

        Ok(Self { state })
    }

    pub fn frame(&mut self, dt: f32) -> MidiFrame {
        self.state.borrow_mut().frame(dt)
    }
}

async fn connect(state: Rc<RefCell<MidiState>>, button: HtmlButtonElement) {
    let navigator = web_sys::window().unwrap().navigator();
    let access = match navigator.request_midi_access() {
        Ok(request) => JsFuture::from(request).await,
        Err(error) => Err(error),
    };
    let access: MidiAccess = match access {
        Ok(access) => access.unchecked_into(),
        Err(error) => {
            log::warn!("MIDI unavailable: {error:?}");
            button.set_text_content(Some("MIDI unavailable"));
            button.set_title("This browser doesn't support MIDI, or access was refused.");
            return;
        }
    };

    let on_message = Closure::<dyn FnMut(MidiMessageEvent)>::new(move |event: MidiMessageEvent| {
        if let Ok(data) = event.data() {
            state.borrow_mut().handle(&data);
        }
    });
    let listen = {
        let (access, button) = (access.clone(), button.clone());
        move || listen_to_inputs(&access, &button, on_message.as_ref().unchecked_ref())
    };
    listen();
    // Devices plugged in or out later.
    let on_state_change = Closure::<dyn FnMut()>::new(listen);
    access.set_onstatechange(Some(on_state_change.as_ref().unchecked_ref()));
    on_state_change.forget();
}

/// Listens to every input, and shows their names on the button.
fn listen_to_inputs(
    access: &MidiAccess,
    button: &HtmlButtonElement,
    on_message: &js_sys::Function,
) {
    let mut names = Vec::new();
    for input in access.inputs().values() {
        let Ok(input) = input.map(JsCast::unchecked_into::<MidiInput>) else {
            continue;
        };
        input.set_onmidimessage(Some(on_message));
        names.push(input.name().unwrap_or_else(|| "unnamed device".to_owned()));
    }
    let label = match names.len() {
        0 => "MIDI: no device".to_owned(),
        1 => "MIDI: 1 device".to_owned(),
        count => format!("MIDI: {count} devices"),
    };
    button.set_text_content(Some(&label));
    button.set_title(&names.join(", "));
}
