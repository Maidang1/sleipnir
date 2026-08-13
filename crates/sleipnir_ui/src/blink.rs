//! Cursor blink ease-in-out (M11).

use sleipnir_settings::TerminalBlink;
use std::time::Duration;

/// Half-period of the fade cycle (macOS-ish ~530ms).
pub const BLINK_HALF_PERIOD: Duration = Duration::from_millis(530);

/// Keep the cursor solid for this long after the last keystroke.
pub const BLINK_SOLID_AFTER_INPUT: Duration = Duration::from_millis(200);

/// Compute cursor opacity in `0.0..=1.0` for the paint path.
///
/// - Settings `off` → always solid (1.0)
/// - Settings `on` → always animate after the solid window
/// - Settings `terminal_controlled` → animate only when the app reports blinking
pub fn cursor_blink_alpha(
    elapsed_since_input: Duration,
    terminal_wants_blink: bool,
    settings: TerminalBlink,
) -> f32 {
    let should_animate = match settings {
        TerminalBlink::Off => false,
        TerminalBlink::On => true,
        TerminalBlink::TerminalControlled => terminal_wants_blink,
    };
    if !should_animate {
        return 1.0;
    }
    if elapsed_since_input < BLINK_SOLID_AFTER_INPUT {
        return 1.0;
    }
    let t = (elapsed_since_input - BLINK_SOLID_AFTER_INPUT).as_secs_f32();
    let half = BLINK_HALF_PERIOD.as_secs_f32().max(0.001);
    // Cosine ease-in-out over a full cycle (two half-periods).
    // phase 0 → solid, 0.5 → invisible, 1 → solid.
    let cycle = (t / (half * 2.0)).fract();
    let alpha = 0.5 - 0.5 * (std::f32::consts::PI * 2.0 * cycle).cos();
    alpha.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_is_always_solid() {
        assert_eq!(
            cursor_blink_alpha(Duration::from_secs(10), true, TerminalBlink::Off),
            1.0
        );
    }

    #[test]
    fn solid_right_after_input() {
        assert_eq!(
            cursor_blink_alpha(Duration::from_millis(50), true, TerminalBlink::On),
            1.0
        );
    }

    #[test]
    fn animates_after_solid_window() {
        let a = cursor_blink_alpha(Duration::from_millis(800), true, TerminalBlink::On);
        assert!(a < 1.0, "expected fade mid-cycle, got {a}");
        assert!(a >= 0.0);
    }

    #[test]
    fn terminal_controlled_respects_app_flag() {
        assert_eq!(
            cursor_blink_alpha(
                Duration::from_secs(2),
                false,
                TerminalBlink::TerminalControlled
            ),
            1.0
        );
        let a = cursor_blink_alpha(
            Duration::from_secs(2),
            true,
            TerminalBlink::TerminalControlled,
        );
        assert!(a <= 1.0);
    }
}
