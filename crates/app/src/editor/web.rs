//! In-browser shader editor. The panel's elements are defined in `index.html`.
//!
//! Edits are sent to the app as [`UserEvent::ShaderEdited`] once typing
//! pauses, and saved in the browser's local storage.
//!
//! Highlighting: the textarea's text is transparent, and a `<pre>` with the
//! highlighted code sits exactly behind it. Typing, selection, undo and
//! copy/paste all stay native. Completion and hover docs are in `popups.rs`.

use std::cell::Cell;
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use web_sys::{
    Document, Event, HtmlButtonElement, HtmlElement, HtmlOptionElement, HtmlSelectElement,
    HtmlTextAreaElement, InputEvent, KeyboardEvent, MouseEvent, Storage,
};
use winit::event_loop::EventLoopProxy;

use super::highlight::{highlight, line_numbers};
use super::names::{file_name, name_from_file, unique_name};
use super::popups::{Popups, insert_text};
use crate::UserEvent;
use render::{ShaderError, TEMPLATE_SHADER};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Blob, BlobPropertyBag, HtmlAnchorElement, HtmlInputElement, Url};

/// How long typing must pause before the shader is recompiled.
const COMPILE_DELAY_MS: i32 = 150;

pub struct Editor {
    /// Picks the visual to display and edit.
    visuals: HtmlSelectElement,
    /// Number of built-in visuals, listed first; the others can be deleted.
    builtin_count: Rc<Cell<usize>>,
    delete: HtmlButtonElement,
    code: HtmlTextAreaElement,
    /// Re-highlights the code after it changed.
    refresh: Rc<dyn Fn()>,
    popups: Rc<Popups>,
    error: HtmlElement,
    /// Red band behind the line of the error.
    error_line: HtmlElement,
    /// Line of the current error, from 1, shown in red in the gutter.
    error_at: Rc<Cell<Option<usize>>>,
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
        let gutter: HtmlElement = element(&document, "editor-gutter")?;
        let error_at = Rc::new(Cell::new(None));
        let visuals: HtmlSelectElement = element(&document, "editor-title")?;
        let visual = Rc::new(Cell::new(0));
        let pending = Rc::new(Cell::new(None));
        let popups = Popups::new(
            code.clone(),
            element(&document, "editor-complete")?,
            element(&document, "editor-hover")?,
        );

        let refresh: Rc<dyn Fn()> = {
            let (code, gutter, error_at) = (code.clone(), gutter.clone(), error_at.clone());
            Rc::new(move || {
                let source = code.value();
                // Extra lines, so the backdrop and gutter can scroll as far as
                // the textarea.
                highlighted.set_inner_html(&(highlight(&source) + "\n\n\n"));
                gutter.set_inner_html(&(line_numbers(&source, error_at.get()) + "\n\n\n"));
            })
        };

        let on_scroll = {
            let (code, popups) = (code.clone(), popups.clone());
            Closure::<dyn FnMut()>::new(move || {
                backdrop.set_scroll_top(code.scroll_top());
                backdrop.set_scroll_left(code.scroll_left());
                gutter.set_scroll_top(code.scroll_top());
                popups.hide();
                popups.hide_tooltip();
            })
        };
        code.set_onscroll(Some(on_scroll.as_ref().unchecked_ref()));
        on_scroll.forget();

        // Moving the cursor or leaving the editor closes the popups.
        let hide_popups = {
            let popups = popups.clone();
            Closure::<dyn FnMut()>::new(move || {
                popups.hide();
                popups.hide_tooltip();
            })
        };
        code.set_onclick(Some(hide_popups.as_ref().unchecked_ref()));
        code.set_onblur(Some(hide_popups.as_ref().unchecked_ref()));
        hide_popups.forget();

        let on_mousemove = {
            let popups = popups.clone();
            Closure::<dyn FnMut(MouseEvent)>::new(move |event: MouseEvent| {
                popups.hover_at(event.offset_x() as f64, event.offset_y() as f64);
            })
        };
        code.set_onmousemove(Some(on_mousemove.as_ref().unchecked_ref()));
        on_mousemove.forget();

        let on_mouseleave = {
            let popups = popups.clone();
            Closure::<dyn FnMut()>::new(move || popups.hide_tooltip())
        };
        code.set_onmouseleave(Some(on_mouseleave.as_ref().unchecked_ref()));
        on_mouseleave.forget();

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
            let (refresh, popups) = (refresh.clone(), popups.clone());
            Closure::<dyn FnMut(Event)>::new(move |event: Event| {
                refresh();
                schedule_compile();
                let kind = event.dyn_ref::<InputEvent>().map(InputEvent::input_type);
                match kind.as_deref() {
                    Some("insertText") => popups.update(true),
                    Some(kind) if kind.starts_with("delete") => popups.update(false),
                    _ => popups.hide(),
                }
            })
        };
        code.set_oninput(Some(on_input.as_ref().unchecked_ref()));
        on_input.forget();

        let on_keydown = {
            let (code, popups) = (code.clone(), popups.clone());
            Closure::<dyn FnMut(KeyboardEvent)>::new(move |event: KeyboardEvent| {
                popups.hide_tooltip();
                if popups.handle_key(&event) {
                    return;
                }
                // Tab indents instead of moving the focus out of the editor.
                if event.key() == "Tab" && !event.ctrl_key() && !event.alt_key() {
                    event.prevent_default();
                    insert_text(&code, "    ");
                }
            })
        };
        code.set_onkeydown(Some(on_keydown.as_ref().unchecked_ref()));
        on_keydown.forget();

        let on_change = {
            let (proxy, visuals) = (proxy.clone(), visuals.clone());
            Closure::<dyn FnMut()>::new(move || {
                if let Ok(visual) = usize::try_from(visuals.selected_index()) {
                    let _ = proxy.send_event(UserEvent::VisualSelected(visual));
                }
            })
        };
        visuals.set_onchange(Some(on_change.as_ref().unchecked_ref()));
        on_change.forget();

        let builtin_count = Rc::new(Cell::new(0));
        let names = {
            let visuals = visuals.clone();
            move || -> Vec<String> {
                let options = visuals.options();
                (0..options.length())
                    .filter_map(|i| options.item(i))
                    .filter_map(|option| option.text_content())
                    .collect()
            }
        };

        on_click(&document, "editor-new", {
            let (proxy, names) = (proxy.clone(), names.clone());
            move || {
                let window = web_sys::window().unwrap();
                let Ok(Some(wanted)) =
                    window.prompt_with_message_and_default("Name of the new visual:", "my visual")
                else {
                    return;
                };
                if let Some(name) = unique_name(&wanted, &names()) {
                    let source = TEMPLATE_SHADER.to_owned();
                    let _ = proxy.send_event(UserEvent::VisualAdded { name, source });
                }
            }
        })?;

        // Import: the button opens a hidden file input.
        let import_input: HtmlInputElement = element(&document, "editor-import-file")?;
        let on_import = {
            let (proxy, names, input) = (proxy.clone(), names.clone(), import_input.clone());
            Closure::<dyn FnMut()>::new(move || {
                let Some(file) = input.files().and_then(|files| files.get(0)) else {
                    return;
                };
                // Picking the same file again must fire `change` again.
                input.set_value("");
                let (proxy, names) = (proxy.clone(), names.clone());
                wasm_bindgen_futures::spawn_local(async move {
                    let Ok(text) = JsFuture::from(file.text()).await else {
                        return;
                    };
                    let source = text.as_string().unwrap_or_default();
                    if let Some(name) = unique_name(name_from_file(&file.name()), &names()) {
                        let _ = proxy.send_event(UserEvent::VisualAdded { name, source });
                    }
                });
            })
        };
        import_input.set_onchange(Some(on_import.as_ref().unchecked_ref()));
        on_import.forget();
        on_click(&document, "editor-import", move || import_input.click())?;

        on_click(&document, "editor-export", {
            let (code, visuals) = (code.clone(), visuals.clone());
            move || {
                let name = visuals
                    .selected_options()
                    .item(0)
                    .and_then(|option| option.text_content())
                    .unwrap_or_else(|| "shader".to_owned());
                if let Err(error) = download(&file_name(&name), &code.value()) {
                    log::error!("export failed: {error:?}");
                }
            }
        })?;

        on_click(&document, "editor-delete", {
            let (proxy, visual, builtin_count) =
                (proxy.clone(), visual.clone(), builtin_count.clone());
            move || {
                let index = visual.get();
                let Some(name) = names().into_iter().nth(index) else {
                    return;
                };
                let question = format!("Delete the visual \"{name}\"? This can't be undone.");
                let confirmed = web_sys::window()
                    .unwrap()
                    .confirm_with_message(&question)
                    .unwrap_or(false);
                if index >= builtin_count.get() && confirmed {
                    let _ = proxy.send_event(UserEvent::VisualDeleted(index));
                }
            }
        })?;

        on_click(&document, "edit", {
            let panel = panel.clone();
            move || {
                let _ = panel.class_list().toggle("open");
            }
        })?;
        on_click(&document, "editor-close", move || {
            let _ = panel.class_list().remove_1("open");
        })?;
        /*on_click(&document, "editor-reset", {
            let visual = visual.clone();
            move || {
                let _ = proxy.send_event(UserEvent::ShaderReset {
                    visual: visual.get(),
                });
            }
        })?;*/

        Ok(Self {
            visuals,
            builtin_count,
            delete: element(&document, "editor-delete")?,
            error: element(&document, "editor-error")?,
            error_line: element(&document, "editor-error-line")?,
            error_at,
            refresh,
            popups,
            storage: window.local_storage().ok().flatten(),
            code,
            visual,
            pending,
        })
    }

    /// Fills the visual picker, in the renderer's order. The first
    /// `builtin_count` are built in, and can't be deleted.
    pub fn set_visuals(&self, names: &[&str], builtin_count: usize) {
        self.builtin_count.set(builtin_count);
        self.visuals.set_length(0);
        for name in names {
            if let Ok(option) = HtmlOptionElement::new_with_text(name) {
                let _ = self.visuals.add_with_html_option_element(&option);
            }
        }
    }

    /// Loads a visual's source into the editor.
    pub fn show(&self, visual: usize, source: &str) {
        // Drop a recompile still pending for the previous content.
        if let Some(timer) = self.pending.take() {
            web_sys::window().unwrap().clear_timeout_with_handle(timer);
        }
        self.popups.hide();
        self.popups.hide_tooltip();
        self.visual.set(visual);
        self.visuals.set_selected_index(visual as i32);
        self.delete.set_disabled(visual < self.builtin_count.get());
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
        self.error_at.set(line);
        (self.refresh)();
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

    /// Names of the visuals the user added, in order.
    pub fn saved_visuals(&self) -> Vec<String> {
        let list = self
            .storage
            .as_ref()
            .and_then(|storage| storage.get_item(VISUALS_KEY).ok().flatten())
            .unwrap_or_default();
        list.lines().map(str::to_owned).collect()
    }

    pub fn save_visuals(&self, names: &[&str]) {
        if let Some(storage) = &self.storage {
            let _ = storage.set_item(VISUALS_KEY, &names.join("\n"));
        }
    }
}

/// Storage key of the list of visuals the user added; their sources are
/// stored like edits, under [`storage_key`].
const VISUALS_KEY: &str = "ongakue.visuals";

/// Makes the browser download `text` as a file.
fn download(file_name: &str, text: &str) -> Result<(), JsValue> {
    let options = BlobPropertyBag::new();
    options.set_type("text/plain");
    let parts = js_sys::Array::of1(&JsValue::from_str(text));
    let blob = Blob::new_with_str_sequence_and_options(&parts, &options)?;
    let url = Url::create_object_url_with_blob(&blob)?;

    let window = web_sys::window().unwrap();
    let link: HtmlAnchorElement = window.document().unwrap().create_element("a")?.dyn_into()?;
    link.set_href(&url);
    link.set_download(file_name);
    link.click();
    // Free the file's memory once the download has started.
    let revoke = Closure::once_into_js(move || Url::revoke_object_url(&url));
    window.set_timeout_with_callback_and_timeout_and_arguments_0(revoke.unchecked_ref(), 10_000)?;
    Ok(())
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
