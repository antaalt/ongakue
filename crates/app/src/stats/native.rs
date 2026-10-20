//! Natively, the stats go in the window's title.

use winit::window::Window;

use super::{FrameReport, Target};

pub struct Stats;

impl Stats {
    pub fn new() -> Result<Self, String> {
        Ok(Self)
    }

    /// Natively, the surface must match the window: always drawn at its size.
    pub fn scale(&self) -> f32 {
        1.0
    }

    pub fn show(&self, report: &FrameReport, target: &Target, window: &Window) {
        window.set_title(&format!(
            "Ongakue · {:.0} fps, {:.1} ms (worst {:.1}) · {} × {}{} · {} · {}",
            report.fps,
            report.frame_ms,
            report.worst_ms,
            target.width,
            target.height,
            if target.scale < 1.0 {
                format!(" ({:.0} %)", target.scale * 100.0)
            } else {
                String::new()
            },
            if target.gpu.is_empty() {
                "unknown GPU"
            } else {
                target.gpu
            },
            target.backend,
        ));
    }
}
