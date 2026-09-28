//! In-browser shader editor. The panel's elements are defined in `index.html`.
//!
//! Edits are sent to the app as [`UserEvent::ShaderEdited`] once typing
//! pauses, and saved in the browser's local storage.
//!
//! Highlighting: the textarea's text is transparent, and a `<pre>` with the
//! highlighted code sits exactly behind it. Typing, selection, undo and
//! copy/paste all stay native.

use std::cell::Cell;
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use web_sys::{
    Document, HtmlButtonElement, HtmlElement, HtmlTextAreaElement, KeyboardEvent, Storage,
};
use winit::event_loop::EventLoopProxy;

use super::highlight::highlight;
use crate::UserEvent;
use render::ShaderError;

/// How long typing must pause before the shader is recompiled.
const COMPILE_DELAY_MS: i32 = 150;

pub struct Editor {
    title: HtmlElement,
    code: HtmlTextAreaElement,
    /// Re-highlights the code after it changed.
    refresh: Rc<dyn Fn()>,
    error: HtmlElement,
    /// Red band behind the line of the error.
    error_line: HtmlElement,
    storage: Option<Storage>,
    /// Index of the visual whose source is in the editor.
    visual: Rc<Cell<usize>>,
    /// Timer of the recompile waiting for typing to pause.
    pending: Rc<Cell<Option<i32>>>,
}

impl Editor {
    pub fn new(proxy: EventLoopProxy<UserEvent>) -> Result<Self, JsValue> {
        let window = web_sys::window().unwrap();
        let document = window.document().unwrap();
        let panel: HtmlElement = element(&document, "editor")?;
        let code: HtmlTextAreaElement = element(&document, "editor-code")?;
        let backdrop: HtmlElement = element(&document, "editor-backdrop")?;
        let highlighted: HtmlElement = element(&document, "editor-highlight")?;
        let visual = Rc::new(Cell::new(0));
        let pending = Rc::new(Cell::new(None));

        let refresh: Rc<dyn Fn()> = {
            let code = code.clone();
            // Extra lines, so the backdrop can scroll as far as the textarea.
            Rc::new(move || highlighted.set_inner_html(&(highlight(&code.value()) + "\n\n\n")))
        };

        let on_scroll = {
            let code = code.clone();
            Closure::<dyn FnMut()>::new(move || {
                backdrop.set_scroll_top(code.scroll_top());
                backdrop.set_scroll_left(code.scroll_left());
            })
        };
        code.set_onscroll(Some(on_scroll.as_ref().unchecked_ref()));
        on_scroll.forget();

        let send = {
            let (proxy, code, visual) = (proxy.clone(), code.clone(), visual.clone());
            Closure::<dyn FnMut()>::new(move || {
                let _ = proxy.send_event(UserEvent::ShaderEdited {
                    visual: visual.get(),
                    source: code.value(),
                });
            })
        };
        let schedule_compile: Rc<dyn Fn()> = {
            let pending = pending.clone();
            Rc::new(move || {
                let window = web_sys::window().unwrap();
                if let Some(timer) = pending.take() {
                    window.clear_timeout_with_handle(timer);
                }
                let timer = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                    send.as_ref().unchecked_ref(),
                    COMPILE_DELAY_MS,
                );
                pending.set(timer.ok());
            })
        };

        let on_input = {
            let (refresh, schedule_compile) = (refresh.clone(), schedule_compile.clone());
            Closure::<dyn FnMut()>::new(move || {
                refresh();
                schedule_compile();
            })
        };
        code.set_oninput(Some(on_input.as_ref().unchecked_ref()));
        on_input.forget();

        // Tab indents instead of moving the focus out of the editor.
        let on_keydown = {
            let (code, refresh) = (code.clone(), refresh.clone());
            Closure::<dyn FnMut(KeyboardEvent)>::new(move |event: KeyboardEvent| {
                if event.key() != "Tab" || event.ctrl_key() || event.alt_key() {
                    return;
                }
                event.prevent_default();
                let start = code.selection_start().ok().flatten().unwrap_or(0);
                let end = code.selection_end().ok().flatten().unwrap_or(start);
                let _ = code.set_range_text_with_start_and_end_and_mode("    ", start, end, "end");
                refresh();
                schedule_compile();
            })
        };
        code.set_onkeydown(Some(on_keydown.as_ref().unchecked_ref()));
        on_keydown.forget();

        on_click(&document, "edit", {
            let panel = panel.clone();
            move || {
                let _ = panel.class_list().toggle("open");
            }
        })?;
        on_click(&document, "editor-close", move || {
            let _ = panel.class_list().remove_1("open");
        })?;
        on_click(&document, "editor-reset", {
            let visual = visual.clone();
            move || {
                let _ = proxy.send_event(UserEvent::ShaderReset {
                    visual: visual.get(),
                });
            }
        })?;

        Ok(Self {
            title: element(&document, "editor-title")?,
            error: element(&document, "editor-error")?,
            error_line: element(&document, "editor-error-line")?,
            refresh,
            storage: window.local_storage().ok().flatten(),
            code,
            visual,
            pending,
        })
    }

    /// Loads a visual's source into the editor.
    pub fn show(&self, visual: usize, name: &str, source: &str) {
        // Drop a recompile still pending for the previous content.
        if let Some(timer) = self.pending.take() {
            web_sys::window().unwrap().clear_timeout_with_handle(timer);
        }
        self.visual.set(visual);
        self.title.set_text_content(Some(name));
        self.code.set_value(source);
        (self.refresh)();
    }

    /// Shows a compile error, or hides it with `None`.
    pub fn set_error(&self, error: Option<&ShaderError>) {
        self.error
            .set_text_content(error.map(|e| e.message.as_str()));
        self.error.set_hidden(error.is_none());

        let line = error.and_then(|e| e.line);
        if let Some(line) = line {
            // Must match the padding and line height of the editor's CSS.
            let top = format!("calc(8px + {} * 1.45em)", line - 1);
            let _ = self.error_line.style().set_property("top", &top);
        }
        self.error_line.set_hidden(line.is_none());
    }

    /// The source saved for a visual, if it was edited.
    pub fn saved_source(&self, name: &str) -> Option<String> {
        self.storage.as_ref()?.get_item(&storage_key(name)).ok()?
    }

    pub fn save_source(&self, name: &str, source: &str) {
        if let Some(storage) = &self.storage {
            let _ = storage.set_item(&storage_key(name), source);
        }
    }

    pub fn forget_source(&self, name: &str) {
        if let Some(storage) = &self.storage {
            let _ = storage.remove_item(&storage_key(name));
        }
    }
}

fn storage_key(name: &str) -> String {
    format!("ongakue.shader.{name}")
}

fn on_click(document: &Document, id: &str, handler: impl FnMut() + 'static) -> Result<(), JsValue> {
    let button: HtmlButtonElement = element(document, id)?;
    let handler = Closure::<dyn FnMut()>::new(handler);
    button.set_onclick(Some(handler.as_ref().unchecked_ref()));
    // The listeners live as long as the page.
    handler.forget();
    Ok(())
}

fn element<T: JsCast>(document: &Document, id: &str) -> Result<T, JsValue> {
    document
        .get_element_by_id(id)
        .ok_or_else(|| JsValue::from_str(&format!("missing #{id} element")))?
        .dyn_into()
        .map_err(|_| JsValue::from_str(&format!("#{id} has the wrong element type")))
}
