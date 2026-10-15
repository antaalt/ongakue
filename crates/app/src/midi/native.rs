//! No MIDI input natively (yet): every value stays at 0.

use super::state::{MidiFrame, MidiState};

pub struct Midi {
    state: MidiState,
}

impl Midi {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            state: MidiState::default(),
        })
    }

    pub fn frame(&mut self, dt: f32) -> MidiFrame {
        self.state.frame(dt)
    }

    /// Feeds a MIDI message from another input, like the keyboard piano.
    pub fn handle(&mut self, message: &[u8]) {
        self.state.handle(message);
    }
}
