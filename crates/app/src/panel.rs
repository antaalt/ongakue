//! The panels opened by the buttons of the controls bar (Tune, Edit, Stats):
//! a panel is shown with its `open` class, and its button is lit meanwhile.

use web_sys::HtmlElement;

/// Opens or closes `panel`, lighting `button` while it's open.
pub fn show_panel(panel: &HtmlElement, button: &HtmlElement, open: bool) {
    let _ = panel.class_list().toggle_with_force("open", open);
    let _ = button.class_list().toggle_with_force("on", open);
    let _ = button.set_attribute("aria-pressed", if open { "true" } else { "false" });
}

/// Whether `panel` is open.
pub fn is_open(panel: &HtmlElement) -> bool {
    panel.class_list().contains("open")
}
