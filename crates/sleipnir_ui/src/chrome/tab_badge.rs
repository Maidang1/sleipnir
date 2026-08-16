//! Tab / pane Run badges: color, label, and elapsed-time formatting.

use gpui::Hsla;
use run_ledger::BadgeKind;
use sleipnir_settings::TerminalPalette;

/// `mm:ss` for a Running badge. Minutes are not wrapped at 60.
pub fn format_elapsed(ms: u64) -> String {
    let total_secs = ms / 1000;
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    format!("{mins}:{secs:02}")
}

/// Icon, plus a count when the badge stands for more than one run.
pub fn badge_label(kind: BadgeKind, count: usize) -> String {
    let icon = match kind {
        BadgeKind::Failed => "✗",
        BadgeKind::Succeeded => "✓",
        BadgeKind::Running => "●",
    };
    if count <= 1 {
        icon.to_string()
    } else {
        format!("{icon}{count}")
    }
}

/// Failed → ANSI red, Running → yellow, Succeeded → green.
pub fn badge_color(kind: BadgeKind, palette: &TerminalPalette) -> Hsla {
    match kind {
        BadgeKind::Failed => palette.ansi[1],
        BadgeKind::Succeeded => palette.ansi[2],
        BadgeKind::Running => palette.ansi[3],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sleipnir_settings::{Appearance, ThemeName, palette_for_theme};

    #[test]
    fn badge_colors_come_from_the_palette() {
        let palette = palette_for_theme(ThemeName::Mocha, Appearance::Dark);
        assert_eq!(badge_color(BadgeKind::Failed, &palette), palette.ansi[1]);
        assert_eq!(badge_color(BadgeKind::Running, &palette), palette.ansi[3]);
        assert_eq!(badge_color(BadgeKind::Succeeded, &palette), palette.ansi[2]);
    }

    #[test]
    fn running_badge_formats_elapsed_as_mm_ss() {
        assert_eq!(format_elapsed(134_000), "2:14");
        assert_eq!(format_elapsed(59_000), "0:59");
        assert_eq!(format_elapsed(3_601_000), "60:01");
    }

    #[test]
    fn count_is_hidden_when_one_and_shown_when_many() {
        assert_eq!(badge_label(BadgeKind::Failed, 1), "✗");
        assert_eq!(badge_label(BadgeKind::Failed, 2), "✗2");
        assert_eq!(badge_label(BadgeKind::Succeeded, 1), "✓");
        assert_eq!(badge_label(BadgeKind::Running, 3), "●3");
    }
}
