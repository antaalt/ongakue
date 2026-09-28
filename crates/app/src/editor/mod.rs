//! Shader editor. Each platform exposes the same `Editor` interface.

// Only the web editor uses these; compiled everywhere so their tests run natively.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
mod assist;
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
mod highlight;
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
mod wgsl;

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod popups;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(not(target_arch = "wasm32"))]
pub use native::*;
#[cfg(target_arch = "wasm32")]
pub use web::*;
