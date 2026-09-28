//! The completion list and hover tooltip over the editor's textarea. What
//! they contain comes from `assist.rs`; this places and drives them.
//!
//! The code uses a monospace font without line wrapping, so a (line, column)
//! position maps to pixels with a character width and a line height.

use std::cell::{Cell, RefCell};
use std::fmt::Write;
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use web_sys::{
    CanvasRenderingContext2d, Element, Event, HtmlCanvasElement, HtmlDocument, HtmlElement,
    HtmlTextAreaElement, KeyboardEvent, MouseEvent,
};

use super::assist::{self, Suggestion, escape_html};

/// Must match the padding of the editor's CSS.
const PADDING: f64 = 8.0;

struct Open {
    /// Where the word being completed starts, in bytes.
    start: usize,
    suggestions: Vec<Suggestion>,
    selected: usize,
}

pub struct Popups {
    code: HtmlTextAreaElement,
    list: HtmlElement,
    tooltip: HtmlElement,
    open: RefCell<Option<Open>>,
    /// Set while inserting an accepted suggestion, so the edit doesn't reopen
    /// the list.
    accepting: Cell<bool>,
    /// Start of the word the tooltip is showing.
    hovered: Cell<Option<usize>>,
    /// Character width and line height, in pixels, measured once.
    metrics: Cell<Option<(f64, f64)>>,
}

impl Popups {
    pub fn new(code: HtmlTextAreaElement, list: HtmlElement, tooltip: HtmlElement) -> Rc<Self> {
        let popups = Rc::new(Self {
            code,
            list,
            tooltip,
            open: RefCell::new(None),
            accepting: Cell::new(false),
            hovered: Cell::new(None),
            metrics: Cell::new(None),
        });

        // Clicking a suggestion accepts it. On mousedown, before the textarea
        // loses the focus.
        let on_mousedown = {
            let popups = popups.clone();
            Closure::<dyn FnMut(MouseEvent)>::new(move |event: MouseEvent| {
                event.prevent_default();
                let item = event
                    .target()
                    .and_then(|target| target.dyn_into::<Element>().ok())
                    .and_then(|target| target.closest("li").ok().flatten());
                let index = item
                    .and_then(|item| item.get_attribute("data-index"))
                    .and_then(|index| index.parse().ok());
                if let Some(index) = index {
                    popups.accept(index);
                }
            })
        };
        popups
            .list
            .set_onmousedown(Some(on_mousedown.as_ref().unchecked_ref()));
        on_mousedown.forget();
        popups
    }

    /// Shows, updates or hides the suggestions for the word at the cursor.
    /// Call after each edit. `typed` is whether text was typed: other edits
    /// (deleting, undo, paste…) only update a list that is already open.
    pub fn update(&self, typed: bool) {
        self.hide_tooltip();
        if self.accepting.replace(false) || !(typed || self.open.borrow().is_some()) {
            self.hide();
            return;
        }
        let source = self.code.value();
        let (Ok(Some(start)), Ok(Some(end))) =
            (self.code.selection_start(), self.code.selection_end())
        else {
            return self.hide();
        };
        if start != end {
            return self.hide();
        }
        let cursor = assist::utf16_to_byte(&source, start as usize);
        match assist::complete(&source, cursor) {
            Some(completion) => {
                *self.open.borrow_mut() = Some(Open {
                    start: completion.start,
                    suggestions: completion.suggestions,
                    selected: 0,
                });
                self.render(&source);
            }
            None => self.hide(),
        }
    }

    /// Handles a key while the list is open. Returns whether it was used.
    pub fn handle_key(&self, event: &KeyboardEvent) -> bool {
        let mut open = self.open.borrow_mut();
        let Some(state) = open.as_mut() else {
            return false;
        };
        let count = state.suggestions.len();
        match event.key().as_str() {
            "ArrowDown" => state.selected = (state.selected + 1) % count,
            "ArrowUp" => state.selected = (state.selected + count - 1) % count,
            "Enter" | "Tab" => {
                let selected = state.selected;
                drop(open);
                event.prevent_default();
                self.accept(selected);
                return true;
            }
            "Escape" => {
                drop(open);
                event.prevent_default();
                self.hide();
                return true;
            }
            // The cursor moves away from the word.
            "ArrowLeft" | "ArrowRight" | "Home" | "End" | "PageUp" | "PageDown" => {
                drop(open);
                self.hide();
                return false;
            }
            _ => return false,
        }
        drop(open);
        event.prevent_default();
        self.render(&self.code.value());
        true
    }

    pub fn hide(&self) {
        self.open.borrow_mut().take();
        self.list.set_hidden(true);
    }

    /// Shows the docs of the word under the mouse, at (x, y) in the textarea.
    pub fn hover_at(&self, x: f64, y: f64) {
        if self.open.borrow().is_some() {
            return;
        }
        let (char_width, line_height) = self.metrics();
        let line = (y - PADDING + self.code.scroll_top() as f64) / line_height;
        let column = (x - PADDING + self.code.scroll_left() as f64) / char_width;
        let source = self.code.value();
        let hover = (line >= 0.0 && column >= 0.0)
            .then(|| assist::offset_at(&source, line as usize, column as usize))
            .flatten()
            .and_then(|offset| assist::hover(&source, offset));
        let Some(hover) = hover else {
            return self.hide_tooltip();
        };
        if self.hovered.replace(Some(hover.start)) == Some(hover.start) {
            return;
        }
        self.tooltip.set_inner_html(&format!(
            "<div class=\"title\">{}</div><div class=\"doc\">{}</div>",
            escape_html(&hover.title),
            escape_html(&hover.doc)
        ));
        let (line, column) = assist::position(&source, hover.start);
        self.place(&self.tooltip, line, column);
    }

    pub fn hide_tooltip(&self) {
        self.hovered.set(None);
        self.tooltip.set_hidden(true);
    }

    fn accept(&self, index: usize) {
        let Some(state) = self.open.borrow_mut().take() else {
            return;
        };
        let Some(suggestion) = state.suggestions.get(index) else {
            return;
        };
        let source = self.code.value();
        let start = assist::byte_to_utf16(&source, state.start) as u32;
        let cursor = self.code.selection_start().ok().flatten().unwrap_or(start);
        // Select the start of the word, and type the suggestion over it.
        let _ = self.code.set_selection_range(start, cursor);
        self.accepting.set(true);
        insert_text(&self.code, &suggestion.label);
        self.hide();
    }

    fn render(&self, source: &str) {
        let open = self.open.borrow();
        let Some(state) = open.as_ref() else {
            return;
        };
        let mut html = String::from("<ul>");
        for (i, suggestion) in state.suggestions.iter().enumerate() {
            let _ = write!(
                html,
                "<li data-index=\"{i}\"{}><span class=\"label {}\">{}</span><span class=\"detail\">{}</span></li>",
                if i == state.selected {
                    " class=\"selected\""
                } else {
                    ""
                },
                suggestion.kind.class(),
                escape_html(&suggestion.label),
                escape_html(&suggestion.detail),
            );
        }
        html.push_str("</ul>");
        let doc = &state.suggestions[state.selected].doc;
        if !doc.is_empty() {
            let _ = write!(html, "<div class=\"doc\">{}</div>", escape_html(doc));
        }
        self.list.set_inner_html(&html);
        let (line, column) = assist::position(source, state.start);
        self.place(&self.list, line, column);
    }

    /// Shows a popup under the text at (line, column), or above it if there's
    /// no room below.
    fn place(&self, popup: &HtmlElement, line: usize, column: usize) {
        let (char_width, line_height) = self.metrics();
        popup.set_hidden(false);
        // The popups are placed in the editor's body, where the code starts
        // after the line numbers.
        let x = self.code.offset_left() as f64 + PADDING + column as f64 * char_width
            - self.code.scroll_left() as f64;
        let below = PADDING + (line + 1) as f64 * line_height - self.code.scroll_top() as f64;
        let (width, height) = (popup.offset_width() as f64, popup.offset_height() as f64);
        let (area_width, area_height) = (
            self.code.client_width() as f64,
            self.code.client_height() as f64,
        );
        let above = below - line_height - height;
        let top = if below + height > area_height && above >= 0.0 {
            above
        } else {
            below
        };
        let left = x
            .min(self.code.offset_left() as f64 + area_width - width)
            .max(0.0);
        let style = popup.style();
        let _ = style.set_property("left", &format!("{left}px"));
        let _ = style.set_property("top", &format!("{top}px"));
    }

    fn metrics(&self) -> (f64, f64) {
        if let Some(metrics) = self.metrics.get() {
            return metrics;
        }
        let measured = measure(&self.code).unwrap_or((7.8, 18.85));
        self.metrics.set(Some(measured));
        measured
    }
}

/// Types `text` over the textarea's selection, as if typed: keeps undo
/// (Ctrl+Z) working, and fires the `input` event.
pub fn insert_text(code: &HtmlTextAreaElement, text: &str) {
    let document: HtmlDocument = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .unchecked_into();
    let inserted = document
        .exec_command_with_show_ui_and_value("insertText", false, text)
        .unwrap_or(false);
    if inserted {
        return;
    }
    // The browser refused (e.g. its window isn't focused): insert directly.
    // Undo won't cover this edit, but it isn't lost.
    let start = code.selection_start().ok().flatten().unwrap_or(0);
    let end = code.selection_end().ok().flatten().unwrap_or(start);
    let _ = code.set_range_text_with_start_and_end_and_mode(text, start, end, "end");
    if let Ok(event) = Event::new("input") {
        let _ = code.dispatch_event(&event);
    }
}

/// Character width and line height of the textarea's font, in pixels.
fn measure(code: &HtmlTextAreaElement) -> Option<(f64, f64)> {
    let window = web_sys::window()?;
    let style = window.get_computed_style(code).ok()??;
    let property = |name| style.get_property_value(name).ok();
    let font = format!("{} {}", property("font-size")?, property("font-family")?);
    let line_height = property("line-height")?
        .trim_end_matches("px")
        .parse()
        .ok()?;

    let canvas: HtmlCanvasElement = window
        .document()?
        .create_element("canvas")
        .ok()?
        .dyn_into()
        .ok()?;
    let context: CanvasRenderingContext2d = canvas.get_context("2d").ok()??.dyn_into().ok()?;
    context.set_font(&font);
    let sample = "0".repeat(100);
    let width = context.measure_text(&sample).ok()?.width() / 100.0;
    Some((width, line_height))
}
