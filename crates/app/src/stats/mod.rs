//! Frame rate and timings, with the GPU in use. Each platform exposes the
//! same `Stats` interface to show them; the measuring is shared.

mod meter;

pub use meter::{FrameMeter, FrameReport};

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(not(target_arch = "wasm32"))]
pub use native::*;
#[cfg(target_arch = "wasm32")]
pub use web::*;

/// What's drawn: the canvas size and the GPU, e.g.
/// "1920 × 1080 · NVIDIA GeForce GTX 960M · WebGL2".
pub struct Target<'a> {
    pub width: u32,
    pub height: u32,
    /// The size drawn at, from the window's: see [`scaled`].
    pub scale: f32,
    /// Empty when the backend doesn't tell.
    pub gpu: &'a str,
    pub backend: &'a str,
    /// Whether the GPU's time per frame is measured (not on WebGL2).
    pub measures_gpu_time: bool,
}

/// The GPU's time per frame, e.g. "GPU 3.21 ms" or "GPU 12 µs" (fast GPUs
/// draw simple visuals in microseconds), or why it's unknown.
pub fn gpu_time(report: &FrameReport, target: &Target) -> String {
    match report.gpu_ms {
        Some(ms) if ms < 1.0 => format!("GPU {:.0} µs", ms * 1000.0),
        Some(ms) => format!("GPU {ms:.2} ms"),
        None if target.measures_gpu_time => "GPU …".to_owned(),
        None => format!("GPU time not measurable on {}", target.backend),
    }
}

/// The size to draw at, scaled from the window's, at least a pixel. The GPU
/// runs the visual for each pixel, so at half the size it draws a quarter of
/// them, and the browser stretches the canvas back to the window. Above 1,
/// the browser shrinks it instead: smoother edges, for more pixels to draw.
pub fn scaled(width: u32, height: u32, scale: f32) -> (u32, u32) {
    let scale = |size: u32| ((size as f32 * scale).round() as u32).max(1);
    (scale(width), scale(height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scales_the_size() {
        assert_eq!(scaled(1920, 1080, 1.0), (1920, 1080));
        assert_eq!(scaled(1920, 1080, 0.5), (960, 540));
        assert_eq!(scaled(1583, 805, 0.25), (396, 201));
        // Supersampling.
        assert_eq!(scaled(1920, 1080, 2.0), (3840, 2160));
        // Never empty, even for a collapsed window.
        assert_eq!(scaled(0, 1, 0.25), (1, 1));
    }

    #[test]
    fn gpu_time_in_readable_units() {
        let target = |measures_gpu_time| Target {
            width: 800,
            height: 600,
            scale: 1.0,
            gpu: "",
            backend: "WebGL2",
            measures_gpu_time,
        };
        let report = |gpu_ms| FrameReport {
            fps: 60.0,
            frame_ms: 16.7,
            worst_ms: 17.0,
            analysis_ms: 0.2,
            draw_ms: 0.2,
            gpu_ms,
        };
        assert_eq!(
            gpu_time(&report(Some(12.345)), &target(true)),
            "GPU 12.35 ms"
        );
        assert_eq!(gpu_time(&report(Some(0.0123)), &target(true)), "GPU 12 µs");
        assert_eq!(gpu_time(&report(None), &target(true)), "GPU …");
        assert_eq!(
            gpu_time(&report(None), &target(false)),
            "GPU time not measurable on WebGL2"
        );
    }
}
