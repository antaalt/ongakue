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
}

/// The size to draw at: a fraction of the window's, at least a pixel. The
/// GPU runs the visual for each pixel, so at half the size it draws a
/// quarter of them; the browser stretches the canvas back to the window.
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
        // Never empty, even for a collapsed window.
        assert_eq!(scaled(0, 1, 0.25), (1, 1));
    }
}
