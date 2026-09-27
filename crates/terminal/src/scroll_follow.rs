//! Stick to the bottom of the scrollback while the user leaves the view there.
//!
//! Appended output and a user scroll both show up as a new display offset.
//! The overflow (`history_size`, the furthest the viewport can move) is what
//! separates them: if it changed, the content grew and the pin stays as the
//! user last set it; if it did not, the offset moved because the user moved
//! it, and sitting on the bottom is what re-pins.

/// Rows from the bottom that still count as following.
///
/// `display_offset` is a whole row, so the slack is zero: anything above the
/// last row is a deliberate scroll. The gpui analogue uses a few pixels
/// because its offset is fractional.
pub const FOLLOW_SLACK_LINES: usize = 0;

/// Whether `offset` is at the end of the scrollback, within `slack` rows.
///
/// `max_offset` is the overflow (history length). `offset` is the grid
/// display offset: `0` is the bottom, and larger values are scrolled up.
/// Content that fits is always at the bottom — there is nowhere else to be.
pub fn at_bottom(max_offset: usize, offset: usize, slack: usize) -> bool {
    if max_offset == 0 {
        return true;
    }
    offset <= slack
}

/// Whether the viewport is still pinned, and the overflow it last saw.
#[derive(Clone, Debug)]
pub struct ScrollFollow {
    pinned: bool,
    last_max: usize,
    /// [`Self::follow`] asked for a snap on the next observe, before a stable
    /// overflow would be read as a user scroll and clear the pin.
    pending: bool,
}

impl Default for ScrollFollow {
    fn default() -> Self {
        // A transcript opens on its newest line.
        Self {
            pinned: true,
            last_max: 0,
            pending: false,
        }
    }
}

impl ScrollFollow {
    pub fn following(&self) -> bool {
        self.pinned
    }

    /// Re-pin. The next observe snaps to the bottom if the view is not there.
    pub fn follow(&mut self) {
        self.pinned = true;
        self.pending = true;
    }

    /// Update the pin from this frame's scrollback.
    ///
    /// Returns whether the viewport should be forced to the bottom. Content
    /// growth keeps the existing pin; a still overflow means the offset change
    /// was the user's, and only the bottom re-attaches.
    pub fn observe(&mut self, max_offset: usize, offset: usize) -> bool {
        if self.pending {
            self.pending = false;
            self.pinned = true;
            self.last_max = max_offset;
            return !at_bottom(max_offset, offset, FOLLOW_SLACK_LINES);
        }
        let content_changed = max_offset.abs_diff(self.last_max) > 0;
        if !content_changed {
            self.pinned = at_bottom(max_offset, offset, FOLLOW_SLACK_LINES);
        }
        self.last_max = max_offset;
        self.pinned && !at_bottom(max_offset, offset, FOLLOW_SLACK_LINES)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_growth_keeps_the_pin_and_snaps_when_it_was_set() {
        let mut follow = ScrollFollow::default();
        assert!(!follow.observe(10, 0));
        assert!(follow.following());

        // History grew and the offset is no longer the bottom. The pin was
        // already set, so this frame has to catch up.
        assert!(follow.observe(30, 8));
        assert!(follow.following());

        // The snap landed. Same overflow, offset at the bottom: stay pinned.
        assert!(!follow.observe(30, 0));
        assert!(follow.following());
    }

    #[test]
    fn a_user_scroll_up_releases_and_scrolling_back_re_pins() {
        let mut follow = ScrollFollow::default();
        assert!(!follow.observe(40, 0));

        assert!(!follow.observe(40, 6));
        assert!(!follow.following());

        // More output while unpinned moves the offset with the history.
        // That must not look like the user returning to the bottom.
        assert!(!follow.observe(55, 21));
        assert!(!follow.following());

        assert!(!follow.observe(55, 0));
        assert!(follow.following());

        // Jump-to-latest re-pins even when the offset has not reached bottom yet.
        assert!(!follow.observe(55, 4));
        assert!(!follow.following());
        follow.follow();
        assert!(follow.following());
        assert!(follow.observe(55, 4));
    }

    #[test]
    fn content_that_fits_is_at_the_bottom() {
        assert!(at_bottom(0, 0, FOLLOW_SLACK_LINES));
        let mut follow = ScrollFollow::default();
        assert!(!follow.observe(0, 0));
        assert!(follow.following());
    }
}
