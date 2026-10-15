//! The computer keyboard as a piano, on the web: listens to the whole page,
//! except while typing in a text field (the shader editor, a name prompt…),
//! so typing never plays notes.

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use web_sys::{Element, HtmlElement, HtmlInputElement, KeyboardEvent};
use winit::event::WindowEvent;

use super::keys::KeyPiano;

pub struct Piano;

impl Piano {
    /// `play` receives the MIDI messages of the keys pressed and released.
    pub fn new(play: impl Fn(&[u8]) + 'static) -> Result<Self, JsValue> {
        let window = web_sys::window().unwrap();
        let keys = Rc::new(RefCell::new(KeyPiano::default()));
        let play = Rc::new(play);

        let on_down = {
            let (keys, play) = (keys.clone(), play.clone());
            Closure::<dyn FnMut(KeyboardEvent)>::new(move |event: KeyboardEvent| {
                // Shortcuts (Ctrl+Z…) and AltGr characters aren't notes.
                let modified = event.ctrl_key() || event.alt_key() || event.meta_key();
                if event.repeat() || modified || is_typing(&event) {
                    return;
                }
                if let Some(message) = keys.borrow_mut().key_down(&event.code()) {
                    play(&message);
                }
            })
        };
        // Releases even while typing, so no note stays stuck.
        let on_up = {
            let (keys, play) = (keys.clone(), play.clone());
            Closure::<dyn FnMut(KeyboardEvent)>::new(move |event: KeyboardEvent| {
                if let Some(message) = keys.borrow_mut().key_up(&event.code()) {
                    play(&message);
                }
            })
        };
        // Key-ups never arrive once the window has lost the focus.
        let on_blur = Closure::<dyn FnMut()>::new(move || {
            for message in keys.borrow_mut().release_all() {
                play(&message);
            }
        });
        window.add_event_listener_with_callback("keydown", on_down.as_ref().unchecked_ref())?;
        window.add_event_listener_with_callback("keyup", on_up.as_ref().unchecked_ref())?;
        window.add_event_listener_with_callback("blur", on_blur.as_ref().unchecked_ref())?;
        // The listeners live as long as the page.
        on_down.forget();
        on_up.forget();
        on_blur.forget();
        Ok(Self)
    }

    /// Keys are handled by the page's listeners: nothing to do here.
    pub fn window_event(&mut self, _event: &WindowEvent) -> Vec<[u8; 3]> {
        Vec::new()
    }
}

/// Whether the key goes to a field where it types text.
fn is_typing(event: &KeyboardEvent) -> bool {
    let Some(target) = event
        .target()
        .and_then(|target| target.dyn_into::<Element>().ok())
    else {
        return false;
    };
    match target.tag_name().as_str() {
        // The visual picker counts: letters would jump between visuals.
        "TEXTAREA" | "SELECT" => true,
        "INPUT" => {
            let kind = target.unchecked_ref::<HtmlInputElement>().type_();
            !matches!(
                kind.as_str(),
                "range" | "checkbox" | "radio" | "button" | "submit" | "reset" | "file" | "color"
            )
        }
        _ => target
            .dyn_ref::<HtmlElement>()
            .is_some_and(HtmlElement::is_content_editable),
    }
}
