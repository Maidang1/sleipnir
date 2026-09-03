//! Chrome contribution points (ADR-0017 third mount, ADR-0016 §7).
//!
//! Chrome is trusted UI: tab chips, the command palette, a compact status
//! slot. Plugins contribute through one `Render` to
//! [`plugin_protocol::v2::RenderTarget::Status`], laid out by
//! [`sleipnir_widget`] — never by a third layout implementation.
//!
//! **Capability.** All three contributions require
//! [`Capability::RenderStatus`]. They occupy the same trusted mount (the
//! wire target is `Status`); splitting them would not match the protocol.
//! No grant, no contribution.
//!
//! **The running-plugins indicator is not a contribution.** It is host-drawn
//! and this registry has no field or method that can hide it.
//!
//! **Tab-badge vs run-ledger.** Plugin badges are a distinct type. Run-ledger
//! Failed attention is a chip *wash* the user already relies on; a plugin
//! must not replace or suppress it. The plugin badge is an extra attributed
//! label.
//!
//! Pure state. No gpui, no window.

use plugin_protocol::v2::{Tone, Widget};
use sleipnir_widget::{Layout, layout};
use std::collections::{BTreeMap, BTreeSet};

use crate::pane_tree::PaneKey;

/// Badge label cap. Tab chips are already short.
pub const MAX_BADGE_CHARS: usize = 8;
/// Status strip budget in cells. Chrome is ~32px tall; this is one slot.
pub const MAX_STATUS_COLS: u16 = 24;
/// Per-plugin dynamic palette cap.
pub const MAX_PALETTE_PER_PLUGIN: usize = 8;
/// Global dynamic palette cap.
pub const MAX_PALETTE_TOTAL: usize = 32;
/// One tab badge per plugin.
pub const MAX_BADGES_PER_PLUGIN: usize = 1;

/// Outcome of applying a Status tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplyChrome {
    Applied,
    DeniedGrant,
}

/// A plugin tab badge. Not a [`run_ledger::Badge`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginTabBadge {
    pub plugin_id: String,
    pub text: String,
    pub tone: Tone,
    pub pane: Option<PaneKey>,
}

/// Dynamic palette row. Routed as Action; never a built-in command id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteContribution {
    pub plugin_id: String,
    pub title: String,
    pub action: String,
    pub arg: Option<String>,
    pub surface_id: uuid::Uuid,
}

#[derive(Clone, Debug, PartialEq)]
struct PluginChrome {
    surface_id: uuid::Uuid,
    tree: Widget,
    hint_pane: Option<PaneKey>,
    /// Text-capped, at most [`MAX_BADGES_PER_PLUGIN`]. Derived at apply so
    /// tab paint does not walk the widget tree.
    badges: Vec<(String, Tone)>,
    /// Per-plugin-capped Btn rows. Derived at apply so the palette rebuild
    /// does not walk the tree.
    btns: Vec<(String, String, Option<String>)>,
    drop_badge: u64,
    drop_palette: u64,
    drop_status: u64,
}

/// Cached status layout. Chrome paints every frame; layout is paid only when
/// the contributing trees or the available width change. Trees are replaced
/// only in [`ChromeRegistry::apply_status`] / [`ChromeRegistry::sync_live`],
/// which drop this cache — the hot path compares `cols` and returns.
#[derive(Clone, Debug)]
pub struct StatusLayoutCache {
    pub cols: u16,
    pub layout: Layout,
    pub computes: u32,
}

/// Accounted truncations so a flooding plugin is visible in the Monitor.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChromeDrops {
    pub badge_truncated: u64,
    pub palette_dropped: u64,
    pub status_truncated: u64,
}

/// Host-owned chrome contributions. BTreeMap so paint order is plugin-id order
/// and does not flicker between frames.
#[derive(Clone, Debug, Default)]
pub struct ChromeRegistry {
    plugins: BTreeMap<String, PluginChrome>,
    status_cache: Option<StatusLayoutCache>,
    cached_palette: Vec<PaletteContribution>,
    pub drops: ChromeDrops,
}

impl ChromeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Elm-style whole-tree replacement for one plugin's chrome.
    pub fn apply_status(
        &mut self,
        plugin_id: &str,
        tree: Widget,
        granted: bool,
        hint_pane: Option<PaneKey>,
    ) -> ApplyChrome {
        if !granted {
            return ApplyChrome::DeniedGrant;
        }
        let surface_id = self
            .plugins
            .get(plugin_id)
            .map(|p| p.surface_id)
            .unwrap_or_else(uuid::Uuid::new_v4);
        self.plugins.insert(
            plugin_id.to_string(),
            derive_plugin(surface_id, tree, hint_pane),
        );
        self.invalidate_derived();
        ApplyChrome::Applied
    }

    fn invalidate_derived(&mut self) {
        self.status_cache = None;
        self.recompute_drops_and_palette();
    }

    fn recompute_drops_and_palette(&mut self) {
        let mut drops = ChromeDrops::default();
        let mut palette = Vec::new();
        for (id, plug) in &self.plugins {
            drops.badge_truncated += plug.drop_badge;
            drops.palette_dropped += plug.drop_palette;
            drops.status_truncated += plug.drop_status;
            for (title, action, arg) in &plug.btns {
                if palette.len() >= MAX_PALETTE_TOTAL {
                    drops.palette_dropped += 1;
                    continue;
                }
                palette.push(PaletteContribution {
                    plugin_id: id.clone(),
                    title: attributed_title(id, title),
                    action: action.clone(),
                    arg: arg.clone(),
                    surface_id: plug.surface_id,
                });
            }
        }
        self.cached_palette = palette;
        self.drops = drops;
    }

    /// Dead / uninstalled plugins vanish from chrome. Palette entries must
    /// not remain invokable; badges and status must not occupy trusted UI
    /// after the process is gone. (Panel keeps a stale tree because it is a
    /// large host-owned surface; chrome is too small and too trusted.)
    /// Returns true when the set of contributing plugins changed.
    pub fn sync_live(&mut self, live: &BTreeSet<String>) -> bool {
        let before = self.plugins.len();
        self.plugins.retain(|id, _| live.contains(id));
        let changed = self.plugins.len() != before;
        if changed {
            self.invalidate_derived();
        }
        changed
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Tab badges that apply to `tab_panes`.
    ///
    /// A pane-scoped badge shows only on tabs that contain that pane. A
    /// global badge (no pane hint) shows only on the active tab, so one
    /// plugin cannot stamp every chip.
    pub fn badges_for_tab(
        &self,
        tab_panes: &[PaneKey],
        tab_is_active: bool,
    ) -> Vec<PluginTabBadge> {
        let mut out = Vec::new();
        for (id, plug) in &self.plugins {
            let Some((text, tone)) = plug.badges.first() else {
                continue;
            };
            let pane = plug.hint_pane;
            let show = match pane {
                Some(key) => tab_panes.contains(&key),
                None => tab_is_active,
            };
            if show {
                out.push(PluginTabBadge {
                    plugin_id: id.clone(),
                    text: text.clone(),
                    tone: *tone,
                    pane,
                });
            }
        }
        out
    }

    /// Dynamic palette entries in plugin-id order, after per-plugin and
    /// global caps. Attribution is baked into the display title.
    pub fn palette_entries(&self) -> &[PaletteContribution] {
        &self.cached_palette
    }

    /// Status layout for `cols`, cached. Concatenates live plugins in id
    /// order into one Row so a single [`layout`] call covers the strip.
    ///
    /// Does **not** call [`layout`] on unchanged (`cols`, trees) frames —
    /// chrome paints every frame and a plugin must not be able to make that
    /// janky.
    pub fn status_layout(&mut self, cols: u16) -> Option<&Layout> {
        let cols = cols.clamp(1, MAX_STATUS_COLS);
        let cache_hit = self.status_cache.as_ref().is_some_and(|c| c.cols == cols);
        if cache_hit {
            return self.status_cache.as_ref().map(|c| &c.layout);
        }
        if self.plugins.is_empty() {
            self.status_cache = None;
            return None;
        }
        let children: Vec<Widget> = self.plugins.values().map(|p| p.tree.clone()).collect();
        let tree = Widget::Row { gap: 1, children };
        let attr = if self.plugins.len() == 1 {
            self.plugins
                .keys()
                .next()
                .map(|s| s.as_str())
                .unwrap_or("chrome")
        } else {
            "chrome"
        };
        let layout = layout(&tree, cols, attr);
        let computes = self
            .status_cache
            .as_ref()
            .map(|c| c.computes.saturating_add(1))
            .unwrap_or(1);
        self.status_cache = Some(StatusLayoutCache {
            cols,
            layout,
            computes,
        });
        self.status_cache.as_ref().map(|c| &c.layout)
    }

    #[cfg(test)]
    pub fn status_computes(&self) -> u32 {
        self.status_cache.as_ref().map(|c| c.computes).unwrap_or(0)
    }
}

/// Display title that cannot be mistaken for a built-in. The plugin id is
/// prefix, not suffix: a plugin named "Reload Settings" still reads as
/// `other: Reload Settings`.
pub fn attributed_title(plugin_id: &str, title: &str) -> String {
    format!("{plugin_id}: {title}")
}

fn derive_plugin(surface_id: uuid::Uuid, tree: Widget, hint_pane: Option<PaneKey>) -> PluginChrome {
    let badges_all = extract_badges(&tree);
    let mut drop_badge = 0u64;
    for (_, _, truncated) in &badges_all {
        if *truncated {
            drop_badge += 1;
        }
    }
    if badges_all.len() > MAX_BADGES_PER_PLUGIN {
        drop_badge += (badges_all.len() - MAX_BADGES_PER_PLUGIN) as u64;
    }
    let badges: Vec<(String, Tone)> = badges_all
        .into_iter()
        .take(MAX_BADGES_PER_PLUGIN)
        .map(|(text, tone, _)| (text, tone))
        .collect();

    let btns_all = extract_btns(&tree);
    let drop_palette = btns_all.len().saturating_sub(MAX_PALETTE_PER_PLUGIN) as u64;
    let btns = btns_all.into_iter().take(MAX_PALETTE_PER_PLUGIN).collect();

    let drop_status = u64::from(leaf_wider_than_status(&tree));

    PluginChrome {
        surface_id,
        tree,
        hint_pane,
        badges,
        btns,
        drop_badge,
        drop_palette,
        drop_status,
    }
}

fn leaf_wider_than_status(tree: &Widget) -> bool {
    let mut over = false;
    walk(tree, &mut |w| {
        let s = match w {
            Widget::Text { s, .. } | Widget::Badge { s, .. } | Widget::Btn { s, .. } => {
                Some(s.as_str())
            }
            Widget::Code { s, .. } => Some(s.as_str()),
            _ => None,
        };
        if let Some(s) = s {
            if sleipnir_widget::cell_cols(s) > u32::from(MAX_STATUS_COLS) {
                over = true;
            }
        }
    });
    over
}

fn extract_badges(tree: &Widget) -> Vec<(String, Tone, bool)> {
    let mut out = Vec::new();
    walk(tree, &mut |w| {
        if let Widget::Badge { s, tone } = w {
            let (text, truncated) = sleipnir_widget::fit_cols(s, MAX_BADGE_CHARS as u32);
            out.push((text, *tone, truncated));
        }
    });
    out
}

fn extract_btns(tree: &Widget) -> Vec<(String, String, Option<String>)> {
    let mut out = Vec::new();
    walk(tree, &mut |w| {
        if let Widget::Btn { s, action, arg } = w {
            out.push((s.clone(), action.clone(), arg.clone()));
        }
    });
    out
}

fn walk(w: &Widget, f: &mut impl FnMut(&Widget)) {
    f(w);
    match w {
        Widget::Col { children, .. } | Widget::Row { children, .. } => {
            for c in children {
                walk(c, f);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn text(s: &str) -> Widget {
        Widget::Text {
            s: s.into(),
            fg: Tone::Fg,
            bold: false,
        }
    }

    fn badge(s: &str, tone: Tone) -> Widget {
        Widget::Badge { s: s.into(), tone }
    }

    fn btn(s: &str, action: &str) -> Widget {
        Widget::Btn {
            s: s.into(),
            action: action.into(),
            arg: None,
        }
    }

    fn key(n: u128) -> PaneKey {
        Uuid::from_u128(n)
    }

    #[test]
    fn status_denied_without_grant() {
        let mut reg = ChromeRegistry::new();
        assert_eq!(
            reg.apply_status("demo", text("hi"), false, None),
            ApplyChrome::DeniedGrant
        );
        assert!(reg.is_empty());
    }

    #[test]
    fn badge_and_palette_denied_without_render_status() {
        let mut reg = ChromeRegistry::new();
        let tree = Widget::Col {
            gap: 0,
            children: vec![badge("ok", Tone::Ok), btn("Go", "go")],
        };
        assert_eq!(
            reg.apply_status("demo", tree, false, Some(key(1))),
            ApplyChrome::DeniedGrant
        );
        assert!(reg.badges_for_tab(&[key(1)], true).is_empty());
        assert!(reg.palette_entries().is_empty());
    }

    #[test]
    fn plugin_badge_is_not_a_ledger_failed_badge() {
        // The wash is the ledger's own Failed bool; a plugin badge is only an
        // extra attributed label and can never set or suppress it.
        let mut reg = ChromeRegistry::new();
        reg.apply_status("demo", badge("ok", Tone::Ok), true, Some(key(1)));
        let badges = reg.badges_for_tab(&[key(1)], true);
        assert_eq!(badges.len(), 1);
        assert_eq!(badges[0].text, "ok");
        // Plugin Err tone is not a ledger Failed badge type.
        let mut reg = ChromeRegistry::new();
        reg.apply_status("demo", badge("no", Tone::Err), true, Some(key(1)));
        let badges = reg.badges_for_tab(&[key(1)], true);
        assert_ne!(
            format!("{:?}", badges[0]),
            "Badge { kind: Failed, count: 1, elapsed_ms: 0 }"
        );
    }

    #[test]
    fn palette_entries_are_attributed_and_not_builtin_ids() {
        let mut reg = ChromeRegistry::new();
        reg.apply_status(
            "demo",
            btn("Reload Settings", "reload_settings"),
            true,
            None,
        );
        let entries = reg.palette_entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title, "demo: Reload Settings");
        assert_eq!(entries[0].action, "reload_settings");
        assert!(entries[0].title.starts_with("demo:"));
        // The contribution type is not a CommandId; impersonation is a
        // dispatch concern tested via CommandId::PluginContribution.
    }

    #[test]
    fn badge_text_cap_truncates_with_marker() {
        let mut reg = ChromeRegistry::new();
        let long: String = std::iter::repeat_n('x', MAX_BADGE_CHARS + 10).collect();
        reg.apply_status("demo", badge(&long, Tone::Accent), true, Some(key(1)));
        let badges = reg.badges_for_tab(&[key(1)], false);
        assert_eq!(badges[0].text.chars().count(), MAX_BADGE_CHARS);
        assert!(badges[0].text.ends_with(sleipnir_widget::ELLIPSIS));
        assert!(reg.drops.badge_truncated > 0);
    }

    #[test]
    fn extra_badges_beyond_one_are_dropped_and_accounted() {
        let mut reg = ChromeRegistry::new();
        reg.apply_status(
            "demo",
            Widget::Row {
                gap: 0,
                children: vec![badge("a", Tone::Fg), badge("b", Tone::Fg)],
            },
            true,
            Some(key(1)),
        );
        assert_eq!(
            reg.badges_for_tab(&[key(1)], true).len(),
            MAX_BADGES_PER_PLUGIN
        );
        assert!(reg.drops.badge_truncated >= 1);
    }

    #[test]
    fn palette_caps_drop_extras_and_account() {
        let mut reg = ChromeRegistry::new();
        let children: Vec<Widget> = (0..MAX_PALETTE_PER_PLUGIN + 3)
            .map(|i| btn(&format!("e{i}"), "act"))
            .collect();
        reg.apply_status("demo", Widget::Col { gap: 0, children }, true, None);
        let entries = reg.palette_entries();
        assert_eq!(entries.len(), MAX_PALETTE_PER_PLUGIN);
        assert!(reg.drops.palette_dropped >= 3);
    }

    #[test]
    fn palette_total_cap_across_plugins() {
        let mut reg = ChromeRegistry::new();
        for i in 0..6 {
            let children: Vec<Widget> = (0..MAX_PALETTE_PER_PLUGIN)
                .map(|j| btn(&format!("{i}-{j}"), "a"))
                .collect();
            reg.apply_status(
                &format!("p{i}"),
                Widget::Col { gap: 0, children },
                true,
                None,
            );
        }
        let entries = reg.palette_entries();
        assert_eq!(entries.len(), MAX_PALETTE_TOTAL);
        assert!(reg.drops.palette_dropped > 0);
    }

    #[test]
    fn status_width_cap_and_layout_cache() {
        let mut reg = ChromeRegistry::new();
        let long: String = std::iter::repeat_n('x', MAX_STATUS_COLS as usize + 20).collect();
        reg.apply_status("demo", text(&long), true, None);
        assert!(
            reg.drops.status_truncated > 0,
            "a leaf wider than the status slot must be an accounted drop"
        );
        let a = reg.status_layout(MAX_STATUS_COLS).unwrap();
        assert!(a.width <= u32::from(MAX_STATUS_COLS));
        let computes = reg.status_computes();
        let _ = reg.status_layout(MAX_STATUS_COLS);
        assert_eq!(
            reg.status_computes(),
            computes,
            "unchanged width and tree must not relayout"
        );
        let _ = reg.status_layout(8);
        assert!(
            reg.status_computes() > computes,
            "width change must relayout"
        );
        let _ = reg.status_layout(8);
        assert_eq!(
            reg.status_computes(),
            computes + 1,
            "a second paint at the new width must hit the cache"
        );
    }

    #[test]
    fn multi_plugin_order_is_plugin_id() {
        let mut reg = ChromeRegistry::new();
        reg.apply_status("zeta", btn("Z", "z"), true, None);
        reg.apply_status("alpha", btn("A", "a"), true, None);
        let entries = reg.palette_entries();
        assert_eq!(entries[0].plugin_id, "alpha");
        assert_eq!(entries[1].plugin_id, "zeta");
        let badges_tree = {
            let mut r = ChromeRegistry::new();
            r.apply_status("zeta", badge("z", Tone::Fg), true, Some(key(1)));
            r.apply_status("alpha", badge("a", Tone::Fg), true, Some(key(1)));
            r.badges_for_tab(&[key(1)], true)
        };
        assert_eq!(badges_tree[0].plugin_id, "alpha");
        assert_eq!(badges_tree[1].plugin_id, "zeta");
    }

    #[test]
    fn dead_plugin_contributions_are_removed() {
        let mut reg = ChromeRegistry::new();
        reg.apply_status(
            "demo",
            Widget::Col {
                gap: 0,
                children: vec![badge("x", Tone::Warn), btn("Go", "go")],
            },
            true,
            Some(key(1)),
        );
        assert!(!reg.badges_for_tab(&[key(1)], true).is_empty());
        assert!(!reg.palette_entries().is_empty());
        let mut live = BTreeSet::new();
        live.insert("other".into());
        reg.sync_live(&live);
        assert!(reg.badges_for_tab(&[key(1)], true).is_empty());
        assert!(reg.palette_entries().is_empty());
        assert!(reg.status_layout(12).is_none());
    }

    #[test]
    fn running_indicator_is_not_a_registry_field() {
        // The host draws the indicator. A plugin cannot apply_status a
        // hide flag because none exists.
        let reg = ChromeRegistry::new();
        let _ = reg;
        assert_eq!(
            crate::plugin_monitor_panel::running_indicator_label(0),
            "0 plugins"
        );
    }

    #[test]
    fn global_badge_only_on_active_tab() {
        let mut reg = ChromeRegistry::new();
        reg.apply_status("demo", badge("hi", Tone::Accent), true, None);
        assert!(reg.badges_for_tab(&[key(1)], false).is_empty());
        assert_eq!(reg.badges_for_tab(&[key(1)], true).len(), 1);
    }
}
