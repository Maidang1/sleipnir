//! Plugin surface painting: the Panel leaf element, the titlebar chrome
//! status band, and the widget-tree painters they share.
//!
//! A child module of `app_shell`, split out of `layout.rs` so that file stays
//! pane geometry and this one owns plugin paint. Pure projection lives in
//! `panel_scene_paint.rs`; this module is the gpui element/painter side.

use gpui::{
    App, Bounds, ClickEvent, Context, Hsla, InteractiveElement as _, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, ParentElement as _, Pixels, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, canvas, div, px,
};

use super::AppShell;
use crate::chrome::ChromeTokens;
use crate::pane_tree::{PaneId, PaneKey};
use crate::plugin_panel::{action_at, cell_from_pixels, cols_from_pixels, layout_surface};
use sleipnir_widget::Tone;

impl AppShell {
    pub(super) fn render_plugin_panel(
        &self,
        pane_id: PaneId,
        pane_key: PaneKey,
        tokens: &ChromeTokens,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let (cell_w, line_h, font_family, font_size) =
            panel_cell_metrics(window, cx, self.font_size_override);
        let pixel_width = self
            .pane_rects
            .iter()
            .find(|r| r.id == pane_id)
            .map(|r| f32::from(r.bounds.size.width))
            .or_else(|| self.content_bounds.map(|b| f32::from(b.size.width)))
            .unwrap_or(80.0);
        let cols = cols_from_pixels(pixel_width, cell_w);
        let Some(surface) = self.plugin_panels.get(pane_key) else {
            return div()
                .size_full()
                .bg(tokens.surface)
                .child(
                    div()
                        .p_2()
                        .text_xs()
                        .text_color(tokens.fg_muted)
                        .child("plugin panel"),
                )
                .into_any_element();
        };
        let laid = layout_surface(surface, cols);
        let stale = surface.stale;
        let plugin_id = surface.plugin_id.clone();
        let surface_id = surface.surface_id;
        let panel_scene = surface.scene.clone();
        let mut body = div()
            .id(("plugin-panel", pane_id))
            .size_full()
            .relative()
            .bg(tokens.content_bg)
            .font_family(font_family)
            .text_size(font_size)
            .overflow_hidden();

        if let Some(scene) = panel_scene {
            let border = tokens.accent;
            body = body.child(
                canvas(
                    move |_, _, _| {},
                    move |bounds, _, window, _| {
                        paint_panel_scene(&scene, bounds, border, window);
                    },
                )
                .size_full()
                .absolute()
                .top_0()
                .left_0(),
            );
        }

        body = paint_laid_out(body, &laid, tokens, cell_w, line_h);

        if stale {
            body = body.child(
                div()
                    .id(("plugin-panel-stale", pane_id))
                    .absolute()
                    .top_0()
                    // Sit left of the close control so both stay legible.
                    .right(px(26.0))
                    .px_2()
                    .py_0p5()
                    .bg(tokens.surface)
                    .text_xs()
                    .text_color(tokens.fg_muted)
                    .child("plugin stopped"),
            );
        }

        // Host-owned close control (ADR-0017): the panel is a host surface, so
        // the user can always dismiss it even when the plugin offers no way out.
        // Closing removes the leaf and drops the surface from the registry.
        body = body.child(
            div()
                .id(("plugin-panel-close", pane_id))
                .absolute()
                .top(px(2.0))
                .right(px(4.0))
                .w(px(18.0))
                .h(px(18.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.0))
                .cursor_pointer()
                .text_color(tokens.fg_muted)
                .hover(|el| el.bg(tokens.hover).text_color(tokens.fg))
                .child("×")
                // Win over the panel's own mouse-down (focus / camera drag) so a
                // click closes the panel instead of starting a rotate.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.close_panel_pane(pane_id, pane_key, window, cx);
                })),
        );

        let cell_w_click = cell_w;
        let line_h_click = line_h;
        let has_scene = self
            .plugin_panels
            .get(pane_key)
            .map(|s| s.scene.is_some())
            .unwrap_or(false);
        let down_plugin_id = plugin_id;
        body = body.on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                if let Some(tab) = this.tabs.get_mut(this.active) {
                    if tab.active_pane != pane_id {
                        tab.active_pane = pane_id;
                    }
                }
                this.focus_active(window, cx);
                let origin = this
                    .pane_rects
                    .iter()
                    .find(|r| r.id == pane_id)
                    .map(|r| r.bounds.origin)
                    .or_else(|| this.content_bounds.map(|b| b.origin));
                let Some(origin) = origin else {
                    cx.notify();
                    return;
                };
                let local_x = f32::from(ev.position.x) - f32::from(origin.x);
                let local_y = f32::from(ev.position.y) - f32::from(origin.y);
                let pos = cell_from_pixels(local_x, local_y, cell_w_click, line_h_click);
                if let Some(surface) = this.plugin_panels.get(pane_key) {
                    if surface.stale {
                        cx.notify();
                        return;
                    }
                    let laid = layout_surface(surface, cols);
                    // A button always wins over camera drag: chrome controls sit
                    // on top of the scene.
                    if let Some(hit) = action_at(&laid, pos.col, pos.row) {
                        crate::plugin_runtime::push_action(
                            &down_plugin_id,
                            surface_id,
                            hit.action,
                            hit.arg,
                            cx,
                        );
                        this.panel_drag = None;
                        cx.notify();
                        return;
                    }
                    // Otherwise, if the panel carries a scene, begin a camera
                    // drag. The host owns the camera; the plugin is only told
                    // the result, throttled, so the legend stays in sync.
                    if surface.scene.is_some() {
                        this.panel_drag = Some(super::PanelDrag {
                            pane_key,
                            plugin_id: down_plugin_id.clone(),
                            surface_id,
                            last: ev.position,
                        });
                    }
                }
                cx.notify();
            }),
        );

        if has_scene {
            body =
                body.on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, _window, cx| {
                    this.drag_panel_camera(pane_key, ev.position, cx);
                }));
            body = body.on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, _ev: &gpui::MouseUpEvent, _window, cx| {
                    this.end_panel_camera_drag(pane_key, cx);
                }),
            );
            body = body.on_scroll_wheel(cx.listener(
                move |this, ev: &gpui::ScrollWheelEvent, _window, cx| {
                    this.zoom_panel_camera(pane_key, ev, cx);
                },
            ));
        }
        body.into_any_element()
    }
    pub(super) fn render_plugin_chrome_status(
        &mut self,
        tokens: &ChromeTokens,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        use crate::plugin_chrome::MAX_STATUS_COLS;
        if self.plugin_chrome.is_empty() {
            return div().into_any_element();
        }
        let (cell_w, line_h, font_family, font_size) =
            panel_cell_metrics(window, cx, self.font_size_override);
        let Some(laid) = self.plugin_chrome.status_layout(MAX_STATUS_COLS) else {
            return div().into_any_element();
        };
        let width = px((laid.width as f32 * cell_w).max(1.0));
        // Chrome is one band high. Clip rather than grow the titlebar.
        let height = px((laid.height as f32 * line_h).clamp(1.0, 28.0));
        let mut body = div()
            .id("plugin-chrome-status")
            .flex_shrink_0()
            .w(width)
            .h(height)
            .relative()
            .overflow_hidden()
            .font_family(font_family)
            .text_size(font_size);
        body = paint_laid_out(body, laid, tokens, cell_w, line_h);
        body.into_any_element()
    }
}

pub(super) fn panel_cell_metrics(
    window: &Window,
    cx: &App,
    font_size_override: Option<Pixels>,
) -> (f32, f32, SharedString, Pixels) {
    use gpui::TextStyle;
    use sleipnir_settings::{TerminalPalette, TerminalSettings};
    let settings = TerminalSettings::get_global(cx);
    let font_family = settings
        .font_family
        .clone()
        .unwrap_or_else(|| sleipnir_settings::default_font_family().into());
    let font_size = font_size_override
        .or(settings.font_size)
        .unwrap_or(px(14.))
        .max(px(8.));
    let line_height_factor = settings.line_height.value().max(1.0);
    let font_features = settings
        .font_features
        .clone()
        .unwrap_or_else(gpui::FontFeatures::disable_ligatures);
    let text_style = TextStyle {
        font_family: font_family.clone().into(),
        font_features,
        font_weight: settings.font_weight.unwrap_or_default(),
        font_size: font_size.into(),
        font_fallbacks: settings.font_fallbacks.clone(),
        color: TerminalPalette::get_global(cx).foreground,
        ..Default::default()
    };
    let font_id = cx.text_system().resolve_font(&text_style.font());
    let cell_width = cx
        .text_system()
        .advance(font_id, font_size, 'm')
        .map(|a| a.width)
        .unwrap_or(px(8.))
        .max(px(4.));
    let line_height = px(f32::from(font_size) * line_height_factor).max(px(10.));
    let _ = window;
    (
        f32::from(cell_width),
        f32::from(line_height),
        font_family.into(),
        font_size,
    )
}
pub(super) fn slot_color(tokens: &ChromeTokens, tone: Tone) -> Hsla {
    match tone {
        Tone::Fg => tokens.fg,
        Tone::Dim => tokens.fg_muted,
        Tone::Accent => tokens.accent,
        Tone::Ok => tokens.ok,
        Tone::Warn => tokens.warn,
        Tone::Err => tokens.err,
    }
}
pub(super) fn paint_laid_out(
    mut root: gpui::Stateful<gpui::Div>,
    laid: &sleipnir_widget::Layout,
    tokens: &ChromeTokens,
    cell_w: f32,
    line_h: f32,
) -> gpui::Stateful<gpui::Div> {
    for (i, node) in laid.walk().enumerate() {
        root = root.child(paint_node(i, node, tokens, cell_w, line_h, false));
    }
    root.child(paint_node(
        usize::MAX,
        &laid.attribution,
        tokens,
        cell_w,
        line_h,
        true,
    ))
}
pub(super) fn paint_node(
    key: usize,
    node: &sleipnir_widget::LaidOut,
    tokens: &ChromeTokens,
    cell_w: f32,
    line_h: f32,
    attribution: bool,
) -> gpui::AnyElement {
    use sleipnir_widget::LaidOutKind;
    let r = node.rect;
    let left = px(r.col as f32 * cell_w);
    let top = px(r.row as f32 * line_h);
    let width = px((r.width as f32 * cell_w).max(1.0));
    let height = px((r.height as f32 * line_h).max(1.0));
    let mut el = div()
        .id(("w", key as u64))
        .absolute()
        .left(left)
        .top(top)
        .w(width)
        .h(height)
        .overflow_hidden();
    if attribution {
        el = el.bg(tokens.surface);
    }
    match &node.kind {
        LaidOutKind::Col | LaidOutKind::Row => el.into_any_element(),
        LaidOutKind::Text { lines, tone, bold } => {
            let color = slot_color(tokens, *tone);
            let mut col = div().flex().flex_col();
            for line in lines {
                let mut row = div().h(px(line_h)).text_color(color).child(line.clone());
                if *bold {
                    row = row.font_weight(gpui::FontWeight::BOLD);
                }
                col = col.child(row);
            }
            el.child(col).into_any_element()
        }
        LaidOutKind::Code { lines } => {
            let mut col = div().flex().flex_col().bg(tokens.surface);
            for line in lines {
                col = col.child(
                    div()
                        .h(px(line_h))
                        .text_color(tokens.fg)
                        .child(line.text.clone()),
                );
            }
            el.child(col).into_any_element()
        }
        LaidOutKind::Badge { text, tone } => el
            .bg(tokens.surface)
            .px_1()
            .text_color(slot_color(tokens, *tone))
            .child(text.clone())
            .into_any_element(),
        LaidOutKind::Bar { filled, width: w } => {
            let fill_w = px((*filled as f32 / (*w).max(1) as f32) * f32::from(width));
            el.bg(tokens.surface)
                .child(div().h_full().w(fill_w).bg(tokens.accent))
                .into_any_element()
        }
        LaidOutKind::Spark { levels } => el
            .text_color(tokens.accent)
            .child(sleipnir_widget::spark_glyphs(levels))
            .into_any_element(),
        LaidOutKind::Sep => el.bg(tokens.border).h(px(1.0)).into_any_element(),
        LaidOutKind::Btn { text, .. } => el
            .bg(tokens.hover)
            .px_1()
            .cursor_pointer()
            .text_color(tokens.accent)
            .child(text.clone())
            .into_any_element(),
        LaidOutKind::Unknown => el
            .text_color(tokens.fg_muted)
            .child("[?]")
            .into_any_element(),
        LaidOutKind::Truncated => el
            .text_color(tokens.warn)
            .child("… truncated")
            .into_any_element(),
        LaidOutKind::Attribution { label, .. } => el
            .text_color(tokens.fg_muted)
            .text_xs()
            .child(label.clone())
            .into_any_element(),
    }
}
/// Project a plugin scene against the panel's real pixel bounds and paint it as
/// filled polygons, back-to-front. Host-side projection is what keeps the chart
/// crisp on resize (no bitmap scaling) and lets the camera move without a plugin
/// round-trip. The selected bar's faces get a thin accent outline so the eye
/// lands on the row the legend names.
pub(super) fn paint_panel_scene(
    scene: &plugin_protocol::v2::SceneData,
    bounds: Bounds<Pixels>,
    border: Hsla,
    window: &mut Window,
) {
    use crate::panel_scene_paint::project_scene;
    use gpui::{Background, PathBuilder, point as gpui_point, px as gpui_px};

    let origin = bounds.origin;
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    if width <= 1.0 || height <= 1.0 {
        return;
    }
    let projected = project_scene(scene, width, height);
    for face in &projected.faces {
        let pts: Vec<gpui::Point<Pixels>> = face
            .pts
            .iter()
            .map(|p| gpui_point(origin.x + gpui_px(p[0]), origin.y + gpui_px(p[1])))
            .collect();
        let mut builder = PathBuilder::fill();
        builder.add_polygon(&pts, true);
        if let Ok(path) = builder.build() {
            let color = gpui::Rgba {
                r: face.color[0] as f32 / 255.0,
                g: face.color[1] as f32 / 255.0,
                b: face.color[2] as f32 / 255.0,
                a: 1.0,
            };
            window.paint_path(path, Background::from(color));
        }
        if face.selected {
            // Outline each edge of the selected face with a thin stroke.
            let mut stroke = PathBuilder::stroke(gpui_px(1.5));
            stroke.add_polygon(&pts, true);
            if let Ok(path) = stroke.build() {
                window.paint_path(path, Background::from(border));
            }
        }
    }
}
