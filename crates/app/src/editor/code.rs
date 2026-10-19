//! The code editor itself: CodeMirror (`web/codemirror/editor.js`, bundled
//! into `codemirror.js`), driven from here. CodeMirror handles typing, undo,
//! search, the line numbers and the popups; what to suggest, explain on hover
//! and color comes from `assist.rs` and `wgsl.rs`, until shader-language-server
//! is started and takes over (see `web/codemirror/lsp.js`).

use js_sys::{Array, Object, Reflect};
use wasm_bindgen::prelude::*;
use web_sys::HtmlElement;

use super::assist::{self, Kind};
use super::wgsl::{BUILTINS, GLSL, Language, WGSL};

#[wasm_bindgen]
extern "C" {
    /// An editor made by `OngakueCodeMirror.create`.
    #[derive(Clone)]
    pub type CodeEditor;

    #[wasm_bindgen(js_namespace = OngakueCodeMirror, js_name = create)]
    fn create_editor(parent: &HtmlElement, hooks: &JsValue) -> CodeEditor;

    #[wasm_bindgen(method, js_name = getValue)]
    pub fn value(this: &CodeEditor) -> String;

    /// Shows another visual's code, with its own undo history.
    #[wasm_bindgen(method, js_name = setValue)]
    pub fn set_value(this: &CodeEditor, code: &str, visual: &str);

    /// `"wgsl"` or `"glsl"`: how to color the code.
    #[wasm_bindgen(method, js_name = setLanguage)]
    pub fn set_language(this: &CodeEditor, name: &str);

    /// Marks an error's line (from 1), or clears the mark with line 0.
    #[wasm_bindgen(method, js_name = setError)]
    pub fn set_error(this: &CodeEditor, line: u32, message: &str);

    /// Starts shader-language-server, from its worker script and wasm file
    /// (addresses relative to the page). Does nothing once started.
    #[wasm_bindgen(method, js_name = startLanguageServer)]
    pub fn start_language_server(this: &CodeEditor, worker_url: &str, wasm_url: &str);
}

/// Creates the editor in `parent`. `changed` gets the code after each edit,
/// and returns its language's name, to color it accordingly. `compile` is
/// called on Ctrl+Enter. `server_status` gets the language server's state
/// ("loading", "ready", "error" or "exited") and details, once started.
pub fn create(
    parent: &HtmlElement,
    changed: impl FnMut(String) -> String + 'static,
    compile: impl FnMut() + 'static,
    server_status: impl FnMut(String, String) + 'static,
) -> CodeEditor {
    let hooks = Object::new();
    set(
        &hooks,
        "changed",
        Closure::<dyn FnMut(String) -> String>::new(changed).into_js_value(),
    );
    set(
        &hooks,
        "compile",
        Closure::<dyn FnMut()>::new(compile).into_js_value(),
    );
    set(
        &hooks,
        "serverStatus",
        Closure::<dyn FnMut(String, String)>::new(server_status).into_js_value(),
    );
    // The code around a visual's, for the language server, per language.
    let context = Object::new();
    for language in [render::ShaderLanguage::Wgsl, render::ShaderLanguage::Glsl] {
        let (before, after) = render::language_server_context(language);
        let pair = Array::of2(&before.into(), &after.into());
        set(&context, language_name(language), pair.into());
    }
    set(&hooks, "context", context.into());
    set(
        &hooks,
        "complete",
        Closure::<dyn FnMut(String, u32) -> JsValue>::new(complete).into_js_value(),
    );
    set(
        &hooks,
        "hover",
        Closure::<dyn FnMut(String, u32) -> JsValue>::new(hover).into_js_value(),
    );
    let words = Object::new();
    set(&words, "wgsl", word_lists(&WGSL));
    set(&words, "glsl", word_lists(&GLSL));
    set(&hooks, "words", words.into());
    create_editor(parent, &hooks)
}

/// The language name the editor uses.
pub fn language_name(language: render::ShaderLanguage) -> &'static str {
    match language {
        render::ShaderLanguage::Wgsl => "wgsl",
        render::ShaderLanguage::Glsl => "glsl",
    }
}

/// Suggestions at `position` (in UTF-16 units), for CodeMirror's completion.
fn complete(code: String, position: u32) -> JsValue {
    let cursor = assist::utf16_to_byte(&code, position as usize);
    let Some(completion) = assist::complete(&code, cursor) else {
        return JsValue::NULL;
    };
    let options = Array::new();
    for suggestion in completion.suggestions {
        let option = Object::new();
        set(&option, "label", suggestion.label.into());
        set(&option, "type", kind_name(suggestion.kind).into());
        if !suggestion.detail.is_empty() {
            set(&option, "detail", suggestion.detail.into());
        }
        if !suggestion.doc.is_empty() {
            set(&option, "info", suggestion.doc.into());
        }
        options.push(&option);
    }
    let result = Object::new();
    set(&result, "from", position_of(&code, completion.start));
    set(&result, "options", options.into());
    result.into()
}

/// What the word at `position` (in UTF-16 units) is, for the hover tooltip.
fn hover(code: String, position: u32) -> JsValue {
    let offset = assist::utf16_to_byte(&code, position as usize);
    let Some(hover) = assist::hover(&code, offset) else {
        return JsValue::NULL;
    };
    let result = Object::new();
    set(&result, "from", position_of(&code, hover.start));
    set(&result, "title", hover.title.into());
    set(&result, "doc", hover.doc.into());
    result.into()
}

/// CodeMirror's names for the kinds of suggestions (they pick the icons).
fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Local => "variable",
        Kind::Field => "property",
        Kind::Input => "constant",
        Kind::Function => "function",
        Kind::Type => "type",
        Kind::Keyword => "keyword",
    }
}

/// The words to color for one language.
fn word_lists(language: &Language) -> JsValue {
    let list = |words: &mut dyn Iterator<Item = &str>| -> JsValue {
        words.map(JsValue::from_str).collect::<Array>().into()
    };
    let lists = Object::new();
    set(
        &lists,
        "keywords",
        list(&mut language.keywords.iter().copied()),
    );
    set(&lists, "types", list(&mut language.types.iter().copied()));
    set(
        &lists,
        "functions",
        list(&mut BUILTINS.iter().map(|doc| doc.name)),
    );
    set(
        &lists,
        "inputs",
        list(&mut language.inputs.iter().map(|doc| doc.name)),
    );
    lists.into()
}

/// A byte offset in `code` as a CodeMirror position (UTF-16 units).
fn position_of(code: &str, byte: usize) -> JsValue {
    (assist::byte_to_utf16(code, byte) as u32).into()
}

fn set(object: &Object, key: &str, value: JsValue) {
    let _ = Reflect::set(object, &key.into(), &value);
}
