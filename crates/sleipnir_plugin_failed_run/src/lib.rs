//! Decision logic for the failed-run example plugin.
//!
//! Kept as a lib so the Block tree can be tested without speaking the wire
//! protocol. `main` is a thin [`sleipnir_plugin::v2::run`] wrapper.

use sleipnir_plugin::v2::{Tone, Widget, badge, btn, col, row, text};

/// First tree: redacted command, exit, duration, Retry button.
///
/// `arg` on the button is the `RunId` so the Action can re-render the same
/// Block without the plugin inventing host-assigned `BlockId`s.
pub fn failure_tree(command: &str, exit_code: i32, duration_ms: u64, run_id: &str) -> Widget {
    col()
        .gap(1)
        .child(
            row()
                .gap(2)
                .child(badge("failed", Tone::Err))
                .child(text(command).bold()),
        )
        .child(
            row()
                .gap(2)
                .child(text(format!("exit {exit_code}")).tone(Tone::Err))
                .child(text(format!("{duration_ms}ms")).tone(Tone::Dim)),
        )
        .child(btn("Retry", "retry").arg(run_id))
        .into()
}

/// Same Block after Retry: the tree must *look* different so the
/// Render → click → Action → re-Render loop is visible by eye.
pub fn retried_tree(command: &str, exit_code: i32, duration_ms: u64) -> Widget {
    col()
        .gap(1)
        .child(
            row()
                .gap(2)
                .child(badge("queued", Tone::Warn))
                .child(text(command).bold()),
        )
        .child(
            row()
                .gap(2)
                .child(text(format!("exit {exit_code}")).tone(Tone::Dim))
                .child(text(format!("{duration_ms}ms")).tone(Tone::Dim)),
        )
        .child(text("Retry requested — run the command again from the shell.").tone(Tone::Accent))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sleipnir_plugin::v2::Widget;

    #[test]
    fn failure_tree_has_a_retry_button() {
        let tree = failure_tree("git push", 1, 1500, "run-1");
        assert!(contains_btn(&tree, "Retry", "retry"));
        assert!(!contains_text(&tree, "Retry requested"));
    }

    #[test]
    fn retried_tree_is_visibly_different() {
        let before = failure_tree("git push", 1, 1500, "run-1");
        let after = retried_tree("git push", 1, 1500);
        assert_ne!(before, after);
        assert!(!contains_btn(&after, "Retry", "retry"));
        assert!(contains_text(&after, "Retry requested"));
    }

    fn contains_btn(tree: &Widget, label: &str, action: &str) -> bool {
        match tree {
            Widget::Btn { s, action: a, .. } => s == label && a == action,
            Widget::Col { children, .. } | Widget::Row { children, .. } => {
                children.iter().any(|c| contains_btn(c, label, action))
            }
            _ => false,
        }
    }

    fn contains_text(tree: &Widget, needle: &str) -> bool {
        match tree {
            Widget::Text { s, .. } => s.contains(needle),
            Widget::Col { children, .. } | Widget::Row { children, .. } => {
                children.iter().any(|c| contains_text(c, needle))
            }
            _ => false,
        }
    }
}
