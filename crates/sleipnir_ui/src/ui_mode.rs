//! Window UI modes and focused-pane facts state.
//!
//! At most one modal overlay is open at a time, and `OverlayKind` makes that a
//! property of the type rather than something the callers have to maintain. The
//! find bar and quick-select remain independent transient modes because they
//! intentionally coexist with normal terminal content.
//!
//! Note that the confirm dialog (`AppShell::close_confirm`) is a third modal
//! surface that lives outside this enum and can coexist with any overlay; the
//! key-down chain gives it priority.

use crate::chrome::pane_facts::PaneFacts;
use run_ledger::PaneKey;
use std::time::{Duration, Instant};

/// The modal overlay currently on screen, if any.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum OverlayKind {
    #[default]
    None,
    Settings,
    Update,
    Palette,
    PaneFacts,
    RunLedger,
    History,
    Diff,
}

#[derive(Default)]
pub(crate) struct UiMode {
    pub overlay: OverlayKind,
    pub find_open: bool,
    pub quick_select_open: bool,
}

impl UiMode {
    pub fn is(&self, overlay: OverlayKind) -> bool {
        self.overlay == overlay
    }

    /// Show `overlay`, replacing whatever was open.
    ///
    /// Idempotent: re-opening the overlay that is already on screen changes
    /// nothing, so a refresh path can call this without disturbing the find bar.
    /// Use `close` to dismiss; `OverlayKind::None` is not a thing to "open".
    pub fn open(&mut self, overlay: OverlayKind) {
        debug_assert_ne!(
            overlay,
            OverlayKind::None,
            "use close()/close_any() to dismiss an overlay"
        );
        if self.overlay == overlay {
            return;
        }
        self.overlay = overlay;
        // A newly opened overlay takes the keyboard, so the find bar goes away.
        self.find_open = false;
    }

    /// Toggle `overlay`, returning whether it is now open.
    pub fn toggle(&mut self, overlay: OverlayKind) -> bool {
        if self.overlay == overlay {
            self.overlay = OverlayKind::None;
            false
        } else {
            self.open(overlay);
            true
        }
    }

    /// Close `overlay` if it is the one on screen. Returns whether it closed.
    pub fn close(&mut self, overlay: OverlayKind) -> bool {
        if self.overlay != overlay {
            return false;
        }
        self.overlay = OverlayKind::None;
        true
    }

    /// Show the find bar. Find is not a modal overlay, but it takes the
    /// keyboard, so any open overlay closes first.
    pub fn open_find(&mut self) {
        self.overlay = OverlayKind::None;
        self.find_open = true;
    }

    pub fn close_find(&mut self) {
        self.find_open = false;
    }

    pub fn toggle_quick_select(&mut self) -> bool {
        self.quick_select_open = !self.quick_select_open;
        self.quick_select_open
    }
}

/// How long a facts snapshot stays current before the panel collects again.
pub(crate) const PANE_FACTS_MAX_AGE: Duration = Duration::from_secs(1);

/// Async collection state for the focused-pane facts panel.
///
/// `pane` is part of the identity of a result, not decoration: focus can move
/// to a different pane while a collection is in flight, and rendering that
/// result would show one pane's process tree under another pane's heading.
///
/// The in-flight flag and the snapshot timestamp both live here rather than in
/// sibling fields, because both are only meaningful relative to a particular
/// snapshot: "is a collection running" and "how old is what's on screen" cannot
/// drift out of sync with the data they describe.
#[derive(Default)]
pub(crate) enum PaneFactsState {
    #[default]
    Idle,
    /// First collection for `pane`; nothing to show yet.
    Loading { pane: PaneKey },
    Ready {
        pane: PaneKey,
        facts: PaneFacts,
        /// When this snapshot landed, for the refresh poll.
        at: Instant,
        /// A refresh for `pane` is in flight; `facts` stays on screen.
        refreshing: bool,
    },
}

impl PaneFactsState {
    /// The cached snapshot, but only when it belongs to `pane`.
    pub fn facts_for(&self, pane: PaneKey) -> Option<&PaneFacts> {
        match self {
            Self::Ready {
                pane: ready_pane,
                facts,
                ..
            } if *ready_pane == pane => Some(facts),
            _ => None,
        }
    }

    /// True when a collection for `pane` is already in flight. Render runs every
    /// frame, so without this the panel would queue a background collection per
    /// frame until the first result lands. Unlike a bare "is Loading" check this
    /// also covers refreshes behind an existing snapshot.
    pub fn is_collecting_for(&self, pane: PaneKey) -> bool {
        match self {
            Self::Loading { pane: loading } => *loading == pane,
            Self::Ready {
                pane: ready_pane,
                refreshing,
                ..
            } => *refreshing && *ready_pane == pane,
            Self::Idle => false,
        }
    }

    /// True when `pane`'s snapshot is missing or older than `max_age`, so the
    /// panel should collect again.
    pub fn needs_refresh_for(&self, pane: PaneKey, max_age: Duration) -> bool {
        match self {
            Self::Ready {
                pane: ready_pane,
                at,
                ..
            } if *ready_pane == pane => at.elapsed() >= max_age,
            // Idle, Loading, or a snapshot belonging to another pane: there is
            // nothing current to show for `pane`.
            _ => true,
        }
    }

    /// Mark the start of a collection for `pane`, keeping any snapshot that is
    /// already on screen so the panel does not flicker between polls.
    pub fn begin_collection(&mut self, pane: PaneKey) {
        match self {
            Self::Ready {
                pane: ready_pane,
                refreshing,
                ..
            } if *ready_pane == pane => *refreshing = true,
            _ => *self = Self::Loading { pane },
        }
    }

    /// Store a landed snapshot for `pane`.
    pub fn finish_collection(&mut self, pane: PaneKey, facts: PaneFacts) {
        *self = Self::Ready {
            pane,
            facts,
            at: Instant::now(),
            refreshing: false,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modal_open_replaces_previous_overlay() {
        let mut mode = UiMode::default();
        mode.open(OverlayKind::Settings);
        mode.open(OverlayKind::Diff);
        assert!(mode.is(OverlayKind::Diff));
        assert!(!mode.is(OverlayKind::Settings));
    }

    #[test]
    fn find_closes_modal_overlay() {
        let mut mode = UiMode::default();
        mode.open(OverlayKind::Palette);
        mode.open_find();
        assert_eq!(mode.overlay, OverlayKind::None);
        assert!(mode.find_open);
    }

    /// Regression: a diff refresh re-opens the overlay that is already on
    /// screen. That must not close the find bar the user is typing in.
    #[test]
    fn reopening_the_current_overlay_leaves_find_alone() {
        let mut mode = UiMode::default();
        mode.open(OverlayKind::Diff);
        mode.find_open = true;

        mode.open(OverlayKind::Diff);
        assert!(mode.find_open, "re-opening the same overlay is a no-op");
        assert!(mode.is(OverlayKind::Diff));

        // Switching to a *different* overlay does take the keyboard.
        mode.open(OverlayKind::Settings);
        assert!(!mode.find_open);
    }

    #[test]
    fn toggle_closes_only_the_matching_overlay() {
        let mut mode = UiMode::default();
        assert!(mode.toggle(OverlayKind::RunLedger));
        assert!(!mode.toggle(OverlayKind::RunLedger));
        assert_eq!(mode.overlay, OverlayKind::None);
        assert!(!mode.close(OverlayKind::Diff));
    }

    #[test]
    fn quick_select_survives_modal_overlays() {
        let mut mode = UiMode::default();
        assert!(mode.toggle_quick_select());
        mode.open(OverlayKind::Settings);
        assert!(mode.quick_select_open);
    }

    #[test]
    fn facts_are_scoped_to_their_pane() {
        let pane = PaneKey::from_u128(1);
        let other = PaneKey::from_u128(2);
        let mut state = PaneFactsState::default();
        state.finish_collection(pane, PaneFacts::default());
        assert!(state.facts_for(pane).is_some());
        assert!(state.facts_for(other).is_none());
        // A collection still in flight has nothing to show yet.
        assert!(PaneFactsState::Loading { pane }.facts_for(pane).is_none());
        assert!(PaneFactsState::Idle.facts_for(pane).is_none());
    }

    /// Regression: a refresh behind an existing snapshot must still count as
    /// in flight. Render polls every frame, so if this reports "not collecting"
    /// a slow `lsof` walk would stack one collection per frame.
    #[test]
    fn refresh_behind_a_snapshot_is_still_collecting() {
        let pane = PaneKey::from_u128(1);
        let other = PaneKey::from_u128(2);
        let mut state = PaneFactsState::default();

        state.begin_collection(pane);
        assert!(
            state.is_collecting_for(pane),
            "first collection is in flight"
        );

        state.finish_collection(pane, PaneFacts::default());
        assert!(
            !state.is_collecting_for(pane),
            "landed result is not in flight"
        );

        state.begin_collection(pane);
        assert!(
            state.is_collecting_for(pane),
            "a refresh must be visible as in flight even with facts on screen"
        );
        assert!(
            state.facts_for(pane).is_some(),
            "the old snapshot stays on screen while refreshing"
        );
        assert!(!state.is_collecting_for(other));
    }

    #[test]
    fn a_snapshot_is_only_current_for_its_own_pane() {
        let pane = PaneKey::from_u128(1);
        let other = PaneKey::from_u128(2);
        let day = Duration::from_secs(86_400);
        let mut state = PaneFactsState::default();

        assert!(
            state.needs_refresh_for(pane, day),
            "Idle always needs facts"
        );

        state.finish_collection(pane, PaneFacts::default());
        assert!(
            !state.needs_refresh_for(pane, day),
            "a fresh snapshot must not be re-collected"
        );
        assert!(
            state.needs_refresh_for(pane, Duration::ZERO),
            "a zero max-age makes every snapshot stale"
        );
        assert!(
            state.needs_refresh_for(other, day),
            "focus moving to another pane needs a new collection"
        );
    }
}
