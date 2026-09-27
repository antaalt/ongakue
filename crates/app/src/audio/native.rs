//! Native audio. Silent for now; cpal + symphonia will come later.

/// Number of samples returned by [`Backend::latest_samples`].
pub const SAMPLE_COUNT: usize = 2048;

pub struct Backend;

impl Backend {
    pub fn new() -> Result<Self, String> {
        Ok(Self)
    }

    pub fn sample_rate(&self) -> f32 {
        48_000.0
    }

    /// Copies the most recently played samples (mono, -1..1) into `out`.
    pub fn latest_samples(&mut self, out: &mut [f32; SAMPLE_COUNT]) {
        out.fill(0.0);
    }
}
