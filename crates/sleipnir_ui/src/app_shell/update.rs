//! Auto-update: the check/apply flow and its overlay.
//!
//! A child module of `app_shell` so it can drive the shell's private update
//! state without widening it to the crate.

use gpui::{
    AppContext as _, Context, Hsla, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    deferred, div, px,
};

use super::{AppShell, CheckForUpdates};
use crate::chrome::ChromeTokens;
use crate::ui_mode::OverlayKind;
use crate::{AvailableUpdate, UpdateModel, UpdateUiState};

impl AppShell {
    pub(super) fn on_check_for_updates(
        &mut self,
        _: &CheckForUpdates,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.begin_update_check(window, cx);
    }

    /// Single entry point for "check for updates", shared by the menu action and
    /// the command palette. On platforms without in-place update support this
    /// opens the releases page instead of the dialog.
    pub(super) fn begin_update_check(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !updater::in_place_update_supported() {
            cx.open_url(updater::RELEASES_PAGE);
            return;
        }
        // Open the update dialog and start a check.
        self.mode.open(OverlayKind::Update);
        self.spawn_update_check(window, cx);
    }

    pub(super) fn close_update(&mut self, cx: &mut Context<Self>) {
        self.mode.close(OverlayKind::Update);
        cx.notify();
    }

    /// Query GitHub for a newer release; result is shown in the update dialog.
    pub(crate) fn spawn_update_check(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(
            cx.global::<UpdateModel>().state,
            UpdateUiState::Checking | UpdateUiState::Downloading(_)
        ) {
            return;
        }
        cx.global_mut::<UpdateModel>().state = UpdateUiState::Checking;
        cx.notify();

        let current = release_channel::AppVersion::global(cx).to_string();
        cx.spawn_in(window, async move |this, cx| {
            // ureq is blocking — run it on the background executor so we never
            // touch a (nonexistent) Tokio reactor on the main thread.
            let result = cx
                .background_spawn(async move { updater::fetch_latest(&current) })
                .await;
            this.update(cx, |_this, cx| {
                match result {
                    Ok(updater::UpdateStatus::Available(info)) => {
                        cx.global_mut::<UpdateModel>().state =
                            UpdateUiState::Available(AvailableUpdate {
                                version: info.version.to_string(),
                                tag: info.tag,
                                notes: info.notes,
                                artifact_url: info.artifact_url,
                                sha256_url: info.sha256_url,
                                expected_sha256: info.expected_sha256,
                                expected_size: info.expected_size,
                            });
                    }
                    Ok(updater::UpdateStatus::UpToDate) => {
                        cx.global_mut::<UpdateModel>().state = UpdateUiState::UpToDate;
                    }
                    Err(err) => {
                        log::warn!("update check failed: {err:#}");
                        cx.global_mut::<UpdateModel>().state =
                            UpdateUiState::Failed(format!("Update check failed: {err}"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Download + verify the available release, then stage it for restart.
    fn start_download(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let update = match &cx.global::<UpdateModel>().state {
            UpdateUiState::Available(u) => u.clone(),
            _ => return,
        };
        cx.global_mut::<UpdateModel>().state = UpdateUiState::Downloading(update.clone());
        cx.notify();

        let info = updater::ReleaseInfo {
            version: match updater::parse_tag(&update.tag) {
                Ok(v) => v,
                Err(err) => {
                    cx.global_mut::<UpdateModel>().state = UpdateUiState::Failed(format!("{err}"));
                    cx.notify();
                    return;
                }
            },
            tag: update.tag.clone(),
            notes: update.notes.clone(),
            artifact_url: update.artifact_url.clone(),
            sha256_url: update.sha256_url.clone(),
            expected_sha256: update.expected_sha256.clone(),
            expected_size: update.expected_size,
        };
        let dest = std::env::temp_dir().join(format!("sleipnir-update-{}", std::process::id()));

        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { updater::download_and_verify(&info, &dest) })
                .await;
            this.update(cx, |_this, cx| {
                match result {
                    Ok(dmg_path) => {
                        // Remember where the verified dmg landed so a restart
                        // can install it.
                        cx.global_mut::<UpdateModel>().staged_dmg = Some(dmg_path);
                        cx.global_mut::<UpdateModel>().state =
                            UpdateUiState::ReadyToRestart(update.clone());
                    }
                    Err(err) => {
                        log::warn!("update download failed: {err:#}");
                        cx.global_mut::<UpdateModel>().state =
                            UpdateUiState::Failed(format!("Download failed: {err}"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Hand the staged update to the transactional supervisor and quit only
    /// after it has durably registered its old-process exit watch.
    fn install_and_restart(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dmg) = cx.global::<UpdateModel>().staged_dmg.clone() else {
            return;
        };
        let update = match &cx.global::<UpdateModel>().state {
            UpdateUiState::ReadyToRestart(update) => update.clone(),
            _ => return,
        };
        let Some(app) = updater::current_app_bundle_path() else {
            cx.global_mut::<UpdateModel>().state = UpdateUiState::ManualInstallRequired {
                artifact: dmg,
                message: "This build is not running from an application bundle.".into(),
            };
            cx.notify();
            return;
        };
        cx.global_mut::<UpdateModel>().state = UpdateUiState::WaitingForHelper(update);
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let install_dmg = dmg.clone();
            let result = cx
                .background_spawn(async move { updater::install_and_relaunch(&install_dmg, &app) })
                .await;
            this.update(cx, |_this, cx| match result {
                Ok(()) => cx.quit(),
                Err(err) => {
                    log::warn!("install failed: {err:#}");
                    cx.global_mut::<UpdateModel>().state = UpdateUiState::ManualInstallRequired {
                        artifact: dmg,
                        message: format!("Automatic install failed: {err}"),
                    };
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// A pill button for the update dialog.
    fn update_button(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        tokens: &ChromeTokens,
        primary: bool,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        let (bg, fg) = if primary {
            (tokens.accent, tokens.content_bg)
        } else {
            (tokens.hover, tokens.fg)
        };
        div()
            .id(id)
            .px_3()
            .py_1p5()
            .rounded_md()
            .bg(bg)
            .text_color(fg)
            .text_size(px(13.0))
            .cursor_pointer()
            .hover(|el| el.opacity(0.9))
            .child(label.into())
            .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
    }

    /// Modal update dialog reflecting the current [`UpdateUiState`].
    pub(super) fn render_update_overlay(
        &self,
        tokens: &ChromeTokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let current = release_channel::AppVersion::global(cx).to_string();

        // Headline + detail + action buttons per state.
        let (headline, detail, buttons): (SharedString, SharedString, Vec<gpui::AnyElement>) =
            match &cx.global::<UpdateModel>().state {
                UpdateUiState::Idle | UpdateUiState::Checking => (
                    "Checking for updates…".into(),
                    format!("Current version {current}").into(),
                    Vec::new(),
                ),
                UpdateUiState::UpToDate => (
                    "You’re up to date".into(),
                    format!("Sleipnir {current} is the latest version.").into(),
                    vec![
                        self.update_button(
                            "upd-close",
                            "Close",
                            tokens,
                            true,
                            cx,
                            |this, _, cx| {
                                this.close_update(cx);
                            },
                        )
                        .into_any_element(),
                    ],
                ),
                UpdateUiState::Available(u) => (
                    "Update available".into(),
                    format!("Sleipnir {} is available (you have {current}).", u.version).into(),
                    vec![
                        self.update_button(
                            "upd-notes",
                            "Release Notes",
                            tokens,
                            false,
                            cx,
                            |_, _, cx| cx.open_url(updater::RELEASES_PAGE),
                        )
                        .into_any_element(),
                        self.update_button(
                            "upd-later",
                            "Later",
                            tokens,
                            false,
                            cx,
                            |this, _, cx| {
                                this.close_update(cx);
                            },
                        )
                        .into_any_element(),
                        self.update_button(
                            "upd-install",
                            "Download Update",
                            tokens,
                            true,
                            cx,
                            |this, window, cx| this.start_download(window, cx),
                        )
                        .into_any_element(),
                    ],
                ),
                UpdateUiState::Downloading(u) => (
                    "Downloading update…".into(),
                    format!("Fetching and verifying Sleipnir {}…", u.version).into(),
                    Vec::new(),
                ),
                UpdateUiState::ReadyToRestart(u) => (
                    "Ready to install".into(),
                    format!(
                        "Sleipnir {} is downloaded and verified. Restarting will close running terminal processes.",
                        u.version
                    )
                    .into(),
                    vec![
                        self.update_button(
                            "upd-later2",
                            "Later",
                            tokens,
                            false,
                            cx,
                            |this, _, cx| {
                                this.close_update(cx);
                            },
                        )
                        .into_any_element(),
                        self.update_button(
                            "upd-restart",
                            "Restart & Update",
                            tokens,
                            true,
                            cx,
                            |this, window, cx| this.install_and_restart(window, cx),
                        )
                        .into_any_element(),
                    ],
                ),
                UpdateUiState::WaitingForHelper(u) => (
                    "Restarting…".into(),
                    format!("Sleipnir {} will open after the update is verified.", u.version).into(),
                    Vec::new(),
                ),
                UpdateUiState::Updated { version } => (
                    "Update complete".into(),
                    format!("Sleipnir was updated to {version}.").into(),
                    vec![self.update_button("upd-ack-success", "Close", tokens, true, cx, |this, _, cx| {
                        let _ = updater::install::acknowledge_active_outcome();
                        cx.global_mut::<UpdateModel>().state = UpdateUiState::Idle;
                        this.close_update(cx);
                    }).into_any_element()],
                ),
                UpdateUiState::RolledBack { from, to, reason } => (
                    "Update couldn’t be completed".into(),
                    format!("Sleipnir {from} failed to start ({reason}), so version {to} was restored.").into(),
                    vec![self.update_button("upd-ack-rollback", "Close", tokens, true, cx, |this, _, cx| {
                        let _ = updater::install::acknowledge_active_outcome();
                        cx.global_mut::<UpdateModel>().state = UpdateUiState::Idle;
                        this.close_update(cx);
                    }).into_any_element()],
                ),
                UpdateUiState::ManualInstallRequired { artifact, message } => (
                    "Install update manually".into(),
                    format!("{message} Verified disk image: {}", artifact.display()).into(),
                    vec![
                        self.update_button("upd-manual-open", "Open Disk Image", tokens, false, cx, {
                            let artifact = artifact.clone();
                            move |_, _, _| {
                                let _ = std::process::Command::new("/usr/bin/open").arg(&artifact).spawn();
                            }
                        }).into_any_element(),
                        self.update_button("upd-ack-manual", "Close", tokens, true, cx, |this, _, cx| {
                            let _ = updater::install::acknowledge_active_outcome();
                            cx.global_mut::<UpdateModel>().state = UpdateUiState::Idle;
                            this.close_update(cx);
                        }).into_any_element(),
                    ],
                ),
                UpdateUiState::RecoveryRequired { message } => (
                    "Update needs manual recovery".into(),
                    message.clone().into(),
                    vec![
                        self.update_button("upd-recovery-releases", "Open Releases", tokens, false, cx, |_, _, cx| cx.open_url(updater::RELEASES_PAGE)).into_any_element(),
                        self.update_button("upd-ack-recovery", "Close", tokens, true, cx, |this, _, cx| {
                            let _ = updater::install::acknowledge_active_outcome();
                            cx.global_mut::<UpdateModel>().state = UpdateUiState::Idle;
                            this.close_update(cx);
                        }).into_any_element(),
                    ],
                ),
                UpdateUiState::Failed(msg) => (
                    "Update failed".into(),
                    msg.clone().into(),
                    vec![
                        self.update_button(
                            "upd-close2",
                            "Close",
                            tokens,
                            true,
                            cx,
                            |this, _, cx| {
                                this.close_update(cx);
                            },
                        )
                        .into_any_element(),
                    ],
                ),
            };

        let panel = div()
            .id("update-panel")
            .w(px(420.0))
            .flex()
            .flex_col()
            .gap_3()
            .rounded(px(12.0))
            .bg(tokens.surface)
            .border_1()
            .border_color(tokens.border)
            .text_color(tokens.fg)
            .px_5()
            .py_4()
            // Keep clicks inside the panel from reaching the backdrop.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .text_size(px(15.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(headline),
            )
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(tokens.fg_muted)
                    .child(detail),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_end()
                    .gap_2()
                    .pt_2()
                    .children(buttons),
            );

        deferred(
            div()
                .id("update-overlay")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                // BlockMouse: otherwise TermElement under the overlay still
                // sees should_handle_scroll() and the terminal scrolls too.
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .id("update-backdrop")
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .bg(Hsla::black().opacity(0.5))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _window, cx| {
                                this.close_update(cx);
                            }),
                        ),
                )
                .child(panel),
        )
    }
}
