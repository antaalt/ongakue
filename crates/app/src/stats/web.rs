//! The stats panel, shown with the "Stats" button. Its elements are defined
//! in `index.html`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use js_sys::{Function, Promise, Reflect};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{HtmlButtonElement, HtmlElement, Storage};
use winit::window::Window;

use super::{FrameReport, Target};

/// Storage key of whether the panel is shown: "on" or "off".
const VISIBLE_KEY: &str = "ongakue.stats";

pub struct Stats {
    panel: HtmlElement,
    visible: Rc<Cell<bool>>,
    /// The GPU as WebGPU names it, when wgpu doesn't: e.g. "nvidia ampere".
    webgpu_name: Rc<RefCell<String>>,
}

impl Stats {
    pub fn new() -> Result<Self, JsValue> {
        let window = web_sys::window().unwrap();
        let document = window.document().unwrap();
        let panel: HtmlElement = element(&document, "stats")?;
        let button: HtmlButtonElement = element(&document, "stats-toggle")?;
        let storage = window.local_storage().ok().flatten();
        let visible = Rc::new(Cell::new(
            storage
                .as_ref()
                .and_then(|storage| storage.get_item(VISIBLE_KEY).ok().flatten())
                .as_deref()
                == Some("on"),
        ));
        show_visible(&panel, &button, visible.get());

        let on_click = {
            let (panel, button, visible) = (panel.clone(), button.clone(), visible.clone());
            Closure::<dyn FnMut()>::new(move || {
                visible.set(!visible.get());
                show_visible(&panel, &button, visible.get());
                save(storage.as_ref(), visible.get());
            })
        };
        button.set_onclick(Some(on_click.as_ref().unchecked_ref()));
        on_click.forget();

        let webgpu_name = Rc::new(RefCell::new(String::new()));
        wasm_bindgen_futures::spawn_local({
            let webgpu_name = webgpu_name.clone();
            async move {
                if let Some(name) = webgpu_adapter_name().await {
                    *webgpu_name.borrow_mut() = name;
                }
            }
        });
        Ok(Self {
            panel,
            visible,
            webgpu_name,
        })
    }

    pub fn show(&self, report: &FrameReport, target: &Target, _window: &Window) {
        if !self.visible.get() {
            return;
        }
        let webgpu_name = self.webgpu_name.borrow();
        let gpu = match (target.gpu, webgpu_name.as_str()) {
            ("", "") => "unknown GPU",
            ("", name) => name,
            (name, _) => name,
        };
        self.panel.set_text_content(Some(&format!(
            "{:.0} fps · {:.1} ms (worst {:.1})\n\
             analysis {:.2} ms · draw {:.2} ms\n\
             {} × {}\n\
             {gpu} · {}",
            report.fps,
            report.frame_ms,
            report.worst_ms,
            report.analysis_ms,
            report.draw_ms,
            target.width,
            target.height,
            target.backend,
        )));
    }
}

fn show_visible(panel: &HtmlElement, button: &HtmlButtonElement, visible: bool) {
    panel.set_hidden(!visible);
    let _ = button.class_list().toggle_with_force("on", visible);
    let _ = button.set_attribute("aria-pressed", if visible { "true" } else { "false" });
}

fn save(storage: Option<&Storage>, visible: bool) {
    if let Some(storage) = storage {
        let _ = storage.set_item(VISIBLE_KEY, if visible { "on" } else { "off" });
    }
}

/// The GPU's vendor and architecture as WebGPU reports them (browsers don't
/// tell more), e.g. "nvidia ampere", if the browser supports WebGPU.
async fn webgpu_adapter_name() -> Option<String> {
    let navigator = web_sys::window()?.navigator();
    let gpu = Reflect::get(&navigator, &"gpu".into()).ok()?;
    if gpu.is_undefined() {
        return None;
    }
    let request: Function = Reflect::get(&gpu, &"requestAdapter".into())
        .ok()?
        .dyn_into()
        .ok()?;
    let options = js_sys::Object::new();
    Reflect::set(
        &options,
        &"powerPreference".into(),
        &"high-performance".into(),
    )
    .ok()?;
    let promise: Promise = request.call1(&gpu, &options).ok()?.dyn_into().ok()?;
    let adapter = JsFuture::from(promise).await.ok()?;
    if adapter.is_null() || adapter.is_undefined() {
        return None;
    }
    let info = Reflect::get(&adapter, &"info".into()).ok()?;
    let field = |name: &str| {
        Reflect::get(&info, &name.into())
            .ok()
            .and_then(|value| value.as_string())
            .unwrap_or_default()
    };
    let description = field("description");
    let name = if description.is_empty() {
        format!("{} {}", field("vendor"), field("architecture"))
    } else {
        description
    };
    Some(name.trim().to_owned()).filter(|name| !name.is_empty())
}

fn element<T: JsCast>(document: &web_sys::Document, id: &str) -> Result<T, JsValue> {
    document
        .get_element_by_id(id)
        .ok_or_else(|| JsValue::from_str(&format!("missing #{id} element")))?
        .dyn_into()
        .map_err(|_| JsValue::from_str(&format!("#{id} has the wrong element type")))
}
