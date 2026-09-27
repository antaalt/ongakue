//! Turns raw audio samples into a compact, visual-friendly spectrum.
//!
//! Pure Rust with no platform dependencies, so it runs (and is tested)
//! identically on native and on the web.

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

/// Number of frequency bands in a [`Spectrum`].
pub const BAND_COUNT: usize = 64;

const MIN_FREQUENCY: f32 = 30.0;
const MAX_FREQUENCY: f32 = 16_000.0;

/// Loudness range mapped to 0..1. Anything quieter is 0, anything louder is 1.
const MIN_DB: f32 = -72.0;
const MAX_DB: f32 = -12.0;

/// Music has much less energy in the highs than in the lows. Boosting by a few
/// dB per octave (relative to 1 kHz) keeps the whole spectrum visible.
const TILT_DB_PER_OCTAVE: f32 = 3.0;

/// How much of its previous value a band keeps each frame when the signal
/// drops. Rises are instant.
const DECAY: f32 = 0.9;

/// Loudness per frequency band, from low to high, each normalized to 0..1.
#[derive(Clone, Debug)]
pub struct Spectrum {
    pub bands: [f32; BAND_COUNT],
}

impl Default for Spectrum {
    fn default() -> Self {
        Self {
            bands: [0.0; BAND_COUNT],
        }
    }
}

/// Which FFT bins make up a band.
enum Bins {
    /// Wide band: the loudest bin in the range.
    Range(std::ops::Range<usize>),
    /// Band narrower than one bin (in the lows): interpolate between the two
    /// bins around its center.
    Interpolate { bin: usize, frac: f32 },
}

struct Band {
    center: f32,
    bins: Bins,
}

pub struct Analyzer {
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    input: Vec<f32>,
    output: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    /// Amplitude per bin, scaled so a full-scale sine reads 1.0.
    magnitudes: Vec<f32>,
    bands: Vec<Band>,
    bin_width: f32,
    spectrum: Spectrum,
}

impl Analyzer {
    /// `fft_size` is the number of samples passed to [`Analyzer::process`].
    pub fn new(sample_rate: f32, fft_size: usize) -> Self {
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(fft_size);

        // Hann window, to limit spectral leakage between bins.
        let window: Vec<f32> = (0..fft_size)
            .map(|i| {
                let phase = std::f32::consts::TAU * i as f32 / fft_size as f32;
                0.5 - 0.5 * phase.cos()
            })
            .collect();

        let bin_width = sample_rate / fft_size as f32;
        let bin_count = fft_size / 2 + 1;
        let max_frequency = MAX_FREQUENCY.min(sample_rate / 2.0 - bin_width);

        // Log-spaced edges: each band covers the same musical interval.
        let edge = |i: usize| {
            MIN_FREQUENCY * (max_frequency / MIN_FREQUENCY).powf(i as f32 / BAND_COUNT as f32)
        };
        let bands = (0..BAND_COUNT)
            .map(|i| {
                let (low, high) = (edge(i), edge(i + 1));
                let center = (low * high).sqrt();
                let first = (low / bin_width).ceil() as usize;
                let last = ((high / bin_width).floor() as usize).min(bin_count - 1);
                let bins = if first <= last {
                    Bins::Range(first..last + 1)
                } else {
                    let position = center / bin_width;
                    Bins::Interpolate {
                        bin: position as usize,
                        frac: position.fract(),
                    }
                };
                Band { center, bins }
            })
            .collect();

        Self {
            input: fft.make_input_vec(),
            output: fft.make_output_vec(),
            scratch: fft.make_scratch_vec(),
            magnitudes: vec![0.0; bin_count],
            fft,
            window,
            bands,
            bin_width,
            spectrum: Spectrum::default(),
        }
    }

    /// Analyzes the latest `fft_size` samples (mono, -1..1) and returns the
    /// updated spectrum. Call once per frame: smoothing is applied per call.
    pub fn process(&mut self, samples: &[f32]) -> &Spectrum {
        assert_eq!(samples.len(), self.window.len(), "wrong number of samples");

        for ((input, sample), window) in self.input.iter_mut().zip(samples).zip(&self.window) {
            *input = sample * window;
        }
        self.fft
            .process_with_scratch(&mut self.input, &mut self.output, &mut self.scratch)
            .expect("buffer sizes match the FFT plan");

        // With a Hann window, a sine of amplitude A peaks at A * N / 4.
        let scale = 4.0 / self.window.len() as f32;
        for (magnitude, bin) in self.magnitudes.iter_mut().zip(&self.output) {
            *magnitude = bin.norm() * scale;
        }

        for (value, band) in self.spectrum.bands.iter_mut().zip(&self.bands) {
            let magnitude = match &band.bins {
                Bins::Range(range) => self.magnitudes[range.clone()]
                    .iter()
                    .copied()
                    .fold(0.0, f32::max),
                Bins::Interpolate { bin, frac } => {
                    let (a, b) = (self.magnitudes[*bin], self.magnitudes[bin + 1]);
                    a + (b - a) * frac
                }
            };
            let db = 20.0 * magnitude.max(1e-10).log10()
                + TILT_DB_PER_OCTAVE * (band.center / 1000.0).log2();
            let target = ((db - MIN_DB) / (MAX_DB - MIN_DB)).clamp(0.0, 1.0);
            *value = target.max(*value * DECAY);
        }

        &self.spectrum
    }

    /// Center frequency of a band, in Hz.
    pub fn band_center(&self, band: usize) -> f32 {
        self.bands[band].center
    }

    /// Frequency resolution of the FFT, in Hz.
    pub fn bin_width(&self) -> f32 {
        self.bin_width
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: f32 = 48_000.0;
    const FFT_SIZE: usize = 2048;

    fn sine(frequency: f32, amplitude: f32) -> Vec<f32> {
        (0..FFT_SIZE)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE;
                amplitude * (std::f32::consts::TAU * frequency * t).sin()
            })
            .collect()
    }

    fn loudest_band(spectrum: &Spectrum) -> usize {
        (0..BAND_COUNT)
            .max_by(|&a, &b| spectrum.bands[a].total_cmp(&spectrum.bands[b]))
            .unwrap()
    }

    #[test]
    fn silence_is_zero() {
        let mut analyzer = Analyzer::new(SAMPLE_RATE, FFT_SIZE);
        let spectrum = analyzer.process(&[0.0; FFT_SIZE]);
        assert!(spectrum.bands.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn sine_peaks_in_matching_band() {
        for frequency in [50.0, 120.0, 440.0, 1000.0, 3000.0, 8000.0, 14000.0] {
            let mut analyzer = Analyzer::new(SAMPLE_RATE, FFT_SIZE);
            // Quiet enough that nearby bands don't all clip at 1.0.
            let spectrum = analyzer.process(&sine(frequency, 0.01)).clone();
            let found = analyzer.band_center(loudest_band(&spectrum));
            // Precision is limited by the FFT bin width in the lows and by the
            // band width in the highs.
            let tolerance = analyzer.bin_width().max(frequency * 0.15);
            assert!(
                (found - frequency).abs() <= tolerance,
                "{frequency} Hz peaked in the band centered on {found} Hz"
            );
        }
    }

    #[test]
    fn sine_does_not_leak_far_away() {
        let mut analyzer = Analyzer::new(SAMPLE_RATE, FFT_SIZE);
        let spectrum = analyzer.process(&sine(1000.0, 0.5)).clone();
        for band in 0..BAND_COUNT {
            let center = analyzer.band_center(band);
            // More than two octaves away.
            if !(250.0..4000.0).contains(&center) {
                assert!(
                    spectrum.bands[band] < 0.1,
                    "band at {center} Hz reads {}",
                    spectrum.bands[band]
                );
            }
        }
    }

    #[test]
    fn louder_sine_reads_higher() {
        let mut analyzer = Analyzer::new(SAMPLE_RATE, FFT_SIZE);
        let quiet = analyzer.process(&sine(1000.0, 0.005)).clone();
        let mut analyzer = Analyzer::new(SAMPLE_RATE, FFT_SIZE);
        let loud = analyzer.process(&sine(1000.0, 0.05)).clone();
        let band = loudest_band(&loud);
        assert!(loud.bands[band] > quiet.bands[band]);
    }

    #[test]
    fn bands_decay_after_sound_stops() {
        let mut analyzer = Analyzer::new(SAMPLE_RATE, FFT_SIZE);
        let loud = analyzer.process(&sine(1000.0, 0.05)).clone();
        let band = loudest_band(&loud);

        let after_one = analyzer.process(&[0.0; FFT_SIZE]).bands[band];
        assert!(after_one > 0.0 && after_one < loud.bands[band]);

        for _ in 0..200 {
            analyzer.process(&[0.0; FFT_SIZE]);
        }
        assert!(analyzer.process(&[0.0; FFT_SIZE]).bands[band] < 0.01);
    }

    #[test]
    fn band_centers_increase() {
        let analyzer = Analyzer::new(SAMPLE_RATE, FFT_SIZE);
        for band in 1..BAND_COUNT {
            assert!(analyzer.band_center(band) > analyzer.band_center(band - 1));
        }
        assert!(analyzer.band_center(0) >= MIN_FREQUENCY);
        assert!(analyzer.band_center(BAND_COUNT - 1) <= MAX_FREQUENCY);
    }
}
