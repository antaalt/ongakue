//! MIDI input: notes and controllers (knobs, faders). Each platform exposes
//! the same `Midi` interface.

mod state;

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

pub use state::MIDI_COUNT;

#[cfg(not(target_arch = "wasm32"))]
pub use native::*;
#[cfg(target_arch = "wasm32")]
pub use web::*;
