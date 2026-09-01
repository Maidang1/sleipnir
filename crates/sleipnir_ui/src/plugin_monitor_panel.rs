//! Pure helpers for the Plugin Monitor overlay and the consent dialog
//! (ADR-0016 §5–§7).
//!
//! Provenance is mandatory once a plugin can draw into the app: the user must
//! always be able to tell running plugins from program output, kill a wedged
//! process, and re-consent when a binary or capability set changes. GPUI is
//! optional; these stay testable without a window.
//!
//! Rows are copied off [`ConnectionSnapshot`] so the panel does not hold the
//! supervisor. Consent copy is derived from [`plugin_grants::check`]'s
//! [`ConsentReason`] — the three reasons mean very different things, and the
//! UI must not collapse them.

use plugin_grants::{ConsentReason, Tier};
use plugin_host::resident::{ConnectionSnapshot, ConnectionState};
use plugin_protocol::v2::Capability;
use std::collections::BTreeMap;

/// One overlay row, copied off a [`ConnectionSnapshot`] so the panel does not
/// hold the supervisor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonitorRow {
    pub plugin_id: String,
    pub name: String,
    pub tier: Tier,
    pub state: ConnectionState,
    pub pid: Option<u32>,
    pub started_at_ms: u64,
    pub last_activity_ms: u64,
    pub in_flight: usize,
    pub restart_count: u32,
    pub inbound_dropped: u64,
    pub malformed_lines: u64,
    pub events_dropped: u64,
    /// HostCall rate-limit drops (UI-side). The Monitor must show a plugin
    /// that is hammering Notify / OpenPane.
    pub host_calls_dropped: u64,
    pub stderr: Vec<String>,
}

/// Copied consent prompt. The dialog renders this; it never borrows grants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsentPrompt {
    pub plugin_id: String,
    pub plugin_name: String,
    pub tier: Tier,
    pub reason: ConsentReason,
    pub missing: Vec<Capability>,
    pub previously_granted: Vec<Capability>,
}

/// Title + lead paragraph for the dialog. `is_security_warning` is true only
/// for a binary change: that case must read stronger than first-run or an
/// added capability.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsentCopy {
    pub title: &'static str,
    pub lead: String,
    pub is_security_warning: bool,
}

/// Build overlay rows from snapshots. `names` and `tiers` are lookups copied
/// off the catalog / grants file; missing entries fall back to the plugin id
/// and [`Tier::Local`] (user-authored is the only shippable source today).
///
/// Sort: unhealthy first, then plugin id. A wedged or crash-looping plugin is
/// why the Monitor exists; burying it under healthy rows would hide the kill
/// switch. Id is the stable tie-break so equal health does not reshuffle.
pub fn rows_from_snapshots(
    snapshots: &[ConnectionSnapshot],
    names: &BTreeMap<String, String>,
    tiers: &BTreeMap<String, Tier>,
    _now_ms: u64,
) -> Vec<MonitorRow> {
    let mut rows: Vec<MonitorRow> = snapshots
        .iter()
        .map(|snap| row_from_snapshot(snap, names, tiers))
        .collect();
    rows.sort_by(|a, b| {
        health_rank(a)
            .cmp(&health_rank(b))
            .then_with(|| a.plugin_id.cmp(&b.plugin_id))
    });
    rows
}

fn row_from_snapshot(
    snap: &ConnectionSnapshot,
    names: &BTreeMap<String, String>,
    tiers: &BTreeMap<String, Tier>,
) -> MonitorRow {
    MonitorRow {
        plugin_id: snap.plugin_id.clone(),
        name: names
            .get(&snap.plugin_id)
            .cloned()
            .unwrap_or_else(|| snap.plugin_id.clone()),
        tier: tiers.get(&snap.plugin_id).copied().unwrap_or(Tier::Local),
        state: snap.state,
        pid: snap.pid,
        started_at_ms: snap.started_at_ms,
        last_activity_ms: snap.last_activity_ms,
        in_flight: snap.in_flight,
        restart_count: snap.restart_count,
        inbound_dropped: snap.inbound_dropped,
        malformed_lines: snap.malformed_lines,
        events_dropped: snap.events_dropped,
        host_calls_dropped: 0,
        stderr: snap.stderr.clone(),
    }
}

/// 0 = most urgent. Dead / crash-looping plugins outrank a quiet live one.
fn health_rank(row: &MonitorRow) -> u8 {
    match row.state {
        ConnectionState::Dead => 0,
        ConnectionState::ShuttingDown => 1,
        ConnectionState::Live
            if row.restart_count > 0
                || row.inbound_dropped > 0
                || row.malformed_lines > 0
                || row.events_dropped > 0
                || row.host_calls_dropped > 0 =>
        {
            2
        }
        ConnectionState::Live => 3,
    }
}

pub fn live_plugin_count(snapshots: &[ConnectionSnapshot]) -> usize {
    snapshots
        .iter()
        .filter(|s| s.state == ConnectionState::Live)
        .count()
}

/// Persistent chrome / header copy. Always names the count so "zero" is
/// visible, not an empty chip that looks like the feature is missing.
pub fn running_indicator_label(n: usize) -> String {
    match n {
        0 => "0 plugins".into(),
        1 => "1 plugin".into(),
        n => format!("{n} plugins"),
    }
}

pub fn format_uptime(elapsed_ms: u64) -> String {
    let secs = elapsed_ms / 1000;
    if secs < 60 {
        return format!("{secs}s");
    }
    let mins = secs / 60;
    if mins < 60 {
        let s = secs % 60;
        if s == 0 {
            format!("{mins}m")
        } else {
            format!("{mins}m {s}s")
        }
    } else {
        let hours = mins / 60;
        let m = mins % 60;
        if hours < 48 {
            if m == 0 {
                format!("{hours}h")
            } else {
                format!("{hours}h {m}m")
            }
        } else {
            format!("{}d", hours / 24)
        }
    }
}

pub fn format_activity(ago_ms: u64) -> String {
    if ago_ms < 2_000 {
        "just now".into()
    } else if ago_ms < 60_000 {
        format!("{}s ago", ago_ms / 1000)
    } else if ago_ms < 3_600_000 {
        format!("{}m ago", ago_ms / 60_000)
    } else {
        format!("{}h ago", ago_ms / 3_600_000)
    }
}

pub fn format_pid(pid: Option<u32>) -> String {
    match pid {
        Some(pid) => format!("pid {pid}"),
        None => "pid —".into(),
    }
}

pub fn state_label(state: ConnectionState) -> &'static str {
    match state {
        ConnectionState::Live => "running",
        ConnectionState::Dead => "dead",
        ConnectionState::ShuttingDown => "stopping",
    }
}

/// ADR-0016 §6: Local is labelled "unsandboxed, local" in the UI.
pub fn tier_badge(tier: Tier) -> &'static str {
    match tier {
        Tier::BuiltIn => "built-in",
        Tier::Local => "unsandboxed, local",
        Tier::Sandboxed => "sandboxed",
    }
}

/// Plain-language capability copy. Never the raw enum / snake_case name:
/// `SubscribeEvents` is continuous observation, not a flag.
pub fn capability_label(cap: Capability) -> &'static str {
    match cap {
        Capability::ReadSelection => "can read the current selection",
        Capability::ReadVisibleScreen => "can read what's on screen",
        Capability::ReadCwd => "can read the working directory",
        Capability::ReadTitle => "can read the pane title",
        Capability::WriteTerminal => "can type into the terminal",
        Capability::Clipboard => "can use the clipboard",
        Capability::Network => "can use the network",
        Capability::Resident => "keeps running in the background",
        Capability::SubscribeEvents => "can watch every command you run",
        Capability::RenderBlock => "can draw into the scrollback",
        Capability::RenderPanel => "can draw a side panel",
        Capability::RenderStatus => "can draw into the status area",
        Capability::HostCallNotify => "can show notifications",
        Capability::HostCallReadScreen => "can read any pane's screen",
        Capability::HostCallListPanes => "can list open panes",
        Capability::HostCallOpenPane => "can open a new pane",
        Capability::HostCallDrawScene => "can draw a 3D scene in a panel",
    }
}

pub fn consent_prompt(
    plugin_id: &str,
    plugin_name: &str,
    tier: Tier,
    reason: ConsentReason,
    missing: &[Capability],
    previously_granted: &[Capability],
) -> ConsentPrompt {
    ConsentPrompt {
        plugin_id: plugin_id.to_string(),
        plugin_name: plugin_name.to_string(),
        tier,
        reason,
        missing: missing.to_vec(),
        previously_granted: previously_granted.to_vec(),
    }
}

pub fn consent_copy(prompt: &ConsentPrompt) -> ConsentCopy {
    match prompt.reason {
        ConsentReason::FirstRun => ConsentCopy {
            title: "This plugin is asking for permission",
            lead: format!(
                "{} is asking for these capabilities for the first time.",
                prompt.plugin_name
            ),
            is_security_warning: false,
        },
        ConsentReason::BinaryChanged => ConsentCopy {
            title: "The plugin binary changed",
            lead: format!(
                "THE PLUGIN BINARY CHANGED SINCE YOU APPROVED IT. \
                 Previously approved permissions do not carry over. \
                 Treat {} as a new plugin and review every capability below.",
                prompt.plugin_name
            ),
            is_security_warning: true,
        },
        ConsentReason::NewCapabilities => ConsentCopy {
            title: "This plugin wants additional permissions",
            lead: format!(
                "{} already had some permissions and now wants additional ones. \
                 Only the new capabilities are listed.",
                prompt.plugin_name
            ),
            is_security_warning: false,
        },
    }
}

/// Deny is the default. The renderer uses this for the primary button label
/// so Approve cannot be mistaken for the safe action.
pub fn deny_label() -> &'static str {
    "Deny"
}

pub fn approve_label() -> &'static str {
    "Approve"
}

#[cfg(test)]
mod tests {
    use super::*;
    use plugin_grants::{Decision, GrantRecord};
    use uuid::Uuid;

    fn snap(id: &str, state: ConnectionState, restart_count: u32) -> ConnectionSnapshot {
        ConnectionSnapshot {
            plugin_id: id.into(),
            instance_id: Uuid::nil(),
            pid: Some(42),
            started_at_ms: 1_000,
            last_activity_ms: 1_500,
            in_flight: 0,
            restart_count,
            stderr: vec!["log line".into()],
            state,
            inbound_dropped: 0,
            malformed_lines: 0,
            events_dropped: 0,
        }
    }

    #[test]
    fn rows_copy_snapshot_fields_and_lookups() {
        let mut names = BTreeMap::new();
        names.insert("port-watcher".into(), "Port Watcher".into());
        let mut tiers = BTreeMap::new();
        tiers.insert("port-watcher".into(), Tier::Sandboxed);
        let snap = ConnectionSnapshot {
            plugin_id: "port-watcher".into(),
            instance_id: Uuid::nil(),
            pid: Some(99),
            started_at_ms: 10,
            last_activity_ms: 20,
            in_flight: 2,
            restart_count: 1,
            stderr: vec!["a".into(), "b".into()],
            state: ConnectionState::Live,
            inbound_dropped: 3,
            malformed_lines: 4,
            events_dropped: 5,
        };
        let rows = rows_from_snapshots(&[snap], &names, &tiers, 50);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Port Watcher");
        assert_eq!(rows[0].tier, Tier::Sandboxed);
        assert_eq!(rows[0].pid, Some(99));
        assert_eq!(rows[0].in_flight, 2);
        assert_eq!(rows[0].restart_count, 1);
        assert_eq!(rows[0].inbound_dropped, 3);
        assert_eq!(rows[0].malformed_lines, 4);
        assert_eq!(rows[0].events_dropped, 5);
        assert_eq!(rows[0].stderr, ["a", "b"]);
    }

    #[test]
    fn unknown_plugin_falls_back_to_id_and_local_tier() {
        let rows = rows_from_snapshots(
            &[snap("demo", ConnectionState::Live, 0)],
            &BTreeMap::new(),
            &BTreeMap::new(),
            0,
        );
        assert_eq!(rows[0].name, "demo");
        assert_eq!(rows[0].tier, Tier::Local);
        assert_eq!(tier_badge(rows[0].tier), "unsandboxed, local");
    }

    #[test]
    fn sort_puts_unhealthy_before_healthy_then_by_id() {
        let snapshots = [
            snap("zeta", ConnectionState::Live, 0),
            snap("alpha", ConnectionState::Dead, 0),
            snap("mu", ConnectionState::Live, 3),
            snap("beta", ConnectionState::ShuttingDown, 0),
        ];
        let rows = rows_from_snapshots(&snapshots, &BTreeMap::new(), &BTreeMap::new(), 0);
        let ids: Vec<_> = rows.iter().map(|r| r.plugin_id.as_str()).collect();
        assert_eq!(
            ids,
            ["alpha", "beta", "mu", "zeta"],
            "dead, stopping, crash-looping live, then healthy live; id breaks ties"
        );
    }

    #[test]
    fn sort_is_stable_by_id_within_the_same_health() {
        let snapshots = [
            snap("b", ConnectionState::Dead, 0),
            snap("a", ConnectionState::Dead, 0),
        ];
        let rows = rows_from_snapshots(&snapshots, &BTreeMap::new(), &BTreeMap::new(), 0);
        assert_eq!(rows[0].plugin_id, "a");
        assert_eq!(rows[1].plugin_id, "b");
    }

    #[test]
    fn uptime_and_activity_formatting() {
        assert_eq!(format_uptime(0), "0s");
        assert_eq!(format_uptime(1_200), "1s");
        assert_eq!(format_uptime(59_000), "59s");
        assert_eq!(format_uptime(60_000), "1m");
        assert_eq!(format_uptime(65_000), "1m 5s");
        assert_eq!(format_uptime(3_600_000), "1h");
        assert_eq!(format_uptime(3_720_000), "1h 2m");
        assert_eq!(format_uptime(48 * 3_600_000), "2d");

        assert_eq!(format_activity(0), "just now");
        assert_eq!(format_activity(1_500), "just now");
        assert_eq!(format_activity(3_000), "3s ago");
        assert_eq!(format_activity(120_000), "2m ago");
        assert_eq!(format_activity(7_200_000), "2h ago");
    }

    #[test]
    fn running_indicator_always_names_the_count() {
        assert_eq!(running_indicator_label(0), "0 plugins");
        assert_eq!(running_indicator_label(1), "1 plugin");
        assert_eq!(running_indicator_label(3), "3 plugins");
        assert_eq!(
            live_plugin_count(&[
                snap("a", ConnectionState::Live, 0),
                snap("b", ConnectionState::Dead, 1),
            ]),
            1
        );
    }

    #[test]
    fn every_v2_capability_has_plain_language_copy() {
        let caps = [
            Capability::ReadSelection,
            Capability::ReadVisibleScreen,
            Capability::ReadCwd,
            Capability::ReadTitle,
            Capability::WriteTerminal,
            Capability::Clipboard,
            Capability::Network,
            Capability::Resident,
            Capability::SubscribeEvents,
            Capability::RenderBlock,
            Capability::RenderPanel,
            Capability::RenderStatus,
            Capability::HostCallNotify,
            Capability::HostCallReadScreen,
            Capability::HostCallListPanes,
            Capability::HostCallOpenPane,
            Capability::HostCallDrawScene,
        ];
        for cap in caps {
            let label = capability_label(cap);
            let wire = serde_json::to_string(&cap).unwrap();
            let wire = wire.trim_matches('"');
            assert!(
                !label.contains('_'),
                "{cap:?} leaked a snake_case name: {label}"
            );
            assert_ne!(
                label, wire,
                "{cap:?} must not be shown as the wire name {wire}"
            );
        }
        assert_eq!(
            capability_label(Capability::SubscribeEvents),
            "can watch every command you run"
        );
        assert_eq!(
            capability_label(Capability::Resident),
            "keeps running in the background"
        );
    }

    #[test]
    fn consent_copy_distinguishes_all_three_reasons() {
        let first = consent_prompt(
            "demo",
            "Demo",
            Tier::Local,
            ConsentReason::FirstRun,
            &[Capability::ReadCwd],
            &[],
        );
        let first_copy = consent_copy(&first);
        assert!(!first_copy.is_security_warning);
        assert!(first_copy.lead.contains("first time"));

        let changed = consent_prompt(
            "demo",
            "Demo",
            Tier::Local,
            ConsentReason::BinaryChanged,
            &[Capability::ReadCwd, Capability::Network],
            &[Capability::ReadCwd],
        );
        let changed_copy = consent_copy(&changed);
        assert!(changed_copy.is_security_warning);
        assert!(changed_copy.lead.contains("BINARY CHANGED"));
        assert!(changed_copy.lead.contains("do not carry over"));

        let added = consent_prompt(
            "demo",
            "Demo",
            Tier::Local,
            ConsentReason::NewCapabilities,
            &[Capability::SubscribeEvents],
            &[Capability::ReadCwd],
        );
        let added_copy = consent_copy(&added);
        assert!(!added_copy.is_security_warning);
        assert!(added_copy.lead.contains("additional"));
        assert_eq!(added.missing, [Capability::SubscribeEvents]);
        assert_eq!(added.previously_granted, [Capability::ReadCwd]);

        assert_ne!(first_copy.title, changed_copy.title);
        assert_ne!(first_copy.title, added_copy.title);
        assert_ne!(changed_copy.title, added_copy.title);
        assert_eq!(tier_badge(Tier::Local), "unsandboxed, local");
        assert_eq!(deny_label(), "Deny");
    }

    #[test]
    fn consent_prompt_follows_grants_check_without_reimplementing_it() {
        // The dialog must consume check(), not invent its own hash/capability
        // rules. A matching grant is Allowed; a changed hash is BinaryChanged.
        let record = GrantRecord {
            granted: [Capability::ReadCwd].into_iter().collect(),
            binary_hash: "sha256:aaaa".into(),
            granted_at: "2026-01-01T00:00:00Z".into(),
            tier: Tier::Local,
        };
        assert_eq!(
            plugin_grants::check(&[Capability::ReadCwd], Some(&record), "sha256:aaaa"),
            Decision::Allowed
        );
        let Decision::NeedsConsent { reason, missing } =
            plugin_grants::check(&[Capability::ReadCwd], Some(&record), "sha256:bbbb")
        else {
            panic!("hash mismatch must ask again");
        };
        assert_eq!(reason, ConsentReason::BinaryChanged);
        assert_eq!(missing, [Capability::ReadCwd]);

        let Decision::NeedsConsent { reason, missing } = plugin_grants::check(
            &[Capability::ReadCwd, Capability::SubscribeEvents],
            Some(&record),
            "sha256:aaaa",
        ) else {
            panic!("new cap must ask again");
        };
        assert_eq!(reason, ConsentReason::NewCapabilities);
        assert_eq!(missing, [Capability::SubscribeEvents]);
    }
}
