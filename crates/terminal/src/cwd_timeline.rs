use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
struct Entry {
    /// Line offset in the retained scrollback buffer.
    scrollback_position: i32,
    working_directory: PathBuf,
}

/// Associates working-directory changes with retained scrollback lines.
///
/// The timeline owns command-boundary capture and lookup fallback rules. The
/// terminal supplies only the current line/history coordinates and the current
/// process directory; callers do not manipulate entries directly.
#[derive(Clone, Debug, Default)]
pub(crate) struct CwdTimeline {
    entries: Vec<Entry>,
    pending_boundary: Option<i32>,
}

impl CwdTimeline {
    pub(crate) fn new(initial: Option<PathBuf>) -> Self {
        let mut timeline = Self::default();
        timeline.reset(initial);
        timeline
    }

    pub(crate) fn mark_boundary(&mut self, line: i32, history_size: usize) {
        self.pending_boundary = Some(scrollback_position(line, history_size));
    }

    pub(crate) fn record(
        &mut self,
        working_directory: PathBuf,
        current_line: i32,
        history_size: usize,
    ) {
        let scrollback_position = self
            .pending_boundary
            .take()
            .unwrap_or_else(|| scrollback_position(current_line, history_size));
        self.entries.push(Entry {
            scrollback_position,
            working_directory,
        });
    }

    pub(crate) fn reset(&mut self, current: Option<PathBuf>) {
        self.pending_boundary = None;
        self.entries = current
            .map(|working_directory| {
                vec![Entry {
                    scrollback_position: i32::MIN,
                    working_directory,
                }]
            })
            .unwrap_or_default();
    }

    pub(crate) fn cwd_at_line(
        &self,
        line: i32,
        history_size: usize,
        history_limit: usize,
        current: Option<PathBuf>,
    ) -> Option<PathBuf> {
        // Once the cap is reached, evictions move retained lines without
        // changing history_size, so stored offsets no longer identify lines.
        if self.entries.is_empty() || history_size >= history_limit {
            return current;
        }

        let position = scrollback_position(line, history_size);
        self.entries
            .iter()
            .rev()
            .find(|entry| entry.scrollback_position <= position)
            .map(|entry| entry.working_directory.clone())
            .or(current)
    }
}

fn scrollback_position(line: i32, history_size: usize) -> i32 {
    let history_size = i32::try_from(history_size).unwrap_or(i32::MAX);
    history_size.saturating_add(line)
}

#[cfg(test)]
mod tests {
    use super::CwdTimeline;
    use std::path::PathBuf;

    fn path(value: &str) -> PathBuf {
        PathBuf::from(value)
    }

    #[test]
    fn boundary_is_consumed_by_the_next_directory_change() {
        let mut timeline = CwdTimeline::new(Some(path("/root")));
        timeline.mark_boundary(2, 10);
        timeline.record(path("/child"), 8, 10);

        assert_eq!(timeline.cwd_at_line(1, 10, 100, None), Some(path("/root")));
        assert_eq!(timeline.cwd_at_line(2, 10, 100, None), Some(path("/child")));
        assert!(timeline.pending_boundary.is_none());
    }

    #[test]
    fn record_without_boundary_uses_current_position() {
        let mut timeline = CwdTimeline::new(Some(path("/root")));
        timeline.record(path("/child"), -2, 10);

        assert_eq!(timeline.cwd_at_line(-3, 10, 100, None), Some(path("/root")));
        assert_eq!(
            timeline.cwd_at_line(-2, 10, 100, None),
            Some(path("/child"))
        );
    }

    #[test]
    fn reaching_history_cap_falls_back_to_current_directory() {
        let mut timeline = CwdTimeline::new(Some(path("/root")));
        timeline.record(path("/child"), 0, 2);

        assert_eq!(
            timeline.cwd_at_line(0, 100, 100, Some(path("/live"))),
            Some(path("/live"))
        );
    }

    #[test]
    fn reset_discards_entries_and_pending_boundary() {
        let mut timeline = CwdTimeline::new(Some(path("/root")));
        timeline.mark_boundary(0, 0);
        timeline.record(path("/child"), 0, 0);
        timeline.mark_boundary(1, 0);
        timeline.reset(None);

        assert!(timeline.entries.is_empty());
        assert!(timeline.pending_boundary.is_none());
    }
}
