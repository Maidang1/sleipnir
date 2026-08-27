//! Driving the diff inspector from the shell: opening and closing it,
//! refreshing its patch, its key handling, and piping a diff to the PTY.
//! The overlay itself is rendered by `crate::diff`.
//!
//! A child module of `app_shell` so it can drive the shell's private diff state
//! without widening it to the crate.

use gpui::{AppContext as _, Context, Window};

use super::{AppShell, SendGitDiff, ToggleDiff};
use crate::ui_mode::OverlayKind;

impl AppShell {
    pub(super) fn on_send_git_diff(
        &mut self,
        _: &SendGitDiff,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.send_git_diff_to_pty(cx);
    }

    pub(super) fn on_toggle_diff(
        &mut self,
        _: &ToggleDiff,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_diff(window, cx);
    }

    pub(crate) fn toggle_diff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode.is(OverlayKind::Diff) {
            self.close_diff(window, cx);
            return;
        }
        self.mode.open(OverlayKind::Diff);
        self.refresh_diff(false, window, cx);
        cx.notify();
    }

    pub(crate) fn close_diff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.mode.close(OverlayKind::Diff) {
            return;
        }
        self.focus_active(window, cx);
        cx.notify();
    }

    pub(crate) fn refresh_diff(
        &mut self,
        force: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cwd = self.active_working_directory(cx).unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
        });
        let root = crate::chrome::workspace::git_root(&cwd).unwrap_or_else(|| cwd.clone());
        if !force {
            if let Some(crate::diff::DiffView::Ready(session)) = self.diff_view.as_ref() {
                if session.still_fresh(&root) {
                    self.mode.open(OverlayKind::Diff);
                    cx.notify();
                    return;
                }
            }
        }
        let mode = match &self.diff_view {
            Some(crate::diff::DiffView::Ready(session)) => session.mode,
            _ => crate::diff::ViewMode::default(),
        };
        let minimap_visible = match &self.diff_view {
            Some(crate::diff::DiffView::Ready(session)) => session.minimap_visible,
            _ => true,
        };
        self.diff_gen = self.diff_gen.wrapping_add(1);
        let generation = self.diff_gen;
        let title = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string());
        self.diff_view = Some(crate::diff::DiffView::Loading { title, generation });
        self.mode.open(OverlayKind::Diff);
        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_spawn(async move { crate::git_service::fetch_worktree_patch(&cwd) })
                .await;
            this.update(cx, |this, cx| {
                if this.diff_gen != generation {
                    return;
                }
                this.diff_view = Some(match outcome {
                    crate::git_service::PatchOutcome::Ready(ready) => {
                        let root = ready.root.clone();
                        let jobs: Vec<crate::diff::upgrade::UpgradeJob> = ready
                            .parsed
                            .files
                            .iter()
                            .enumerate()
                            .filter(|(_, f)| {
                                f.status != diff_core::FileStatus::Binary && !f.hunks.is_empty()
                            })
                            .map(|(ix, f)| crate::diff::upgrade::UpgradeJob {
                                file_ix: ix,
                                old_path: f.old_path.clone(),
                                new_path: f.new_path.clone(),
                                status: f.status,
                            })
                            .collect();
                        let mut session = crate::diff::DiffSession::from_ready(ready, mode);
                        session.minimap_visible = minimap_visible;
                        if !jobs.is_empty() {
                            this.spawn_diff_upgrade(generation, root, jobs, cx);
                        }
                        crate::diff::DiffView::Ready(session)
                    }
                    crate::git_service::PatchOutcome::Clean { title } => {
                        crate::diff::DiffView::Message {
                            title,
                            body: "Working tree clean".into(),
                        }
                    }
                    crate::git_service::PatchOutcome::Failed { title, message } => {
                        crate::diff::DiffView::Message {
                            title,
                            body: message,
                        }
                    }
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn spawn_diff_upgrade(
        &mut self,
        generation: u64,
        root: std::path::PathBuf,
        jobs: Vec<crate::diff::upgrade::UpgradeJob>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let files = cx
                .background_spawn(async move { crate::diff::upgrade::run_upgrade(&root, jobs) })
                .await;
            this.update(cx, |this, cx| {
                if this.diff_gen != generation {
                    return;
                }
                if let Some(crate::diff::DiffView::Ready(session)) = this.diff_view.as_mut() {
                    session.apply_upgrades(files);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn expand_diff_gap(
        &mut self,
        file_ix: usize,
        gap_ix: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(crate::diff::DiffView::Ready(session)) = self.diff_view.as_mut() else {
            return;
        };
        let Some((gap_row, inserted)) = session.expand_gap(file_ix, gap_ix) else {
            return;
        };
        if session.cursor > gap_row {
            session.cursor += inserted;
        }
        cx.notify();
    }

    pub(crate) fn toggle_diff_minimap(&mut self, cx: &mut Context<Self>) {
        if let Some(crate::diff::DiffView::Ready(session)) = self.diff_view.as_mut() {
            session.minimap_visible = !session.minimap_visible;
            cx.notify();
        }
    }

    pub(crate) fn toggle_diff_mode(&mut self, cx: &mut Context<Self>) {
        if let Some(crate::diff::DiffView::Ready(session)) = self.diff_view.as_mut() {
            let next = session.mode.toggle();
            session.set_mode(next);
            cx.notify();
        }
    }

    pub(crate) fn jump_diff_file(&mut self, row: usize, cx: &mut Context<Self>) {
        if let Some(crate::diff::DiffView::Ready(session)) = self.diff_view.as_mut() {
            session.jump_to_row(row);
            cx.notify();
        }
    }

    pub(crate) fn send_open_diff_to_pty(&mut self, cx: &mut Context<Self>) {
        let Some(crate::diff::DiffView::Ready(session)) = self.diff_view.as_ref() else {
            self.send_git_diff_to_pty(cx);
            return;
        };
        let Some(payload) = crate::chrome::send_context::git_diff_payload(&session.patch) else {
            return;
        };
        let Some(view) = self.active_view(cx) else {
            return;
        };
        view.update(cx, |v, cx| v.input_bytes(payload.into_bytes(), cx));
        cx.notify();
    }

    pub(super) fn handle_diff_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.mode.is(OverlayKind::Diff) {
            return false;
        }
        let key = event.keystroke.key.as_str();
        match key {
            "escape" => {
                self.close_diff(window, cx);
                true
            }
            "n" => {
                if let Some(crate::diff::DiffView::Ready(session)) = self.diff_view.as_mut() {
                    let targets = session.hunk_rows.clone();
                    session.jump_next(&targets);
                }
                cx.notify();
                true
            }
            "p" => {
                if let Some(crate::diff::DiffView::Ready(session)) = self.diff_view.as_mut() {
                    let targets = session.hunk_rows.clone();
                    session.jump_prev(&targets);
                }
                cx.notify();
                true
            }
            "]" => {
                if let Some(crate::diff::DiffView::Ready(session)) = self.diff_view.as_mut() {
                    let targets = session.file_rows.clone();
                    session.jump_next(&targets);
                }
                cx.notify();
                true
            }
            "[" => {
                if let Some(crate::diff::DiffView::Ready(session)) = self.diff_view.as_mut() {
                    let targets = session.file_rows.clone();
                    session.jump_prev(&targets);
                }
                cx.notify();
                true
            }
            "v" => {
                self.toggle_diff_mode(cx);
                true
            }
            "m" => {
                self.toggle_diff_minimap(cx);
                true
            }
            "r" => {
                self.refresh_diff(true, window, cx);
                true
            }
            "home" => {
                if let Some(crate::diff::DiffView::Ready(session)) = self.diff_view.as_mut() {
                    session.jump_home();
                }
                cx.notify();
                true
            }
            "end" => {
                if let Some(crate::diff::DiffView::Ready(session)) = self.diff_view.as_mut() {
                    session.jump_end();
                }
                cx.notify();
                true
            }
            _ => false,
        }
    }

    /// Paste the work-tree patch into the focused PTY.
    ///
    /// `git diff` can take seconds on a large repository, so it runs on the
    /// background executor. This path only needs the raw text, so it uses the
    /// cheap `fetch_patch_text` layer rather than paying for a full patch parse.
    pub(super) fn send_git_diff_to_pty(&mut self, cx: &mut Context<Self>) {
        let cwd = self
            .active_working_directory(cx)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let Some(view) = self.active_view(cx) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_spawn(async move { crate::git_service::fetch_patch_text(&cwd) })
                .await;
            let crate::git_service::PatchOutcome::Ready(text) = outcome else {
                return;
            };
            let Some(payload) = crate::chrome::send_context::git_diff_payload(&text.patch) else {
                return;
            };
            this.update(cx, |this, cx| {
                // Focus may have moved while git was running; pasting a patch
                // into whichever pane is focused now would be wrong.
                if this.active_view(cx).as_ref() != Some(&view) {
                    return;
                }
                view.update(cx, |v, cx| v.input_bytes(payload.into_bytes(), cx));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
