//! Plugin-owned ledger state: the [`Ledger`], the set of host-visible run ids
//! that are still live/jumpable, and `runs.json` persistence.
//!
//! The host already minted stable `run_id`s for `scroll_to_run` and
//! `RenderTarget::Block`, so the plugin preserves those ids inside the ledger
//! rather than reminting local ids. Jumpability is stricter than identity:
//! only rows this process actually observed remain addressable by the host.
//! Imported history keeps its run ids for display, but never becomes jumpable.
//!
//! `Anchor` is a host-process concept; every event is applied with
//! `anchor: None`. Jumping is addressed purely by host run id.

use run_ledger::{
    LaunchId, Ledger, PaneKey, Retention, RunId, default_runs_path, load_runs, save_runs,
};
use std::collections::HashSet;
use std::io;
use std::path::PathBuf;

/// Prune the in-memory ledger (and the jumpable-id set with it) every N
/// applied events, mirroring the bound the core used.
pub const PRUNE_EVERY: u64 = 64;

pub struct LedgerState {
    pub ledger: Ledger,
    /// Host-visible ids observed in this process and still jumpable in the
    /// host. Finished and abandoned rows stay jumpable until their pane is
    /// closed, the row is pruned, or the ledger is cleared.
    jumpable_host_ids: HashSet<RunId>,
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
            jumpable_host_ids: HashSet::new(),
            path,
            since_prune: 0,
        }
    }

    /// Apply a host `RunStarted`. The host id becomes the ledger id, so start
    /// and finish are stitched by the externally visible identity.
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
            .apply_external_start(host, pane, command.into(), cwd, at_ms, inferred, None);
        if self
            .ledger
            .runs()
            .any(|run| run.id == host && run.launch_id == self.ledger.launch_id())
        {
            self.jumpable_host_ids.insert(host);
        }
        self.tick();
        host
    }

    /// Apply a host `RunFinished`. The externally visible run id and duration
    /// are authoritative.
    pub fn apply_finished(&mut self, host: RunId, exit_code: Option<i32>, duration_ms: u64) {
        self.ledger
            .apply_external_finish(host, exit_code, duration_ms);
        self.tick();
    }

    /// Apply a host `PaneClosed`: the pane's rows all lose their host
    /// scrollback anchors, so every observed id in that pane becomes
    /// non-jumpable.
    pub fn apply_pane_closed(&mut self, pane: PaneKey, at_ms: u64) {
        let closing: Vec<RunId> = self
            .ledger
            .runs()
            .filter(|run| run.pane == pane)
            .map(|run| run.id)
            .collect();
        self.ledger
            .apply(run_ledger::RunEvent::PaneClosed { pane, at_ms });
        for id in closing {
            self.jumpable_host_ids.remove(&id);
        }
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
        self.jumpable_host_ids.contains(&local).then_some(local)
    }

    pub fn local_id_for(&self, host: RunId) -> Option<RunId> {
        if self.jumpable_host_ids.contains(&host) && self.ledger.runs().any(|run| run.id == host) {
            return Some(host);
        }
        None
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

    /// Persist only this launch's owned rows under the store's cross-process
    /// lock. Imported history stays read-only, so a stale snapshot from this
    /// launch cannot overwrite a newer completion written by another launch.
    /// Retention is applied on write by the store.
    pub fn save(&self) -> io::Result<()> {
        save_runs(
            &self.path,
            &self.ledger.snapshot(),
            self.ledger.launch_id(),
            Retention::default(),
        )
    }

    /// Drop everything: fresh ledger (new launch), empty map, and the file
    /// itself removed so a cleared ledger does not resurrect on next launch.
    pub fn clear(&mut self) {
        self.ledger = Ledger::new(LaunchId::new_v4());
        self.jumpable_host_ids.clear();
        self.since_prune = 0;
        match std::fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => eprintln!("runledger: could not remove {}: {err}", self.path.display()),
        }
    }

    /// Bound memory and drop jumpable ids whose run was pruned or no longer
    /// belongs to this launch.
    pub fn prune_and_sync(&mut self) {
        self.ledger.prune();
        self.jumpable_host_ids.retain(|id| {
            self.ledger
                .runs()
                .any(|run| run.id == *id && run.launch_id == self.ledger.launch_id())
        });
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
        LedgerState::with_ledger(default_runs_path(dir), Ledger::new(LaunchId::new_v4()))
    }

    #[test]
    fn started_maps_the_host_id_to_the_fresh_local_run() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = state_in(dir.path());
        let host = RunId::new_v4();
        let p = pane();
        let local = state.apply_started(host, p, "cargo test", None, false, 0);
        assert_eq!(local, host);
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
        assert_eq!(
            state.host_id_for(local_a),
            Some(host_a),
            "same-pane predecessor stays jumpable until pane close, prune, or clear"
        );
    }

    #[test]
    fn save_then_load_round_trips_and_history_is_seen() {
        let dir = tempfile::tempdir().unwrap();
        let path = default_runs_path(dir.path());
        let mut state = state_in(dir.path());
        let p = pane();
        let host = RunId::new_v4();
        state.apply_started(host, p, "cargo test", None, false, 0);
        state.apply_finished(host, Some(1), 100);
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
        assert_eq!(
            loaded.ledger.runs().next().unwrap().state,
            RunState::Abandoned
        );
    }

    #[test]
    fn finished_uses_the_host_exit_code() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = state_in(dir.path());
        let p = pane();
        let host = RunId::new_v4();
        state.apply_started(host, p, "make", None, false, 0);
        state.apply_finished(host, Some(0), 50);
        let run = state.ledger.runs().next().unwrap();
        assert_eq!(run.state, RunState::Succeeded);
        assert_eq!(run.id, host);
        assert_eq!(run.duration.as_millis(), 50);
        assert_eq!(
            state.host_id_for(host),
            Some(host),
            "finished rows stay jumpable until pane close, prune, or clear"
        );
    }

    #[test]
    fn mark_host_seen_clears_attention_for_a_finished_run() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = state_in(dir.path());
        let host = RunId::new_v4();
        let p = pane();
        state.apply_started(host, p, "boom", None, false, 0);
        state.apply_finished(host, Some(1), 100);
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
        let pane_a = pane();
        let local_a = state.apply_started(host_a, pane_a, "old", None, false, 0);
        state.apply_finished(host_a, Some(0), 5);
        let local_b = state.apply_started(host_b, pane(), "new", None, false, 10);
        state.prune_and_sync();
        assert_eq!(state.ledger.runs().count(), 1);
        assert_eq!(state.local_id_for(host_b), Some(local_b));
        assert_eq!(state.host_id_for(local_a), None);
    }

    #[test]
    fn history_is_not_jumpable_after_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = default_runs_path(dir.path());
        let mut state = state_in(dir.path());
        let host = RunId::new_v4();
        let p = pane();
        state.apply_started(host, p, "cargo test", None, false, 0);
        state.save().unwrap();

        let loaded = LedgerState::load(path);
        let run = loaded.ledger.runs().next().unwrap();
        assert_eq!(run.id, host);
        assert_eq!(loaded.host_id_for(host), None);
    }

    #[test]
    fn completed_prior_run_stays_jumpable_after_next_start_on_the_same_pane() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = state_in(dir.path());
        let p = pane();
        let first = RunId::new_v4();
        let second = RunId::new_v4();
        state.apply_started(first, p, "first", None, false, 0);
        state.apply_finished(first, Some(0), 10);
        state.apply_started(second, p, "second", None, false, 20);

        assert_eq!(state.host_id_for(first), Some(first));
        assert_eq!(state.local_id_for(first), Some(first));
        assert_eq!(state.host_id_for(second), Some(second));
    }

    #[test]
    fn pane_close_drops_all_jump_ids_in_that_pane() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = state_in(dir.path());
        let p = pane();
        let keep_pane = pane();
        let first = RunId::new_v4();
        let second = RunId::new_v4();
        let keep = RunId::new_v4();
        state.apply_started(first, p, "first", None, false, 0);
        state.apply_finished(first, Some(0), 10);
        state.apply_started(second, p, "second", None, false, 20);
        state.apply_started(keep, keep_pane, "keep", None, false, 30);

        state.apply_pane_closed(p, 40);

        assert_eq!(state.host_id_for(first), None);
        assert_eq!(state.host_id_for(second), None);
        assert_eq!(state.host_id_for(keep), Some(keep));
    }

    #[test]
    fn duplicate_start_for_a_historical_id_does_not_make_history_jumpable() {
        let dir = tempfile::tempdir().unwrap();
        let path = default_runs_path(dir.path());
        let host = RunId::new_v4();
        {
            let mut state = state_in(dir.path());
            let p = pane();
            state.apply_started(host, p, "cargo test", None, false, 0);
            state.apply_finished(host, Some(0), 10);
            state.save().unwrap();
        }

        let mut loaded = LedgerState::load(path);
        let p = pane();
        loaded.apply_started(host, p, "cargo test", None, false, 20);

        let runs: Vec<_> = loaded.ledger.runs().collect();
        assert_eq!(runs.len(), 1, "historical duplicate start stays idempotent");
        assert_eq!(runs[0].id, host);
        assert_ne!(runs[0].launch_id, loaded.ledger.launch_id());
        assert_eq!(loaded.host_id_for(host), None);
        assert_eq!(loaded.local_id_for(host), None);
    }

    #[test]
    fn duplicate_finish_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = state_in(dir.path());
        let host = RunId::new_v4();
        let p = pane();
        state.apply_started(host, p, "cargo test", None, false, 0);
        state.apply_finished(host, Some(1), 50);
        state.apply_finished(host, Some(0), 100);
        let run = state.ledger.runs().next().unwrap();
        assert_eq!(run.state, RunState::Failed);
        assert_eq!(run.exit_code, Some(1));
        assert_eq!(run.duration.as_millis(), 50);
    }

    #[test]
    fn duplicate_start_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = state_in(dir.path());
        let host = RunId::new_v4();
        let p = pane();
        state.apply_started(host, p, "cargo test", None, false, 0);
        state.apply_started(host, p, "cargo test", None, false, 10);
        let runs: Vec<_> = state.ledger.runs().collect();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].id, host);
        assert_eq!(state.host_id_for(host), Some(host));
    }

    #[test]
    fn focus_marks_the_whole_pane_seen() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = state_in(dir.path());
        let p = pane();
        state.apply_started(RunId::new_v4(), p, "f1", None, false, 0);
        let host_a = state.ledger.runs().next().unwrap().id;
        state.apply_finished(host_a, Some(1), 10);
        state.apply_started(RunId::new_v4(), p, "f2", None, false, 20);
        let host_b = state.ledger.runs().last().unwrap().id;
        state.apply_finished(host_b, Some(1), 30);
        assert_eq!(state.ledger.failed_attention_count(), 2);
        state.focus(p);
        assert_eq!(state.ledger.failed_attention_count(), 0);
    }

    #[test]
    fn finished_without_a_live_start_does_not_invent_a_run() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = state_in(dir.path());
        state.apply_finished(RunId::new_v4(), Some(1), 100);
        assert_eq!(state.ledger.runs().count(), 0);
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
