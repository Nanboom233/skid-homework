use serde::Serialize;

use crate::scanner_detect::ScannerPoint;

/// Tracks document corner points over multiple frames to determine stability.
///
/// Port of the TypeScript `StabilityTracker` from `stability-tracker.ts`.
pub struct StabilityTracker {
    history: Vec<[ScannerPoint; 4]>,
    max_frames: usize,
    variance_threshold: f32,
    consecutive_nones: u32,
    miss_grace_frames: u32,
}

impl StabilityTracker {
    pub fn new(max_frames: usize, variance_threshold: f32, miss_grace_frames: u32) -> Self {
        Self {
            history: Vec::with_capacity(max_frames),
            max_frames,
            variance_threshold,
            consecutive_nones: 0,
            miss_grace_frames,
        }
    }

    /// Pushes a new set of 4 corner points.
    /// Returns `true` if the document is stable across the tracked window.
    pub fn push(&mut self, points: Option<[ScannerPoint; 4]>) -> bool {
        let Some(pts) = points else {
            self.consecutive_nones += 1;
            if self.consecutive_nones > self.miss_grace_frames {
                self.history.clear();
            }
            // Never stable when missing input.
            return false;
        };

        self.consecutive_nones = 0;
        self.history.push(pts);
        if self.history.len() > self.max_frames {
            self.history.remove(0);
        }

        self.is_stable()
    }

    fn is_stable(&self) -> bool {
        if self.history.len() < self.max_frames {
            return false;
        }

        for point_idx in 0..4 {
            let mut sum_x = 0.0_f32;
            let mut sum_y = 0.0_f32;

            for frame in &self.history {
                sum_x += frame[point_idx].x;
                sum_y += frame[point_idx].y;
            }

            let n = self.max_frames as f32;
            let mean_x = sum_x / n;
            let mean_y = sum_y / n;

            let mut var_x = 0.0_f32;
            let mut var_y = 0.0_f32;

            for frame in &self.history {
                var_x += (frame[point_idx].x - mean_x).powi(2);
                var_y += (frame[point_idx].y - mean_y).powi(2);
            }

            var_x /= n;
            var_y /= n;

            if var_x.sqrt() > self.variance_threshold || var_y.sqrt() > self.variance_threshold {
                return false;
            }
        }

        true
    }

    pub fn reset(&mut self) {
        self.history.clear();
    }
}

// ---------------------------------------------------------------------------
// DetectionPresenceTracker
// ---------------------------------------------------------------------------

/// State returned by `DetectionPresenceTracker::push`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectionPresenceState {
    pub raw_detected: bool,
    pub effective_detected: bool,
    pub effective_points: Option<Vec<ScannerPoint>>,
    pub retained_from_history: bool,
    pub smoothed_from_detection: bool,
    pub missing_frames: u32,
}

/// Applies short-lived hysteresis to document detection presence so the
/// overlay does not disappear immediately when one or two frames fail.
///
/// Port of the TypeScript `DetectionPresenceTracker`
/// from `detection-presence-tracker.ts`.
pub struct DetectionPresenceTracker {
    last_effective_points: Option<[ScannerPoint; 4]>,
    last_detected_at: Option<f64>,
    missing_frames: u32,
    max_missing_frames: u32,
    retain_ms: f64,
    smoothing_threshold_px: f32,
    smoothing_factor: f32,
}

impl DetectionPresenceTracker {
    pub fn new(
        max_missing_frames: u32,
        retain_ms: f64,
        smoothing_threshold_px: f32,
        smoothing_factor: f32,
    ) -> Self {
        Self {
            last_effective_points: None,
            last_detected_at: None,
            missing_frames: 0,
            max_missing_frames,
            retain_ms,
            smoothing_threshold_px,
            smoothing_factor,
        }
    }

    pub fn push(
        &mut self,
        points: Option<[ScannerPoint; 4]>,
        now_ms: f64,
    ) -> DetectionPresenceState {
        if let Some(pts) = points {
            let should_smooth = self.last_effective_points.map_or(false, |prev| {
                max_corner_delta(&prev, &pts) <= self.smoothing_threshold_px
            });

            let effective = if should_smooth {
                if let Some(prev) = self.last_effective_points {
                    blend_points(&prev, &pts, self.smoothing_factor)
                } else {
                    pts
                }
            } else {
                pts
            };

            self.last_effective_points = Some(effective);
            self.last_detected_at = Some(now_ms);
            self.missing_frames = 0;

            return DetectionPresenceState {
                raw_detected: true,
                effective_detected: true,
                effective_points: Some(effective.to_vec()),
                retained_from_history: false,
                smoothed_from_detection: should_smooth,
                missing_frames: 0,
            };
        }

        self.missing_frames += 1;
        let within_frame_grace = self.missing_frames <= self.max_missing_frames;
        let within_time_grace = self
            .last_detected_at
            .map_or(false, |at| now_ms - at <= self.retain_ms);

        if let Some(effective) = self.last_effective_points {
            if within_frame_grace && within_time_grace {
                return DetectionPresenceState {
                    raw_detected: false,
                    effective_detected: true,
                    effective_points: Some(effective.to_vec()),
                    retained_from_history: true,
                    smoothed_from_detection: false,
                    missing_frames: self.missing_frames,
                };
            }
        }

        self.last_effective_points = None;
        self.last_detected_at = None;

        DetectionPresenceState {
            raw_detected: false,
            effective_detected: false,
            effective_points: None,
            retained_from_history: false,
            smoothed_from_detection: false,
            missing_frames: self.missing_frames,
        }
    }

    pub fn reset(&mut self) {
        self.last_effective_points = None;
        self.last_detected_at = None;
        self.missing_frames = 0;
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn max_corner_delta(a: &[ScannerPoint; 4], b: &[ScannerPoint; 4]) -> f32 {
    let mut max_d = 0.0_f32;
    for i in 0..4 {
        let dx = a[i].x - b[i].x;
        let dy = a[i].y - b[i].y;
        let d = (dx * dx + dy * dy).sqrt();
        if d > max_d {
            max_d = d;
        }
    }
    max_d
}

fn blend_points(
    previous: &[ScannerPoint; 4],
    next: &[ScannerPoint; 4],
    factor: f32,
) -> [ScannerPoint; 4] {
    let alpha = factor.clamp(0.0, 1.0);
    core::array::from_fn(|i| ScannerPoint {
        x: previous[i].x + (next[i].x - previous[i].x) * alpha,
        y: previous[i].y + (next[i].y - previous[i].y) * alpha,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_quad(offset: f32) -> [ScannerPoint; 4] {
        [
            ScannerPoint { x: 10.0 + offset, y: 10.0 + offset },
            ScannerPoint { x: 300.0 + offset, y: 10.0 + offset },
            ScannerPoint { x: 300.0 + offset, y: 200.0 + offset },
            ScannerPoint { x: 10.0 + offset, y: 200.0 + offset },
        ]
    }

    #[test]
    fn stability_tracker_needs_enough_frames() {
        let mut tracker = StabilityTracker::new(3, 8.0);
        assert!(!tracker.push(Some(make_quad(0.0))));
        assert!(!tracker.push(Some(make_quad(0.0))));
        assert!(tracker.push(Some(make_quad(0.0))));
    }

    #[test]
    fn stability_tracker_resets_on_null() {
        let mut tracker = StabilityTracker::new(3, 8.0);
        tracker.push(Some(make_quad(0.0)));
        tracker.push(Some(make_quad(0.0)));
        assert!(!tracker.push(None));
        assert!(!tracker.push(Some(make_quad(0.0))));
    }

    #[test]
    fn stability_tracker_detects_motion() {
        let mut tracker = StabilityTracker::new(3, 5.0);
        tracker.push(Some(make_quad(0.0)));
        tracker.push(Some(make_quad(20.0)));
        assert!(!tracker.push(Some(make_quad(40.0))));
    }

    #[test]
    fn presence_tracker_retains_through_grace() {
        let mut tracker = DetectionPresenceTracker::new(3, 360.0, 18.0, 0.35);
        let r1 = tracker.push(Some(make_quad(0.0)), 1000.0);
        assert!(r1.effective_detected);

        let r2 = tracker.push(None, 1100.0);
        assert!(r2.effective_detected);
        assert!(r2.retained_from_history);

        tracker.push(None, 1200.0);
        tracker.push(None, 1300.0);
        let r5 = tracker.push(None, 1400.0);
        assert!(!r5.effective_detected);
    }
}
