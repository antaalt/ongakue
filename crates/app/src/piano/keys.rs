//! The computer keyboard as a piano: keys turn into MIDI note messages. Keys
//! are identified by their physical position (`KeyboardEvent.code` on the web,
//! named after a QWERTY layout), so the layout is the same on every keyboard.
//!
//! On AZERTY:
//!
//! ```text
//!  Z   E       T   Y   U       O   P        black keys
//! Q   S   D   F   G   H   J   K   L   M     white keys, from C
//! ```
//! W and X move an octave down or up.

/// Physical key → semitones above the octave's C.
const NOTE_KEYS: &[(&str, u8)] = &[
    ("KeyA", 0),
    ("KeyW", 1),
    ("KeyS", 2),
    ("KeyE", 3),
    ("KeyD", 4),
    ("KeyF", 5),
    ("KeyT", 6),
    ("KeyG", 7),
    ("KeyY", 8),
    ("KeyH", 9),
    ("KeyU", 10),
    ("KeyJ", 11),
    ("KeyK", 12),
    ("KeyO", 13),
    ("KeyL", 14),
    ("KeyP", 15),
    ("Semicolon", 16),
];
const OCTAVE_DOWN: &str = "KeyZ";
const OCTAVE_UP: &str = "KeyX";

/// Velocity of the notes played, out of 127.
pub const VELOCITY: u8 = 100;
/// The starting octave: its C is middle C, MIDI note 60.
const DEFAULT_OCTAVE: i32 = 4;
/// Highest octave whose keys all stay within MIDI notes 0..=127.
const MAX_OCTAVE: i32 = 8;

pub struct KeyPiano {
    octave: i32,
    /// Keys held down, with the note each one started.
    held: Vec<(String, u8)>,
}

impl Default for KeyPiano {
    fn default() -> Self {
        Self {
            octave: DEFAULT_OCTAVE,
            held: Vec::new(),
        }
    }
}

impl KeyPiano {
    /// A key was pressed: the note-on message to send, if it plays one. A key
    /// already held (auto-repeat) plays nothing.
    pub fn key_down(&mut self, code: &str) -> Option<[u8; 3]> {
        match code {
            OCTAVE_DOWN => self.octave = (self.octave - 1).max(-1),
            OCTAVE_UP => self.octave = (self.octave + 1).min(MAX_OCTAVE),
            _ => {}
        }
        let &(_, offset) = NOTE_KEYS.iter().find(|(key, _)| *key == code)?;
        if self.held.iter().any(|(key, _)| key == code) {
            return None;
        }
        let note = u8::try_from(12 * (self.octave + 1) + i32::from(offset)).ok()?;
        self.held.push((code.to_owned(), note));
        Some([0x90, note, VELOCITY])
    }

    /// A key was released: the note-off message for the note it started.
    pub fn key_up(&mut self, code: &str) -> Option<[u8; 3]> {
        let index = self.held.iter().position(|(key, _)| key == code)?;
        let (_, note) = self.held.remove(index);
        Some([0x80, note, 0])
    }

    /// Note-off messages for every held key, e.g. when the window loses the
    /// focus and their key-ups would never arrive.
    pub fn release_all(&mut self) -> Vec<[u8; 3]> {
        self.held
            .drain(..)
            .map(|(_, note)| [0x80, note, 0])
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plays_from_middle_c() {
        let mut piano = KeyPiano::default();
        assert_eq!(piano.key_down("KeyA"), Some([0x90, 60, VELOCITY]));
        assert_eq!(piano.key_down("KeyW"), Some([0x90, 61, VELOCITY]));
        assert_eq!(piano.key_down("Semicolon"), Some([0x90, 76, VELOCITY]));
        assert_eq!(piano.key_up("KeyA"), Some([0x80, 60, 0]));
    }

    #[test]
    fn other_keys_and_repeats_play_nothing() {
        let mut piano = KeyPiano::default();
        assert_eq!(piano.key_down("Space"), None);
        assert_eq!(piano.key_down("KeyB"), None);
        assert!(piano.key_down("KeyA").is_some());
        assert_eq!(piano.key_down("KeyA"), None);
        assert_eq!(piano.key_up("KeyB"), None);
    }

    #[test]
    fn octaves() {
        let mut piano = KeyPiano::default();
        assert_eq!(piano.key_down(OCTAVE_UP), None);
        assert_eq!(piano.key_down("KeyA"), Some([0x90, 72, VELOCITY]));
        for _ in 0..20 {
            piano.key_down(OCTAVE_DOWN);
        }
        // The lowest octave starts at note 0.
        assert_eq!(piano.key_down("KeyS"), Some([0x90, 2, VELOCITY]));
        for _ in 0..20 {
            piano.key_down(OCTAVE_UP);
        }
        // The highest one still fits within 127.
        assert_eq!(piano.key_down("Semicolon"), Some([0x90, 124, VELOCITY]));
    }

    #[test]
    fn releases_the_note_a_key_started() {
        let mut piano = KeyPiano::default();
        piano.key_down("KeyA");
        piano.key_down(OCTAVE_UP);
        // Still note 60, although the octave changed meanwhile.
        assert_eq!(piano.key_up("KeyA"), Some([0x80, 60, 0]));
    }

    #[test]
    fn release_all() {
        let mut piano = KeyPiano::default();
        piano.key_down("KeyA");
        piano.key_down("KeyD");
        assert_eq!(piano.release_all(), vec![[0x80, 60, 0], [0x80, 64, 0]]);
        assert!(piano.release_all().is_empty());
        // Released keys play again.
        assert!(piano.key_down("KeyA").is_some());
    }
}
