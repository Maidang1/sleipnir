//! Viewport-anchored stars with slow, staggered brightness and position breathing.
//! Each star returns to a deterministic anchor, so motion never drifts or jitters.

use gpui::{Bounds, Context, Hsla, Pixels, Task, Window, fill, point, px, size};
use sleipnir_settings::TerminalSettings;
use std::time::{Duration, Instant};

use crate::TermView;

const TILE_SIZE: f32 = 48.0;
const MAX_TILES_PER_AXIS: usize = 256;
const MAX_STARS: usize = 2048;
const STAR_DIAMETER_SCALE: f32 = 1.5;
const FRAME_INTERVAL: Duration = Duration::from_millis(50);
const MIN_BRIGHTNESS: f32 = 0.28;
const MIN_DRIFT: f32 = 0.8;
const MAX_DRIFT: f32 = 2.4;
const STAR_MARGIN: f32 = MAX_DRIFT + 2.0;

#[derive(Default)]
struct Clock {
    elapsed: Duration,
    last_frame: Option<Instant>,
}

impl Clock {
    fn sample(&mut self, now: Instant, animate: bool) -> Duration {
        if animate && let Some(previous) = self.last_frame {
            let delta = now.saturating_duration_since(previous);
            // Hidden tabs/panes do not render. Do not jump forward when they
            // return, or after a stalled event loop.
            if delta <= FRAME_INTERVAL * 4 {
                self.elapsed += delta;
            }
        }
        self.last_frame = animate.then_some(now);
        self.elapsed
    }
}

fn should_animate(enabled: bool, active: bool, reduce_motion: bool) -> bool {
    enabled && active && !reduce_motion
}

#[derive(Default)]
pub(crate) struct Animation {
    clock: Clock,
    pending_frame: Option<Task<()>>,
}

impl Animation {
    pub(crate) fn frame(
        &mut self,
        enabled: bool,
        window: &Window,
        cx: &mut Context<TermView>,
    ) -> Duration {
        let animate = should_animate(enabled, window.is_window_active(), cx.reduce_motion());
        let elapsed = self.clock.sample(Instant::now(), animate);
        if !animate {
            self.pending_frame = None;
        } else if self.pending_frame.is_none() {
            // One outstanding, one-shot wakeup per visible pane, not a
            // detached loop. Only rendering can re-arm it: hidden tabs stop,
            // dropped views cancel it, and cursor/PTY redraws cannot stack it.
            self.pending_frame = Some(cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(FRAME_INTERVAL).await;
                this.update_in(cx, |this, window, cx| {
                    this.starfield_animation.pending_frame = None;
                    if should_animate(
                        TerminalSettings::get_global(cx).starfield,
                        window.is_window_active(),
                        cx.reduce_motion(),
                    ) {
                        cx.notify();
                    }
                })
                .ok();
            }));
        }
        elapsed
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Star {
    x: f32,
    y: f32,
    diameter: f32,
    opacity: f32,
    opacity_phase: f64,
    opacity_period_secs: f64,
    motion_phase: f64,
    motion_period_secs: f64,
    drift_x: f32,
    drift_y: f32,
}

impl Star {
    fn opacity_at(self, elapsed: Duration) -> f32 {
        // Cosine easing has zero velocity at each turning point. Independent
        // phases and 5-9 second periods avoid a synchronized flashing sheet.
        let cycle = (elapsed.as_secs_f64() % self.opacity_period_secs) / self.opacity_period_secs
            + self.opacity_phase;
        let breath = (0.5 - 0.5 * (std::f64::consts::TAU * cycle).cos()) as f32;
        self.opacity * (MIN_BRIGHTNESS + (1.0 - MIN_BRIGHTNESS) * breath)
    }

    fn position_at(self, elapsed: Duration) -> (f32, f32) {
        // A cosine moves each star out and back along its own short vector.
        // It slows to zero at both ends, which reads as breathing rather than
        // linear sliding. The anchor never changes, so there is no random walk.
        let cycle = (elapsed.as_secs_f64() % self.motion_period_secs) / self.motion_period_secs
            + self.motion_phase;
        let breath = (std::f64::consts::TAU * cycle).cos() as f32;
        (
            self.x + self.drift_x * breath,
            self.y + self.drift_y * breath,
        )
    }

    fn fits(self, width: f32, height: f32) -> bool {
        let extent_x = self.drift_x.abs();
        let extent_y = self.drift_y.abs();
        self.x - extent_x >= 0.0
            && self.y - extent_y >= 0.0
            && self.x + extent_x + self.diameter <= width
            && self.y + extent_y + self.diameter <= height
    }
}

fn hash(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^ (value >> 31)
}

fn tile_count(length: f32) -> usize {
    if !length.is_finite() || length <= 0.0 {
        return 0;
    }
    ((length / TILE_SIZE).ceil() as usize).min(MAX_TILES_PER_AXIS)
}

fn stars(width: f32, height: f32, seed: u64) -> impl Iterator<Item = Star> {
    let cols = tile_count(width);
    let rows = tile_count(height);
    (0..rows)
        .flat_map(move |row| (0..cols).map(move |col| (col, row)))
        .filter_map(move |(col, row)| {
            let bits = hash(seed ^ ((row as u64) << 32) ^ col as u64 ^ 0x9e3779b97f4a7c15);
            // Leave some tiles empty so the field does not read as a grid.
            if bits & 3 == 0 {
                return None;
            }
            let unit = |shift: u32| ((bits >> shift) & 0xffff_u64) as f32 / 65535.0;
            let bright = (bits >> 4) & 7 == 0;
            let diameter = (if bright { 1.6 } else { 0.7 + unit(40) * 0.5 }) * STAR_DIAMETER_SCALE;
            let motion = hash(bits ^ 0xd1b54a32d192ed03);
            let motion_unit = |shift: u32| ((motion >> shift) & 0xffff) as f64 / 65535.0;
            let angle = std::f64::consts::TAU * motion_unit(32);
            let drift = MIN_DRIFT as f64 + (MAX_DRIFT - MIN_DRIFT) as f64 * motion_unit(48);
            let star = Star {
                x: col as f32 * TILE_SIZE + STAR_MARGIN + unit(8) * (TILE_SIZE - STAR_MARGIN * 2.0),
                y: row as f32 * TILE_SIZE
                    + STAR_MARGIN
                    + unit(24) * (TILE_SIZE - STAR_MARGIN * 2.0),
                diameter,
                opacity: if bright { 0.38 } else { 0.10 + unit(40) * 0.14 },
                opacity_phase: (motion & 0xffff) as f64 / 65536.0,
                opacity_period_secs: 5.0 + motion_unit(16) * 4.0,
                motion_phase: motion_unit(24),
                motion_period_secs: 7.0 + motion_unit(8) * 6.0,
                drift_x: (angle.cos() * drift) as f32,
                drift_y: (angle.sin() * drift) as f32,
            };
            star.fits(width, height).then_some(star)
        })
        .take(MAX_STARS)
}

/// Called after app-supplied cell backgrounds, before selection, search, glyphs,
/// plugin blocks, and cursor. The caller owns the terminal's content mask.
pub(crate) fn paint(
    bounds: Bounds<Pixels>,
    seed: u64,
    color: Hsla,
    elapsed: Duration,
    window: &mut Window,
) {
    for star in stars(bounds.size.width.into(), bounds.size.height.into(), seed) {
        let (x, y) = star.position_at(elapsed);
        let rect = Bounds::new(
            bounds.origin + point(px(x), px(y)),
            size(px(star.diameter), px(star.diameter)),
        );
        window.paint_quad(
            fill(rect, color.opacity(star.opacity_at(elapsed)))
                .corner_radii(px(star.diameter / 2.0)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_different_per_pane() {
        let first: Vec<_> = stars(1000.0, 600.0, 1).collect();
        assert_eq!(first, stars(1000.0, 600.0, 1).collect::<Vec<_>>());
        assert_ne!(first, stars(1000.0, 600.0, 2).collect::<Vec<_>>());
        assert!((150..250).contains(&first.len()), "sparse, but visible");
        assert!(first.iter().any(|s| s.opacity > 0.3));
        assert!(first.iter().any(|s| s.opacity < 0.2));
    }

    #[test]
    fn resizing_reveals_stars_without_repositioning() {
        let small: Vec<_> = stars(420.0, 280.0, 1).collect();
        let large: Vec<_> = stars(1000.0, 600.0, 1)
            .filter(|s| s.fits(420.0, 280.0))
            .collect();
        assert_eq!(small, large);
    }

    #[test]
    fn stars_stay_inside_the_viewport_and_remain_subtle() {
        for (width, height) in [(1.0, 1.0), (375.0, 250.0), (1024.0, 768.0)] {
            for star in stars(width, height, 42) {
                assert!(
                    (0.7 * STAR_DIAMETER_SCALE..=1.6 * STAR_DIAMETER_SCALE)
                        .contains(&star.diameter)
                );
                assert!((0.1..=0.38).contains(&star.opacity));
                for seconds in [0.0, 1.0, 4.0, 9.0, 27.0] {
                    let (x, y) = star.position_at(Duration::from_secs_f64(seconds));
                    assert!(x >= 0.0 && x + star.diameter <= width);
                    assert!(y >= 0.0 && y + star.diameter <= height);
                }
            }
        }
    }

    #[test]
    fn larger_stars_preserve_the_dim_and_bright_size_tiers() {
        let stars: Vec<_> = stars(1000.0, 600.0, 42).collect();
        let bright: Vec<_> = stars.iter().filter(|star| star.opacity > 0.3).collect();
        let dim: Vec<_> = stars.iter().filter(|star| star.opacity <= 0.3).collect();
        assert!(!bright.is_empty() && !dim.is_empty());
        assert!(
            bright
                .iter()
                .all(|star| (star.diameter - 2.4).abs() < 0.001)
        );
        assert!(
            dim.iter()
                .all(|star| star.diameter >= 1.049 && star.diameter <= 1.801)
        );
    }

    #[test]
    fn invalid_and_extreme_bounds_have_bounded_work() {
        for invalid in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(stars(invalid, 600.0, 1).count(), 0);
            assert_eq!(stars(1000.0, invalid, 1).count(), 0);
        }
        assert_eq!(stars(f32::MAX, f32::MAX, 1).count(), MAX_STARS);
    }

    #[test]
    fn breathing_is_smooth_periodic_and_never_blinks_out() {
        let star = Star {
            x: 0.0,
            y: 0.0,
            diameter: 1.0,
            opacity: 0.38,
            opacity_phase: 0.0,
            opacity_period_secs: 6.0,
            motion_phase: 0.0,
            motion_period_secs: 10.0,
            drift_x: 2.0,
            drift_y: 0.0,
        };
        let at = |secs| star.opacity_at(Duration::from_secs_f64(secs));
        assert!((at(0.0) - star.opacity * MIN_BRIGHTNESS).abs() < 0.0001);
        assert!((at(3.0) - star.opacity).abs() < 0.0001);
        assert!((at(6.0) - at(0.0)).abs() < 0.0001);
        assert!(at(0.0) < at(1.0) && at(1.0) < at(2.0) && at(2.0) < at(3.0));
        assert!(at(3.0) > at(4.0) && at(4.0) > at(5.0) && at(5.0) > at(6.0));
        for step in 0..120 {
            let t = step as f64 * FRAME_INTERVAL.as_secs_f64();
            assert!((at(t + FRAME_INTERVAL.as_secs_f64()) - at(t)).abs() < 0.01);
        }
    }

    #[test]
    fn stars_breathe_at_independent_phases_and_stay_within_original_brightness() {
        let stars: Vec<_> = stars(1000.0, 600.0, 42).collect();
        assert!(
            stars
                .windows(2)
                .any(|s| s[0].opacity_phase != s[1].opacity_phase)
        );
        assert!(
            stars
                .windows(2)
                .any(|s| s[0].opacity_period_secs != s[1].opacity_period_secs)
        );
        let mut brightening = 0;
        let mut dimming = 0;
        for star in stars {
            assert!((5.0..=9.0).contains(&star.opacity_period_secs));
            if star.opacity_at(Duration::from_secs(1)) > star.opacity_at(Duration::ZERO) {
                brightening += 1;
            } else {
                dimming += 1;
            }
            for seconds in [0.0, 1.5, 5.0, 17.0, 86400.0] {
                let opacity = star.opacity_at(Duration::from_secs_f64(seconds));
                assert!(opacity >= star.opacity * MIN_BRIGHTNESS && opacity <= star.opacity);
            }
        }
        assert!(
            brightening > 40 && dimming > 40,
            "stars must not pulse in unison"
        );
    }

    #[test]
    fn position_breathing_is_smooth_periodic_and_returns_to_its_anchor() {
        let star = Star {
            x: 20.0,
            y: 30.0,
            diameter: 1.0,
            opacity: 0.2,
            opacity_phase: 0.0,
            opacity_period_secs: 6.0,
            motion_phase: 0.0,
            motion_period_secs: 8.0,
            drift_x: 2.0,
            drift_y: -1.0,
        };
        let at = |secs| star.position_at(Duration::from_secs_f64(secs));
        assert_eq!(at(0.0), (22.0, 29.0));
        assert_eq!(at(2.0), (20.0, 30.0));
        assert_eq!(at(4.0), (18.0, 31.0));
        assert_eq!(at(6.0), (20.0, 30.0));
        assert_eq!(at(8.0), at(0.0));
        for step in 0..160 {
            let t = step as f64 * FRAME_INTERVAL.as_secs_f64();
            let (x1, y1) = at(t);
            let (x2, y2) = at(t + FRAME_INTERVAL.as_secs_f64());
            assert!((x2 - x1).abs() < 0.09);
            assert!((y2 - y1).abs() < 0.05);
        }
    }

    #[test]
    fn position_breathing_is_staggered_bounded_and_directionally_varied() {
        let stars: Vec<_> = stars(1000.0, 600.0, 42).collect();
        assert!(
            stars
                .windows(2)
                .any(|s| s[0].motion_phase != s[1].motion_phase)
        );
        assert!(
            stars
                .windows(2)
                .any(|s| s[0].motion_period_secs != s[1].motion_period_secs)
        );
        assert!(stars.iter().any(|s| s.drift_x > 0.5));
        assert!(stars.iter().any(|s| s.drift_x < -0.5));
        assert!(stars.iter().any(|s| s.drift_y > 0.5));
        assert!(stars.iter().any(|s| s.drift_y < -0.5));
        for star in stars {
            let drift = star.drift_x.hypot(star.drift_y);
            assert!((MIN_DRIFT - 0.001..=MAX_DRIFT + 0.001).contains(&drift));
            assert!((7.0..=13.0).contains(&star.motion_period_secs));
            assert!(star.fits(1000.0, 600.0));
        }
    }

    #[test]
    fn clock_pauses_when_inactive_and_does_not_catch_up_after_hidden_time() {
        let start = Instant::now();
        let mut clock = Clock::default();
        assert_eq!(clock.sample(start, true), Duration::ZERO);
        assert_eq!(clock.sample(start + FRAME_INTERVAL, true), FRAME_INTERVAL);
        assert_eq!(
            clock.sample(start + FRAME_INTERVAL * 2, false),
            FRAME_INTERVAL
        );
        assert_eq!(
            clock.sample(start + Duration::from_secs(60), true),
            FRAME_INTERVAL
        );
        assert_eq!(
            clock.sample(start + Duration::from_secs(60) + FRAME_INTERVAL, true),
            FRAME_INTERVAL * 2,
        );
        // A hidden pane never samples false, because it does not render.
        assert_eq!(
            clock.sample(start + Duration::from_secs(120), true),
            FRAME_INTERVAL * 2
        );
    }

    #[test]
    fn animation_requires_enabled_active_window_without_reduced_motion() {
        for enabled in [true, false] {
            for active in [true, false] {
                for reduced in [true, false] {
                    assert_eq!(
                        should_animate(enabled, active, reduced),
                        enabled && active && !reduced
                    );
                }
            }
        }
    }
}
