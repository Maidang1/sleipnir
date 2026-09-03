//! Staleness policy shared by the Panel and Block mounts (ADR-0017).
//!
//! Both mounts hold host-owned surfaces keyed differently — [`PaneKey`] for
//! Panel, [`BlockId`] for Block — but they answer plugin death with the same
//! rule: **the last tree stays and is marked, it is never dropped.** A plugin
//! cannot un-draw itself from beyond the grave, so the user keeps seeing what
//! the plugin last said, visibly flagged as no longer live.
//!
//! That rule lived twice, byte-identical, in `plugin_panel` and `plugin_block`.
//! It lives here once. The key type stays with each registry; only the policy
//! is shared.
//!
//! [`crate::plugin_chrome::ChromeRegistry`] deliberately does **not** implement
//! this. Chrome is transient decoration (badges, a status slot, palette rows)
//! and its policy is `retain` + drop: a dead plugin's badge is removed, not
//! dimmed, because a stale badge in the chrome would misreport live state.
//!
//! [`PaneKey`]: crate::pane_tree::PaneKey
//! [`BlockId`]: plugin_protocol::v2::BlockId

use std::collections::BTreeSet;

/// One host-owned plugin surface, viewed only through what staleness needs.
pub trait Surface {
    /// The plugin that drew this surface.
    fn plugin_id(&self) -> &str;
    fn set_stale(&mut self, stale: bool);
}

/// A registry of [`Surface`]s that answers plugin death by marking.
///
/// Implementors supply access to their surfaces; the policy is inherited.
pub trait StaleRegistry {
    type Surface: Surface;

    fn surfaces_mut(&mut self) -> impl Iterator<Item = &mut Self::Surface>;

    /// Any surface whose plugin is absent from `live` is stale.
    ///
    /// This is the whole policy. There is deliberately no per-plugin variant:
    /// the shell polls a snapshot list that holds at most one entry per
    /// `plugin_id`, so "this plugin died" is already expressible as "it is not
    /// in `live`", and a second entry point would be a way for the two to
    /// disagree. See `plugin_host::resident::tests`
    /// `snapshots_hold_at_most_one_entry_per_plugin_id`.
    fn mark_missing_stale(&mut self, live: &BTreeSet<String>) {
        for surface in self.surfaces_mut() {
            if !live.contains(surface.plugin_id()) {
                surface.set_stale(true);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct Fake {
        plugin_id: String,
        stale: bool,
    }

    impl Surface for Fake {
        fn plugin_id(&self) -> &str {
            &self.plugin_id
        }
        fn set_stale(&mut self, stale: bool) {
            self.stale = stale;
        }
    }

    #[derive(Debug, Default)]
    struct FakeRegistry {
        surfaces: Vec<Fake>,
    }

    impl StaleRegistry for FakeRegistry {
        type Surface = Fake;
        fn surfaces_mut(&mut self) -> impl Iterator<Item = &mut Fake> {
            self.surfaces.iter_mut()
        }
    }

    fn registry(ids: &[&str]) -> FakeRegistry {
        FakeRegistry {
            surfaces: ids
                .iter()
                .map(|id| Fake {
                    plugin_id: (*id).into(),
                    stale: false,
                })
                .collect(),
        }
    }

    #[test]
    fn mark_missing_stale_spares_the_live_set() {
        let mut reg = registry(&["a", "b"]);
        let mut live = BTreeSet::new();
        live.insert("a".to_string());
        reg.mark_missing_stale(&live);
        let stale: Vec<bool> = reg.surfaces.iter().map(|s| s.stale).collect();
        assert_eq!(stale, vec![false, true]);
    }

    /// The load-bearing half of the policy: marking never removes.
    #[test]
    fn marking_never_drops_a_surface() {
        let mut reg = registry(&["a", "b"]);
        reg.mark_missing_stale(&BTreeSet::new());
        assert_eq!(reg.surfaces.len(), 2);
    }
}
