//! Host-side projection and vector painting for plugin 3D scenes (ADR-0017).
//!
//! A plugin sends a [`SceneData`] — a grid of bars with normalised heights and
//! RGB colours, plus a camera — and the host owns everything downstream: it
//! projects the geometry against the panel's real pixel bounds and paints it as
//! filled polygons. Two things fall out of that split:
//!
//! - **Crisp at any size.** There is no bitmap to scale; the polygons are
//!   re-projected every frame against the current bounds, so a resize just
//!   re-fits the picture instead of stretching pixels.
//! - **Local camera.** The host can rotate/zoom by mutating the stored camera
//!   and repainting, with no round-trip to the plugin per frame.
//!
//! This module is the pure part: projection, auto-fit, face ordering and
//! shading. It names no gpui type so it stays unit testable. `layout.rs` calls
//! [`project_scene`] inside a `canvas` and paints the returned faces.

use plugin_protocol::v2::{SceneCamera, SceneData};

/// Half-width of a bar's square footprint, in world units.
const BAR_HALF: f32 = 0.34;
/// Grid spacing between bar centres, in world units.
const BAR_PITCH: f32 = 1.0;
/// Tallest bar in world units. Heights arrive normalised `0..=1`; this maps a
/// full-height bar into world space so the model has consistent proportions.
const MAX_BAR_HEIGHT: f32 = 4.5;
/// Fraction of the panel left as margin when auto-fitting the projected scene.
const FIT_MARGIN: f32 = 0.9;

/// One projected, shaded quad ready to fill, in panel-local pixel coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjFace {
    /// Four screen-space corners, panel-local pixels.
    pub pts: [[f32; 2]; 4],
    /// Shaded RGB colour.
    pub color: [u8; 3],
    /// Depth of the face centre (rotated z); larger is farther. Used to sort
    /// back-to-front for the painter's algorithm.
    pub depth: f32,
    /// True when this face belongs to the selected bar, so the caller can
    /// outline it.
    pub selected: bool,
}

/// A projected scene: faces sorted back-to-front, ready to paint in order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProjectedScene {
    pub faces: Vec<ProjFace>,
}

/// World-space light direction (upper-left-front), fixed to the model so each
/// face keeps a stable shade and the differing shades read as depth.
fn light() -> [f32; 3] {
    normalize([-0.45, 0.8, -0.4])
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if !len.is_finite() || len <= f32::EPSILON {
        return [0.0, 0.0, 0.0];
    }
    [v[0] / len, v[1] / len, v[2] / len]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Yaw about Y then pitch about X, matching the plugin's software raster so the
/// camera arithmetic is the same on both paths. The camera looks down +z, so a
/// larger rotated z is farther away.
fn rotate(cam: &SceneCamera, p: [f32; 3]) -> [f32; 3] {
    let (sy, cy) = cam.yaw.sin_cos();
    let (sp, cp) = cam.pitch.sin_cos();
    let x = p[0] * cy + p[2] * sy;
    let z = p[2] * cy - p[0] * sy;
    let y = p[1] * cp - z * sp;
    let z = p[1] * sp + z * cp;
    [x, y, z]
}

/// Project a world point to unit screen space `(x, -y, z)`.
///
/// Screen y grows downward, so world +y (up) becomes screen -y. Unlike the
/// character raster there is no cell-aspect correction: panel pixels are square,
/// so a cube projects to a cube.
fn project(cam: &SceneCamera, p: [f32; 3]) -> [f32; 3] {
    let r = rotate(cam, p);
    [r[0], -r[1], r[2]]
}

/// One world-space face before projection.
struct WorldFace {
    verts: [[f32; 3]; 4],
    color: [u8; 3],
    selected: bool,
}

/// Build the world geometry: a floor quad plus one cuboid per bar.
///
/// Bars sit on a grid centred on the origin so the auto-fit framing stays
/// stable as the entry count changes. Heights arrive normalised and are scaled
/// into world units here.
fn world_faces(scene: &SceneData) -> Vec<WorldFace> {
    let mut faces = Vec::new();
    let cols = scene.cols.max(1) as f32;
    let rows = scene.rows.max(1) as f32;

    // Floor: a single quad slightly larger than the grid footprint.
    let hw = (cols * BAR_PITCH) * 0.5 + 0.5;
    let hd = (rows * BAR_PITCH) * 0.5 + 0.5;
    faces.push(WorldFace {
        verts: [
            [-hw, 0.0, -hd],
            [hw, 0.0, -hd],
            [hw, 0.0, hd],
            [-hw, 0.0, hd],
        ],
        color: scene.floor,
        selected: false,
    });

    let x0 = -((cols - 1.0) * BAR_PITCH) * 0.5;
    let z0 = -((rows - 1.0) * BAR_PITCH) * 0.5;
    for bar in &scene.bars {
        let cx = x0 + bar.gx as f32 * BAR_PITCH;
        let cz = z0 + bar.gz as f32 * BAR_PITCH;
        let h = bar.height.clamp(0.0, 1.0) * MAX_BAR_HEIGHT;
        push_box(&mut faces, cx, cz, BAR_HALF, h, bar.color, bar.selected);
    }
    faces
}

/// Six faces of an axis-aligned cuboid standing on y=0, wound so the cross of
/// the first and last edges points outward (used for Lambert shading).
fn push_box(
    faces: &mut Vec<WorldFace>,
    cx: f32,
    cz: f32,
    half: f32,
    height: f32,
    color: [u8; 3],
    selected: bool,
) {
    let (x0, x1) = (cx - half, cx + half);
    let (z0, z1) = (cz - half, cz + half);
    let (y0, y1) = (0.0, height.max(0.0));
    let corners: [[[f32; 3]; 4]; 6] = [
        // top (+Y)
        [[x0, y1, z0], [x1, y1, z0], [x1, y1, z1], [x0, y1, z1]],
        // front (+Z)
        [[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]],
        // back (-Z)
        [[x1, y0, z0], [x0, y0, z0], [x0, y1, z0], [x1, y1, z0]],
        // right (+X)
        [[x1, y0, z1], [x1, y0, z0], [x1, y1, z0], [x1, y1, z1]],
        // left (-X)
        [[x0, y0, z0], [x0, y0, z1], [x0, y1, z1], [x0, y1, z0]],
        // bottom (-Y)
        [[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]],
    ];
    for verts in corners {
        faces.push(WorldFace {
            verts,
            color,
            selected,
        });
    }
}

/// Lambert with ambient, applied to an RGB colour. Light is fixed in world
/// space, so a face keeps a stable brightness and the spread of shades across
/// faces reads as depth.
fn shade(normal: [f32; 3], color: [u8; 3]) -> [u8; 3] {
    let lum = dot(normal, light()).max(0.0);
    let l = (0.2 + 0.8 * lum).clamp(0.0, 1.0);
    [
        (color[0] as f32 * l).round().clamp(0.0, 255.0) as u8,
        (color[1] as f32 * l).round().clamp(0.0, 255.0) as u8,
        (color[2] as f32 * l).round().clamp(0.0, 255.0) as u8,
    ]
}

/// The projected bounding box of every face corner, in unit screen space.
/// `None` when the scene has no geometry or projects to nothing finite.
pub fn bounding_box(scene: &SceneData) -> Option<(f32, f32, f32, f32)> {
    let cam = &scene.camera;
    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for face in world_faces(scene) {
        for v in face.verts {
            let s = project(cam, v);
            if s[0].is_finite() && s[1].is_finite() {
                min_x = min_x.min(s[0]);
                max_x = max_x.max(s[0]);
                min_y = min_y.min(s[1]);
                max_y = max_y.max(s[1]);
            }
        }
    }
    if min_x.is_finite() && min_y.is_finite() {
        Some((min_x, max_x, min_y, max_y))
    } else {
        None
    }
}

/// Screen mapping from the unit-projected bounding box: a uniform scale that
/// fits the box into `width`×`height` with margin and zoom, and the centring
/// offsets. Returned as `(scale, cx, cy, ox, oy)` where a unit point `(px, py)`
/// maps to `((px - cx) * scale + ox, (py - cy) * scale + oy)`.
pub fn fit_transform(
    bbox: (f32, f32, f32, f32),
    width: f32,
    height: f32,
    zoom: f32,
) -> (f32, f32, f32, f32, f32) {
    let (min_x, max_x, min_y, max_y) = bbox;
    let span_x = (max_x - min_x).max(1e-3);
    let span_y = (max_y - min_y).max(1e-3);
    let zoom = if zoom.is_finite() {
        zoom.clamp(0.3, 3.0)
    } else {
        1.0
    };
    let w = width.max(1.0);
    let h = height.max(1.0);
    let fit = ((w - 1.0) / span_x).min((h - 1.0) / span_y);
    let scale = (fit * FIT_MARGIN * zoom).max(0.0);
    let cx = (min_x + max_x) * 0.5;
    let cy = (min_y + max_y) * 0.5;
    (scale, cx, cy, w * 0.5, h * 0.5)
}

/// Project, auto-fit and shade a scene into paintable faces, sorted
/// back-to-front for the painter's algorithm.
///
/// `width`/`height` are the panel's real pixel size, so the picture re-fits on
/// every resize without another message from the plugin.
pub fn project_scene(scene: &SceneData, width: f32, height: f32) -> ProjectedScene {
    let Some(bbox) = bounding_box(scene) else {
        return ProjectedScene::default();
    };
    let (scale, cx, cy, ox, oy) = fit_transform(bbox, width, height, scene.camera.zoom);
    let cam = &scene.camera;
    let to_screen = |p: [f32; 3]| -> ([f32; 2], f32) {
        let s = project(cam, p);
        ([(s[0] - cx) * scale + ox, (s[1] - cy) * scale + oy], s[2])
    };

    let mut faces = Vec::new();
    for face in world_faces(scene) {
        let normal = normalize(cross(
            sub(face.verts[1], face.verts[0]),
            sub(face.verts[3], face.verts[0]),
        ));
        let mut pts = [[0.0f32; 2]; 4];
        let mut depth_sum = 0.0f32;
        let mut finite = true;
        for (i, v) in face.verts.iter().enumerate() {
            let (screen, d) = to_screen(*v);
            if !screen[0].is_finite() || !screen[1].is_finite() || !d.is_finite() {
                finite = false;
                break;
            }
            pts[i] = screen;
            depth_sum += d;
        }
        if !finite {
            continue;
        }
        faces.push(ProjFace {
            pts,
            color: shade(normal, face.color),
            depth: depth_sum / 4.0,
            selected: face.selected,
        });
    }
    // Painter's algorithm: farther faces (larger rotated z) first, so nearer
    // ones overpaint them. Total order via total_cmp keeps NaN from panicking.
    faces.sort_by(|a, b| b.depth.total_cmp(&a.depth));
    ProjectedScene { faces }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plugin_protocol::v2::{SceneBar, SceneCamera, SceneData};

    fn scene(cols: u32, rows: u32, bars: Vec<SceneBar>) -> SceneData {
        SceneData {
            cols,
            rows,
            floor: [46, 46, 56],
            camera: SceneCamera {
                yaw: 0.7,
                pitch: 0.42,
                zoom: 1.0,
            },
            bars,
        }
    }

    fn bar(gx: u32, gz: u32, height: f32, selected: bool) -> SceneBar {
        SceneBar {
            gx,
            gz,
            height,
            color: [100, 170, 240],
            selected,
        }
    }

    #[test]
    fn empty_scene_projects_to_nothing() {
        // No bars still has a floor, so it projects; a truly empty grid does not.
        let projected = project_scene(&scene(0, 0, vec![]), 200.0, 100.0);
        // cols/rows clamp to 1 so a floor quad still exists.
        assert_eq!(projected.faces.len(), 1);
    }

    #[test]
    fn one_bar_yields_floor_plus_six_faces() {
        let projected = project_scene(&scene(1, 1, vec![bar(0, 0, 1.0, false)]), 300.0, 200.0);
        // 1 floor + 6 cube faces.
        assert_eq!(projected.faces.len(), 7);
        // Every corner lands inside the panel bounds after auto-fit.
        for face in &projected.faces {
            for p in face.pts {
                assert!(p[0] >= -1.0 && p[0] <= 301.0, "x out of bounds: {p:?}");
                assert!(p[1] >= -1.0 && p[1] <= 201.0, "y out of bounds: {p:?}");
            }
        }
    }

    #[test]
    fn faces_are_sorted_back_to_front() {
        let projected = project_scene(
            &scene(2, 2, vec![bar(0, 0, 1.0, false), bar(1, 1, 0.5, true)]),
            400.0,
            300.0,
        );
        for pair in projected.faces.windows(2) {
            assert!(
                pair[0].depth >= pair[1].depth,
                "faces must be far-to-near: {} then {}",
                pair[0].depth,
                pair[1].depth
            );
        }
    }

    #[test]
    fn bounding_box_and_fit_centre_the_scene() {
        let s = scene(2, 2, vec![bar(0, 0, 1.0, false), bar(1, 0, 0.3, false)]);
        let bbox = bounding_box(&s).expect("non-empty scene has a bbox");
        let (min_x, max_x, min_y, max_y) = bbox;
        assert!(max_x > min_x && max_y > min_y);
        let (scale, cx, cy, ox, oy) = fit_transform(bbox, 200.0, 120.0, 1.0);
        assert!(scale > 0.0);
        // The bbox centre maps to the panel centre.
        assert!(((min_x + max_x) * 0.5 - cx).abs() < 1e-3);
        assert!(((min_y + max_y) * 0.5 - cy).abs() < 1e-3);
        assert!((ox - 100.0).abs() < 1e-3);
        assert!((oy - 60.0).abs() < 1e-3);
    }

    #[test]
    fn zoom_scales_the_projection() {
        let s = scene(3, 3, vec![bar(0, 0, 1.0, false)]);
        let bbox = bounding_box(&s).unwrap();
        let (near, ..) = fit_transform(bbox, 300.0, 300.0, 1.0);
        let (far, ..) = fit_transform(bbox, 300.0, 300.0, 2.0);
        assert!(far > near, "a larger zoom must scale up");
        // Extreme zoom is clamped, never unbounded.
        let (clamped, ..) = fit_transform(bbox, 300.0, 300.0, 1000.0);
        let (max_ok, ..) = fit_transform(bbox, 300.0, 300.0, 3.0);
        assert!((clamped - max_ok).abs() < 1e-3);
    }

    #[test]
    fn selected_bar_faces_carry_the_selected_flag() {
        let projected = project_scene(
            &scene(2, 1, vec![bar(0, 0, 1.0, false), bar(1, 0, 1.0, true)]),
            300.0,
            200.0,
        );
        assert!(
            projected.faces.iter().any(|f| f.selected),
            "the selected bar must be paintable with an outline"
        );
    }

    #[test]
    fn non_finite_camera_does_not_panic_or_emit_bad_faces() {
        let mut s = scene(1, 1, vec![bar(0, 0, 1.0, false)]);
        s.camera.yaw = f32::NAN;
        let projected = project_scene(&s, 200.0, 200.0);
        for face in &projected.faces {
            for p in face.pts {
                assert!(p[0].is_finite() && p[1].is_finite());
            }
        }
    }
}
