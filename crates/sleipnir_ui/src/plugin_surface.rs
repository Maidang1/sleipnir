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
use uuid::Uuid;

/// One host-owned plugin surface, viewed only through what staleness needs.
pub trait Surface {
    /// The exact plugin process instance that owns this surface.
    fn owner_instance_id(&self) -> Uuid;
    fn set_stale(&mut self, stale: bool);
}

/// A registry of [`Surface`]s that answers plugin death by marking.
///
/// Implementors supply access to their surfaces; the policy is inherited.
pub trait StaleRegistry {
    type Surface: Surface;

    fn surfaces_mut(&mut self) -> impl Iterator<Item = &mut Self::Surface>;

    /// Any surface whose owning plugin instance is absent from `live` is stale.
    ///
    /// This is the whole policy. There is deliberately no per-plugin variant:
    /// stale UI must now follow the exact owner instance, not just `plugin_id`:
    /// on-demand sessions and resident restarts can leave one instance dead
    /// while another with the same plugin id is alive.
    fn mark_missing_stale(&mut self, live: &BTreeSet<Uuid>) {
        for surface in self.surfaces_mut() {
            if !live.contains(&surface.owner_instance_id()) {
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
        owner_instance_id: Uuid,
        stale: bool,
    }

    impl Surface for Fake {
        fn owner_instance_id(&self) -> Uuid {
            self.owner_instance_id
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
                    owner_instance_id: Uuid::new_v4(),
                    stale: false,
                })
                .collect(),
        }
    }

    #[test]
    fn mark_missing_stale_spares_the_live_set() {
        let mut reg = registry(&["a", "b"]);
        let mut live = BTreeSet::new();
        live.insert(reg.surfaces[0].owner_instance_id);
        reg.mark_missing_stale(&live);
        let stale: Vec<bool> = reg.surfaces.iter().map(|s| s.stale).collect();
        assert_eq!(stale, vec![false, true]);
    }

    #[test]
    fn same_plugin_id_different_owner_instances_do_not_keep_each_other_fresh() {
        let owner_a = Uuid::new_v4();
        let owner_b = Uuid::new_v4();
        let mut reg = FakeRegistry {
            surfaces: vec![
                Fake {
                    plugin_id: "demo".into(),
                    owner_instance_id: owner_a,
                    stale: false,
                },
                Fake {
                    plugin_id: "demo".into(),
                    owner_instance_id: owner_b,
                    stale: false,
                },
            ],
        };
        let live = BTreeSet::from([owner_b]);
        reg.mark_missing_stale(&live);
        assert_eq!(
            reg.surfaces.iter().map(|s| s.stale).collect::<Vec<_>>(),
            vec![true, false]
        );
    }

    /// The load-bearing half of the policy: marking never removes.
    #[test]
    fn marking_never_drops_a_surface() {
        let mut reg = registry(&["a", "b"]);
        reg.mark_missing_stale(&BTreeSet::new());
        assert_eq!(reg.surfaces.len(), 2);
    }
}
