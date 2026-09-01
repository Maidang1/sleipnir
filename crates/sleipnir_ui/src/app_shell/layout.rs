//! Pane layout and dragging: geometry for the split tree, the drag/divider
//! interactions, and the content area that hosts the terminals.
//!
//! A child module of `app_shell` so it can read and mutate the shell's private
//! tab/pane state without widening it to the crate.

use gpui::{
    App, AppContext as _, Bounds, ClickEvent, Corners, Context, ElementId, Hsla,
    InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    ParentElement as _, Pixels, RenderImage, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, WindowControlArea, canvas, deferred, div, point,
    prelude::FluentBuilder as _, px,
};
use std::sync::Arc;

use super::{AppShell, DragState, PaneDrag, TabDragPreview};
use crate::LeafContent;
use crate::chrome::ChromeTokens;
use crate::pane_tree::{
    Branch, MIN_RATIO, PaneId, PaneKey, PaneNode, PaneRect, SplitAxis, SplitPath,
};
use crate::plugin_panel::{
    TokenSlot, action_at, cell_from_pixels, cols_from_pixels, layout_surface, tone_slot,
};

/// A divider's hit rectangle plus the split it controls, produced by layout.
#[derive(Clone)]
struct DividerRect {
    path: SplitPath,
    axis: SplitAxis,
    /// The split container these children live in (for ratio math on drag).
    container: Bounds<Pixels>,
    /// The thin hit strip to render.
    hit: Bounds<Pixels>,
}

impl AppShell {
    pub(crate) fn attach_empty_drag(
        &self,
        id: impl Into<ElementId>,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .window_control_area(WindowControlArea::Drag)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.should_move = true;
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.should_move = false;
                }),
            )
            .on_mouse_down_out(cx.listener(|this, _, _, _| {
                this.should_move = false;
            }))
            .on_mouse_move(cx.listener(|this, _, window, _| {
                if this.should_move {
                    this.should_move = false;
                    window.start_window_move();
                }
            }))
            .on_click(cx.listener(|_, event: &ClickEvent, window, _| {
                if event.click_count() == 2 {
                    window.titlebar_double_click();
                }
            }))
    }

    fn pane_extract_grip(&self, pane_id: PaneId, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id(("pane-grip", pane_id))
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(12.0))
            .flex()
            .justify_center()
            .items_center()
            .cursor_grab()
            .on_drag(
                PaneDrag { pane_id },
                move |dragged, _offset, _window, cx| {
                    let title: SharedString = format!("pane {}", dragged.pane_id).into();
                    cx.new(move |_| TabDragPreview { title })
                },
            )
            .child(
                div()
                    .text_xs()
                    .text_color(gpui::hsla(0.0, 0.0, 0.6, 0.7))
                    .child("···"),
            )
    }

    /// Width/height of a divider's draggable hit strip.
    const DIVIDER_HIT: f32 = 8.0;

    /// Walk the active tab's tree over `area`, producing every leaf's rect and
    /// every divider's hit strip. Purely analytic — mirrors the flex layout.
    fn compute_layout(
        tree: &PaneNode,
        area: Bounds<Pixels>,
        path: SplitPath,
        panes: &mut Vec<PaneRect>,
        dividers: &mut Vec<DividerRect>,
    ) {
        match tree {
            PaneNode::Leaf { id, .. } => {
                panes.push(PaneRect {
                    id: *id,
                    bounds: area,
                });
            }
            PaneNode::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let hit = px(Self::DIVIDER_HIT);
                match axis {
                    SplitAxis::Horizontal => {
                        let w = f32::from(area.size.width);
                        let first_w = (w * *ratio).max(0.0);
                        let first_area =
                            Bounds::new(area.origin, gpui::size(px(first_w), area.size.height));
                        let second_area = Bounds::new(
                            point(px(f32::from(area.origin.x) + first_w), area.origin.y),
                            gpui::size(px(w - first_w), area.size.height),
                        );
                        dividers.push(DividerRect {
                            path: path.clone(),
                            axis: *axis,
                            container: area,
                            hit: Bounds::new(
                                point(
                                    px(f32::from(area.origin.x) + first_w - f32::from(hit) / 2.0),
                                    area.origin.y,
                                ),
                                gpui::size(hit, area.size.height),
                            ),
                        });
                        Self::compute_layout(
                            first,
                            first_area,
                            path.child(Branch::First),
                            panes,
                            dividers,
                        );
                        Self::compute_layout(
                            second,
                            second_area,
                            path.child(Branch::Second),
                            panes,
                            dividers,
                        );
                    }
                    SplitAxis::Vertical => {
                        let h = f32::from(area.size.height);
                        let first_h = (h * *ratio).max(0.0);
                        let first_area =
                            Bounds::new(area.origin, gpui::size(area.size.width, px(first_h)));
                        let second_area = Bounds::new(
                            point(area.origin.x, px(f32::from(area.origin.y) + first_h)),
                            gpui::size(area.size.width, px(h - first_h)),
                        );
                        dividers.push(DividerRect {
                            path: path.clone(),
                            axis: *axis,
                            container: area,
                            hit: Bounds::new(
                                point(
                                    area.origin.x,
                                    px(f32::from(area.origin.y) + first_h - f32::from(hit) / 2.0),
                                ),
                                gpui::size(area.size.width, hit),
                            ),
                        });
                        Self::compute_layout(
                            first,
                            first_area,
                            path.child(Branch::First),
                            panes,
                            dividers,
                        );
                        Self::compute_layout(
                            second,
                            second_area,
                            path.child(Branch::Second),
                            panes,
                            dividers,
                        );
                    }
                }
            }
        }
    }

    pub(super) fn render_content(
        &mut self,
        tokens: &ChromeTokens,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(tab) = self.tabs.get(self.active) else {
            return div().flex_1().size_full().min_h_0().into_any_element();
        };
        let active_pane = tab.active_pane;
        let tab_id = tab.id;
        let zoomed = tab.zoomed_pane;

        // Gather every leaf (terminals and panels) in tree order.
        let mut leaves = Vec::new();
        tab.tree.walk_leaves(&mut leaves);
        let leaves: Vec<(PaneId, PaneKey, LeafContent)> = leaves
            .into_iter()
            .map(|(id, key, content)| (id, key, content.clone()))
            .collect();

        // Pane zoom (M13): only the zoomed leaf is shown full-size.
        if let Some(zid) = zoomed {
            if let Some((id, key, content)) = leaves.iter().find(|(id, _, _)| *id == zid) {
                return div()
                    .id("pane-area-zoomed")
                    .flex_1()
                    .size_full()
                    .min_h_0()
                    .relative()
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full()
                            .child(self.leaf_element(*id, *key, content, tokens, window, cx)),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(6.0))
                            .right(px(8.0))
                            .px_2()
                            .py_0p5()
                            .rounded(px(4.0))
                            .bg(tokens.accent.opacity(0.85))
                            .text_size(px(11.0))
                            .text_color(gpui::hsla(0.0, 0.0, 1.0, 1.0))
                            .child(format!(
                                "Zoomed · {} to restore",
                                crate::display_shortcut("toggle_pane_zoom")
                            )),
                    )
                    .into_any_element();
            }
        }

        // Analytic layout over last frame's content bounds (if known and non-zero).
        // A 0×0 measure (collapsed canvas) must not drive absolute pane layout.
        let mut pane_rects = Vec::new();
        let mut dividers = Vec::new();
        let usable_bounds = self
            .content_bounds
            .filter(|area| f32::from(area.size.width) > 1.0 && f32::from(area.size.height) > 1.0);
        if let Some(area) = usable_bounds {
            // Lay out relative to a zero origin; absolute children are positioned
            // relative to the content container, not the window.
            let local = Bounds::new(point(px(0.0), px(0.0)), area.size);
            Self::compute_layout(
                &tab.tree,
                local,
                SplitPath::new(),
                &mut pane_rects,
                &mut dividers,
            );
        }
        // Record rects (with true screen origin) for neighbor navigation.
        self.pane_rects = if let Some(area) = usable_bounds {
            pane_rects
                .iter()
                .map(|r| PaneRect {
                    id: r.id,
                    bounds: Bounds::new(
                        point(
                            px(f32::from(area.origin.x) + f32::from(r.bounds.origin.x)),
                            px(f32::from(area.origin.y) + f32::from(r.bounds.origin.y)),
                        ),
                        r.bounds.size,
                    ),
                })
                .collect()
        } else {
            Vec::new()
        };

        // Single pane: render the view directly, still capturing bounds.
        let single = leaves.len() == 1;
        let allow_pane_extract = leaves.len() > 1;

        // Measure the content area with a full-size absolute canvas (Zed pattern).
        // Without size_full the canvas collapses to 0×0, which makes multi-pane
        // absolute layout produce empty rects and a blank terminal area.
        let mut container = div()
            .id("pane-area")
            .flex_1()
            .size_full()
            .min_h_0()
            .relative()
            // Drop a *different* tab here to merge it as a pane. Dropping the
            // visible tab still detaches it into a new window.
            .on_drop::<u64>(cx.listener(move |this, dragged: &u64, window, cx| {
                this.on_tab_dropped_on_pane_area(*dragged, window, cx);
            }))
            .child(
                canvas(
                    {
                        let shell = cx.weak_entity();
                        move |bounds, _window, cx| {
                            let _ = shell.update(cx, |this, cx| {
                                if this.content_bounds != Some(bounds) {
                                    this.content_bounds = Some(bounds);
                                    // Re-render so multi-pane absolute layout
                                    // picks up the measured size.
                                    cx.notify();
                                }
                            });
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            );

        if single {
            let (id, key, content) = &leaves[0];
            container = container.child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .child(self.leaf_element(*id, *key, content, tokens, window, cx)),
            );
            return container.into_any_element();
        }

        // Multi-pane: absolutely position each leaf by its computed rect.
        // If we have no measured layout yet (first frame after open/split),
        // fall back to equal flex so panes never disappear entirely.
        if pane_rects.is_empty() {
            let mut row = div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .flex_row()
                .min_h_0();
            for (id, key, content) in &leaves {
                let is_active = *id == active_pane;
                let pane_id = *id;
                let mut pane = div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .relative()
                    .overflow_hidden()
                    .child(self.leaf_element(*id, *key, content, tokens, window, cx))
                    .when(allow_pane_extract && content.is_terminal(), |el| {
                        el.child(self.pane_extract_grip(pane_id, cx))
                    });
                if !is_active {
                    pane = pane.border_1().border_color(tokens.border);
                } else {
                    pane = pane.border_1().border_color(tokens.accent);
                }
                pane = pane.on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        if let Some(tab) = this.tabs.get_mut(this.active) {
                            if tab.active_pane != pane_id {
                                tab.active_pane = pane_id;
                                this.focus_active(window, cx);
                                cx.notify();
                            }
                        }
                    }),
                );
                row = row.child(pane);
            }
            container = container.child(row);
            return container.into_any_element();
        }

        for (id, key, content) in &leaves {
            let rect = pane_rects.iter().find(|r| r.id == *id);
            let Some(rect) = rect else { continue };
            let is_active = *id == active_pane;
            let b = rect.bounds;
            let pane_id = *id;
            let mut pane = div()
                .absolute()
                .left(b.origin.x)
                .top(b.origin.y)
                .w(b.size.width)
                .h(b.size.height)
                .overflow_hidden()
                .child(self.leaf_element(*id, *key, content, tokens, window, cx))
                .when(allow_pane_extract && content.is_terminal(), |el| {
                    el.child(self.pane_extract_grip(pane_id, cx))
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        if let Some(tab) = this.tabs.get_mut(this.active) {
                            if tab.active_pane != pane_id {
                                tab.active_pane = pane_id;
                                this.focus_active(window, cx);
                                cx.notify();
                            }
                        }
                    }),
                );
            if !is_active {
                // Unfocused split dim (M13): dark overlay ~20% + muted border.
                // Overlay also receives clicks so focusing still works.
                pane = pane.border_1().border_color(tokens.border).child(
                    div()
                        .id(("pane-dim", pane_id))
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .bg(Hsla::black().opacity(0.22))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                if let Some(tab) = this.tabs.get_mut(this.active) {
                                    if tab.active_pane != pane_id {
                                        tab.active_pane = pane_id;
                                        this.focus_active(window, cx);
                                        cx.notify();
                                    }
                                }
                            }),
                        ),
                );
            } else {
                pane = pane.border_1().border_color(tokens.accent);
            }
            container = container.child(pane);
        }

        // Divider hit strips.
        for divider in &dividers {
            let h = divider.hit;
            let is_h = matches!(divider.axis, SplitAxis::Horizontal);
            let path = divider.path.clone();
            let axis = divider.axis;
            let container_bounds = divider.container;
            let strip = div()
                .absolute()
                .left(h.origin.x)
                .top(h.origin.y)
                .w(h.size.width)
                .h(h.size.height)
                .when(is_h, |s| s.cursor_col_resize())
                .when(!is_h, |s| s.cursor_row_resize())
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        // Recover this divider's live container in screen space.
                        let screen_container = this
                            .content_bounds
                            .map(|area| {
                                Bounds::new(
                                    point(
                                        px(f32::from(area.origin.x)
                                            + f32::from(container_bounds.origin.x)),
                                        px(f32::from(area.origin.y)
                                            + f32::from(container_bounds.origin.y)),
                                    ),
                                    container_bounds.size,
                                )
                            })
                            .unwrap_or(container_bounds);
                        this.drag = Some(DragState {
                            tab_id,
                            path: path.clone(),
                            axis,
                            container: screen_container,
                        });
                        this.set_all_blocks_frozen(true, cx);
                        cx.notify();
                    }),
                );
            container = container.child(strip);
        }

        // While dragging, an overlay captures move/up across the whole area.
        if self.drag.is_some() {
            let overlay = div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _window, cx| {
                    this.update_drag(ev.position, cx);
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.drag = None;
                        this.set_all_blocks_frozen(false, cx);
                        this.schedule_session_save(cx);
                        cx.notify();
                    }),
                );
            container = container.child(deferred(overlay));
        }

        container.into_any_element()
    }

    /// Update the dragged split's ratio from the pointer position.
    fn update_drag(&mut self, position: gpui::Point<Pixels>, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.clone() else {
            return;
        };
        let Some(idx) = self.tabs.iter().position(|t| t.id == drag.tab_id) else {
            return;
        };
        let c = drag.container;
        let ratio = match drag.axis {
            SplitAxis::Horizontal => {
                let w = f32::from(c.size.width).max(1.0);
                (f32::from(position.x) - f32::from(c.origin.x)) / w
            }
            SplitAxis::Vertical => {
                let h = f32::from(c.size.height).max(1.0);
                (f32::from(position.y) - f32::from(c.origin.y)) / h
            }
        };
        let ratio = ratio.clamp(MIN_RATIO, 1.0 - MIN_RATIO);
        self.tabs[idx].tree.set_ratio(&drag.path, ratio);
        cx.notify();
    }

    fn leaf_element(
        &self,
        pane_id: PaneId,
        pane_key: PaneKey,
        content: &LeafContent,
        tokens: &ChromeTokens,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        match content {
            LeafContent::Terminal(view) => view.clone().into_any_element(),
            LeafContent::Panel { .. } => {
                self.render_plugin_panel(pane_id, pane_key, tokens, window, cx)
            }
        }
    }

    fn render_plugin_panel(
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
        let panel_image = surface.image.clone();
        let mut body = div()
            .id(("plugin-panel", pane_id))
            .size_full()
            .relative()
            .bg(tokens.content_bg)
            .font_family(font_family)
            .text_size(font_size)
            .overflow_hidden();

        if let Some(img) = panel_image {
            let render_image = build_panel_render_image(&img);
            body = body.child(
                canvas(
                    move |_, _, _| {},
                    move |bounds, _, window, _| {
                        if let Some(ref ri) = render_image {
                            let _ = window.paint_image(
                                bounds,
                                bounds,
                                Corners::default(),
                                ri.clone(),
                                0,
                                false,
                            );
                        }
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
                    .right_0()
                    .px_2()
                    .py_0p5()
                    .bg(tokens.surface)
                    .text_xs()
                    .text_color(tokens.fg_muted)
                    .child("plugin stopped"),
            );
        }

        let cell_w_click = cell_w;
        let line_h_click = line_h;
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
                    if let Some(hit) = action_at(&laid, pos.col, pos.row) {
                        crate::plugin_runtime::push_action(
                            &plugin_id, surface_id, hit.action, hit.arg, cx,
                        );
                    }
                }
                cx.notify();
            }),
        );
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

fn panel_cell_metrics(
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

fn slot_color(tokens: &ChromeTokens, slot: TokenSlot) -> Hsla {
    match slot {
        TokenSlot::Fg => tokens.fg,
        TokenSlot::Muted => tokens.fg_muted,
        TokenSlot::Accent => tokens.accent,
        TokenSlot::Ok => tokens.ok,
        TokenSlot::Warn => tokens.warn,
        TokenSlot::Err => tokens.err,
    }
}

fn paint_laid_out(
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

fn paint_node(
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
        LaidOutKind::Text { lines, role, bold } => {
            let color = slot_color(tokens, tone_slot(*role));
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
        LaidOutKind::Badge { text, role } => el
            .bg(tokens.surface)
            .px_1()
            .text_color(slot_color(tokens, tone_slot(*role)))
            .child(text.clone())
            .into_any_element(),
        LaidOutKind::Bar { filled, width: w } => {
            let fill_w = px((*filled as f32 / (*w).max(1) as f32) * f32::from(width));
            el.bg(tokens.surface)
                .child(div().h_full().w(fill_w).bg(tokens.accent))
                .into_any_element()
        }
        LaidOutKind::Spark { levels } => {
            const BLOCKS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
            let s: String = levels
                .iter()
                .map(|&lv| BLOCKS[lv.min(8) as usize])
                .collect();
            el.text_color(tokens.accent).child(s).into_any_element()
        }
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

fn build_panel_render_image(
    img: &crate::plugin_panel::PanelImage,
) -> Option<Arc<RenderImage>> {
    use image::ImageBuffer;
    use smallvec::SmallVec;

    let w = img.width;
    let h = img.height;
    if w == 0 || h == 0 {
        return None;
    }
    let expected = (w as usize) * (h as usize) * 4;
    if img.data.len() < expected {
        return None;
    }
    let mut bgra = img.data[..expected].to_vec();
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let buffer = ImageBuffer::from_raw(w, h, bgra)?;
    let frame = image::Frame::new(buffer);
    Some(Arc::new(RenderImage::new(SmallVec::from_elem(frame, 1))))
}
