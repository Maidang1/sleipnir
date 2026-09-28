//! Named motion: a timeline, a CSS cubic-bezier, and pure phase math.
//!
//! Three ways to drive a frame, and no springs:
//! - [`MotionSpec::animation`] is a one-shot [`gpui::Animation`] for
//!   `with_animation`.
//! - [`Lease`] is the repeating clock. A loader asks for 30fps and samples
//!   [`phase`]; nothing in the crate pins the window to the display rate.
//! - [`wall_progress`] tweens from a start instant, so a remount does not
//!   replay the curve from zero.

use std::time::{Duration, Instant};

use gpui::Animation;

pub mod phase;

/// A CSS `cubic-bezier(x1, y1, x2, y2)`. Endpoints stay at (0, 0) and (1, 1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CubicBezier {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

impl CubicBezier {
    pub const fn new(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Self { x1, y1, x2, y2 }
    }

    fn coefficients(a: f32, b: f32) -> (f32, f32, f32) {
        let c = 3.0 * a;
        let bb = 3.0 * (b - a) - c;
        let aa = 1.0 - c - bb;
        (aa, bb, c)
    }

    fn sample_x(&self, t: f32) -> f32 {
        let (a, b, c) = Self::coefficients(self.x1, self.x2);
        ((a * t + b) * t + c) * t
    }

    fn sample_y(&self, t: f32) -> f32 {
        let (a, b, c) = Self::coefficients(self.y1, self.y2);
        ((a * t + b) * t + c) * t
    }

    fn sample_x_derivative(&self, t: f32) -> f32 {
        let (a, b, c) = Self::coefficients(self.x1, self.x2);
        (3.0 * a * t + 2.0 * b) * t + c
    }

    fn solve_t_for_x(&self, x: f32) -> f32 {
        let mut t = x;
        for _ in 0..8 {
            let err = self.sample_x(t) - x;
            if err.abs() < 1e-6 {
                return t;
            }
            let slope = self.sample_x_derivative(t);
            if slope.abs() < 1e-6 {
                break;
            }
            t -= err / slope;
        }
        let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
        for _ in 0..32 {
            let mid = (lo + hi) / 2.0;
            if self.sample_x(mid) < x {
                lo = mid
            } else {
                hi = mid
            }
        }
        (lo + hi) / 2.0
    }

    /// Eased output for progress `x` in 0..1.
    pub fn eval(&self, x: f32) -> f32 {
        if x <= 0.0 {
            return 0.0;
        }
        if x >= 1.0 {
            return 1.0;
        }
        self.sample_y(self.solve_t_for_x(x)).clamp(0.0, 1.0)
    }
}

pub const EASE_OUT_EXPO: CubicBezier = CubicBezier::new(0.16, 1.0, 0.3, 1.0);
pub const EASE_OUT: CubicBezier = CubicBezier::new(0.0, 0.0, 0.58, 1.0);
pub const EASE: CubicBezier = CubicBezier::new(0.25, 0.1, 0.25, 1.0);
pub const EASE_RESORT: CubicBezier = CubicBezier::new(0.22, 1.0, 0.36, 1.0);
pub const EASE_IN_OUT: CubicBezier = CubicBezier::new(0.42, 0.0, 0.58, 1.0);
pub const EASE_TAILWIND: CubicBezier = CubicBezier::new(0.4, 0.0, 0.2, 1.0);

/// One catalog entry. Delay is folded into the timeline because gpui's
/// [`Animation`] has no delay of its own.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionSpec {
    pub duration_ms: u64,
    pub delay_ms: u64,
    pub curve: CubicBezier,
}

impl MotionSpec {
    pub const fn new(duration_ms: u64, curve: CubicBezier) -> Self {
        Self {
            duration_ms,
            delay_ms: 0,
            curve,
        }
    }

    pub const fn with_delay(mut self, delay_ms: u64) -> Self {
        self.delay_ms = delay_ms;
        self
    }

    pub fn total(&self) -> Duration {
        Duration::from_millis(self.delay_ms + self.duration_ms)
    }

    /// Eased progress for a raw timeline delta across [`Self::total`].
    pub fn progress(&self, raw_delta: f32) -> f32 {
        let total = (self.delay_ms + self.duration_ms) as f32;
        if total <= 0.0 || self.duration_ms == 0 {
            return 1.0;
        }
        let t =
            (raw_delta.clamp(0.0, 1.0) * total - self.delay_ms as f32) / self.duration_ms as f32;
        self.curve.eval(t.clamp(0.0, 1.0))
    }

    /// One-shot drive: hand this to `with_animation`.
    pub fn animation(&self) -> Animation {
        let spec = *self;
        Animation::new(spec.total()).with_easing(move |delta| spec.progress(delta))
    }
}

pub const FADE_IN: MotionSpec = MotionSpec::new(500, EASE_OUT_EXPO);
pub const FADE_QUICK: MotionSpec = MotionSpec::new(150, EASE);
pub const MENU_IN: MotionSpec = MotionSpec::new(140, EASE);
pub const RESIZE: MotionSpec = MotionSpec::new(200, EASE_OUT);
/// Tab reorder and activation slide: 150ms ease-out.
pub const TAB_SLIDE: MotionSpec = MotionSpec::new(150, EASE_OUT);
pub const LAYOUT: MotionSpec = MotionSpec::new(200, EASE_OUT);
pub const SCROLL_GLIDE: MotionSpec = MotionSpec::new(500, EASE_IN_OUT);
pub const HOVER_FADE: MotionSpec = MotionSpec::new(150, EASE_TAILWIND);

/// Repeating drive. Callers sample [`phase`] once per [`Self::interval`]
/// instead of leaving a `with_animation` repeat pinned to the display rate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lease {
    pub fps: f32,
}

impl Lease {
    pub const PULSE: Self = Self { fps: 30.0 };
    pub const HOVER: Self = Self { fps: 60.0 };

    pub fn interval(self) -> Duration {
        Duration::from_secs_f32(1.0 / self.fps.max(1.0))
    }
}

/// Phase in 0..1 for a lease that started at `epoch`.
pub fn phase(spec: MotionSpec, epoch: Instant, now: Instant) -> f32 {
    let period = spec.total().as_secs_f32();
    if period <= 0.0 {
        return 0.0;
    }
    (now.saturating_duration_since(epoch).as_secs_f32() / period).fract()
}

/// Wall-clock drive. `started` is the moment the tween began, including a
/// retarget: set it to now and the curve continues from the current value.
pub fn wall_progress(spec: MotionSpec, started: Instant, now: Instant) -> f32 {
    let total = spec.total().as_secs_f32();
    if total <= 0.0 {
        return 1.0;
    }
    let raw = now.saturating_duration_since(started).as_secs_f32() / total;
    spec.progress(raw.min(1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ease_out_leads_linear_halfway_through_a_tab_slide() {
        let mid = TAB_SLIDE.progress(0.5);
        assert!(mid > 0.5, "ease-out should be ahead of linear, got {mid}");
        assert!(mid < 1.0);
    }

    #[test]
    fn a_delay_holds_progress_at_zero() {
        let spec = FADE_IN.with_delay(500);
        assert_eq!(spec.progress(0.0), 0.0);
        assert!(spec.progress(0.4) < 0.05);
        assert_eq!(spec.progress(1.0), 1.0);
    }

    #[test]
    fn the_pulse_lease_is_thirty_frames() {
        let interval = Lease::PULSE.interval();
        assert!(interval.as_millis().abs_diff(33) <= 1);
    }

    #[test]
    fn wall_progress_finishes_when_the_timeline_does() {
        let start = Instant::now();
        assert_eq!(wall_progress(TAB_SLIDE, start, start), 0.0);
        let end = start + TAB_SLIDE.total();
        assert_eq!(wall_progress(TAB_SLIDE, start, end), 1.0);
    }
}
