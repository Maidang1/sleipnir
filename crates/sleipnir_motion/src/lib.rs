//! Named motion: a CSS cubic-bezier and the one curve the chrome uses.
//!
//! One call site in the workspace drives this crate — the tab activation slide
//! — and it wants a one-shot [`gpui::Animation`], which is what
//! [`MotionSpec::animation`] is for. There is no repeating lease and no
//! wall-clock tween here: nothing to drive them.

use std::time::Duration;

use gpui::Animation;

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
        let (mut lo, mut hi) = (0.0_f32, 1.0);
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

pub const EASE_OUT: CubicBezier = CubicBezier::new(0.0, 0.0, 0.58, 1.0);

/// One catalog entry: how long a tween runs, and the curve it follows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionSpec {
    pub duration_ms: u64,
    pub curve: CubicBezier,
}

impl MotionSpec {
    pub const fn new(duration_ms: u64, curve: CubicBezier) -> Self {
        Self { duration_ms, curve }
    }

    pub fn total(&self) -> Duration {
        Duration::from_millis(self.duration_ms)
    }

    /// Eased progress for a raw timeline delta across [`Self::total`].
    pub fn progress(&self, raw_delta: f32) -> f32 {
        if self.duration_ms == 0 {
            return 1.0;
        }
        self.curve.eval(raw_delta.clamp(0.0, 1.0))
    }

    /// One-shot drive: hand this to `with_animation`.
    pub fn animation(&self) -> Animation {
        let spec = *self;
        Animation::new(spec.total()).with_easing(move |delta| spec.progress(delta))
    }
}

/// Tab reorder and activation slide: 150ms ease-out.
pub const TAB_SLIDE: MotionSpec = MotionSpec::new(150, EASE_OUT);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ease_out_leads_linear_halfway_through_a_tab_slide() {
        let mid = TAB_SLIDE.progress(0.5);
        assert!(mid > 0.5, "ease-out should be ahead of linear, got {mid}");
        assert!(mid < 1.0);
    }
}
