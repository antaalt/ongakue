//! The stats panel, shown with the "Stats" button, with the resolution
//! slider. Its elements are defined in `index.html`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use js_sys::{Function, Promise, Reflect};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{HtmlButtonElement, HtmlElement, HtmlInputElement, Storage};
use winit::window::Window;

use super::{FrameReport, Target};
use crate::panel::show_panel;

/// Storage key of whether the panel is shown: "on" or "off".
const VISIBLE_KEY: &str = "ongakue.stats";
/// Storage key of the resolution, in percent of the window's.
const SCALE_KEY: &str = "ongakue.resolution";

pub struct Stats {
    text: HtmlElement,
    visible: Rc<Cell<bool>>,
    /// The size drawn at, from the window's: 0.25 to 2 (above 1, the
    /// browser shrinks the result to the window: supersampling).
    scale: Rc<Cell<f32>>,
    /// The GPU as WebGPU names it, when wgpu doesn't: e.g. "nvidia ampere".
    webgpu_name: Rc<RefCell<String>>,
}

impl Stats {
    pub fn new() -> Result<Self, JsValue> {
        let window = web_sys::window().unwrap();
        let document = window.document().unwrap();
        let panel: HtmlElement = element(&document, "stats")?;
        let text: HtmlElement = element(&document, "stats-text")?;
        let button: HtmlButtonElement = element(&document, "stats-toggle")?;
        let storage = window.local_storage().ok().flatten();

        // The resolution slider, in percent.
        let slider: HtmlInputElement = element(&document, "stats-resolution")?;
        let slider_value: HtmlElement = element(&document, "stats-resolution-value")?;
        let percent = storage
            .as_ref()
            .and_then(|storage| storage.get_item(SCALE_KEY).ok().flatten())
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap_or(100.0)
            .clamp(25.0, 200.0);
        slider.set_value(&percent.to_string());
        slider_value.set_text_content(Some(&format!("{percent:.0} %")));
        let scale = Rc::new(Cell::new(percent / 100.0));
        let on_input = {
            let (slider, scale, storage) = (slider.clone(), scale.clone(), storage.clone());
            Closure::<dyn FnMut()>::new(move || {
                let percent = slider.value_as_number() as f32;
                scale.set(percent / 100.0);
                slider_value.set_text_content(Some(&format!("{percent:.0} %")));
                if let Some(storage) = &storage {
                    let _ = storage.set_item(SCALE_KEY, &percent.to_string());
                }
            })
        };
        slider.set_oninput(Some(on_input.as_ref().unchecked_ref()));
        on_input.forget();
        let visible = Rc::new(Cell::new(
            storage
                .as_ref()
                .and_then(|storage| storage.get_item(VISIBLE_KEY).ok().flatten())
                .as_deref()
                == Some("on"),
        ));
        show_panel(&panel, &button, visible.get());

        let on_click = {
            let (panel, button, visible) = (panel.clone(), button.clone(), visible.clone());
            Closure::<dyn FnMut()>::new(move || {
                visible.set(!visible.get());
                show_panel(&panel, &button, visible.get());
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
            text,
            visible,
            scale,
            webgpu_name,
        })
    }

    /// The size to draw at, from the window's: 0.25 to 2.
    pub fn scale(&self) -> f32 {
        self.scale.get()
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
        self.text.set_text_content(Some(&format!(
            "{:.0} fps · {:.1} ms (worst {:.1})\n\
             {}\n\
             CPU: analysis {:.2} ms · draw {:.2} ms\n\
             {} × {} ({:.0} % of the window)\n\
             {gpu} · {}",
            report.fps,
            report.frame_ms,
            report.worst_ms,
            super::gpu_time(report, target),
            report.analysis_ms,
            report.draw_ms,
            target.width,
            target.height,
            target.scale * 100.0,
            target.backend,
        )));
    }
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
