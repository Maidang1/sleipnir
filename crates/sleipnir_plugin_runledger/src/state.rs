//! Plugin-owned ledger state: the [`Ledger`], the local↔host run-id map, and
//! `runs.json` persistence.
//!
//! Two id spaces meet here. The ledger mints its own [`RunId`] on every
//! `Started` (`Run::start` is crate-private, so events are the only way in),
//! while the host addresses runs by the `run_id` it allocated for
//! `scroll_to_run` and `RenderTarget::Block`. The map joins them: a panel
//! row's `jump` arg is the host id, `mark_run_seen` needs the local one.
//!
//! `Anchor` is a host-process concept; every event is applied with
//! `anchor: None`. Jumping is addressed purely by host run id.

use run_ledger::{
    LaunchId, Ledger, PaneKey, Retention, RunEvent, RunId, RunState, default_runs_path, load_runs,
    save_runs,
};
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;

/// Prune the in-memory ledger (and the id map with it) every N applied
/// events, mirroring the bound the core used.
pub const PRUNE_EVERY: u64 = 64;

pub struct LedgerState {
    pub ledger: Ledger,
    /// Local (ledger-minted) → host (event-carried) run id.
    host_ids: HashMap<RunId, RunId>,
    path: PathBuf,
    since_prune: u64,
}

impl LedgerState {
    /// Load history from `path`: every restored run is already seen, and a
    /// run still `Running` at shutdown comes back `Abandoned` (both rules
    /// live in `Ledger::load_history`).
    pub fn load(path: PathBuf) -> Self {
        let mut ledger = Ledger::new(LaunchId::new_v4());
        let (runs, _) = load_runs(&path);
        ledger.load_history(runs);
        Self::with_ledger(path, ledger)
    }

    /// Test seam: an explicit ledger and path, no disk read.
    pub fn with_ledger(path: PathBuf, ledger: Ledger) -> Self {
        Self {
            ledger,
            host_ids: HashMap::new(),
            path,
            since_prune: 0,
        }
    }

    /// Apply a host `RunStarted`. Returns the local id of the fresh run and
    /// records `local → host`. The fresh run is the pane's newest `Running`
    /// one by construction (`apply` abandons any predecessor first).
    pub fn apply_started(
        &mut self,
        host: RunId,
        pane: PaneKey,
        command: &str,
        cwd: Option<String>,
        inferred: bool,
        at_ms: u64,
    ) -> RunId {
        self.ledger
            .apply(RunEvent::started_at(pane, command, cwd, at_ms, inferred, None));
        let local = self
            .ledger
            .runs()
            .filter(|run| run.pane == pane && run.state == RunState::Running)
            .last()
            .map(|run| run.id)
            .expect("apply(Started) always leaves a Running run for the pane");
        self.host_ids.insert(local, host);
        self.tick();
        local
    }

    /// Apply a host `RunFinished`. An orphan finish (plugin started after the
    /// run did) is dropped by the ledger, never half-recorded.
    pub fn apply_finished(&mut self, pane: PaneKey, exit_code: Option<i32>, at_ms: u64) {
        self.ledger.apply(RunEvent::Finished {
            pane,
            exit_code,
            at_ms,
        });
        self.tick();
    }

    /// Apply a host `PaneClosed`: the pane's open run will never finish.
    pub fn apply_pane_closed(&mut self, pane: PaneKey, at_ms: u64) {
        self.ledger.apply(RunEvent::PaneClosed { pane, at_ms });
        self.tick();
    }

    /// Focusing a pane marks every one of its runs seen, and arms the
    /// finish-while-focused rule (`set_focus`) so a run that ends under the
    /// user's eyes never raises Attention.
    pub fn focus(&mut self, pane: PaneKey) {
        self.ledger.set_focus(Some(pane), true);
        self.ledger.mark_pane_seen(pane);
    }

    pub fn host_id_for(&self, local: RunId) -> Option<RunId> {
        self.host_ids.get(&local).copied()
    }

    pub fn local_id_for(&self, host: RunId) -> Option<RunId> {
        self.host_ids
            .iter()
            .find(|(_, h)| **h == host)
            .map(|(l, _)| *l)
    }

    /// Mark the run behind a host id seen (panel jump). False when the id is
    /// unknown — e.g. a stale tree from before a `clear`.
    pub fn mark_host_seen(&mut self, host: RunId) -> bool {
        let Some(local) = self.local_id_for(host) else {
            return false;
        };
        self.ledger.mark_run_seen(local);
        true
    }

    /// Persist the snapshot under the store's cross-process lock. History is
    /// merged by run id, so two Sleipnir windows cannot drop each other's
    /// runs. Retention is applied on write by the store.
    pub fn save(&self) -> io::Result<()> {
        save_runs(&self.path, &self.ledger.snapshot(), Retention::default())
    }

    /// Drop everything: fresh ledger (new launch), empty map, and the file
    /// itself removed so a cleared ledger does not resurrect on next launch.
    pub fn clear(&mut self) {
        self.ledger = Ledger::new(LaunchId::new_v4());
        self.host_ids.clear();
        self.since_prune = 0;
        match std::fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => eprintln!("runledger: could not remove {}: {err}", self.path.display()),
        }
    }

    /// Bound memory and drop map entries whose run was pruned.
    pub fn prune_and_sync(&mut self) {
        self.ledger.prune();
        self.host_ids
            .retain(|local, _| self.ledger.runs().any(|run| run.id == *local));
    }

    fn tick(&mut self) {
        self.since_prune += 1;
        if self.since_prune >= PRUNE_EVERY {
            self.since_prune = 0;
            self.prune_and_sync();
        }
    }
}

/// Where `runs.json` lives for this user. Mirrors
/// `plugin_host::default_plugin_dir_for` minus the trailing `plugins`:
/// `~/.config/sleipnir` everywhere but Windows, so the plugin keeps reading
/// the same file the core used to write.
pub fn default_config_dir() -> PathBuf {
    default_config_dir_for(cfg!(windows))
}

pub fn default_config_dir_for(windows: bool) -> PathBuf {
    if windows {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("sleipnir")
    } else {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".config/sleipnir")
    }
}

/// The default ledger file for this user.
pub fn default_runs_file() -> PathBuf {
    default_runs_path(&default_config_dir())
}

#[cfg(test)]
mod tests {
    use super::*;
    use run_ledger::{LaunchId, RunState};
    use std::path::Path;

    fn pane() -> PaneKey {
        PaneKey::new_v4()
    }

    fn state_in(dir: &Path) -> LedgerState {
        LedgerState::with_ledger(
            default_runs_path(dir),
            Ledger::new(LaunchId::new_v4()),
        )
    }

    #[test]
    fn started_maps_the_host_id_to_the_fresh_local_run() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = state_in(dir.path());
        let host = RunId::new_v4();
        let p = pane();
        let local = state.apply_started(host, p, "cargo test", None, false, 0);
        assert_eq!(state.host_id_for(local), Some(host));
        assert_eq!(state.local_id_for(host), Some(local));
        assert_eq!(state.ledger.runs().next().unwrap().id, local);
    }

    #[test]
    fn second_start_on_a_pane_maps_the_new_run() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = state_in(dir.path());
        let p = pane();
        let host_a = RunId::new_v4();
        let host_b = RunId::new_v4();
        let local_a = state.apply_started(host_a, p, "first", None, false, 0);
        let local_b = state.apply_started(host_b, p, "second", None, false, 10);
        assert_ne!(local_a, local_b);
        assert_eq!(state.host_id_for(local_b), Some(host_b));
        // The abandoned predecessor keeps its mapping; its row is still shown.
        assert_eq!(state.host_id_for(local_a), Some(host_a));
    }

    #[test]
    fn save_then_load_round_trips_and_history_is_seen() {
        let dir = tempfile::tempdir().unwrap();
        let path = default_runs_path(dir.path());
        let mut state = state_in(dir.path());
        let p = pane();
        state.apply_started(RunId::new_v4(), p, "cargo test", None, false, 0);
        state.apply_finished(p, Some(1), 100);
        state.save().unwrap();

        let loaded = LedgerState::load(path);
        let runs: Vec<_> = loaded.ledger.runs().collect();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].command, "cargo test");
        assert_eq!(runs[0].state, RunState::Failed);
        assert_eq!(
            loaded.ledger.attention().count(),
            0,
            "history never resurrects Attention"
        );
    }

    #[test]
    fn a_run_left_running_comes_back_abandoned() {
        let dir = tempfile::tempdir().unwrap();
        let path = default_runs_path(dir.path());
        let mut state = state_in(dir.path());
        state.apply_started(RunId::new_v4(), pane(), "sleep 100", None, false, 0);
        state.save().unwrap();

        let loaded = LedgerState::load(path);
        assert_eq!(loaded.ledger.runs().next().unwrap().state, RunState::Abandoned);
    }

    #[test]
    fn finished_uses_the_host_exit_code() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = state_in(dir.path());
        let p = pane();
        state.apply_started(RunId::new_v4(), p, "make", None, false, 0);
        state.apply_finished(p, Some(0), 50);
        assert_eq!(state.ledger.runs().next().unwrap().state, RunState::Succeeded);
    }

    #[test]
    fn mark_host_seen_clears_attention() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = state_in(dir.path());
        let host = RunId::new_v4();
        let p = pane();
        state.apply_started(host, p, "boom", None, false, 0);
        state.apply_finished(p, Some(1), 100);
        assert_eq!(state.ledger.failed_attention_count(), 1);
        assert!(state.mark_host_seen(host));
        assert_eq!(state.ledger.failed_attention_count(), 0);
        assert!(!state.mark_host_seen(RunId::new_v4()), "unknown host id");
    }

    #[test]
    fn clear_resets_the_ledger_and_removes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = default_runs_path(dir.path());
        let mut state = state_in(dir.path());
        state.apply_started(RunId::new_v4(), pane(), "x", None, false, 0);
        state.save().unwrap();
        assert!(path.exists());

        state.clear();
        assert!(!path.exists(), "runs.json must be deleted");
        assert_eq!(state.ledger.runs().count(), 0);
        // A stale jump after clear is a no-op, not a panic.
        assert!(!state.mark_host_seen(RunId::new_v4()));
    }

    #[test]
    fn prune_drops_map_entries_for_pruned_runs() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = state_in(dir.path());
        state.ledger.set_retention(Retention {
            days: 7,
            max_runs: 1,
        });
        let host_a = RunId::new_v4();
        let host_b = RunId::new_v4();
        let local_a = state.apply_started(host_a, pane(), "old", None, false, 0);
        let local_b = state.apply_started(host_b, pane(), "new", None, false, 10);
        state.prune_and_sync();
        assert_eq!(state.ledger.runs().count(), 1);
        assert_eq!(state.host_id_for(local_b), Some(host_b));
        assert_eq!(state.host_id_for(local_a), None);
    }

    #[test]
    fn focus_marks_the_whole_pane_seen() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = state_in(dir.path());
        let p = pane();
        state.apply_started(RunId::new_v4(), p, "f1", None, false, 0);
        state.apply_finished(p, Some(1), 10);
        state.apply_started(RunId::new_v4(), p, "f2", None, false, 20);
        state.apply_finished(p, Some(1), 30);
        assert_eq!(state.ledger.failed_attention_count(), 2);
        state.focus(p);
        assert_eq!(state.ledger.failed_attention_count(), 0);
    }

    #[test]
    fn config_dir_mirrors_plugin_host_layout() {
        let unix = default_config_dir_for(false);
        assert!(unix.ends_with(".config/sleipnir"), "unix: {unix:?}");
        let win = default_config_dir_for(true);
        assert!(win.ends_with("sleipnir"), "windows: {win:?}");
        assert!(!win.ends_with("plugins"));
    }
}
