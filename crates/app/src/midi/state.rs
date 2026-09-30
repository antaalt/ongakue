//! What the app keeps from MIDI messages: notes and controllers. Pure logic,
//! shared by every platform and tested natively.

/// Number of MIDI notes and controllers.
pub const MIDI_COUNT: usize = 128;

/// Seconds for a released note to fall to half its velocity.
const RELEASE_HALF_LIFE: f32 = 0.15;

/// The MIDI values for one frame.
pub struct MidiFrame {
    /// Per note (60 = middle C): its velocity (0..1) while held, then fading
    /// out after release.
    pub notes: [f32; MIDI_COUNT],
    /// Per controller (knobs, faders): its position, 0..1.
    pub controls: [f32; MIDI_COUNT],
    /// The last controller moved since the previous frame, if any.
    pub moved: Option<u8>,
}

pub struct MidiState {
    /// Velocity of each held note, 0 when released.
    held: [f32; MIDI_COUNT],
    notes: [f32; MIDI_COUNT],
    controls: [f32; MIDI_COUNT],
    moved: Option<u8>,
}

impl Default for MidiState {
    fn default() -> Self {
        Self {
            held: [0.0; MIDI_COUNT],
            notes: [0.0; MIDI_COUNT],
            controls: [0.0; MIDI_COUNT],
            moved: None,
        }
    }
}

impl MidiState {
    /// Applies one MIDI message (status byte, then data bytes). Messages from
    /// every channel are merged; the ones the app doesn't use are ignored.
    // No MIDI input natively yet.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn handle(&mut self, message: &[u8]) {
        let &[status, first, second, ..] = message else {
            return;
        };
        let (index, value) = (usize::from(first & 0x7F), f32::from(second & 0x7F) / 127.0);
        match status & 0xF0 {
            // Note on. A velocity of 0 means note off.
            0x90 if value > 0.0 => {
                self.held[index] = value;
                self.notes[index] = value;
            }
            0x80 | 0x90 => self.held[index] = 0.0,
            // Control change.
            0xB0 => {
                self.controls[index] = value;
                self.moved = Some(first & 0x7F);
            }
            _ => {}
        }
    }

    /// Advances released notes' fading by `dt` seconds and returns the values
    /// for this frame.
    pub fn frame(&mut self, dt: f32) -> MidiFrame {
        let decay = 0.5f32.powf(dt / RELEASE_HALF_LIFE);
        for (note, &held) in self.notes.iter_mut().zip(&self.held) {
            *note = if held > 0.0 { held } else { *note * decay };
        }
        MidiFrame {
            notes: self.notes,
            controls: self.controls,
            moved: self.moved.take(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: f32 = 1.0 / 60.0;

    #[test]
    fn notes_follow_velocity_while_held() {
        let mut midi = MidiState::default();
        midi.handle(&[0x90, 60, 127]);
        midi.handle(&[0x91, 64, 64]); // Another channel.
        let frame = midi.frame(FRAME);
        assert_eq!(frame.notes[60], 1.0);
        assert!((frame.notes[64] - 64.0 / 127.0).abs() < 1e-6);
        // Held notes don't fade.
        for _ in 0..100 {
            midi.frame(FRAME);
        }
        assert_eq!(midi.frame(FRAME).notes[60], 1.0);
    }

    #[test]
    fn released_notes_fade_out() {
        for note_off in [[0x80, 60, 0], [0x90, 60, 0]] {
            let mut midi = MidiState::default();
            midi.handle(&[0x90, 60, 127]);
            midi.handle(&note_off);
            let after_release = midi.frame(RELEASE_HALF_LIFE).notes[60];
            assert!((after_release - 0.5).abs() < 1e-3, "{after_release}");
            for _ in 0..200 {
                midi.frame(FRAME);
            }
            assert!(midi.frame(FRAME).notes[60] < 0.001);
        }
    }

    #[test]
    fn controllers_and_the_last_one_moved() {
        let mut midi = MidiState::default();
        midi.handle(&[0xB0, 21, 127]);
        midi.handle(&[0xB2, 74, 0]);
        let frame = midi.frame(FRAME);
        assert_eq!(frame.controls[21], 1.0);
        assert_eq!(frame.controls[74], 0.0);
        assert_eq!(frame.moved, Some(74));
        // Reported once.
        let frame = midi.frame(FRAME);
        assert_eq!(frame.moved, None);
        assert_eq!(frame.controls[21], 1.0);
    }

    #[test]
    fn ignores_other_and_short_messages() {
        let mut midi = MidiState::default();
        midi.handle(&[0xE0, 0, 64]); // Pitch bend.
        midi.handle(&[0xF8]); // Clock.
        midi.handle(&[0x90, 60]); // Truncated.
        let frame = midi.frame(FRAME);
        assert!(frame.notes.iter().chain(&frame.controls).all(|&v| v == 0.0));
        assert_eq!(frame.moved, None);
    }
}
