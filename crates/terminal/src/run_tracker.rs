//! Turn OSC 133 markers (and the busy probe, when markers are absent) into
//! Run start/finish facts. Pure state machine — no gpui, no PTY.

use crate::Osc133Kind;

/// Command-line text used when the grid between B and C is empty or unreadable.
pub const UNRECOGNIZED_COMMAND: &str = "(无法识别的命令)";

/// Max characters kept from the first line of a command (ellipsis not included).
const MAX_COMMAND_CHARS: usize = 256;

/// What the tracker reports to the terminal (which turns it into a gpui Event).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrackerOut {
    Started {
        command: String,
        inferred: bool,
        at_ms: u64,
    },
    Finished {
        exit_code: Option<i32>,
        at_ms: u64,
    },
}

#[derive(Default)]
pub struct RunTracker {
    running: bool,
    /// True while the current Run came from OSC 133 (blocks the busy fallback).
    from_osc133: bool,
    /// Last produced fact, drained by the PTY path that has a `cx`.
    pending: Option<TrackerOut>,
}

impl RunTracker {
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Drain the last produced fact. Display-only paths never call this, so they
    /// never emit a Run even though they still feed the tracker.
    pub fn take_output(&mut self) -> Option<TrackerOut> {
        self.pending.take()
    }

    pub fn on_marker(&mut self, kind: Osc133Kind, at_ms: u64) -> Option<TrackerOut> {
        match kind {
            Osc133Kind::CommandExecuted => self.on_marker_with_command(kind, at_ms, None),
            Osc133Kind::CommandFinished { status } => self.finish(status, at_ms),
            Osc133Kind::PromptStart | Osc133Kind::CommandStart => None,
        }
    }

    pub fn on_marker_with_command(
        &mut self,
        kind: Osc133Kind,
        at_ms: u64,
        command: Option<String>,
    ) -> Option<TrackerOut> {
        match kind {
            Osc133Kind::CommandExecuted => {
                self.running = true;
                self.from_osc133 = true;
                self.push(TrackerOut::Started {
                    command: normalize_command(command.as_deref()),
                    inferred: false,
                    at_ms,
                })
            }
            other => self.on_marker(other, at_ms),
        }
    }

    /// `busy` comes from `terminal_looks_busy`; `command` from
    /// `foreground_process_command_name`. Ignored while an OSC 133 Run is live.
    pub fn on_busy_change(
        &mut self,
        busy: bool,
        command: Option<String>,
        at_ms: u64,
    ) -> Option<TrackerOut> {
        if self.from_osc133 {
            return None;
        }
        match (self.running, busy) {
            (false, true) => {
                self.running = true;
                self.from_osc133 = false;
                self.push(TrackerOut::Started {
                    command: normalize_command(command.as_deref()),
                    inferred: true,
                    at_ms,
                })
            }
            (true, false) => self.finish(None, at_ms),
            _ => None,
        }
    }

    fn finish(&mut self, exit_code: Option<i32>, at_ms: u64) -> Option<TrackerOut> {
        if !self.running {
            return None;
        }
        self.running = false;
        self.from_osc133 = false;
        self.push(TrackerOut::Finished { exit_code, at_ms })
    }

    fn push(&mut self, out: TrackerOut) -> Option<TrackerOut> {
        self.pending = Some(out.clone());
        Some(out)
    }
}

/// First line, trimmed. Empty → placeholder. Multiline or over-long → `…`.
///
/// This is display text for the tracker, not a security boundary; redaction
/// happens later in `run_ledger`.
pub fn normalize_command(raw: Option<&str>) -> String {
    let Some(raw) = raw else {
        return UNRECOGNIZED_COMMAND.to_string();
    };
    let mut lines = raw.lines();
    let first = lines.next().unwrap_or("").trim();
    if first.is_empty() {
        return UNRECOGNIZED_COMMAND.to_string();
    }
    let multiline = lines.next().is_some();
    let char_count = first.chars().count();
    if char_count > MAX_COMMAND_CHARS {
        let mut out: String = first.chars().take(MAX_COMMAND_CHARS - 1).collect();
        out.push('…');
        out
    } else if multiline {
        format!("{first}…")
    } else {
        first.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{RunTracker, TrackerOut, UNRECOGNIZED_COMMAND, normalize_command};
    use crate::Osc133Kind::*;

    #[test]
    fn c_then_d_yields_start_then_finish() {
        let mut t = RunTracker::default();
        assert_eq!(t.on_marker(CommandStart, 10), None);
        assert!(matches!(
            t.on_marker_with_command(CommandExecuted, 11, Some("cargo build".into())),
            Some(TrackerOut::Started { .. })
        ));
        assert!(matches!(
            t.on_marker(CommandFinished { status: Some(0) }, 40_000),
            Some(TrackerOut::Finished {
                exit_code: Some(0),
                ..
            })
        ));
    }

    #[test]
    fn d_without_c_is_dropped() {
        let mut t = RunTracker::default();
        assert_eq!(t.on_marker(CommandFinished { status: Some(0) }, 10), None);
        assert!(!t.is_running());
        assert!(t.take_output().is_none());
    }

    #[test]
    fn empty_command_text_falls_back_to_placeholder() {
        let mut t = RunTracker::default();
        match t.on_marker_with_command(CommandExecuted, 0, None) {
            Some(TrackerOut::Started {
                command, inferred, ..
            }) => {
                assert_eq!(command, UNRECOGNIZED_COMMAND);
                assert!(!inferred);
            }
            other => panic!("expected Started, got {other:?}"),
        }
        match t.on_marker_with_command(CommandExecuted, 1, Some("   \n".into())) {
            Some(TrackerOut::Started { command, .. }) => {
                assert_eq!(command, UNRECOGNIZED_COMMAND);
            }
            other => panic!("expected Started, got {other:?}"),
        }
    }

    #[test]
    fn multiline_command_keeps_first_line_with_ellipsis() {
        assert_eq!(
            normalize_command(Some("for i in 1 2 3\ndo echo $i\ndone")),
            "for i in 1 2 3…"
        );
        let mut t = RunTracker::default();
        match t.on_marker_with_command(
            CommandExecuted,
            0,
            Some("for i in 1 2 3\ndo echo $i\ndone".into()),
        ) {
            Some(TrackerOut::Started { command, .. }) => {
                assert_eq!(command, "for i in 1 2 3…");
            }
            other => panic!("expected Started, got {other:?}"),
        }
    }

    #[test]
    fn busy_probe_only_fires_when_osc133_is_silent() {
        let mut t = RunTracker::default();
        assert!(
            t.on_marker_with_command(CommandExecuted, 1, Some("cargo build".into()))
                .is_some()
        );
        assert!(
            t.on_busy_change(true, Some("cargo".into()), 2).is_none(),
            "busy probe must not start a second Run while OSC 133 is live"
        );
        assert!(
            t.on_busy_change(false, None, 3).is_none(),
            "busy going idle must not finish an OSC 133 Run"
        );
        assert!(t.is_running());
    }

    #[test]
    fn busy_probe_start_and_stop_are_inferred() {
        let mut t = RunTracker::default();
        match t.on_busy_change(true, Some("vim".into()), 10) {
            Some(TrackerOut::Started {
                inferred,
                command,
                at_ms,
            }) => {
                assert!(inferred);
                assert_eq!(command, "vim");
                assert_eq!(at_ms, 10);
            }
            other => panic!("expected inferred Started, got {other:?}"),
        }
        match t.on_busy_change(false, None, 20) {
            Some(TrackerOut::Finished { exit_code, at_ms }) => {
                assert_eq!(exit_code, None);
                assert_eq!(at_ms, 20);
            }
            other => panic!("expected Finished, got {other:?}"),
        }
        assert!(!t.is_running());
    }
}
