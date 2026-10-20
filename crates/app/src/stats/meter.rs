//! Frame timing, summed over short periods. Pure logic, tested natively.

/// Seconds between two reports: long enough to read, short enough to follow.
const PERIOD: f32 = 0.5;

/// Frame times over the last period, in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameReport {
    pub fps: f32,
    /// Average time between two frames.
    pub frame_ms: f32,
    /// The longest time between two frames: hitches show here.
    pub worst_ms: f32,
    /// Average CPU time to analyze the sound (and MIDI) each frame.
    pub analysis_ms: f32,
    /// Average CPU time to prepare and submit the frame to the GPU. The GPU's
    /// own time isn't included: it shows in `frame_ms` when it's the limit.
    pub draw_ms: f32,
}

#[derive(Default)]
pub struct FrameMeter {
    frames: u32,
    elapsed: f32,
    worst: f32,
    analysis: f32,
    draw: f32,
}

impl FrameMeter {
    /// Records a frame: the seconds since the previous one, and the time
    /// spent on its parts. Returns a report once per period.
    pub fn record(&mut self, interval: f32, analysis: f32, draw: f32) -> Option<FrameReport> {
        self.frames += 1;
        self.elapsed += interval;
        self.worst = self.worst.max(interval);
        self.analysis += analysis;
        self.draw += draw;
        if self.elapsed < PERIOD {
            return None;
        }
        let frames = self.frames as f32;
        let report = FrameReport {
            fps: frames / self.elapsed,
            frame_ms: 1000.0 * self.elapsed / frames,
            worst_ms: 1000.0 * self.worst,
            analysis_ms: 1000.0 * self.analysis / frames,
            draw_ms: 1000.0 * self.draw / frames,
        };
        *self = Self::default();
        Some(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_twice_a_second() {
        let mut meter = FrameMeter::default();
        let reports: Vec<_> = (0..120)
            .filter_map(|_| meter.record(1.0 / 60.0, 0.0002, 0.0005))
            .collect();
        assert_eq!(reports.len(), 4);
        let report = reports[0];
        assert!((report.fps - 60.0).abs() < 0.5, "{report:?}");
        assert!((report.frame_ms - 16.67).abs() < 0.1, "{report:?}");
        assert!((report.analysis_ms - 0.2).abs() < 1e-3, "{report:?}");
        assert!((report.draw_ms - 0.5).abs() < 1e-3, "{report:?}");
    }

    #[test]
    fn a_hitch_shows_as_the_worst_frame() {
        let mut meter = FrameMeter::default();
        let mut report = None;
        for frame in 0.. {
            let interval = if frame == 10 { 0.1 } else { 1.0 / 60.0 };
            report = meter.record(interval, 0.0, 0.0);
            if report.is_some() {
                break;
            }
        }
        let report = report.unwrap();
        assert!((report.worst_ms - 100.0).abs() < 1e-3, "{report:?}");
        // The average includes it: fewer frames in the period.
        assert!(report.fps < 60.0, "{report:?}");
        // The next period starts over.
        let next = (0..60)
            .find_map(|_| meter.record(1.0 / 60.0, 0.0, 0.0))
            .unwrap();
        assert!((next.worst_ms - 16.67).abs() < 0.1, "{next:?}");
    }
}
