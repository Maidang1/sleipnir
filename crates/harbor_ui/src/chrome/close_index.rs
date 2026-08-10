//! Pure close-tab index adjustment (no window required).

/// Compute the new active index after removing `closed` from a list of `len` tabs
/// that currently has `active` selected.
///
/// Returns `None` if the list would be empty (caller should open a replacement tab).
/// Returns `Some(new_active)` when at least one tab remains.
pub fn active_after_close(active: usize, closed: usize, len: usize) -> Option<usize> {
    if closed >= len || len == 0 {
        return if len == 0 {
            None
        } else {
            Some(active.min(len - 1))
        };
    }
    if len == 1 {
        return None;
    }
    let new_len = len - 1;
    let mut new_active = active;
    if new_active > closed {
        new_active -= 1;
    } else if new_active >= new_len {
        new_active = new_len - 1;
    }
    Some(new_active)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_before_active_decrements() {
        // tabs [0,1,2], active=2, close 0 → active becomes 1 (was former 2)
        assert_eq!(active_after_close(2, 0, 3), Some(1));
    }

    #[test]
    fn close_after_active_keeps_active() {
        assert_eq!(active_after_close(0, 2, 3), Some(0));
    }

    #[test]
    fn close_active_middle_stays_at_index() {
        // [0,1,2] active=1 close 1 → still index 1 (former 2)
        assert_eq!(active_after_close(1, 1, 3), Some(1));
    }

    #[test]
    fn close_active_last_clamps() {
        // [0,1,2] active=2 close 2 → active becomes 1
        assert_eq!(active_after_close(2, 2, 3), Some(1));
    }

    #[test]
    fn close_only_tab_returns_none() {
        assert_eq!(active_after_close(0, 0, 1), None);
    }

    #[test]
    fn close_active_first_of_two() {
        // [0,1] active=0 close 0 → active 0 (former 1)
        assert_eq!(active_after_close(0, 0, 2), Some(0));
    }
}
