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
    /// Empty when the backend doesn't tell.
    pub gpu: &'a str,
    pub backend: &'a str,
}
