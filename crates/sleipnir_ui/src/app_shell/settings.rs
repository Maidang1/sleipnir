//! Settings overlay: section tabs, the general section, and the theme picker.
//!
//! A child module of `app_shell` so it can use `AppShell`'s private state
//! (`settings_section`, `theme_query`) without widening it to the crate.

use gpui::{
    ClickEvent, Context, ElementId, Hsla, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    deferred, div, prelude::FluentBuilder as _, px,
};
use sleipnir_settings::{
    TerminalSettings, ThemeName, ThemeSetting, UiStyle, default_font_family, palette_for_theme,
};

use super::{AppShell, OpenSettings, SettingsSection, appearance_of};
use crate::chrome::ChromeTokens;
use crate::chrome::pixel;
use crate::ui_mode::OverlayKind;

#[derive(Clone, Debug)]
enum ThemeItemKind {
    Builtin(ThemeName),
    Custom(String),
}

#[derive(Clone, Debug)]
struct ThemeItem {
    kind: ThemeItemKind,
    scroll_ix: usize,
}

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
            self.reset_theme_selection(cx);
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
        self.reset_theme_selection(cx);
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

    /// One keyboard-navigable row in the theme picker. `scroll_ix` is the
    /// row's child index inside the scrollable list (the `# USER THEMES`
    /// header counts as a child), which is what `ScrollHandle::scroll_to_item`
    /// addresses.
    fn filtered_theme_items(&self, cx: &gpui::App) -> Vec<ThemeItem> {
        let query = self.theme_query.trim().to_lowercase();
        let matches = |hay: &str| query.is_empty() || hay.to_lowercase().contains(&query);
        let mut items: Vec<ThemeItem> = Vec::new();
        for &theme in ThemeName::ALL {
            if matches(theme.display_name()) || matches(theme.as_str()) {
                items.push(ThemeItem {
                    kind: ThemeItemKind::Builtin(theme),
                    scroll_ix: 0,
                });
            }
        }
        let catalog = TerminalSettings::user_themes(cx);
        let mut names: Vec<&String> = catalog.keys().filter(|n| matches(n)).collect();
        names.sort();
        let mut ix = 0;
        for item in items.iter_mut() {
            item.scroll_ix = ix;
            ix += 1;
        }
        if !names.is_empty() {
            ix += 1; // the "# USER THEMES" header row
        }
        for name in names {
            items.push(ThemeItem {
                kind: ThemeItemKind::Custom(name.clone()),
                scroll_ix: ix,
            });
            ix += 1;
        }
        items
    }

    /// Point the keyboard selection at the applied theme and scroll to it.
    fn reset_theme_selection(&mut self, cx: &mut Context<Self>) {
        let current = TerminalSettings::get_global(cx).theme.clone();
        let items = self.filtered_theme_items(cx);
        let ix = items
            .iter()
            .position(|item| {
                match &item.kind {
                    ThemeItemKind::Builtin(t) => current == ThemeSetting::Builtin(*t),
                    ThemeItemKind::Custom(n) => current == ThemeSetting::Custom(n.clone()),
                }
            })
            .unwrap_or(0);
        self.settings_theme_selected = ix;
        if let Some(item) = items.get(ix) {
            self.settings_theme_scroll.scroll_to_item(item.scroll_ix);
        }
    }

    /// Arrow/enter navigation for the theme picker (called from the AppShell
    /// key-down chain while the settings overlay is on the Theme section).
    pub(super) fn settings_theme_key_down(&mut self, key: &str, cx: &mut Context<Self>) {
        let items = self.filtered_theme_items(cx);
        if items.is_empty() {
            return;
        }
        let last = items.len() - 1;
        let selected = self.settings_theme_selected.min(last);
        match key {
            "up" | "arrowup" => {
                self.settings_theme_selected = if selected == 0 { last } else { selected - 1 };
            }
            "down" | "arrowdown" => {
                self.settings_theme_selected = if selected == last { 0 } else { selected + 1 };
            }
            "enter" => {
                if let Some(item) = items.get(selected) {
                    match &item.kind {
                        ThemeItemKind::Builtin(theme) => self.select_theme(*theme, cx),
                        ThemeItemKind::Custom(name) => self.select_custom_theme(name.clone(), cx),
                    }
                }
                return;
            }
            _ => {}
        }
        if let Some(item) = items.get(self.settings_theme_selected) {
            self.settings_theme_scroll.scroll_to_item(item.scroll_ix);
        }
        cx.notify();
    }

    pub(super) fn render_settings_overlay(
        &self,
        tokens: &ChromeTokens,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let section = self.settings_section;
        let style = pixel::active_style(cx);
        let border_w = pixel::border_width(style, px(1.0));

        // ── Pixel-terminal tab strip: active section gets a boxed label ──
        let mut tab_strip = div()
            .id("settings-segment")
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0));

        for &s in SettingsSection::ALL {
            let active = s == section;
            let tab_id: ElementId = format!("settings-section-{}", s.id()).into();
            let label: SharedString = if active {
                format!("[ {} ]", s.label()).into()
            } else {
                format!("  {}  ", s.label()).into()
            };
            tab_strip = tab_strip.child(
                div()
                    .id(tab_id)
                    .cursor_pointer()
                    .px(px(8.0))
                    .py(px(4.0))
                    .border(border_w)
                    .text_size(px(12.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .when(active, |el| {
                        el.bg(tokens.hover)
                            .border_color(tokens.accent)
                            .text_color(tokens.accent)
                    })
                    .when(!active, |el| {
                        el.border_color(tokens.surface)
                            .text_color(tokens.fg_muted)
                            .hover(|el| el.text_color(tokens.fg).border_color(tokens.border))
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
            SettingsSection::Shortcuts => self
                .render_settings_shortcuts_section(tokens, style)
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
            .border_t(border_w)
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
                                    .bg(tokens.hover)
                                    .border(border_w)
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

        let mono: SharedString = TerminalSettings::get_global(cx)
            .font_family
            .clone()
            .unwrap_or_else(|| default_font_family().into())
            .into();

        let panel = div()
            .id("settings-panel")
            .w(px(560.0))
            .max_w(px(680.0))
            .h(px(520.0))
            .max_h(px(580.0))
            .flex()
            .flex_col()
            .relative()
            .when(style == UiStyle::Default, |el| {
                el.bg(tokens.surface)
                    .border_2()
                    .border_color(tokens.border)
            })
            .text_color(tokens.fg)
            .font_family(mono)
            .overflow_hidden()
            // Hard offset shadow, no blur: the pixel-art drop shadow.
            .shadow(vec![gpui::BoxShadow {
                color: Hsla::black().opacity(0.55),
                offset: gpui::point(px(6.0), px(6.0)),
                blur_radius: px(0.0),
                spread_radius: px(0.0),
                inset: false,
            }])
            // Keep clicks inside the panel from reaching the backdrop.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            // Pixel mode: staircase-corner background painted behind content.
            .when(style == UiStyle::Pixel, |el| {
                el.child(pixel::pixel_panel_bg(tokens.surface, tokens.border))
            })
            // Header: command-line style title + tab strip
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w_full()
                    .px(px(16.0))
                    .pt(px(14.0))
                    .pb(px(12.0))
                    .gap(px(12.0))
                    .border_b(border_w)
                    .border_color(tokens.border)
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.0))
                            .text_size(px(13.0))
                            .child(
                                div()
                                    .text_color(tokens.accent)
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child(">"),
                            )
                            .child(
                                div()
                                    .text_color(tokens.fg)
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child("settings"),
                            )
                            .child(div().text_color(tokens.accent).child("█")),
                    )
                    .child(tab_strip),
            )
            // Scrollable body (the theme picker owns its own list scrolling
            // so arrow-key navigation can scroll-follow).
            .child(
                div()
                    .id("settings-body")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .when(section == SettingsSection::Theme, |el| el.overflow_hidden())
                    .when(section != SettingsSection::Theme, |el| el.overflow_y_scroll())
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

    /// General section: ligatures, copy-on-select, and pointers for advanced config.
    fn render_settings_general_section(
        &self,
        tokens: &ChromeTokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let settings = TerminalSettings::get_global(cx);
        let ligatures = settings.font_ligatures;
        let style = settings.ui_style;
        let border_w = pixel::border_width(style, px(1.0));

        // Card background: slightly elevated from surface
        let card_bg = tokens.hover;

        div()
            .id("settings-general")
            .flex()
            .flex_col()
            .gap(px(20.0))
            .w_full()
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
                            .child("# TERMINAL"),
                    )
                    .child(
                        div()
                            .bg(card_bg)
                            .border(border_w)
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
                            .child("# ADVANCED"),
                    )
                    .child(
                        div()
                            .bg(card_bg)
                            .border(border_w)
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

    /// Read-only shortcut reference generated from the same command catalog
    /// used by the command palette, so labels stay in sync with the bindings.
    fn render_settings_shortcuts_section(
        &self,
        tokens: &ChromeTokens,
        style: UiStyle,
    ) -> impl IntoElement {
        let border_w = pixel::border_width(style, px(1.0));
        let mut list = div()
            .id("settings-shortcuts")
            .flex()
            .flex_col()
            .gap(px(12.0))
            .w_full()
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(tokens.fg_muted)
                    .child("Built-in shortcuts for this platform. Custom bindings in settings.json take precedence."),
            );

        let mut rows = div()
            .bg(tokens.hover)
            .border(border_w)
            .border_color(tokens.border)
            .overflow_hidden();
        let commands: Vec<_> = crate::command_palette::commands()
            .into_iter()
            .filter(|command| !command.shortcut.is_empty())
            .collect();
        let command_count = commands.len();

        for (index, command) in commands.into_iter().enumerate() {
            rows = rows.child(
                div()
                    .id(("shortcut-row", index))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap(px(16.0))
                    .w_full()
                    .px(px(14.0))
                    .py(px(9.0))
                    .when(index + 1 < command_count, |el| {
                        el.border_b(border_w).border_color(tokens.border)
                    })
                    .child(
                        div()
                            .min_w_0()
                            .text_size(px(12.0))
                            .text_color(tokens.fg)
                            .child(command.title),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .px(px(7.0))
                            .py(px(2.0))
                            .bg(tokens.surface)
                            .border(border_w)
                            .border_color(tokens.border)
                            .text_size(px(11.0))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(tokens.fg_muted)
                            .child(command.shortcut),
                    ),
            );
        }

        list = list.child(rows);
        list
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
        let style = pixel::active_style(cx);
        let (track_bg, knob_bg) = match style {
            UiStyle::Pixel => (
                tokens.content_bg,
                if enabled {
                    tokens.accent
                } else {
                    tokens.fg_muted
                },
            ),
            UiStyle::Default => (
                if enabled {
                    tokens.accent
                } else {
                    tokens.border
                },
                if enabled {
                    Hsla::white()
                } else {
                    Hsla::white().opacity(0.9)
                },
            ),
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
            // Blocky pixel toggle: square track, square knob.
            .child(
                div()
                    .flex_shrink_0()
                    .w(px(34.0))
                    .h(px(20.0))
                    .bg(track_bg)
                    .when(style == UiStyle::Pixel, |el| {
                        el.border_2().border_color(tokens.border)
                    })
                    .relative()
                    .child(
                        div()
                            .absolute()
                            .top(px(2.0))
                            .left(knob_offset)
                            .w(px(16.0))
                            .h(px(16.0))
                            .bg(knob_bg),
                    ),
            )
    }

    /// Theme section body: selectable list with ANSI swatches (type to filter,
    /// arrow keys move the selection with scroll-follow, enter applies).
    fn render_settings_theme_section(
        &self,
        tokens: &ChromeTokens,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let current = TerminalSettings::get_global(cx).theme.clone();
        let appearance = appearance_of(window.appearance());
        let items = self.filtered_theme_items(cx);
        let kb_selected = self
            .settings_theme_selected
            .min(items.len().saturating_sub(1));
        let catalog = TerminalSettings::user_themes(cx);
        let border_w = pixel::border_width(pixel::active_style(cx), px(1.0));

        // Type-to-filter: shell-prompt style search field with block cursor.
        let filter_text: SharedString = if self.theme_query.is_empty() {
            "search themes…".into()
        } else {
            self.theme_query.clone().into()
        };
        let filter_box = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .px(px(10.0))
            .py(px(7.0))
            .mb(px(8.0))
            .bg(tokens.hover)
            .border(border_w)
            .border_color(tokens.border)
            .text_size(px(12.0))
            .child(
                div()
                    .text_color(tokens.accent)
                    .font_weight(gpui::FontWeight::BOLD)
                    .child(">"),
            )
            .child(
                div()
                    .text_color(if self.theme_query.is_empty() {
                        tokens.fg_muted
                    } else {
                        tokens.fg
                    })
                    .child(filter_text),
            )
            .child(div().text_color(tokens.accent).child("█"));

        let mut list = div()
            .id("settings-theme-list")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .w_full()
            .overflow_y_scroll()
            .track_scroll(&self.settings_theme_scroll);

        let mut wrote_custom_header = false;
        for (i, item) in items.iter().enumerate() {
            if matches!(item.kind, ThemeItemKind::Custom(_)) && !wrote_custom_header {
                wrote_custom_header = true;
                list = list.child(
                    div()
                        .px(px(10.0))
                        .pt(px(12.0))
                        .pb(px(4.0))
                        .text_size(px(11.0))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(tokens.fg_muted)
                        .child(SharedString::from("# USER THEMES")),
                );
            }

            let (row_id, label, preview, applied): (ElementId, SharedString, _, bool) =
                match &item.kind {
                    ThemeItemKind::Builtin(theme) => (
                        format!("theme-row-{}", theme.as_str()).into(),
                        theme.display_name().into(),
                        palette_for_theme(*theme, appearance),
                        current == ThemeSetting::Builtin(*theme),
                    ),
                    ThemeItemKind::Custom(name) => (
                        format!("theme-row-custom-{name}").into(),
                        name.clone().into(),
                        catalog[name.as_str()].to_palette(),
                        current == ThemeSetting::Custom(name.clone()),
                    ),
                };
            let kind = item.kind.clone();
            let kb = i == kb_selected;

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
            for (j, color) in swatch_colors.into_iter().enumerate() {
                swatches = swatches.child(
                    div()
                        .id(format!("swatch-row-{i}-{j}"))
                        .w(px(11.0))
                        .h(px(11.0))
                        .bg(color),
                );
            }

            // Pixel checkbox indicator: [x] applied, [ ] otherwise.
            let indicator = div()
                .text_size(px(12.0))
                .font_weight(gpui::FontWeight::BOLD)
                .when(applied, |el| el.text_color(tokens.accent))
                .when(!applied, |el| el.text_color(tokens.fg_muted))
                .child(if applied { "[x]" } else { "[ ]" });

            let row = div()
                .id(row_id)
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .w_full()
                .px(px(10.0))
                .py(px(8.0))
                .cursor_pointer()
                .when(kb, |el| el.bg(tokens.hover))
                .hover(|el| el.bg(tokens.hover))
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    this.settings_theme_selected = i;
                    match &kind {
                        ThemeItemKind::Builtin(theme) => this.select_theme(*theme, cx),
                        ThemeItemKind::Custom(name) => this.select_custom_theme(name.clone(), cx),
                    }
                }))
                .child(indicator)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(13.0))
                        .when(applied, |el| el.text_color(tokens.accent))
                        .when(!applied, |el| el.text_color(tokens.fg))
                        .child(label),
                )
                .child(swatches);

            list = list.child(row);
        }

        if items.is_empty() {
            list = list.child(
                div()
                    .px(px(10.0))
                    .py(px(12.0))
                    .text_size(px(12.0))
                    .text_color(tokens.fg_muted)
                    .child(SharedString::from("no themes match")),
            );
        }

        div()
            .id("settings-theme")
            .flex()
            .flex_col()
            .h_full()
            .min_h_0()
            .w_full()
            .child(filter_box)
            .child(list)
    }
}
