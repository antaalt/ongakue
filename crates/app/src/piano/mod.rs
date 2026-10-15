//! The computer keyboard as a piano, playing MIDI notes. Each platform exposes
//! the same `Piano` interface; the key mapping is shared.

mod keys;

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(not(target_arch = "wasm32"))]
pub use native::*;
#[cfg(target_arch = "wasm32")]
pub use web::*;
