//! Audio playback. Each backend exposes the same interface: a `Backend` type
//! with `latest_samples`, and the `SAMPLE_COUNT` it fills.

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(not(target_arch = "wasm32"))]
pub use native::*;
#[cfg(target_arch = "wasm32")]
pub use web::*;
