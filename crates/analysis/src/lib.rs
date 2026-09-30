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

/// Upper edges of the bass and mid ranges, in Hz.
const BASS_MAX_FREQUENCY: f32 = 250.0;
const MID_MAX_FREQUENCY: f32 = 4000.0;

/// Time over which the beat detector's average rise is computed, in seconds.
const BEAT_HISTORY: f32 = 1.0;
/// Seconds for [`Spectrum::beat`] to fall to half after a beat.
const BEAT_HALF_LIFE: f32 = 0.15;

/// Tunable parameters, read on every call to [`Analyzer::process`]. The
/// defaults suit most music.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// Loudness range mapped to 0..1, in dB: quieter is 0, louder is 1.
    pub min_db: f32,
    pub max_db: f32,
    /// Music has much less energy in the highs than in the lows. Boosting by
    /// a few dB per octave (relative to 1 kHz) keeps the whole spectrum visible.
    pub tilt_db_per_octave: f32,
    /// Seconds for a band to fall to half its value when the sound stops.
    /// Rises are instant.
    pub band_half_life: f32,
    /// Beats are sudden rises in the bass bands (kicks): a rise counts as a
    /// beat when it exceeds the recent average by this many standard
    /// deviations (spectral flux with an adaptive threshold).
    pub beat_sensitivity: f32,
    /// Rises below this never count, so near-silence doesn't produce beats.
    pub beat_min_flux: f32,
    /// Shortest time between two beats, in seconds.
    pub beat_min_interval: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            min_db: -72.0,
            max_db: -12.0,
            tilt_db_per_octave: 3.0,
            band_half_life: 0.11,
            beat_sensitivity: 1.5,
            beat_min_flux: 0.05,
            beat_min_interval: 0.25,
        }
    }
}

/// The analysis of one frame. All values are normalized to 0..1.
#[derive(Clone, Debug)]
pub struct Spectrum {
    /// Loudness per frequency band, from low to high.
    pub bands: [f32; BAND_COUNT],
    /// Average loudness below 250 Hz.
    pub bass: f32,
    /// Average loudness from 250 Hz to 4 kHz.
    pub mid: f32,
    /// Average loudness above 4 kHz.
    pub treble: f32,
    /// 1.0 on a beat, then fading out.
    pub beat: f32,
}

impl Default for Spectrum {
    fn default() -> Self {
        Self {
            bands: [0.0; BAND_COUNT],
            bass: 0.0,
            mid: 0.0,
            treble: 0.0,
            beat: 0.0,
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
    pub settings: Settings,
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    input: Vec<f32>,
    output: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    /// Amplitude per bin, scaled so a full-scale sine reads 1.0.
    magnitudes: Vec<f32>,
    bands: Vec<Band>,
    bin_width: f32,
    /// Upper edge of the highest band, in Hz.
    max_frequency: f32,
    /// First band of the mid and treble ranges.
    mid_start: usize,
    treble_start: usize,
    /// Band values of the previous frame, before smoothing.
    previous: [f32; BAND_COUNT],
    /// Running mean and variance of the bass flux.
    flux_mean: f32,
    flux_variance: f32,
    since_beat: f32,
    /// Latest bass rise and the threshold it was compared to.
    flux: f32,
    threshold: f32,
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
            .collect::<Vec<_>>();
        let first_above = |frequency: f32| {
            bands
                .iter()
                .position(|band| band.center >= frequency)
                .unwrap_or(BAND_COUNT)
        };

        Self {
            mid_start: first_above(BASS_MAX_FREQUENCY),
            treble_start: first_above(MID_MAX_FREQUENCY),
            input: fft.make_input_vec(),
            output: fft.make_output_vec(),
            scratch: fft.make_scratch_vec(),
            magnitudes: vec![0.0; bin_count],
            fft,
            window,
            bands,
            bin_width,
            max_frequency,
            previous: [0.0; BAND_COUNT],
            flux_mean: 0.0,
            flux_variance: 0.0,
            since_beat: f32::INFINITY,
            flux: 0.0,
            threshold: 0.0,
            settings: Settings::default(),
            spectrum: Spectrum::default(),
        }
    }

    /// Analyzes the latest `fft_size` samples (mono, -1..1) and returns the
    /// updated spectrum. `dt` is the time since the previous call, in seconds.
    pub fn process(&mut self, samples: &[f32], dt: f32) -> &Spectrum {
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

        let settings = self.settings;
        let decay = 0.5f32.powf(dt / settings.band_half_life);
        let mut current = [0.0; BAND_COUNT];
        for ((value, raw), band) in self
            .spectrum
            .bands
            .iter_mut()
            .zip(&mut current)
            .zip(&self.bands)
        {
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
                + settings.tilt_db_per_octave * (band.center / 1000.0).log2();
            *raw = ((db - settings.min_db) / (settings.max_db - settings.min_db)).clamp(0.0, 1.0);
            *value = raw.max(*value * decay);
        }

        self.update_ranges();
        self.detect_beat(&current, dt);
        self.previous = current;

        &self.spectrum
    }

    fn detect_beat(&mut self, current: &[f32; BAND_COUNT], dt: f32) {
        // How much the bass rose since the last frame.
        let bass = 0..self.mid_start.max(1);
        let flux = current[bass.clone()]
            .iter()
            .zip(&self.previous[bass.clone()])
            .map(|(now, before)| (now - before).max(0.0))
            .sum::<f32>()
            / bass.len() as f32;

        let threshold = (self.flux_mean
            + self.settings.beat_sensitivity * self.flux_variance.sqrt())
        .max(self.settings.beat_min_flux);
        self.flux = flux;
        self.threshold = threshold;
        self.since_beat += dt;
        if flux > threshold && self.since_beat >= self.settings.beat_min_interval {
            self.since_beat = 0.0;
            self.spectrum.beat = 1.0;
        } else {
            self.spectrum.beat *= 0.5f32.powf(dt / BEAT_HALF_LIFE);
        }

        // Exponentially weighted mean and variance, updated after the test so
        // a beat doesn't raise its own threshold.
        let alpha = 1.0 - (-dt / BEAT_HISTORY).exp();
        let delta = flux - self.flux_mean;
        self.flux_mean += alpha * delta;
        self.flux_variance = (1.0 - alpha) * (self.flux_variance + alpha * delta * delta);
    }

    /// How much the bass rose in the latest frame. A beat is detected when it
    /// exceeds [`Analyzer::beat_threshold`]. Useful to tune the detection.
    pub fn beat_flux(&self) -> f32 {
        self.flux
    }

    pub fn beat_threshold(&self) -> f32 {
        self.threshold
    }

    /// Lights the band of each note's frequency with the note's value (e.g.
    /// MIDI velocities, 0..1, indexed by MIDI note number: 69 is A4, 440 Hz),
    /// where it's louder than the sound. Call after [`Analyzer::process`].
    pub fn add_notes(&mut self, notes: &[f32]) -> &Spectrum {
        let mut changed = false;
        for (note, &value) in notes.iter().enumerate() {
            if value <= 0.0 {
                continue;
            }
            let frequency = 440.0 * 2f32.powf((note as f32 - 69.0) / 12.0);
            if let Some(band) = self.band_of(frequency) {
                let current = &mut self.spectrum.bands[band];
                changed |= value > *current;
                *current = current.max(value);
            }
        }
        if changed {
            self.update_ranges();
        }
        &self.spectrum
    }

    /// The band containing a frequency, if it's within the analyzed range.
    pub fn band_of(&self, frequency: f32) -> Option<usize> {
        let position = (frequency / MIN_FREQUENCY).ln() / (self.max_frequency / MIN_FREQUENCY).ln();
        (0.0..1.0)
            .contains(&position)
            .then(|| ((position * BAND_COUNT as f32) as usize).min(BAND_COUNT - 1))
    }

    fn update_ranges(&mut self) {
        let bands = &self.spectrum.bands;
        let average = |range: std::ops::Range<usize>| {
            let len = range.len().max(1) as f32;
            bands[range].iter().sum::<f32>() / len
        };
        self.spectrum.bass = average(0..self.mid_start);
        self.spectrum.mid = average(self.mid_start..self.treble_start);
        self.spectrum.treble = average(self.treble_start..BAND_COUNT);
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
    const FRAME: f32 = 1.0 / 60.0;

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
        let spectrum = analyzer.process(&[0.0; FFT_SIZE], FRAME);
        assert!(spectrum.bands.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn sine_peaks_in_matching_band() {
        // Both window sizes the app uses: the smaller one reacts faster.
        for (fft_size, frequency) in [FFT_SIZE, FFT_SIZE / 2].into_iter().flat_map(|size| {
            [50.0, 120.0, 440.0, 1000.0, 3000.0, 8000.0, 14000.0].map(|frequency| (size, frequency))
        }) {
            let mut analyzer = Analyzer::new(SAMPLE_RATE, fft_size);
            // Quiet enough that nearby bands don't all clip at 1.0.
            let samples = sine(frequency, 0.01);
            let spectrum = analyzer
                .process(&samples[FFT_SIZE - fft_size..], FRAME)
                .clone();
            let found = analyzer.band_center(loudest_band(&spectrum));
            // Precision is limited by the FFT bin width in the lows and by the
            // band width in the highs.
            let tolerance = analyzer.bin_width().max(frequency * 0.15);
            assert!(
                (found - frequency).abs() <= tolerance,
                "{frequency} Hz peaked in the band centered on {found} Hz ({fft_size} samples)"
            );
        }
    }

    #[test]
    fn sine_does_not_leak_far_away() {
        let mut analyzer = Analyzer::new(SAMPLE_RATE, FFT_SIZE);
        let spectrum = analyzer.process(&sine(1000.0, 0.5), FRAME).clone();
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
        let quiet = analyzer.process(&sine(1000.0, 0.005), FRAME).clone();
        let mut analyzer = Analyzer::new(SAMPLE_RATE, FFT_SIZE);
        let loud = analyzer.process(&sine(1000.0, 0.05), FRAME).clone();
        let band = loudest_band(&loud);
        assert!(loud.bands[band] > quiet.bands[band]);
    }

    #[test]
    fn bands_decay_after_sound_stops() {
        let mut analyzer = Analyzer::new(SAMPLE_RATE, FFT_SIZE);
        let loud = analyzer.process(&sine(1000.0, 0.05), FRAME).clone();
        let band = loudest_band(&loud);

        let after_one = analyzer.process(&[0.0; FFT_SIZE], FRAME).bands[band];
        assert!(after_one > 0.0 && after_one < loud.bands[band]);

        for _ in 0..200 {
            analyzer.process(&[0.0; FFT_SIZE], FRAME);
        }
        assert!(analyzer.process(&[0.0; FFT_SIZE], FRAME).bands[band] < 0.01);
    }

    #[test]
    fn decay_does_not_depend_on_frame_rate() {
        let after = |fps: f32| {
            let mut analyzer = Analyzer::new(SAMPLE_RATE, FFT_SIZE);
            let band = loudest_band(analyzer.process(&sine(1000.0, 0.05), 1.0 / fps));
            // 0.1 s of silence.
            for _ in 0..(fps / 10.0).round() as usize {
                analyzer.process(&[0.0; FFT_SIZE], 1.0 / fps);
            }
            analyzer.spectrum.bands[band]
        };
        assert!((after(60.0) - after(144.0)).abs() < 0.02);
    }

    #[test]
    fn ranges_follow_the_signal() {
        let mut analyzer = Analyzer::new(SAMPLE_RATE, FFT_SIZE);
        let low = analyzer.process(&sine(80.0, 0.3), FRAME).clone();
        assert!(low.bass > low.mid && low.bass > low.treble);

        let mut analyzer = Analyzer::new(SAMPLE_RATE, FFT_SIZE);
        let high = analyzer.process(&sine(8000.0, 0.3), FRAME).clone();
        assert!(high.treble > high.mid && high.treble > high.bass);
    }

    /// Feeds `signal` frame by frame, as the app does at 60 fps, and counts
    /// the frames where a beat starts.
    fn count_beats(signal: &[f32], settings: Settings) -> usize {
        beat_times(signal, settings, FFT_SIZE).len()
    }

    /// Feeds `signal` frame by frame, as the app does at 60 fps, with windows
    /// of `fft_size` samples, and returns when beats are detected: the time of
    /// the newest sample analyzed, in seconds.
    fn beat_times(signal: &[f32], settings: Settings, fft_size: usize) -> Vec<f32> {
        let hop = (SAMPLE_RATE * FRAME) as usize;
        let mut analyzer = Analyzer::new(SAMPLE_RATE, fft_size);
        analyzer.settings = settings;
        (fft_size..signal.len())
            .step_by(hop)
            .filter(|&end| analyzer.process(&signal[end - fft_size..end], FRAME).beat == 1.0)
            .map(|end| end as f32 / SAMPLE_RATE)
            .collect()
    }

    /// A quiet steady tone, plus a decaying 60 Hz kick every 0.5 s from
    /// 0.25 s on: 8 kicks in total.
    fn kicks() -> Vec<f32> {
        kicks_every(0.5, 4.0)
    }

    /// Like [`kicks`], with a kick every `period` seconds during `duration`.
    fn kicks_every(period: f32, duration: f32) -> Vec<f32> {
        let len = (SAMPLE_RATE * duration) as usize;
        (0..len)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE;
                let tone = 0.02 * (std::f32::consts::TAU * 1000.0 * t).sin();
                let since_kick = (t - 0.25).rem_euclid(period);
                let kick = if t >= 0.25 {
                    0.5 * (-since_kick / 0.08).exp()
                        * (std::f32::consts::TAU * 60.0 * since_kick).sin()
                } else {
                    0.0
                };
                tone + kick
            })
            .collect()
    }

    #[test]
    fn detects_kicks() {
        assert_eq!(count_beats(&kicks(), Settings::default()), 8);
    }

    /// Average time from a kick's start to its detection, with a window of
    /// `fft_size` samples. Kicks come every 0.51 s, so they land at different
    /// moments within the 60 fps frames.
    fn detection_delay(fft_size: usize) -> f32 {
        let period = 0.51;
        let times = beat_times(&kicks_every(period, 10.0), Settings::default(), fft_size);
        // From 0.25 s to 10 s: 20 kicks.
        assert_eq!(times.len(), 20, "{fft_size} samples: {times:?}");
        let delays: Vec<f32> = times
            .iter()
            .map(|&time| (time - 0.25).rem_euclid(period))
            .collect();
        delays.iter().sum::<f32>() / delays.len() as f32
    }

    #[test]
    fn smaller_window_detects_kicks_sooner() {
        let (normal, fast) = (detection_delay(FFT_SIZE), detection_delay(FFT_SIZE / 2));
        eprintln!(
            "average detection delay: {normal:.4} s with {FFT_SIZE} samples, {fast:.4} s with {}",
            FFT_SIZE / 2
        );
        assert!(fast < normal, "{fast} vs {normal}");
    }

    #[test]
    fn settings_take_effect() {
        let deaf = Settings {
            beat_min_flux: 1.0,
            ..Settings::default()
        };
        assert_eq!(count_beats(&kicks(), deaf), 0);
    }

    #[test]
    fn steady_sound_has_no_beats() {
        let len = (SAMPLE_RATE * 3.0) as usize;
        let signal: Vec<f32> = (0..len)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE;
                0.3 * (std::f32::consts::TAU * 80.0 * t).sin()
            })
            .collect();
        // At most one, when the sound starts.
        assert!(count_beats(&signal, Settings::default()) <= 1);
    }

    #[test]
    fn notes_light_their_band() {
        let mut analyzer = Analyzer::new(SAMPLE_RATE, FFT_SIZE);
        analyzer.process(&[0.0; FFT_SIZE], FRAME);
        let mut notes = [0.0; 128];
        notes[69] = 0.8; // A4, 440 Hz.
        let spectrum = analyzer.add_notes(&notes).clone();
        let band = analyzer.band_of(440.0).unwrap();
        assert_eq!(spectrum.bands[band], 0.8);
        assert_eq!(loudest_band(&spectrum), band);
        assert!(spectrum.mid > 0.0 && spectrum.bass == 0.0 && spectrum.treble == 0.0);
        // Out of the analyzed range: note 0 is about 8 Hz.
        notes = [0.0; 128];
        notes[0] = 1.0;
        let mut analyzer = Analyzer::new(SAMPLE_RATE, FFT_SIZE);
        analyzer.process(&[0.0; FFT_SIZE], FRAME);
        assert!(analyzer.add_notes(&notes).bands.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn notes_never_lower_the_sound() {
        let mut analyzer = Analyzer::new(SAMPLE_RATE, FFT_SIZE);
        let loud = analyzer.process(&sine(440.0, 0.5), FRAME).clone();
        let band = analyzer.band_of(440.0).unwrap();
        let mut notes = [0.0; 128];
        notes[69] = 0.1;
        assert_eq!(analyzer.add_notes(&notes).bands[band], loud.bands[band]);
    }

    #[test]
    fn band_of_matches_band_centers() {
        let analyzer = Analyzer::new(SAMPLE_RATE, FFT_SIZE);
        for band in 0..BAND_COUNT {
            assert_eq!(analyzer.band_of(analyzer.band_center(band)), Some(band));
        }
        assert_eq!(analyzer.band_of(10.0), None);
        assert_eq!(analyzer.band_of(20_000.0), None);
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
