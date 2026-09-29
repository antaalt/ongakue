//! Tuning panel: sliders for the analysis settings and the shaders' free
//! parameters, and a live plot of the beat detector. The panel's container
//! elements are defined in `index.html`; the sliders are built here.
//!
//! Slider values are shared with the app, which reads them every frame, so
//! changes apply on the next frame.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use analysis::Settings;
use wasm_bindgen::prelude::*;
use web_sys::{
    CanvasRenderingContext2d, Document, HtmlButtonElement, HtmlCanvasElement, HtmlElement,
    HtmlInputElement,
};

/// Seconds of beat detector history shown in the plot.
const HISTORY: f32 = 5.0;

#[derive(Clone, Copy, Default)]
struct Values {
    settings: Settings,
    params: [f32; 4],
}

struct Slider {
    label: &'static str,
    min: f32,
    max: f32,
    step: f32,
    unit: &'static str,
    value: fn(&mut Values) -> &mut f32,
}

// One slider per line reads better as a table.
#[rustfmt::skip]
const SECTIONS: &[(&str, &[Slider])] = &[
    ("Shader parameters", &[
        Slider { label: "u.params.x", min: 0.0, max: 1.0, step: 0.01, unit: "", value: |v| &mut v.params[0] },
        Slider { label: "u.params.y", min: 0.0, max: 1.0, step: 0.01, unit: "", value: |v| &mut v.params[1] },
        Slider { label: "u.params.z", min: 0.0, max: 1.0, step: 0.01, unit: "", value: |v| &mut v.params[2] },
        Slider { label: "u.params.w", min: 0.0, max: 1.0, step: 0.01, unit: "", value: |v| &mut v.params[3] },
    ]),
    ("Spectrum", &[
        Slider { label: "Quietest level", min: -100.0, max: -30.0, step: 1.0, unit: " dB", value: |v| &mut v.settings.min_db },
        Slider { label: "Loudest level", min: -40.0, max: 0.0, step: 1.0, unit: " dB", value: |v| &mut v.settings.max_db },
        Slider { label: "Treble boost", min: 0.0, max: 6.0, step: 0.1, unit: " dB/oct", value: |v| &mut v.settings.tilt_db_per_octave },
        Slider { label: "Band fall time", min: 0.02, max: 0.5, step: 0.01, unit: " s", value: |v| &mut v.settings.band_half_life },
    ]),
    ("Beat detection", &[
        Slider { label: "Sensitivity", min: 0.5, max: 4.0, step: 0.1, unit: " σ", value: |v| &mut v.settings.beat_sensitivity },
        Slider { label: "Minimum rise", min: 0.0, max: 1.0, step: 0.01, unit: "", value: |v| &mut v.settings.beat_min_flux },
        Slider { label: "Minimum interval", min: 0.1, max: 0.6, step: 0.01, unit: " s", value: |v| &mut v.settings.beat_min_interval },
    ]),
];

struct Plot {
    canvas: HtmlCanvasElement,
    context: CanvasRenderingContext2d,
    history: VecDeque<Entry>,
}

/// One frame of beat detector state.
struct Entry {
    /// Seconds since start.
    time: f32,
    /// How much the bass rose.
    flux: f32,
    threshold: f32,
    beat: bool,
}

pub struct Tuning {
    panel: HtmlElement,
    values: Rc<RefCell<Values>>,
    plot: RefCell<Plot>,
}

impl Tuning {
    pub fn new() -> Result<Self, JsValue> {
        let document = web_sys::window().unwrap().document().unwrap();
        let panel: HtmlElement = element(&document, "tuning")?;
        let container: HtmlElement = element(&document, "tuning-sliders")?;
        let values = Rc::new(RefCell::new(Values::default()));

        // Each slider with the element showing its value, to reset them.
        let mut inputs = Vec::new();
        for (title, sliders) in SECTIONS {
            let heading = document.create_element("h3")?;
            heading.set_text_content(Some(title));
            container.append_child(&heading)?;

            for slider in *sliders {
                let row = document.create_element("label")?;
                row.set_class_name("slider");
                let label = document.create_element("span")?;
                label.set_text_content(Some(slider.label));
                let input: HtmlInputElement = document.create_element("input")?.dyn_into()?;
                input.set_type("range");
                input.set_min(&slider.min.to_string());
                input.set_max(&slider.max.to_string());
                input.set_step(&slider.step.to_string());
                let output: HtmlElement = document.create_element("output")?.dyn_into()?;
                row.append_child(&label)?;
                row.append_child(&input)?;
                row.append_child(&output)?;
                container.append_child(&row)?;

                let value = *(slider.value)(&mut values.borrow_mut());
                show_value(slider, &input, &output, value);

                let on_input = {
                    let (values, input, output) = (values.clone(), input.clone(), output.clone());
                    Closure::<dyn FnMut()>::new(move || {
                        let value = input.value().parse().unwrap_or(0.0);
                        *(slider.value)(&mut values.borrow_mut()) = value;
                        output.set_text_content(Some(&format_value(slider, value)));
                    })
                };
                input.set_oninput(Some(on_input.as_ref().unchecked_ref()));
                on_input.forget();
                inputs.push((slider, input, output));
            }
        }

        on_click(&document, "tuning-reset", {
            let values = values.clone();
            move || {
                let mut values = values.borrow_mut();
                *values = Values::default();
                for (slider, input, output) in &inputs {
                    let value = *(slider.value)(&mut values);
                    show_value(slider, input, output, value);
                }
            }
        })?;
        on_click(&document, "tuning-close", {
            let panel = panel.clone();
            move || {
                let panel = panel.clone();
                let _ = panel.class_list().remove_1("open");
            }
        })?;
        on_click(&document, "tune", {
            let panel = panel.clone();
            move || {
                let _ = panel.class_list().toggle("open");
            }
        })?;

        let canvas: HtmlCanvasElement = element(&document, "tuning-plot")?;
        let context = canvas
            .get_context("2d")?
            .ok_or("no 2D canvas context")?
            .dyn_into()?;
        Ok(Self {
            panel,
            values,
            plot: RefCell::new(Plot {
                canvas,
                context,
                history: VecDeque::new(),
            }),
        })
    }

    pub fn settings(&self) -> Settings {
        self.values.borrow().settings
    }

    pub fn params(&self) -> [f32; 4] {
        self.values.borrow().params
    }

    /// Adds a frame of beat detector state to the plot. `time` is in seconds.
    pub fn record(&self, time: f32, flux: f32, threshold: f32, beat: bool) {
        let mut plot = self.plot.borrow_mut();
        while plot
            .history
            .front()
            .is_some_and(|entry| entry.time < time - HISTORY)
        {
            plot.history.pop_front();
        }
        plot.history.push_back(Entry {
            time,
            flux,
            threshold,
            beat,
        });
        // Recorded while hidden too, so the plot has history when opened.
        if self.panel.class_list().contains("open") {
            plot.draw();
        }
    }
}

impl Plot {
    fn draw(&self) {
        let (width, height) = (self.canvas.width() as f64, self.canvas.height() as f64);
        let context = &self.context;
        context.clear_rect(0.0, 0.0, width, height);

        let Some(now) = self.history.back().map(|entry| entry.time) else {
            return;
        };
        let peak = self
            .history
            .iter()
            .map(|entry| entry.flux.max(entry.threshold))
            .fold(0.05, f32::max) as f64
            * 1.1;
        // The newest frame on the right edge.
        let x = |time: f32| (1.0 - ((now - time) / HISTORY) as f64) * width;
        let y = |value: f32| height - value as f64 / peak * height;

        context.set_fill_style_str("rgba(255, 170, 60, 0.6)");
        for entry in self.history.iter().filter(|entry| entry.beat) {
            context.fill_rect(x(entry.time) - 1.0, 0.0, 2.0, height);
        }
        let line = |color: &str, value: fn(&Entry) -> f32| {
            context.set_stroke_style_str(color);
            context.begin_path();
            for (i, entry) in self.history.iter().enumerate() {
                let (px, py) = (x(entry.time), y(value(entry)));
                if i == 0 {
                    context.move_to(px, py);
                } else {
                    context.line_to(px, py);
                }
            }
            context.stroke();
        };
        line("#ff6b6b", |entry| entry.threshold);
        line("#8be9fd", |entry| entry.flux);
    }
}

fn show_value(slider: &Slider, input: &HtmlInputElement, output: &HtmlElement, value: f32) {
    input.set_value(&value.to_string());
    output.set_text_content(Some(&format_value(slider, value)));
}

fn format_value(slider: &Slider, value: f32) -> String {
    let decimals = if slider.step >= 1.0 {
        0
    } else if slider.step >= 0.1 {
        1
    } else if slider.step >= 0.01 {
        2
    } else {
        3
    };
    format!("{value:.decimals$}{}", slider.unit)
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
