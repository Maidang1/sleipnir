//! Window UI modes and focused-pane facts state.
//!
//! [`InputMode`] is the owned keyboard owner on [`crate::app_shell::AppShell`].
//! Opening any owner replaces the previous one, so confirm, consent, menus,
//! rename, find, and a modal overlay cannot coexist. Dismissed overlay is
//! [`InputMode::Terminal`] (or Find), not an `Overlay(None)` sentinel.
//!
//! [`OverlayKind`] names the modal overlays that carry no extra session data.
//! Consent is [`InputMode::Consent`], not an overlay kind.
//!
//! Quick-select is independent: it is designed to coexist with terminal
//! content and does not take the keyboard.

use crate::app_shell::{
    CloseConfirmState, PluginConsentPending, RenameState, TabMenuState, TerminalMenuState,
};
use crate::chrome::pane_facts::PaneFacts;
use run_ledger::PaneKey;
use std::time::{Duration, Instant};

/// Modal overlays that do not carry their own session payload.
///
/// There is no `None` and no `PluginConsent`. Dismissed overlay is
/// [`InputMode::Terminal`]; consent is [`InputMode::Consent`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OverlayKind {
    Settings,
    Update,
    Palette,
    PaneFacts,
    History,
    Diff,
    PluginMonitor,
}

/// Whether the browser panel must hide. Any modal keyboard owner, quick
/// select, or broadcast hides it: the WebView is a surface, never an input mode.
pub(crate) fn browser_is_blocked(input: &InputMode, quick_select: bool, broadcast: bool) -> bool {
    !matches!(input, InputMode::Terminal | InputMode::Find) || quick_select || broadcast
}

/// Independent of [`InputMode`]: quick-select banners the terminal without
/// taking capture-phase keys.
#[derive(Default)]
pub(crate) struct UiMode {
    pub quick_select_open: bool,
}

impl UiMode {
    pub fn toggle_quick_select(&mut self) -> bool {
        self.quick_select_open = !self.quick_select_open;
        self.quick_select_open
    }
}

/// Who currently owns capture-phase keyboard input. Stored as one field on
/// the shell; the payload lives in the matching arm.
#[derive(Default)]
pub(crate) enum InputMode {
    #[default]
    Terminal,
    Confirm(CloseConfirmState),
    Consent(PluginConsentPending),
    TabMenu(TabMenuState),
    TerminalMenu(TerminalMenuState),
    Rename(RenameState),
    Find,
    Overlay(OverlayKind),
}

/// Copy tag of [`InputMode`] for capture-key dispatch. Payloads stay in the
/// owned enum; this exists so a match does not hold a borrow across `&mut self`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InputOwner {
    Terminal,
    Confirm,
    Consent,
    TabMenu,
    TerminalMenu,
    Rename,
    Find,
    Overlay(OverlayKind),
}

/// Generate a `take_*` accessor: if `self` is `$variant`, replace it with
/// `Terminal` and return the owned payload, else `None`. The `matches!` guard
/// makes the `mem::replace` arm total, so the fallback is `unreachable!`.
macro_rules! take_variant {
    ($name:ident, $variant:ident, $payload:ty) => {
        pub fn $name(&mut self) -> Option<$payload> {
            if matches!(self, Self::$variant(_)) {
                match std::mem::replace(self, Self::Terminal) {
                    Self::$variant(state) => Some(state),
                    _ => unreachable!(),
                }
            } else {
                None
            }
        }
    };
}

impl InputMode {
    pub fn owner(&self) -> InputOwner {
        match self {
            Self::Terminal => InputOwner::Terminal,
            Self::Confirm(_) => InputOwner::Confirm,
            Self::Consent(_) => InputOwner::Consent,
            Self::TabMenu(_) => InputOwner::TabMenu,
            Self::TerminalMenu(_) => InputOwner::TerminalMenu,
            Self::Rename(_) => InputOwner::Rename,
            Self::Find => InputOwner::Find,
            Self::Overlay(kind) => InputOwner::Overlay(*kind),
        }
    }

    pub fn is_overlay(&self, kind: OverlayKind) -> bool {
        matches!(self, Self::Overlay(k) if *k == kind)
    }

    pub fn is_find(&self) -> bool {
        matches!(self, Self::Find)
    }

    pub fn confirm(&self) -> Option<&CloseConfirmState> {
        match self {
            Self::Confirm(state) => Some(state),
            _ => None,
        }
    }

    take_variant!(take_confirm, Confirm, CloseConfirmState);

    pub fn consent(&self) -> Option<&PluginConsentPending> {
        match self {
            Self::Consent(pending) => Some(pending),
            _ => None,
        }
    }

    take_variant!(take_consent, Consent, PluginConsentPending);

    pub fn dismiss_consent(&mut self) -> bool {
        if matches!(self, Self::Consent(_)) {
            *self = Self::Terminal;
            true
        } else {
            false
        }
    }

    pub fn tab_menu(&self) -> Option<&TabMenuState> {
        match self {
            Self::TabMenu(state) => Some(state),
            _ => None,
        }
    }

    pub fn tab_menu_mut(&mut self) -> Option<&mut TabMenuState> {
        match self {
            Self::TabMenu(state) => Some(state),
            _ => None,
        }
    }

    take_variant!(take_tab_menu, TabMenu, TabMenuState);

    pub fn dismiss_tab_menu(&mut self) -> bool {
        if matches!(self, Self::TabMenu(_)) {
            *self = Self::Terminal;
            true
        } else {
            false
        }
    }

    pub fn terminal_menu(&self) -> Option<&TerminalMenuState> {
        match self {
            Self::TerminalMenu(state) => Some(state),
            _ => None,
        }
    }

    pub fn terminal_menu_mut(&mut self) -> Option<&mut TerminalMenuState> {
        match self {
            Self::TerminalMenu(state) => Some(state),
            _ => None,
        }
    }

    take_variant!(take_terminal_menu, TerminalMenu, TerminalMenuState);

    pub fn dismiss_terminal_menu(&mut self) -> bool {
        if matches!(self, Self::TerminalMenu(_)) {
            *self = Self::Terminal;
            true
        } else {
            false
        }
    }

    pub fn rename(&self) -> Option<&RenameState> {
        match self {
            Self::Rename(state) => Some(state),
            _ => None,
        }
    }

    pub fn rename_mut(&mut self) -> Option<&mut RenameState> {
        match self {
            Self::Rename(state) => Some(state),
            _ => None,
        }
    }

    take_variant!(take_rename, Rename, RenameState);

    /// Replace the current owner, returning the old mode for teardown.
    pub fn replace(&mut self, next: InputMode) -> InputMode {
        std::mem::replace(self, next)
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
    use crate::app_shell::ConfirmKind;
    use gpui::{point, px};

    fn confirm(tab_id: u64) -> InputMode {
        InputMode::Confirm(CloseConfirmState {
            message: "close?".into(),
            kind: ConfirmKind::CloseTab(tab_id),
        })
    }

    fn tab_menu(tab_id: u64) -> InputMode {
        InputMode::TabMenu(TabMenuState {
            tab_id,
            position: point(px(1.0), px(2.0)),
            selected: 0,
        })
    }

    fn terminal_menu(selected: usize) -> InputMode {
        InputMode::TerminalMenu(TerminalMenuState {
            position: point(px(3.0), px(4.0)),
            link: None,
            selected,
        })
    }

    /// Compile-time pin: `OverlayKind` has no `None` and no `PluginConsent`.
    /// Adding either back is a type error here, not a runtime surprise.
    #[test]
    fn overlay_kind_has_no_none_or_plugin_consent() {
        fn name(kind: OverlayKind) -> &'static str {
            match kind {
                OverlayKind::Settings => "settings",
                OverlayKind::Update => "update",
                OverlayKind::Palette => "palette",
                OverlayKind::PaneFacts => "facts",
                OverlayKind::History => "history",
                OverlayKind::Diff => "diff",
                OverlayKind::PluginMonitor => "monitor",
            }
        }
        assert_eq!(name(OverlayKind::Settings), "settings");
        assert_eq!(name(OverlayKind::PluginMonitor), "monitor");
    }

    #[test]
    fn opening_settings_drops_confirm() {
        let mut input = confirm(7);
        input.replace(InputMode::Overlay(OverlayKind::Settings));
        assert!(input.is_overlay(OverlayKind::Settings));
        assert!(input.confirm().is_none());
    }

    #[test]
    fn opening_tab_menu_clears_terminal_menu() {
        let mut input = terminal_menu(2);
        assert!(input.terminal_menu().is_some_and(|m| m.selected == 2));
        input = tab_menu(9);
        assert!(input.tab_menu().is_some_and(|m| m.tab_id == 9));
        assert!(input.terminal_menu().is_none());
    }

    #[test]
    fn consent_payload_exists_only_on_the_consent_arm() {
        assert!(InputMode::Terminal.consent().is_none());
        assert!(confirm(1).consent().is_none());
        assert!(
            InputMode::Overlay(OverlayKind::PluginMonitor)
                .consent()
                .is_none()
        );
        assert!(InputMode::Find.consent().is_none());
    }

    #[test]
    fn modal_open_replaces_previous_overlay() {
        let mut input = InputMode::Overlay(OverlayKind::Settings);
        input.replace(InputMode::Overlay(OverlayKind::Diff));
        assert!(input.is_overlay(OverlayKind::Diff));
        assert!(!input.is_overlay(OverlayKind::Settings));
    }

    #[test]
    fn find_replaces_modal_overlay() {
        let mut input = InputMode::Overlay(OverlayKind::Palette);
        input.replace(InputMode::Find);
        assert!(input.is_find());
        assert!(!input.is_overlay(OverlayKind::Palette));
    }

    #[test]
    fn reopening_the_current_overlay_is_identity() {
        let mut input = InputMode::Overlay(OverlayKind::Diff);
        input.replace(InputMode::Overlay(OverlayKind::Diff));
        assert!(input.is_overlay(OverlayKind::Diff));

        input.replace(InputMode::Overlay(OverlayKind::Settings));
        assert!(input.is_overlay(OverlayKind::Settings));
        assert!(!input.is_overlay(OverlayKind::Diff));
    }

    #[test]
    fn toggle_pattern_closes_only_the_matching_overlay() {
        let mut input = InputMode::Terminal;
        // open
        input.replace(InputMode::Overlay(OverlayKind::History));
        assert!(input.is_overlay(OverlayKind::History));
        // close
        input.replace(InputMode::Terminal);
        assert!(matches!(input, InputMode::Terminal));
        // close_overlay on a different kind is a no-op
        assert!(!input.is_overlay(OverlayKind::Diff));
    }

    #[test]
    fn overlay_replaces_the_plugin_monitor() {
        let mut input = InputMode::Overlay(OverlayKind::PluginMonitor);
        input.replace(InputMode::Overlay(OverlayKind::Settings));
        assert!(input.is_overlay(OverlayKind::Settings));
        assert!(!input.is_overlay(OverlayKind::PluginMonitor));
    }

    #[test]
    fn quick_select_survives_modal_overlays() {
        let mut mode = UiMode::default();
        assert!(mode.toggle_quick_select());
        let input = InputMode::Overlay(OverlayKind::Settings);
        assert!(mode.quick_select_open);
        assert!(input.is_overlay(OverlayKind::Settings));
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

    #[test]
    fn browser_is_hidden_for_every_application_overlay() {
        for overlay in [
            OverlayKind::Settings,
            OverlayKind::Update,
            OverlayKind::Palette,
            OverlayKind::PaneFacts,
            OverlayKind::History,
            OverlayKind::Diff,
            OverlayKind::PluginMonitor,
        ] {
            assert!(browser_is_blocked(
                &InputMode::Overlay(overlay),
                false,
                false
            ));
        }
        assert!(!browser_is_blocked(&InputMode::Terminal, false, false));
        assert!(!browser_is_blocked(&InputMode::Find, false, false));
        assert!(browser_is_blocked(&InputMode::Terminal, true, false));
        assert!(browser_is_blocked(&InputMode::Terminal, false, true));
    }
}
