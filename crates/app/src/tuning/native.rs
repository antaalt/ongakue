//! No tuning panel natively (yet): the defaults are used.

use analysis::Settings;

use crate::midi::MIDI_COUNT;

pub struct Tuning;

impl Tuning {
    pub fn new() -> Result<Self, String> {
        Ok(Self)
    }

    pub fn settings(&self) -> Settings {
        Settings::default()
    }

    pub fn params(&self) -> [f32; 4] {
        [0.0; 4]
    }

    pub fn apply_midi(&self, _controls: &[f32; MIDI_COUNT], _moved: Option<u8>) {}

    pub fn record(&self, _time: f32, _flux: f32, _threshold: f32, _beat: bool) {}
}
