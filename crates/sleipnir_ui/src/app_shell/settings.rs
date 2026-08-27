//! Settings overlay: section tabs, the general section, and the theme picker.
//!
//! A child module of `app_shell` so it can use `AppShell`'s private state
//! (`settings_section`, `theme_query`) without widening it to the crate.

use gpui::{
    ClickEvent, Context, ElementId, Hsla, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    deferred, div, prelude::FluentBuilder as _, px,
};
use sleipnir_settings::{TerminalSettings, ThemeName, ThemeSetting, palette_for_theme};

use super::{AppShell, OpenSettings, SettingsSection, appearance_of};
use crate::chrome::ChromeTokens;
use crate::ui_mode::OverlayKind;

impl AppShell {
    pub(super) fn on_open_settings(
        &mut self,
        _: &OpenSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_settings(window, cx);
    }

    pub(crate) fn toggle_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode.toggle(OverlayKind::Settings) {
            // Always land on Theme when reopening; future sections can restore.
            self.settings_section = SettingsSection::Theme;
        } else {
            self.theme_query.clear();
            self.focus_active(window, cx);
        }
        cx.notify();
    }

    /// Open Settings on the Theme section. Unlike `toggle_settings` this never
    /// closes it, because picking "Settings" from the palette should always land
    /// there.
    pub(super) fn open_settings(&mut self, cx: &mut Context<Self>) {
        self.mode.open(OverlayKind::Settings);
        self.settings_section = SettingsSection::Theme;
        cx.notify();
    }

    pub(super) fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode.close(OverlayKind::Settings) {
            self.theme_query.clear();
            self.focus_active(window, cx);
            cx.notify();
        }
    }

    fn select_settings_section(&mut self, section: SettingsSection, cx: &mut Context<Self>) {
        if self.settings_section != section {
            self.settings_section = section;
            cx.notify();
        }
    }

    fn select_theme(&mut self, theme: ThemeName, cx: &mut Context<Self>) {
        TerminalSettings::set_theme(ThemeSetting::Builtin(theme), cx);
        cx.notify();
    }

    fn select_custom_theme(&mut self, name: String, cx: &mut Context<Self>) {
        TerminalSettings::set_theme(ThemeSetting::Custom(name), cx);
        cx.notify();
    }

    pub(super) fn render_settings_overlay(
        &self,
        tokens: &ChromeTokens,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let section = self.settings_section;

        // ── macOS-style segmented control ────────────────────────────────
        let seg_bg = tokens.hover;
        let seg_active_bg = tokens.surface;
        let mut segmented_control = div()
            .id("settings-segment")
            .flex()
            .flex_row()
            .items_center()
            .rounded(px(8.0))
            .bg(seg_bg)
            .p(px(3.0))
            .gap(px(2.0));

        for &s in SettingsSection::ALL {
            let active = s == section;
            let tab_id: ElementId = format!("settings-section-{}", s.id()).into();
            let label: SharedString = s.label().into();
            segmented_control = segmented_control.child(
                div()
                    .id(tab_id)
                    .cursor_pointer()
                    .px(px(16.0))
                    .py(px(5.0))
                    .rounded(px(6.0))
                    .text_size(px(12.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .when(active, |el| {
                        el.bg(seg_active_bg).text_color(tokens.fg).shadow(vec![
                            gpui::BoxShadow {
                                color: Hsla::black().opacity(0.08),
                                offset: gpui::point(px(0.0), px(1.0)),
                                blur_radius: px(3.0),
                                spread_radius: px(0.0),
                                inset: false,
                            },
                            gpui::BoxShadow {
                                color: Hsla::black().opacity(0.04),
                                offset: gpui::point(px(0.0), px(0.5)),
                                blur_radius: px(1.0),
                                spread_radius: px(0.0),
                                inset: false,
                            },
                        ])
                    })
                    .when(!active, |el| {
                        el.text_color(tokens.fg_muted)
                            .hover(|el| el.text_color(tokens.fg))
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                        this.select_settings_section(s, cx);
                    }))
                    .child(label),
            );
        }

        // ── Body for the active section ──────────────────────────────────
        let body = match section {
            SettingsSection::Theme => self
                .render_settings_theme_section(tokens, window, cx)
                .into_any_element(),
            SettingsSection::General => self
                .render_settings_general_section(tokens, cx)
                .into_any_element(),
        };

        // ── Footer ─────────────────────────────────────────────────────────
        let footer = div()
            .id("settings-footer")
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .w_full()
            .px(px(20.0))
            .py(px(12.0))
            .border_t_1()
            .border_color(tokens.border)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(12.0))
                    .text_size(px(11.0))
                    .text_color(tokens.fg_muted)
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(4.0))
                            .child(
                                div()
                                    .px(px(5.0))
                                    .py(px(1.0))
                                    .rounded(px(4.0))
                                    .bg(tokens.hover)
                                    .border_1()
                                    .border_color(tokens.border)
                                    .text_size(px(10.0))
                                    .text_color(tokens.fg)
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .child("esc"),
                            )
                            .child("close"),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(tokens.fg_muted)
                    .child(sleipnir_settings::config_path().display().to_string()),
            );

        let panel = div()
            .id("settings-panel")
            .w(px(520.0))
            .max_w(px(640.0))
            .h(px(500.0))
            .max_h(px(580.0))
            .flex()
            .flex_col()
            .rounded(px(12.0))
            .bg(tokens.surface)
            .border_1()
            .border_color(tokens.border)
            .text_color(tokens.fg)
            .overflow_hidden()
            .shadow(vec![
                gpui::BoxShadow {
                    color: Hsla::black().opacity(0.25),
                    offset: gpui::point(px(0.0), px(8.0)),
                    blur_radius: px(32.0),
                    spread_radius: px(0.0),
                    inset: false,
                },
                gpui::BoxShadow {
                    color: Hsla::black().opacity(0.12),
                    offset: gpui::point(px(0.0), px(2.0)),
                    blur_radius: px(8.0),
                    spread_radius: px(0.0),
                    inset: false,
                },
            ])
            // Keep clicks inside the panel from reaching the backdrop.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            // Header: title + segmented control
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .w_full()
                    .pt(px(20.0))
                    .pb(px(16.0))
                    .gap(px(14.0))
                    .child(
                        div()
                            .text_size(px(15.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(tokens.fg)
                            .child("Settings"),
                    )
                    .child(segmented_control),
            )
            // Separator
            .child(div().w_full().h(px(1.0)).bg(tokens.border))
            // Scrollable body
            .child(
                div()
                    .id("settings-body")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scroll()
                    .px(px(20.0))
                    .py(px(16.0))
                    .child(body),
            )
            // Footer
            .child(footer);

        deferred(
            div()
                .id("settings-overlay")
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
                        .id("settings-backdrop")
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .bg(Hsla::black().opacity(0.45))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, window, cx| {
                                this.close_settings(window, cx);
                            }),
                        ),
                )
                .child(panel),
        )
    }

    /// General section: session restore, ligatures, and pointers for advanced config.
    fn render_settings_general_section(
        &self,
        tokens: &ChromeTokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let settings = TerminalSettings::get_global(cx);
        let restore = settings.restore_session;
        let ligatures = settings.font_ligatures;

        // Card background: slightly elevated from surface
        let card_bg = tokens.hover;

        div()
            .id("settings-general")
            .flex()
            .flex_col()
            .gap(px(20.0))
            .w_full()
            // ── Application group ─────────────────────────────────────────
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(tokens.fg_muted)
                            .pl(px(2.0))
                            .child("APPLICATION"),
                    )
                    .child(
                        div()
                            .rounded(px(10.0))
                            .bg(card_bg)
                            .border_1()
                            .border_color(tokens.border)
                            .overflow_hidden()
                            .child(self.settings_toggle_row(
                                "restore-session",
                                "Restore session on launch",
                                "Reopen tabs, splits, and working directories from the last quit",
                                restore,
                                tokens,
                                cx,
                                |this, cx| {
                                    let next = !TerminalSettings::get_global(cx).restore_session;
                                    TerminalSettings::set_restore_session(next, cx);
                                    if next {
                                        this.schedule_session_save(cx);
                                    }
                                    cx.notify();
                                },
                            )),
                    ),
            )
            // ── Terminal group ────────────────────────────────────────────
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(tokens.fg_muted)
                            .pl(px(2.0))
                            .child("TERMINAL"),
                    )
                    .child(
                        div()
                            .rounded(px(10.0))
                            .bg(card_bg)
                            .border_1()
                            .border_color(tokens.border)
                            .overflow_hidden()
                            .child(self.settings_toggle_row(
                                "font-ligatures",
                                "Font ligatures",
                                "Enable OpenType ligatures when the font supports them (e.g. JetBrains Mono)",
                                ligatures,
                                tokens,
                                cx,
                                |_this, cx| {
                                    let next = !TerminalSettings::get_global(cx).font_ligatures;
                                    TerminalSettings::set_font_ligatures(next, cx);
                                    cx.notify();
                                },
                            ))
                            // Separator between rows
                            .child(
                                div()
                                    .w_full()
                                    .pl(px(14.0))
                                    .child(
                                        div()
                                            .w_full()
                                            .h(px(1.0))
                                            .bg(tokens.border),
                                    ),
                            )
                            .child(self.settings_toggle_row(
                                "copy-on-select",
                                "Copy on select",
                                "Copy selected text when you release the mouse; shows a brief \u{201c}copied to clipboard\u{201d} toast",
                                TerminalSettings::get_global(cx).copy_on_select,
                                tokens,
                                cx,
                                |_this, cx| {
                                    let next = !TerminalSettings::get_global(cx).copy_on_select;
                                    TerminalSettings::set_copy_on_select(next, cx);
                                    cx.notify();
                                },
                            )),
                    ),
            )
            // ── Advanced card ────────────────────────────────────────────
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(tokens.fg_muted)
                            .pl(px(2.0))
                            .child("ADVANCED"),
                    )
                    .child(
                        div()
                            .rounded(px(10.0))
                            .bg(card_bg)
                            .border_1()
                            .border_color(tokens.border)
                            .px(px(14.0))
                            .py(px(12.0))
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(tokens.fg_muted)
                                    .child(
                                        format!(
                                            "Custom key bindings, font family/size, and shell options live in settings.json. Reload with {}.",
                                            crate::display_shortcut("reload_settings")
                                        ),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(tokens.accent)
                                    .child(sleipnir_settings::config_path().display().to_string()),
                            ),
                    ),
            )
    }

    fn settings_toggle_row(
        &self,
        id: &'static str,
        title: &'static str,
        description: &'static str,
        enabled: bool,
        tokens: &ChromeTokens,
        cx: &mut Context<Self>,
        on_toggle: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        // Toggle switch colors
        let track_bg = if enabled {
            tokens.accent
        } else {
            tokens.border
        };
        let knob_bg = if enabled {
            Hsla::white()
        } else {
            Hsla::white().opacity(0.9)
        };
        let knob_offset = if enabled { px(16.0) } else { px(2.0) };

        div()
            .id(id)
            .flex()
            .flex_row()
            .items_center()
            .gap(px(12.0))
            .w_full()
            .px(px(14.0))
            .py(px(10.0))
            .cursor_pointer()
            .hover(|el| el.bg(Hsla::white().opacity(0.03)))
            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                on_toggle(this, cx);
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(tokens.fg)
                            .child(SharedString::from(title)),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(tokens.fg_muted)
                            .child(SharedString::from(description)),
                    ),
            )
            // macOS-style toggle switch
            .child(
                div()
                    .flex_shrink_0()
                    .w(px(34.0))
                    .h(px(20.0))
                    .rounded(px(10.0))
                    .bg(track_bg)
                    .relative()
                    .child(
                        div()
                            .absolute()
                            .top(px(2.0))
                            .left(knob_offset)
                            .w(px(16.0))
                            .h(px(16.0))
                            .rounded(px(8.0))
                            .bg(knob_bg)
                            .shadow(vec![gpui::BoxShadow {
                                color: Hsla::black().opacity(0.15),
                                offset: gpui::point(px(0.0), px(1.0)),
                                blur_radius: px(2.0),
                                spread_radius: px(0.0),
                                inset: false,
                            }]),
                    ),
            )
    }

    /// Theme section body: selectable list with ANSI swatches (type to filter).
    fn render_settings_theme_section(
        &self,
        tokens: &ChromeTokens,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let current = TerminalSettings::get_global(cx).theme.clone();
        let appearance = appearance_of(window.appearance());
        let query = self.theme_query.trim().to_lowercase();
        let matches = |hay: &str| query.is_empty() || hay.to_lowercase().contains(&query);

        let mut list = div()
            .id("settings-theme-list")
            .flex()
            .flex_col()
            .gap(px(2.0))
            .w_full();

        // Type-to-filter: macOS-style search field.
        let filter_text: SharedString = if self.theme_query.is_empty() {
            "Search themes…".into()
        } else {
            format!("{}|", self.theme_query).into()
        };
        list = list.child(
            div()
                .px(px(10.0))
                .py(px(7.0))
                .mb(px(8.0))
                .rounded(px(8.0))
                .bg(tokens.hover)
                .border_1()
                .border_color(tokens.border)
                .text_size(px(12.0))
                .text_color(if self.theme_query.is_empty() {
                    tokens.fg_muted
                } else {
                    tokens.fg
                })
                .child(filter_text),
        );

        let mut rendered = 0;
        for &theme in ThemeName::ALL {
            if !matches(theme.display_name()) && !matches(theme.as_str()) {
                continue;
            }
            rendered += 1;
            let selected = ThemeSetting::Builtin(theme) == current;
            let preview = palette_for_theme(theme, appearance);
            let label: SharedString = theme.display_name().into();
            let row_id: ElementId = format!("theme-row-{}", theme.as_str()).into();

            let mut swatches = div().flex().flex_row().items_center().gap(px(3.0));
            let swatch_colors = [
                preview.background,
                preview.ansi[1],
                preview.ansi[2],
                preview.ansi[3],
                preview.ansi[4],
                preview.ansi[5],
                preview.ansi[6],
            ];
            for (i, color) in swatch_colors.into_iter().enumerate() {
                swatches = swatches.child(
                    div()
                        .id(format!("swatch-{}-{}", theme.as_str(), i))
                        .w(px(12.0))
                        .h(px(12.0))
                        .rounded(px(3.0))
                        .bg(color)
                        .border_1()
                        .border_color(Hsla::black().opacity(0.1)),
                );
            }

            // Radio-style selection indicator
            let radio = div()
                .w(px(16.0))
                .h(px(16.0))
                .rounded(px(8.0))
                .border_2()
                .flex()
                .items_center()
                .justify_center()
                .when(selected, |el| {
                    el.border_color(tokens.accent).child(
                        div()
                            .w(px(8.0))
                            .h(px(8.0))
                            .rounded(px(4.0))
                            .bg(tokens.accent),
                    )
                })
                .when(!selected, |el| el.border_color(tokens.fg_muted));

            let row = div()
                .id(row_id)
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .w_full()
                .px(px(10.0))
                .py(px(8.0))
                .rounded(px(8.0))
                .cursor_pointer()
                .when(selected, |el| el.bg(tokens.hover))
                .hover(|el| el.bg(tokens.hover))
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    this.select_theme(theme, cx);
                }))
                .child(radio)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(13.0))
                        .text_color(tokens.fg)
                        .child(label),
                )
                .child(swatches);

            list = list.child(row);
        }

        // User theme catalog (`themes.json`), listed after the built-ins.
        let catalog = TerminalSettings::user_themes(cx);
        if !catalog.is_empty() {
            let mut names: Vec<&String> = catalog.keys().collect();
            names.sort();
            list = list.child(
                div()
                    .px(px(10.0))
                    .pt(px(12.0))
                    .pb(px(4.0))
                    .text_size(px(11.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(tokens.fg_muted)
                    .child(SharedString::from("USER THEMES")),
            );
            for name in names {
                if !matches(name) {
                    continue;
                }
                rendered += 1;
                let name = name.clone();
                let selected = current == ThemeSetting::Custom(name.clone());
                let preview = catalog[&name].to_palette();
                let label: SharedString = name.clone().into();
                let row_id: ElementId = format!("theme-row-custom-{name}").into();

                let mut swatches = div().flex().flex_row().items_center().gap(px(3.0));
                let swatch_colors = [
                    preview.background,
                    preview.ansi[1],
                    preview.ansi[2],
                    preview.ansi[3],
                    preview.ansi[4],
                    preview.ansi[5],
                    preview.ansi[6],
                ];
                for (i, color) in swatch_colors.into_iter().enumerate() {
                    swatches = swatches.child(
                        div()
                            .id(format!("swatch-custom-{name}-{i}"))
                            .w(px(12.0))
                            .h(px(12.0))
                            .rounded(px(3.0))
                            .bg(color)
                            .border_1()
                            .border_color(Hsla::black().opacity(0.1)),
                    );
                }

                let radio = div()
                    .w(px(16.0))
                    .h(px(16.0))
                    .rounded(px(8.0))
                    .border_2()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(selected, |el| {
                        el.border_color(tokens.accent).child(
                            div()
                                .w(px(8.0))
                                .h(px(8.0))
                                .rounded(px(4.0))
                                .bg(tokens.accent),
                        )
                    })
                    .when(!selected, |el| el.border_color(tokens.fg_muted));

                let row = div()
                    .id(row_id)
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .w_full()
                    .px(px(10.0))
                    .py(px(8.0))
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .when(selected, |el| el.bg(tokens.hover))
                    .hover(|el| el.bg(tokens.hover))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                        this.select_custom_theme(name.clone(), cx);
                    }))
                    .child(radio)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(13.0))
                            .text_color(tokens.fg)
                            .child(label),
                    )
                    .child(swatches);

                list = list.child(row);
            }
        }

        if rendered == 0 {
            list = list.child(
                div()
                    .px(px(10.0))
                    .py(px(12.0))
                    .text_size(px(12.0))
                    .text_color(tokens.fg_muted)
                    .child(SharedString::from("No themes match")),
            );
        }

        list
    }
}
