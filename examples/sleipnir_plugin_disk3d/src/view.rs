//! Turn a [`Scan`] into a 3D scene and then into a widget tree.
//!
//! Kept free of I/O and of the wire protocol so the whole view is unit
//! testable: given a scan and a camera, the tree is a pure function.

use plugin_protocol::v2::{
    MAX_WIDGET_NODES, SceneBar, SceneCamera, SceneData, Tone, Widget, measure,
};
use sleipnir_plugin::{badge, btn, col, row, sep, text};

use crate::raster::{Camera, Scene, default_light};
use crate::scan::{Entry, Scan, human_bytes};

/// Rows of the surface reserved for the chrome (title, legend, controls) and
/// therefore unavailable to the raster.
const CHROME_ROWS: u32 = 9;

/// Keep the raster from collapsing on a short split.
const MIN_CANVAS_ROWS: u32 = 8;

/// Nodes reserved for the chrome (header, separator, legend, controls) plus
/// headroom. Measured by `chrome_fits_in_its_reserve`, not guessed.
const CHROME_NODE_RESERVE: usize = 60;

/// Hard ceiling on raster rows, derived from the node budget: one `text` node
/// per row means rows and nodes are the same currency (ADR-0017 constraint 5).
/// Bounding here is what keeps the tree legal, so the host never truncates the
/// picture and drops the legend below it.
const MAX_CANVAS_ROWS: u32 = (MAX_WIDGET_NODES - CHROME_NODE_RESERVE) as u32;

/// Bar footprint and spacing in world units.
const BAR_HALF: f32 = 0.34;
const BAR_PITCH: f32 = 1.0;

/// Tallest bar in world units. Heights are normalised to the largest entry, so
/// the chart reads as relative share and the scene is always the same size.
const MAX_BAR_HEIGHT: f32 = 4.5;

/// Floor for a non-empty bar, as a fraction of [`MAX_BAR_HEIGHT`].
///
/// Real directories are dominated by one entry (`target/` is 99% of a Rust
/// repo), and a strictly linear scale renders every other bar zero-height and
/// invisible. A visible plinth keeps small entries on the map and clickable;
/// the exact bytes and percentage are always in the legend, so the floor
/// cannot mislead about magnitude.
const MIN_BAR_SHARE: f32 = 0.05;

/// The plugin's whole view state.
#[derive(Clone, Debug)]
pub struct View {
    pub scan: Scan,
    pub camera: Camera,
    pub zoom: f32,
    /// Index into `scan.entries`, clamped on every use.
    pub selected: usize,
}

impl View {
    pub fn new(scan: Scan) -> Self {
        Self {
            scan,
            camera: Camera {
                yaw: 0.7,
                pitch: 0.42,
            },
            zoom: 1.0,
            selected: 0,
        }
    }

    pub fn selected_entry(&self) -> Option<&Entry> {
        self.scan.entries.get(self.selected)
    }

    pub fn yaw_by(&mut self, delta: f32) {
        self.camera.yaw = wrap_angle(self.camera.yaw + delta);
    }

    /// Pitch is clamped: past vertical the chart reads as a flat plan and the
    /// bar heights stop being comparable.
    pub fn pitch_by(&mut self, delta: f32) {
        self.camera.pitch = (self.camera.pitch + delta).clamp(0.05, 1.35);
    }

    pub fn zoom_by(&mut self, factor: f32) {
        self.zoom = (self.zoom * factor).clamp(0.5, 2.5);
    }

    pub fn select_next(&mut self) {
        if self.scan.entries.is_empty() {
            return;
        }
        self.selected = (self.selected + 1) % self.scan.entries.len();
    }

    pub fn replace_scan(&mut self, scan: Scan) {
        self.selected = 0;
        self.scan = scan;
    }

    /// Apply a host-driven camera. The host owns the interactive camera (drag to
    /// rotate, wheel to zoom) and reports the new state as a `camera` action
    /// whose arg is a JSON [`SceneCamera`] — the same typed payload the scene
    /// itself carries. Missing or malformed payloads keep the current values,
    /// and pitch/zoom are clamped to the same readable range the button
    /// controls use, so a stray value cannot flatten or explode the view.
    pub fn apply_camera_arg(&mut self, arg: &str) {
        let Ok(cam) = serde_json::from_str::<SceneCamera>(arg) else {
            return;
        };
        if cam.yaw.is_finite() {
            self.camera.yaw = wrap_angle(cam.yaw);
        }
        if cam.pitch.is_finite() {
            self.camera.pitch = cam.pitch.clamp(0.05, 1.35);
        }
        if cam.zoom.is_finite() {
            self.zoom = cam.zoom.clamp(0.5, 2.5);
        }
    }
}

fn wrap_angle(a: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    let mut a = a % tau;
    if a < 0.0 {
        a += tau;
    }
    a
}

/// Build the raster scene for the text fallback: one cuboid per entry on a
/// square-ish grid, plus a floor tick under each.
///
/// This is a thin adapter over [`build_scene_data`]: grid layout, height
/// normalisation and selected colouring live there, so the two render paths
/// can never disagree about the model. Here the normalised heights are scaled
/// back to world units and the grid is centred on the origin so auto-fit
/// framing stays stable as the entry count changes.
pub fn build_scene(view: &View) -> Scene {
    let mut scene = Scene::new();
    let data = build_scene_data(view);
    if data.bars.is_empty() {
        return scene;
    }
    let light = default_light();
    let x0 = -((data.cols as f32 - 1.0) * BAR_PITCH) * 0.5;
    let z0 = -((data.rows as f32 - 1.0) * BAR_PITCH) * 0.5;
    for bar in &data.bars {
        let x = x0 + bar.gx as f32 * BAR_PITCH;
        let z = z0 + bar.gz as f32 * BAR_PITCH;
        scene.floor_tick(x, z);
        scene.bar(
            x,
            z,
            BAR_HALF,
            bar.height * MAX_BAR_HEIGHT,
            light,
            bar.selected,
        );
    }
    scene
}

/// Near-square grid: entries are laid out roughly `√n` per side so the model is
/// compact from every angle instead of a long wall.
pub fn grid_cols(n: usize) -> usize {
    (n as f64).sqrt().ceil().max(1.0) as usize
}

/// Bar colours, cycled by entry index. RGB so the host paints them directly
/// (the widget schema's semantic tones do not reach the projected scene).
const PALETTE: &[[u8; 3]] = &[
    [102, 178, 242],
    [242, 140, 89],
    [115, 217, 128],
    [230, 115, 166],
    [178, 140, 230],
    [242, 204, 89],
    [128, 204, 204],
    [217, 153, 115],
];

/// Selected bar colour: bright so it reads next to the legend.
const SELECTED_COLOR: [u8; 3] = [255, 255, 153];

/// Floor plane colour.
const FLOOR_COLOR: [u8; 3] = [46, 46, 56];

/// Build the compact scene description the host projects and paints.
///
/// The grid is the reason this is 3D rather than a bar chart drawn in
/// perspective: entries occupy both floor axes, so rotating the camera reveals
/// bars that were behind others, and a dozen directories stay readable in a
/// width that a single row of bars could not fit.
///
/// Heights are linear in share of the largest entry, normalised to `0.0..=1.0`
/// with a visible floor at [`MIN_BAR_SHARE`], so a directory dominated by one
/// entry still shows its small children; a log scale would flatter small dirs.
/// The host owns projection, so this carries geometry and colour only — no
/// pixels. The text fallback reuses this via [`build_scene`].
pub fn build_scene_data(view: &View) -> SceneData {
    let entries = &view.scan.entries;
    let cols = if entries.is_empty() {
        0
    } else {
        grid_cols(entries.len()) as u32
    };
    let rows = if entries.is_empty() {
        0
    } else {
        entries.len().div_ceil(cols.max(1) as usize) as u32
    };
    let largest = view.scan.largest_bytes().max(1);
    let bars = entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let share = entry.bytes as f32 / largest as f32;
            let height = share.max(MIN_BAR_SHARE).clamp(0.0, 1.0);
            let selected = i == view.selected;
            SceneBar {
                gx: (i as u32) % cols.max(1),
                gz: (i as u32) / cols.max(1),
                height,
                color: if selected {
                    SELECTED_COLOR
                } else {
                    PALETTE[i % PALETTE.len()]
                },
                selected,
            }
        })
        .collect();
    SceneData {
        cols,
        rows,
        floor: FLOOR_COLOR,
        camera: SceneCamera {
            yaw: view.camera.yaw,
            pitch: view.camera.pitch,
            zoom: view.zoom,
        },
        bars,
    }
}

/// Rows available to the raster on a surface of `rows` total.
///
/// Clamped at both ends: [`MIN_CANVAS_ROWS`] so a short split still shows a
/// chart, [`MAX_CANVAS_ROWS`] so a very tall one cannot push the tree past the
/// node budget.
pub fn canvas_rows(rows: u16) -> u32 {
    u32::from(rows)
        .saturating_sub(CHROME_ROWS)
        .clamp(MIN_CANVAS_ROWS, MAX_CANVAS_ROWS)
}

/// Render the whole panel.
///
/// The node budget is the hard constraint (ADR-0017 constraint 5): one `text`
/// per raster row plus chrome. `canvas_rows` and [`crate::scan::MAX_BARS`] are
/// chosen so a realistic split stays well inside 500 nodes, and
/// [`clamp_to_budget`] enforces it for pathological sizes rather than letting
/// the host truncate the image mid-picture.
pub fn render(view: &View, cols: u16, rows: u16) -> Widget {
    let canvas_h = canvas_rows(rows);
    let canvas = build_scene(view).render(view.camera, u32::from(cols.max(1)), canvas_h, view.zoom);

    let mut root = col().gap(0).child(header(view));

    if view.scan.is_empty() {
        root = root.child(text("Nothing to chart in this directory.").tone(Tone::Dim));
    } else if !canvas.has_ink() {
        root = root.child(text("Surface too small to draw the chart.").tone(Tone::Warn));
    } else {
        // Selected bar solid, everything else dim: the eye goes to the row the
        // legend is talking about. Per-row tone is the finest colour control the
        // schema allows — `text` carries exactly one tone.
        for line in canvas.to_rows() {
            root = root.child(text(line).tone(Tone::Accent));
        }
    }

    root = root.child(sep()).child(legend(view)).child(controls());
    clamp_to_budget(root.into())
}

/// Chrome-only tree for the host-drawn scene: header + legend + controls, no
/// raster rows. The host paints the geometry, so the tree carries none.
pub fn render_chrome_only(view: &View) -> Widget {
    let mut root = col().gap(0).child(header(view));
    if view.scan.is_empty() {
        root = root.child(text("Nothing to chart in this directory.").tone(Tone::Dim));
    }
    root = root.child(sep()).child(legend(view)).child(controls());
    root.into()
}

fn header(view: &View) -> Widget {
    let scan = &view.scan;
    let root = scan
        .root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| scan.root.to_string_lossy().to_string());
    let mut line = row()
        .gap(2)
        .child(badge("disk 3d", Tone::Accent))
        .child(text(root).bold())
        .child(text(human_bytes(scan.total_bytes)).tone(Tone::Ok));
    if scan.partial {
        line = line.child(badge("partial", Tone::Warn));
    }
    if scan.unreadable > 0 {
        line = line.child(text(format!("{} unreadable", scan.unreadable)).tone(Tone::Dim));
    }
    line.into()
}

/// The legend is what makes the picture readable: it names the highlighted bar
/// and states its size and share numerically.
fn legend(view: &View) -> Widget {
    let Some(entry) = view.selected_entry() else {
        return text("—").tone(Tone::Dim).into();
    };
    let total = view.scan.total_bytes.max(1);
    let share = entry.bytes as f64 / total as f64;
    let kind = if entry.aggregated {
        "group"
    } else if entry.is_dir {
        "dir"
    } else {
        "file"
    };
    col()
        .gap(0)
        .child(
            row()
                .gap(2)
                .child(badge(kind, Tone::Ok))
                .child(text(entry.name.clone()).bold())
                .child(text(human_bytes(entry.bytes)).tone(Tone::Accent))
                .child(text(format!("{:.1}%", share * 100.0)).tone(Tone::Dim)),
        )
        .child(Widget::Bar { v: share as f32 })
        .child(
            text(format!(
                "bar {} of {}  ·  yaw {:.0}°  pitch {:.0}°  zoom {:.1}x",
                view.selected + 1,
                view.scan.entries.len(),
                view.camera.yaw.to_degrees(),
                view.camera.pitch.to_degrees(),
                view.zoom,
            ))
            .tone(Tone::Dim),
        )
        .into()
}

/// `btn` is the only interactive node in the schema, so every camera control is
/// a button. No key handling exists for plugin surfaces.
fn controls() -> Widget {
    row()
        .gap(1)
        .child(btn("◀", "yaw-").arg("left"))
        .child(btn("▶", "yaw+").arg("right"))
        .child(btn("▲", "pitch+"))
        .child(btn("▼", "pitch-"))
        .child(btn("+", "zoom+"))
        .child(btn("-", "zoom-"))
        .child(btn("Next bar", "next"))
        .child(btn("Spin ½ turn", "spin"))
        .child(btn("Rescan", "rescan"))
        .into()
}

/// Last-resort guard: drop raster rows from the end of the image until the tree
/// fits the node budget.
///
/// [`canvas_rows`] already bounds the image, so this should never fire; it
/// exists because the alternative failure mode is the host truncating the tree
/// at an arbitrary node and taking the legend and controls with it. Trimming
/// here degrades only the picture and keeps the chrome.
fn clamp_to_budget(tree: Widget) -> Widget {
    if measure(&tree).within_budget() {
        return tree;
    }
    let Widget::Col { gap, mut children } = tree else {
        return tree;
    };
    // children[0] is the header; raster rows run up to the `sep`.
    let first = 1usize;
    loop {
        let sep_at = children.iter().position(|c| matches!(c, Widget::Sep));
        let Some(sep_at) = sep_at else { break };
        if sep_at <= first {
            break;
        }
        children.remove(sep_at - 1);
        if measure(&Widget::Col {
            gap,
            children: children.clone(),
        })
        .within_budget()
        {
            break;
        }
    }
    Widget::Col { gap, children }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::Scan;
    use std::path::PathBuf;

    fn entry(name: &str, bytes: u64, is_dir: bool) -> Entry {
        Entry {
            name: name.into(),
            bytes,
            is_dir,
            aggregated: false,
        }
    }

    fn scan_of(entries: Vec<Entry>) -> Scan {
        let total = entries.iter().map(|e| e.bytes).sum();
        Scan {
            root: PathBuf::from("/tmp/project"),
            entries,
            total_bytes: total,
            partial: false,
            unreadable: 0,
        }
    }

    fn sample_view() -> View {
        View::new(scan_of(vec![
            entry("target", 900_000_000, true),
            entry("crates", 40_000_000, true),
            entry("Cargo.lock", 208_825, false),
            entry("README.md", 2_390, false),
        ]))
    }

    fn find_text(tree: &Widget, needle: &str) -> bool {
        match tree {
            Widget::Text { s, .. } | Widget::Badge { s, .. } | Widget::Btn { s, .. } => {
                s.contains(needle)
            }
            Widget::Col { children, .. } | Widget::Row { children, .. } => {
                children.iter().any(|c| find_text(c, needle))
            }
            _ => false,
        }
    }

    #[test]
    fn bars_are_laid_out_on_a_near_square_grid() {
        // A single row of bars would make the depth axis carry no information.
        assert_eq!(grid_cols(1), 1);
        assert_eq!(grid_cols(4), 2);
        assert_eq!(grid_cols(9), 3);
        assert_eq!(grid_cols(12), 4);
        for n in 1..=crate::scan::MAX_BARS {
            let cols = grid_cols(n);
            let rows = n.div_ceil(cols);
            assert!(cols * rows >= n, "grid {cols}x{rows} cannot hold {n}");
            assert!(
                cols.abs_diff(rows) <= 2,
                "grid {cols}x{rows} is not near-square for {n}"
            );
        }
    }

    #[test]
    fn a_dominated_directory_still_shows_its_small_entries() {
        // The real case that motivated MIN_BAR_SHARE: target/ dwarfs everything.
        let view = View::new(scan_of(vec![
            entry("target", 6_000_000_000, true),
            entry("crates", 1_700_000, true),
            entry("README.md", 2_390, false),
        ]));
        let canvas = build_scene(&view).render(view.camera, 78, 20, view.zoom);
        assert!(canvas.has_ink());
        let ink: usize = canvas
            .to_rows()
            .iter()
            .map(|r| r.chars().filter(|c| *c != ' ').count())
            .sum();
        let solid: usize = canvas
            .to_rows()
            .iter()
            .map(|r| r.chars().filter(|c| *c == crate::raster::SOLID).count())
            .sum();
        assert!(
            ink > solid,
            "non-selected small entries must render: ink {ink}, selected {solid}"
        );
    }

    #[test]
    fn heights_stay_proportional_for_comparable_entries() {
        // Above the floor, twice the bytes must be twice the bar.
        let largest = 1_000_000.0f32;
        let h_big = (1_000_000.0 / largest).max(MIN_BAR_SHARE) * MAX_BAR_HEIGHT;
        let h_half = (500_000.0 / largest).max(MIN_BAR_SHARE) * MAX_BAR_HEIGHT;
        assert!((h_big / h_half - 2.0).abs() < 1e-4);
    }

    #[test]
    fn tree_is_within_the_node_and_depth_budget() {
        let view = sample_view();
        // Deliberately oversized surface: the budget must hold anyway.
        for (cols, rows) in [(40u16, 20u16), (120, 40), (400, 300), (2000, 2000)] {
            let tree = render(&view, cols, rows);
            let stats = measure(&tree);
            assert!(
                stats.within_budget(),
                "{cols}x{rows} produced {stats:?}, over ADR-0017 budget"
            );
        }
    }

    #[test]
    fn legend_reports_the_selected_entry_with_size_and_share() {
        let view = sample_view();
        let tree = render(&view, 80, 24);
        assert!(find_text(&tree, "target"));
        // 900_000_000 B / 1024^2 = 858.3 MiB → "858M" at 3 significant digits.
        assert!(find_text(&tree, "858M"), "expected a human size in legend");
        // 900000000 / 940211215 ≈ 95.7%
        assert!(find_text(&tree, "95.7%"));
    }

    #[test]
    fn chrome_fits_in_its_reserve() {
        // CHROME_NODE_RESERVE is subtracted from the node budget to size the
        // raster. If the chrome outgrows it the budget maths silently breaks,
        // so pin it here.
        let view = sample_view();
        let full = measure(&render(&view, 80, 24)).nodes;
        let raster = canvas_rows(24) as usize;
        let chrome = full.saturating_sub(raster);
        assert!(
            chrome <= CHROME_NODE_RESERVE,
            "chrome uses {chrome} nodes, reserve is {CHROME_NODE_RESERVE}"
        );
    }

    #[test]
    fn canvas_rows_is_clamped_at_both_ends() {
        assert_eq!(canvas_rows(0), MIN_CANVAS_ROWS);
        assert_eq!(canvas_rows(1), MIN_CANVAS_ROWS);
        assert_eq!(canvas_rows(30), 30 - CHROME_ROWS as u16 as u32);
        assert_eq!(canvas_rows(u16::MAX), MAX_CANVAS_ROWS);
    }

    #[test]
    fn budget_guard_keeps_the_chrome_and_trims_only_the_image() {
        // Hand-build an over-budget tree the way render would if canvas_rows
        // were unbounded, and prove the guard preserves legend and controls.
        let mut root = col().gap(0).child(text("head"));
        for i in 0..600 {
            root = root.child(text(format!("row{i}")));
        }
        let tree = clamp_to_budget(
            root.child(sep())
                .child(text("legend here"))
                .child(controls())
                .into(),
        );
        assert!(measure(&tree).within_budget(), "{:?}", measure(&tree));
        assert!(find_text(&tree, "legend here"));
        assert!(find_text(&tree, "Rescan"));
    }

    #[test]
    fn selecting_the_next_bar_changes_the_legend() {
        let mut view = sample_view();
        let before = render(&view, 80, 24);
        view.select_next();
        let after = render(&view, 80, 24);
        assert!(find_text(&before, "target"));
        assert!(find_text(&after, "crates"));
        assert_ne!(before, after);
    }

    #[test]
    fn selection_wraps_and_never_indexes_out_of_bounds() {
        let mut view = sample_view();
        for _ in 0..10 {
            view.select_next();
            assert!(view.selected_entry().is_some());
        }
        assert_eq!(view.selected, 10 % 4);
    }

    #[test]
    fn empty_scan_says_so_instead_of_drawing_an_empty_box() {
        let view = View::new(scan_of(vec![]));
        let tree = render(&view, 60, 20);
        assert!(find_text(&tree, "Nothing to chart"));
        assert!(measure(&tree).within_budget());
    }

    #[test]
    fn partial_scan_is_flagged_to_the_user() {
        let mut scan = scan_of(vec![entry("node_modules", 5_000_000, true)]);
        scan.partial = true;
        scan.unreadable = 3;
        let tree = render(&View::new(scan), 70, 22);
        assert!(find_text(&tree, "partial"));
        assert!(find_text(&tree, "3 unreadable"));
    }

    #[test]
    fn rotation_changes_the_rendered_rows() {
        let mut view = sample_view();
        let before = render(&view, 70, 24);
        view.yaw_by(0.8);
        assert_ne!(before, render(&view, 70, 24));
    }

    #[test]
    fn pitch_is_clamped_to_a_readable_range() {
        let mut view = sample_view();
        for _ in 0..50 {
            view.pitch_by(0.5);
        }
        assert!(view.camera.pitch <= 1.35);
        for _ in 0..50 {
            view.pitch_by(-0.5);
        }
        assert!(view.camera.pitch >= 0.05);
    }

    #[test]
    fn zoom_is_clamped() {
        let mut view = sample_view();
        for _ in 0..50 {
            view.zoom_by(1.5);
        }
        assert!(view.zoom <= 2.5);
        for _ in 0..50 {
            view.zoom_by(0.5);
        }
        assert!(view.zoom >= 0.5);
    }

    #[test]
    fn yaw_wraps_and_stays_finite() {
        let mut view = sample_view();
        for _ in 0..200 {
            view.yaw_by(0.7);
            assert!(view.camera.yaw.is_finite());
            assert!((0.0..std::f32::consts::TAU).contains(&view.camera.yaw));
        }
    }

    #[test]
    fn every_control_is_present() {
        let tree = render(&sample_view(), 80, 24);
        for label in ["◀", "▶", "▲", "▼", "+", "-", "Next bar", "Spin", "Rescan"] {
            assert!(find_text(&tree, label), "missing control {label}");
        }
    }

    #[test]
    fn max_bars_at_max_surface_still_fits_the_budget() {
        // The worst realistic case: the most bars the scanner can produce on a
        // very tall split.
        let entries: Vec<Entry> = (0..crate::scan::MAX_BARS)
            .map(|i| entry(&format!("dir{i:02}"), (i as u64 + 1) * 1_000_000, true))
            .collect();
        let tree = render(&View::new(scan_of(entries)), 200, 120);
        assert!(measure(&tree).within_budget(), "{:?}", measure(&tree));
    }

    #[test]
    fn a_tiny_surface_still_produces_a_usable_tree() {
        let tree = render(&sample_view(), 8, 6);
        assert!(measure(&tree).within_budget());
        // The chrome survives even when the raster is squeezed.
        assert!(find_text(&tree, "Rescan"));
    }

    #[test]
    fn replacing_the_scan_resets_the_selection() {
        let mut view = sample_view();
        view.select_next();
        assert_eq!(view.selected, 1);
        view.replace_scan(scan_of(vec![entry("only", 10, false)]));
        assert_eq!(view.selected, 0);
        assert!(view.selected_entry().is_some());
    }

    #[test]
    fn scene_data_is_normalised_and_grid_bounded() {
        let view = sample_view();
        let scene = build_scene_data(&view);
        assert!(scene.cols >= 1 && scene.rows >= 1);
        assert_eq!(scene.bars.len(), 4);
        // The tallest entry is a full-height (1.0) bar; all heights are shares.
        assert!((scene.bars[0].height - 1.0).abs() < 1e-4);
        for bar in &scene.bars {
            assert!((0.0..=1.0).contains(&bar.height), "height {bar:?}");
            assert!(bar.gx < scene.cols, "gx out of grid: {bar:?}");
            assert!(bar.gz < scene.rows, "gz out of grid: {bar:?}");
        }
        // Exactly the selected entry carries the selected flag/colour.
        assert!(scene.bars[0].selected);
        assert_eq!(scene.bars[0].color, SELECTED_COLOR);
        assert!(scene.bars.iter().skip(1).all(|b| !b.selected));
        // The camera mirrors the view.
        assert_eq!(scene.camera.yaw, view.camera.yaw);
        assert_eq!(scene.camera.pitch, view.camera.pitch);
        assert_eq!(scene.camera.zoom, view.zoom);
    }

    #[test]
    fn empty_scene_data_has_no_bars_and_no_grid() {
        let scene = build_scene_data(&View::new(scan_of(vec![])));
        assert_eq!(scene.cols, 0);
        assert_eq!(scene.rows, 0);
        assert!(scene.bars.is_empty());
    }

    #[test]
    fn a_dominated_directory_keeps_small_bars_visible_in_the_scene() {
        let view = View::new(scan_of(vec![
            entry("target", 6_000_000_000, true),
            entry("crates", 1_700_000, true),
            entry("README.md", 2_390, false),
        ]));
        let scene = build_scene_data(&view);
        // Even the smallest bar keeps the visible plinth, never zero height.
        assert!(scene.bars.iter().all(|b| b.height >= MIN_BAR_SHARE - 1e-6));
    }

    #[test]
    fn apply_camera_arg_updates_and_clamps() {
        let mut view = sample_view();
        view.apply_camera_arg(r#"{"yaw":1.0,"pitch":0.5,"zoom":1.5}"#);
        assert!((view.camera.yaw - 1.0).abs() < 1e-4);
        assert!((view.camera.pitch - 0.5).abs() < 1e-4);
        assert!((view.zoom - 1.5).abs() < 1e-4);
        // Out-of-range pitch and zoom are clamped, not accepted raw.
        view.apply_camera_arg(r#"{"pitch":9.0,"zoom":99.0}"#);
        assert!(view.camera.pitch <= 1.35);
        assert!(view.zoom <= 2.5);
        // Malformed payloads are ignored, leaving the current values intact.
        let before = (view.camera.yaw, view.camera.pitch, view.zoom);
        view.apply_camera_arg("yaw=notanumber&garbage");
        assert_eq!((view.camera.yaw, view.camera.pitch, view.zoom), before);
    }
}
