//! In-browser shader editor. The panel's elements are defined in `index.html`.
//!
//! Edits are sent to the app as [`UserEvent::ShaderEdited`] once typing
//! pauses, and saved in the browser's local storage. With auto compile off,
//! they run only when asked (the Compile button, Ctrl+Enter).
//!
//! The code itself is edited with CodeMirror (see `code.rs`).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use web_sys::{
    Document, HtmlButtonElement, HtmlElement, HtmlOptionElement, HtmlSelectElement, KeyboardEvent,
    Storage,
};
use winit::event_loop::EventLoopProxy;

use super::code::{self, CodeEditor};
use super::names::{file_name, name_from_file, unique_name};
use super::share;
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
    code: CodeEditor,
    /// The code as last edited, sent once typing pauses.
    latest: Rc<RefCell<String>>,
    /// Shows the language the code is read as.
    badge: HtmlElement,
    error: HtmlElement,
    storage: Option<Storage>,
    /// Index of the visual whose source is in the editor.
    visual: Rc<Cell<usize>>,
    /// Timer of the recompile waiting for typing to pause.
    pending: Rc<Cell<Option<i32>>>,
    /// Whether edits run as soon as typing pauses.
    auto_compile: Rc<Cell<bool>>,
    auto_button: HtmlButtonElement,
}

impl Editor {
    pub fn new(proxy: EventLoopProxy<UserEvent>) -> Result<Self, JsValue> {
        let window = web_sys::window().unwrap();
        let document = window.document().unwrap();
        let panel: HtmlElement = element(&document, "editor")?;
        let visuals: HtmlSelectElement = element(&document, "editor-title")?;
        let badge: HtmlElement = element(&document, "editor-language")?;
        let visual = Rc::new(Cell::new(0));
        let pending = Rc::new(Cell::new(None));
        let storage = window.local_storage().ok().flatten();
        let auto_compile = Rc::new(Cell::new(
            storage
                .as_ref()
                .and_then(|storage| storage.get_item(AUTO_COMPILE_KEY).ok().flatten())
                .as_deref()
                != Some("off"),
        ));
        let auto_button: HtmlButtonElement = element(&document, "editor-auto")?;
        show_auto_compile(&auto_button, auto_compile.get());

        // Saves the code in the editor once typing pauses, and recompiles it
        // with auto compile on.
        let latest = Rc::new(RefCell::new(String::new()));
        let send_later = {
            let (proxy, latest, visual, auto_compile) = (
                proxy.clone(),
                latest.clone(),
                visual.clone(),
                auto_compile.clone(),
            );
            Closure::<dyn FnMut()>::new(move || {
                let _ = proxy.send_event(UserEvent::ShaderEdited {
                    visual: visual.get(),
                    source: latest.borrow().clone(),
                    compile: auto_compile.get(),
                });
            })
        };
        // Saves and recompiles the code now.
        let compile_now: Rc<dyn Fn()> = {
            let (proxy, latest, visual, pending) = (
                proxy.clone(),
                latest.clone(),
                visual.clone(),
                pending.clone(),
            );
            Rc::new(move || {
                if let Some(timer) = pending.take() {
                    web_sys::window().unwrap().clear_timeout_with_handle(timer);
                }
                let _ = proxy.send_event(UserEvent::ShaderEdited {
                    visual: visual.get(),
                    source: latest.borrow().clone(),
                    compile: true,
                });
            })
        };
        let schedule_compile = {
            let pending = pending.clone();
            move || {
                let window = web_sys::window().unwrap();
                if let Some(timer) = pending.take() {
                    window.clear_timeout_with_handle(timer);
                }
                let timer = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                    send_later.as_ref().unchecked_ref(),
                    COMPILE_DELAY_MS,
                );
                pending.set(timer.ok());
            }
        };
        let changed = {
            let (badge, latest) = (badge.clone(), latest.clone());
            move |source: String| {
                let language = render::shader_language(&source);
                show_language(&badge, language);
                *latest.borrow_mut() = source;
                schedule_compile();
                code::language_name(language).to_owned()
            }
        };
        let code = code::create(&element(&document, "editor-body")?, changed, {
            let compile_now = compile_now.clone();
            move || compile_now()
        });

        on_click(&document, "editor-compile", {
            let compile_now = compile_now.clone();
            move || compile_now()
        })?;
        // Turning auto compile back on runs the code as it is now.
        on_click(&document, "editor-auto", {
            let (button, auto_compile, storage) =
                (auto_button.clone(), auto_compile.clone(), storage.clone());
            move || {
                let on = !auto_compile.get();
                set_auto_compile(&auto_compile, &button, storage.as_ref(), on);
                if on {
                    compile_now();
                }
            }
        })?;

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
            move || option_names(&visuals)
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

        // Downloads the code as `<name>.wgsl` or `.glsl`.
        let export: Rc<dyn Fn()> = {
            let (code, visuals) = (code.clone(), visuals.clone());
            Rc::new(move || {
                let name = selected_name(&visuals).unwrap_or_else(|| "shader".to_owned());
                let source = code.value();
                let extension = code::language_name(render::shader_language(&source));
                if let Err(error) = download(&file_name(&name, extension), &source) {
                    log::error!("export failed: {error:?}");
                }
            })
        };
        on_click(&document, "editor-export", {
            let export = export.clone();
            move || export()
        })?;

        // Share: copies a link carrying the visual's name and code.
        on_click(&document, "editor-share", {
            let (code, visuals) = (code.clone(), visuals.clone());
            move || {
                let name = selected_name(&visuals).unwrap_or_else(|| "shared".to_owned());
                let location = web_sys::window().unwrap().location();
                let page = format!(
                    "{}{}",
                    location.origin().unwrap_or_default(),
                    location.pathname().unwrap_or_default()
                );
                let link = format!("{page}#{}", share::encode(&name, &code.value()));
                wasm_bindgen_futures::spawn_local(copy_link(link));
            }
        })?;

        // A shared link opened while the app is already open.
        let on_hash = {
            let proxy = proxy.clone();
            let visuals = visuals.clone();
            Closure::<dyn FnMut()>::new(move || {
                if let Some((name, source)) = shared_visual(&option_names(&visuals)) {
                    let _ = proxy.send_event(UserEvent::VisualAdded { name, source });
                }
            })
        };
        window.add_event_listener_with_callback("hashchange", on_hash.as_ref().unchecked_ref())?;
        on_hash.forget();

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

        // Ctrl+S (Cmd+S on a Mac) anywhere in the editor: download the code,
        // like Export, instead of the browser's "Save page as".
        let on_save = Closure::<dyn FnMut(KeyboardEvent)>::new(move |event: KeyboardEvent| {
            let save_key = (event.ctrl_key() || event.meta_key())
                && !event.alt_key()
                && event.key().eq_ignore_ascii_case("s");
            if save_key {
                event.prevent_default();
                export();
            }
        });
        panel.add_event_listener_with_callback("keydown", on_save.as_ref().unchecked_ref())?;
        on_save.forget();

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
            storage,
            code,
            latest,
            badge,
            visual,
            pending,
            auto_compile,
            auto_button,
        })
    }

    /// Fills the visual picker, in the renderer's order. The first
    /// `builtin_count` are built in, and can't be deleted: they show
    /// [`BUILTIN_ICON`]. Each option's value is the visual's name.
    pub fn set_visuals(&self, names: &[&str], builtin_count: usize) {
        self.builtin_count.set(builtin_count);
        self.visuals.set_length(0);
        for (index, name) in names.iter().enumerate() {
            let builtin = index < builtin_count;
            let text = if builtin {
                format!("{BUILTIN_ICON} {name}")
            } else {
                name.to_string()
            };
            if let Ok(option) = HtmlOptionElement::new_with_text_and_value(&text, name) {
                if builtin {
                    option.set_title("Built-in visual: comes with the app, can't be deleted");
                }
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
        self.visuals.set_selected_index(visual as i32);
        self.delete.set_disabled(visual < self.builtin_count.get());
        // The same code again (e.g. after a GPU reset): keep the undo history.
        if visual != self.visual.get() || self.code.value() != source {
            self.code.set_value(source);
        }
        let language = render::shader_language(source);
        self.code.set_language(code::language_name(language));
        show_language(&self.badge, language);
        *self.latest.borrow_mut() = source.to_owned();
        self.visual.set(visual);
    }

    /// Whether edits run as soon as typing pauses. Remembered in the browser.
    pub fn set_auto_compile(&self, on: bool) {
        set_auto_compile(
            &self.auto_compile,
            &self.auto_button,
            self.storage.as_ref(),
            on,
        );
    }

    /// Shows a short notice at the bottom of the page.
    pub fn notify(&self, text: &str) {
        notify(text);
    }

    /// Shows a compile error, or hides it with `None`.
    pub fn set_error(&self, error: Option<&ShaderError>) {
        self.error
            .set_text_content(error.map(|e| e.message.as_str()));
        self.error.set_hidden(error.is_none());

        // Marks the line in the editor too, with the message's first line on hover.
        let line = error.and_then(|e| e.line).unwrap_or(0);
        let summary = error.and_then(|e| e.message.lines().next()).unwrap_or("");
        self.code.set_error(line as u32, summary);
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

    /// The name of the visual shown last, to show it again on the next visit.
    pub fn saved_current_visual(&self) -> Option<String> {
        self.storage.as_ref()?.get_item(CURRENT_KEY).ok()?
    }

    pub fn save_current_visual(&self, name: &str) {
        if let Some(storage) = &self.storage {
            let _ = storage.set_item(CURRENT_KEY, name);
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

    /// The visual of a shared link the page was opened with, if any, named
    /// so it doesn't clash with the existing ones. Call once the visuals are
    /// listed.
    pub fn take_shared_visual(&self) -> Option<(String, String)> {
        shared_visual(&option_names(&self.visuals))
    }
}

/// Shown before the names of the built-in visuals in the picker.
const BUILTIN_ICON: &str = "\u{1F4E6}\u{FE0E}";

/// The names of the visuals in the picker: the options' values, as their
/// text can have an icon.
fn option_names(visuals: &HtmlSelectElement) -> Vec<String> {
    let options = visuals.options();
    (0..options.length())
        .filter_map(|i| options.item(i))
        .filter_map(|option| option.dyn_into::<HtmlOptionElement>().ok())
        .map(|option| option.value())
        .collect()
}

/// The name of the visual selected in the picker.
fn selected_name(visuals: &HtmlSelectElement) -> Option<String> {
    Some(visuals.value()).filter(|name| !name.is_empty())
}

/// Reads a shared link from the address bar, then removes it, so reloading
/// doesn't add the visual again. Tells the user what happened.
fn shared_visual(names: &[String]) -> Option<(String, String)> {
    let window = web_sys::window().unwrap();
    let hash = window.location().hash().unwrap_or_default();
    let result = share::decode(hash.strip_prefix('#').unwrap_or(&hash))?;
    if let Ok(history) = window.history() {
        let location = window.location();
        let page = format!(
            "{}{}",
            location.pathname().unwrap_or_default(),
            location.search().unwrap_or_default()
        );
        let _ = history.replace_state_with_url(&JsValue::NULL, "", Some(&page));
    }
    match result {
        Ok((name, source)) => {
            let name = unique_name(&name, names)
                .or_else(|| unique_name("shared", names))
                .unwrap();
            notify(&format!("Added the shared visual \"{name}\""));
            Some((name, source))
        }
        Err(error) => {
            notify(&error.to_string());
            None
        }
    }
}

/// Copies a link to the clipboard; if the browser refuses, shows it to copy
/// by hand.
async fn copy_link(link: String) {
    let window = web_sys::window().unwrap();
    let copied = JsFuture::from(window.navigator().clipboard().write_text(&link)).await;
    if copied.is_ok() {
        notify("Link copied: paste it to share this visual");
    } else {
        let _ =
            window.prompt_with_message_and_default("Copy this link to share the visual:", &link);
    }
}

thread_local! {
    /// The timer hiding the current notice.
    static NOTICE_TIMER: Cell<Option<i32>> = const { Cell::new(None) };
}

/// Shows a short notice at the bottom of the page, long enough to read it.
pub fn notify(text: &str) {
    let window = web_sys::window().unwrap();
    let Some(notice) = window.document().unwrap().get_element_by_id("notice") else {
        return;
    };
    notice.set_text_content(Some(text));
    let _ = notice.class_list().add_1("show");
    if let Some(timer) = NOTICE_TIMER.take() {
        window.clear_timeout_with_handle(timer);
    }
    let hide = Closure::once_into_js(move || {
        let _ = notice.class_list().remove_1("show");
    });
    let duration = (text.len() as i32 * 60).max(3500);
    let timer = window
        .set_timeout_with_callback_and_timeout_and_arguments_0(hide.unchecked_ref(), duration);
    NOTICE_TIMER.set(timer.ok());
}

/// Storage key of the list of visuals the user added; their sources are
/// stored like edits, under [`storage_key`].
const VISUALS_KEY: &str = "ongakue.visuals";

/// Storage key of the name of the visual shown last.
const CURRENT_KEY: &str = "ongakue.current";

/// Storage key of the auto compile setting: "on" or "off".
const AUTO_COMPILE_KEY: &str = "ongakue.autocompile";

fn set_auto_compile(
    state: &Cell<bool>,
    button: &HtmlButtonElement,
    storage: Option<&Storage>,
    on: bool,
) {
    state.set(on);
    show_auto_compile(button, on);
    if let Some(storage) = storage {
        let _ = storage.set_item(AUTO_COMPILE_KEY, if on { "on" } else { "off" });
    }
}

fn show_auto_compile(button: &HtmlButtonElement, on: bool) {
    let _ = button.class_list().toggle_with_force("on", on);
    let _ = button.set_attribute("aria-checked", if on { "true" } else { "false" });
}

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

/// Shows the language the code is read as, and why.
fn show_language(badge: &HtmlElement, language: render::ShaderLanguage) {
    let (name, why) = match language {
        render::ShaderLanguage::Wgsl => (
            "WGSL",
            "Read as WGSL: the code defines fn fs_main. Code defining mainImage is read as GLSL.",
        ),
        render::ShaderLanguage::Glsl => (
            "GLSL",
            "Read as GLSL, Shadertoy style: the code defines mainImage. Otherwise it's read as WGSL.",
        ),
    };
    badge.set_text_content(Some(name));
    badge.set_title(why);
    badge.set_class_name(&name.to_lowercase());
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
