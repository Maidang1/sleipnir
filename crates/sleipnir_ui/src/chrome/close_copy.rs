//! Close-confirm copy. Pure so tests do not need GPUI.

/// Message for a busy-pane close confirm.
///
/// A known foreground process is named; otherwise the generic sentence stays.
pub fn close_confirm_message(process_name: Option<&str>) -> String {
    match process_name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => format!("{name} is still running. Closing ends it."),
        None => "A process is still running. Close this pane anyway?".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_foreground_process_when_known() {
        let message = close_confirm_message(Some("claude"));
        assert!(
            message.contains("claude"),
            "expected process name in {message:?}"
        );
        assert!(
            !message.starts_with("A process is still running"),
            "generic sentence must not be the only copy when a name is known: {message:?}"
        );
    }

    #[test]
    fn falls_back_when_name_is_missing_or_blank() {
        assert_eq!(
            close_confirm_message(None),
            "A process is still running. Close this pane anyway?"
        );
        assert_eq!(
            close_confirm_message(Some("")),
            "A process is still running. Close this pane anyway?"
        );
        assert_eq!(
            close_confirm_message(Some("   ")),
            "A process is still running. Close this pane anyway?"
        );
    }
}
