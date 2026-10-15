//! The computer keyboard as a piano, natively: from the window's key events.
//! Silent for now (no audio output natively), but notes reach the visuals.

use winit::event::{ElementState, WindowEvent};
use winit::keyboard::PhysicalKey;

use super::keys::KeyPiano;

#[derive(Default)]
pub struct Piano {
    keys: KeyPiano,
}

impl Piano {
    pub fn new() -> Self {
        Self::default()
    }

    /// The MIDI messages a window event plays, if any.
    pub fn window_event(&mut self, event: &WindowEvent) -> Vec<[u8; 3]> {
        match event {
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return Vec::new();
                };
                // winit's key names match the web's (`KeyA`, `Semicolon`…).
                let code = format!("{code:?}");
                let message = match event.state {
                    ElementState::Pressed if !event.repeat => self.keys.key_down(&code),
                    ElementState::Released => self.keys.key_up(&code),
                    ElementState::Pressed => None,
                };
                message.into_iter().collect()
            }
            // Key-ups never arrive once the window has lost the focus.
            WindowEvent::Focused(false) => self.keys.release_all(),
            _ => Vec::new(),
        }
    }
}
