//! The frame-timing windows the host owns, independent of where frames came
//! from. Frames arrive one at a time with the host's receipt time; each query
//! prunes what has aged out.
//!
//! `fps` is `1000 / mean(frametime)` over the last second. `low1` needs at least
//! 1000 frames inside the rolling 60-second window, then averages the slowest
//! `ceil(1%)` of those frametimes and converts that mean frametime back to FPS.
//! Converting the mean frametime (not averaging per-frame FPS) is the reading a
//! 1% low is defined by; the two differ whenever the slow frames differ from
//! each other.

const FPS_WINDOW_MS: u64 = 1_000;
const LOW_WINDOW_MS: u64 = 60_000;
const LOW_MIN_FRAMES: usize = 1_000;

#[derive(Debug, Clone, Copy)]
struct Frametime {
    frametime_ms: f64,
    at_ms: u64,
}

#[derive(Debug, Clone, Default)]
pub struct FrameWindow {
    frames: Vec<Frametime>,
}

impl FrameWindow {
    /// Ignores a frametime that is not a positive finite number.
    pub fn add(&mut self, frametime_ms: f64, at_ms: u64) {
        if !frametime_ms.is_finite() || frametime_ms <= 0.0 {
            return;
        }
        // Pruning is left to the next query: a reading is queried at least as
        // often as it arrives, and pruning here would churn the window on every
        // frame.
        self.frames.push(Frametime {
            frametime_ms,
            at_ms,
        });
    }

    pub fn fps(&mut self, now_ms: u64) -> Option<f64> {
        let since = now_ms.saturating_sub(FPS_WINDOW_MS);
        let mut recent = 0_usize;
        let mut total = 0.0;
        for frame in self.prune(now_ms) {
            if frame.at_ms >= since {
                recent += 1;
                total += frame.frametime_ms;
            }
        }
        if recent == 0 {
            return None;
        }
        Some(1000.0 / (total / recent as f64))
    }

    /// `None` below 1000 frames in the window, as a 1% low needs a population.
    pub fn low1(&mut self, now_ms: u64) -> Option<f64> {
        let kept: Vec<f64> = self.prune(now_ms).map(|frame| frame.frametime_ms).collect();
        if kept.len() < LOW_MIN_FRAMES {
            return None;
        }
        let mut sorted = kept;
        sorted.sort_by(|left, right| left.partial_cmp(right).expect("finite frametimes"));
        let slowest = sorted.len().div_ceil(100);
        let slow = &sorted[sorted.len() - slowest..];
        let total: f64 = slow.iter().sum();
        Some(1000.0 / (total / slow.len() as f64))
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn clear(&mut self) {
        self.frames.clear();
    }

    fn prune(&mut self, now_ms: u64) -> impl Iterator<Item = Frametime> + '_ {
        let since = now_ms.saturating_sub(LOW_WINDOW_MS);
        self.frames.retain(|frame| frame.at_ms >= since);
        self.frames.iter().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames_at(window: &mut FrameWindow, count: usize, frametime_ms: f64, start_ms: u64) {
        for index in 0..count {
            window.add(frametime_ms, start_ms + index as u64);
        }
    }

    #[test]
    fn no_frames_is_none_and_never_zero() {
        assert_eq!(FrameWindow::default().fps(0), None);
    }

    #[test]
    fn fps_is_the_reciprocal_of_the_mean_frametime_over_the_last_second() {
        let mut window = FrameWindow::default();
        window.add(5.0, 0);
        window.add(10.0, 500);
        window.add(15.0, 1_000);

        let fps = window.fps(1_000).expect("three frames");
        assert!((fps - 100.0).abs() < 1e-9, "{fps}");
    }

    #[test]
    fn a_frame_exactly_a_second_old_still_counts_and_older_does_not() {
        let mut window = FrameWindow::default();
        window.add(5.0, 0);
        window.add(20.0, 800);

        assert!((window.fps(1_000).expect("two frames") - 80.0).abs() < 1e-9);
        assert!((window.fps(1_800).expect("one frame") - 50.0).abs() < 1e-9);
    }

    #[test]
    fn a_frametime_that_is_not_positive_and_finite_is_ignored() {
        let mut window = FrameWindow::default();
        window.add(0.0, 0);
        window.add(-4.0, 1);
        window.add(f64::NAN, 2);
        window.add(f64::INFINITY, 3);

        assert_eq!(window.fps(3), None);
    }

    #[test]
    fn a_low_needs_a_thousand_frames_in_the_window() {
        let mut window = FrameWindow::default();
        frames_at(&mut window, 999, 5.0, 0);

        assert_eq!(window.low1(998), None);
    }

    #[test]
    fn a_low_averages_the_slowest_percent_as_frametime() {
        let mut window = FrameWindow::default();
        // 990 fast frames and ten slow ones: five at 100 ms, five at 200 ms.
        frames_at(&mut window, 990, 5.0, 0);
        frames_at(&mut window, 5, 100.0, 1_000);
        frames_at(&mut window, 5, 200.0, 2_000);

        // ceil(1% of 1000) = 10 slowest frames; mean frametime 150 ms, so
        // 1000 / 150 FPS. Averaging per-frame FPS would give 7.5 FPS instead.
        let low = window.low1(3_000).expect("a thousand frames");
        assert!((low - 1000.0 / 150.0).abs() < 1e-9, "{low}");
    }

    #[test]
    fn frames_older_than_a_minute_leave_the_window() {
        let mut window = FrameWindow::default();
        frames_at(&mut window, 1_000, 5.0, 0);

        assert!((window.low1(60_000).expect("all inside") - 200.0).abs() < 1e-9);
        // One more millisecond pushes the oldest out, leaving 999 frames.
        assert_eq!(window.low1(60_001), None);
    }
}
