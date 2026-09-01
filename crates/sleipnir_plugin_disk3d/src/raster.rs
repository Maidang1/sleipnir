//! A tiny software 3D rasteriser that outputs character cells.
//!
//! ADR-0004 declines the graphics protocol and ADR-0017 bans images, so the
//! only framebuffer a plugin has is text. That is workable because the widget
//! renderer counts one Unicode scalar as one cell (`sleipnir_widget::cell_cols`)
//! and `wrap_text` preserves spaces and honours `\n`: a `col` of `text` rows is
//! a character raster with stable geometry.
//!
//! So this is a real renderer — orthographic projection, per-pixel z-buffer,
//! Lambert shading quantised onto a glyph ramp — just with cells for pixels.
//! Orthographic (not perspective) is deliberate: a chart must stay measurable,
//! and equal heights must read as equal from any angle.

/// Shading ramp for unselected geometry, dark → light. Each is one scalar, so
/// each occupies exactly one cell under the v1 occupancy rule.
pub const RAMP: [char; 3] = ['░', '▒', '▓'];

/// Selected geometry is drawn solid so it is unmistakable next to the legend.
pub const SOLID: char = '█';

/// Floor grid tick. Drawn slightly below y=0 so it never z-fights a bar base.
pub const FLOOR: char = '·';

/// Terminal cells are about twice as tall as they are wide. Vertical screen
/// distance is scaled by this so a cube looks like a cube instead of a column.
pub const CELL_ASPECT: f32 = 0.5;

/// Fraction of the surface left as margin when auto-fitting the scene.
const FIT_MARGIN: f32 = 0.92;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

pub const fn v3(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3 { x, y, z }
}

impl Vec3 {
    fn sub(self, o: Self) -> Self {
        v3(self.x - o.x, self.y - o.y, self.z - o.z)
    }

    fn cross(self, o: Self) -> Self {
        v3(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }

    fn dot(self, o: Self) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    fn normalized(self) -> Self {
        let len = self.dot(self).sqrt();
        if !len.is_finite() || len <= f32::EPSILON {
            return v3(0.0, 0.0, 0.0);
        }
        v3(self.x / len, self.y / len, self.z / len)
    }
}

/// Yaw about Y then pitch about X. The camera looks down +z, so a larger
/// rotated z is farther away.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub yaw: f32,
    pub pitch: f32,
}

impl Camera {
    pub fn rotate(&self, p: Vec3) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let x = p.x * cy + p.z * sy;
        let z = p.z * cy - p.x * sy;
        let y = p.y * cp - z * sp;
        let z = p.y * sp + z * cp;
        v3(x, y, z)
    }
}

/// One shaded quad in world space.
#[derive(Clone, Copy, Debug)]
struct Quad {
    verts: [Vec3; 4],
    /// Pre-resolved glyph; lighting is evaluated in world space so a face
    /// brightens and darkens as the model turns.
    glyph: char,
}

/// A character raster with a depth buffer.
#[derive(Clone, Debug)]
pub struct Canvas {
    pub cols: u32,
    pub rows: u32,
    cells: Vec<char>,
    depth: Vec<f32>,
}

impl Canvas {
    pub fn new(cols: u32, rows: u32) -> Self {
        let cols = cols.max(1);
        let rows = rows.max(1);
        let n = (cols as usize) * (rows as usize);
        Self {
            cols,
            rows,
            cells: vec![' '; n],
            depth: vec![f32::INFINITY; n],
        }
    }

    /// Depth-tested write. Nearer wins; out-of-range and non-finite are dropped.
    fn put(&mut self, col: u32, row: u32, depth: f32, glyph: char) {
        if col >= self.cols || row >= self.rows || !depth.is_finite() {
            return;
        }
        let i = (row as usize) * (self.cols as usize) + (col as usize);
        if depth < self.depth[i] {
            self.depth[i] = depth;
            self.cells[i] = glyph;
        }
    }

    /// One string per raster row, trailing blanks trimmed so a row can never
    /// exceed the surface width and trigger a wrap that would shear the image.
    pub fn to_rows(&self) -> Vec<String> {
        (0..self.rows)
            .map(|r| {
                let start = (r as usize) * (self.cols as usize);
                let end = start + (self.cols as usize);
                self.cells[start..end]
                    .iter()
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    /// Whether anything was drawn. Used to fall back to a message instead of
    /// showing an empty box.
    pub fn has_ink(&self) -> bool {
        self.cells.iter().any(|c| *c != ' ')
    }
}

/// Geometry accumulated in world space, then projected and rasterised.
#[derive(Clone, Debug, Default)]
pub struct Scene {
    quads: Vec<Quad>,
    points: Vec<(Vec3, char)>,
}

impl Scene {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.quads.is_empty() && self.points.is_empty()
    }

    /// A floor tick, nudged below the ground plane to avoid z-fighting.
    pub fn floor_tick(&mut self, x: f32, z: f32) {
        self.points.push((v3(x, -0.02, z), FLOOR));
    }

    /// An axis-aligned cuboid standing on y=0.
    ///
    /// All six faces are pushed; the depth buffer resolves visibility, which is
    /// simpler and more robust than trying to cull by hand.
    pub fn bar(&mut self, cx: f32, cz: f32, half: f32, height: f32, light: Vec3, selected: bool) {
        let h = height.max(0.0);
        let (x0, x1) = (cx - half, cx + half);
        let (z0, z1) = (cz - half, cz + half);
        let (y0, y1) = (0.0, h);
        // Wound so cross(e1, e2) points outward.
        let faces = [
            [v3(x0, y1, z0), v3(x1, y1, z0), v3(x1, y1, z1), v3(x0, y1, z1)], // top
            [v3(x0, y0, z1), v3(x1, y0, z1), v3(x1, y1, z1), v3(x0, y1, z1)], // front (+z)
            [v3(x1, y0, z0), v3(x0, y0, z0), v3(x0, y1, z0), v3(x1, y1, z0)], // back (-z)
            [v3(x1, y0, z1), v3(x1, y0, z0), v3(x1, y1, z0), v3(x1, y1, z1)], // right (+x)
            [v3(x0, y0, z0), v3(x0, y0, z1), v3(x0, y1, z1), v3(x0, y1, z0)], // left (-x)
            [v3(x0, y0, z0), v3(x1, y0, z0), v3(x1, y0, z1), v3(x0, y0, z1)], // bottom
        ];
        for verts in faces {
            let glyph = if selected {
                SOLID
            } else {
                shade(face_normal(&verts), light)
            };
            self.quads.push(Quad { verts, glyph });
        }
    }

    /// Project, auto-fit to the surface, and rasterise.
    ///
    /// Auto-fit is what makes the view robust: whatever the rotation or the
    /// number of bars, the projected bounding box is measured and mapped into
    /// the available cells, so the scene cannot drift off-surface.
    pub fn render(&self, cam: Camera, cols: u32, rows: u32, zoom: f32) -> Canvas {
        let mut canvas = Canvas::new(cols, rows);
        if self.is_empty() {
            return canvas;
        }

        // Pass 1: project to unit screen space and measure the extent.
        let project = |p: Vec3| {
            let r = cam.rotate(p);
            (r.x, -r.y * CELL_ASPECT, r.z)
        };
        let mut min_x = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        let mut observe = |sx: f32, sy: f32| {
            if sx.is_finite() && sy.is_finite() {
                min_x = min_x.min(sx);
                max_x = max_x.max(sx);
                min_y = min_y.min(sy);
                max_y = max_y.max(sy);
            }
        };
        for q in &self.quads {
            for v in q.verts {
                let (sx, sy, _) = project(v);
                observe(sx, sy);
            }
        }
        for (p, _) in &self.points {
            let (sx, sy, _) = project(*p);
            observe(sx, sy);
        }
        if !min_x.is_finite() || !min_y.is_finite() {
            return canvas;
        }

        let span_x = (max_x - min_x).max(1e-3);
        let span_y = (max_y - min_y).max(1e-3);
        let zoom = if zoom.is_finite() {
            zoom.clamp(0.3, 3.0)
        } else {
            1.0
        };
        let fit = ((cols as f32 - 1.0) / span_x).min((rows as f32 - 1.0) / span_y);
        let scale = fit * FIT_MARGIN * zoom;
        let cx = (min_x + max_x) * 0.5;
        let cy = (min_y + max_y) * 0.5;
        let ox = cols as f32 * 0.5;
        let oy = rows as f32 * 0.5;
        let to_screen = move |p: Vec3| {
            let (sx, sy, d) = project(p);
            [(sx - cx) * scale + ox, (sy - cy) * scale + oy, d]
        };

        // Pass 2: rasterise. Depth testing, not ordering, resolves occlusion.
        for q in &self.quads {
            let p: Vec<[f32; 3]> = q.verts.iter().map(|v| to_screen(*v)).collect();
            raster_tri(&mut canvas, p[0], p[1], p[2], q.glyph);
            raster_tri(&mut canvas, p[0], p[2], p[3], q.glyph);
        }
        for (p, glyph) in &self.points {
            let s = to_screen(*p);
            if s[0] >= 0.0 && s[1] >= 0.0 {
                canvas.put(s[0] as u32, s[1] as u32, s[2], *glyph);
            }
        }
        canvas
    }
}

fn face_normal(verts: &[Vec3; 4]) -> Vec3 {
    verts[1]
        .sub(verts[0])
        .cross(verts[3].sub(verts[0]))
        .normalized()
}

/// Lambert with ambient, quantised onto [`RAMP`]. Light lives in world space so
/// rotation changes shading, which is what sells the depth.
fn shade(normal: Vec3, light: Vec3) -> char {
    let lum = normal.dot(light).max(0.0);
    let lum = (0.18 + 0.82 * lum).clamp(0.0, 1.0);
    let idx = (lum * RAMP.len() as f32).floor() as usize;
    RAMP[idx.min(RAMP.len() - 1)]
}

/// Barycentric triangle fill with interpolated depth.
fn raster_tri(canvas: &mut Canvas, a: [f32; 3], b: [f32; 3], c: [f32; 3], glyph: char) {
    for p in [a, b, c] {
        if !p[0].is_finite() || !p[1].is_finite() || !p[2].is_finite() {
            return;
        }
    }
    let area = edge(a, b, c[0], c[1]);
    if area.abs() < 1e-6 {
        return;
    }
    let min_x = a[0].min(b[0]).min(c[0]).floor().max(0.0) as u32;
    let min_y = a[1].min(b[1]).min(c[1]).floor().max(0.0) as u32;
    let max_x = (a[0].max(b[0]).max(c[0]).ceil() as i64).clamp(0, canvas.cols as i64) as u32;
    let max_y = (a[1].max(b[1]).max(c[1]).ceil() as i64).clamp(0, canvas.rows as i64) as u32;
    for row in min_y..max_y {
        for col in min_x..max_x {
            let px = col as f32 + 0.5;
            let py = row as f32 + 0.5;
            let w0 = edge(b, c, px, py) / area;
            let w1 = edge(c, a, px, py) / area;
            let w2 = edge(a, b, px, py) / area;
            if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                continue;
            }
            let depth = w0 * a[2] + w1 * b[2] + w2 * c[2];
            canvas.put(col, row, depth, glyph);
        }
    }
}

fn edge(p: [f32; 3], q: [f32; 3], px: f32, py: f32) -> f32 {
    (q[0] - p[0]) * (py - p[1]) - (q[1] - p[1]) * (px - p[0])
}

/// Default light: upper-left-front.
pub fn default_light() -> Vec3 {
    v3(-0.45, 0.8, -0.4).normalized()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam() -> Camera {
        Camera {
            yaw: 0.6,
            pitch: 0.5,
        }
    }

    #[test]
    fn empty_scene_renders_blank_without_panicking() {
        let canvas = Scene::new().render(cam(), 20, 10, 1.0);
        assert!(!canvas.has_ink());
        assert_eq!(canvas.to_rows().len(), 10);
        assert!(canvas.to_rows().iter().all(String::is_empty));
    }

    #[test]
    fn a_bar_draws_ink_and_stays_inside_the_surface() {
        let mut scene = Scene::new();
        scene.bar(0.0, 0.0, 0.4, 2.0, default_light(), false);
        let canvas = scene.render(cam(), 40, 20, 1.0);
        assert!(canvas.has_ink());
        let rows = canvas.to_rows();
        assert_eq!(rows.len(), 20);
        // Never wider than the surface: a wrap here would shear the image.
        assert!(rows.iter().all(|r| r.chars().count() <= 40));
    }

    #[test]
    fn render_is_deterministic() {
        let mut scene = Scene::new();
        scene.bar(0.0, 0.0, 0.4, 1.5, default_light(), false);
        scene.floor_tick(1.0, 1.0);
        let a = scene.render(cam(), 30, 14, 1.0).to_rows();
        let b = scene.render(cam(), 30, 14, 1.0).to_rows();
        assert_eq!(a, b);
    }

    #[test]
    fn nearer_geometry_occludes_farther_geometry() {
        // Two bars on the same screen column, different depths. The near one
        // must win at least one cell, proving the depth test is live.
        let mut scene = Scene::new();
        let straight = Camera {
            yaw: 0.0,
            pitch: 0.0,
        };
        scene.bar(0.0, 4.0, 0.5, 1.0, default_light(), false);
        scene.bar(0.0, -4.0, 0.5, 1.0, default_light(), true);
        let rows = scene.render(straight, 30, 16, 1.0).to_rows();
        let joined = rows.join("\n");
        assert!(
            joined.contains(SOLID),
            "the near selected bar must be visible, got:\n{joined}"
        );
    }

    #[test]
    fn selected_bar_is_solid_and_unselected_is_shaded() {
        let mut sel = Scene::new();
        sel.bar(0.0, 0.0, 0.5, 1.0, default_light(), true);
        let sel_rows = sel.render(cam(), 30, 16, 1.0).to_rows().join("\n");
        assert!(sel_rows.contains(SOLID));
        assert!(RAMP.iter().all(|g| !sel_rows.contains(*g)));

        let mut plain = Scene::new();
        plain.bar(0.0, 0.0, 0.5, 1.0, default_light(), false);
        let plain_rows = plain.render(cam(), 30, 16, 1.0).to_rows().join("\n");
        assert!(!plain_rows.contains(SOLID));
        assert!(RAMP.iter().any(|g| plain_rows.contains(*g)));
    }

    #[test]
    fn rotation_changes_the_image() {
        let mut scene = Scene::new();
        scene.bar(-1.0, 0.0, 0.4, 1.0, default_light(), false);
        scene.bar(1.0, 0.0, 0.4, 2.0, default_light(), false);
        let a = scene
            .render(
                Camera {
                    yaw: 0.2,
                    pitch: 0.4,
                },
                40,
                18,
                1.0,
            )
            .to_rows();
        let b = scene
            .render(
                Camera {
                    yaw: 1.4,
                    pitch: 0.4,
                },
                40,
                18,
                1.0,
            )
            .to_rows();
        assert_ne!(a, b);
    }

    #[test]
    fn zero_height_bar_still_renders_a_footprint() {
        let mut scene = Scene::new();
        scene.bar(0.0, 0.0, 0.5, 0.0, default_light(), false);
        assert!(scene.render(cam(), 24, 12, 1.0).has_ink());
    }

    #[test]
    fn non_finite_input_is_ignored_not_drawn() {
        let mut scene = Scene::new();
        scene.bar(f32::NAN, 0.0, 0.5, 1.0, default_light(), false);
        let canvas = scene.render(cam(), 24, 12, 1.0);
        assert!(!canvas.has_ink());
    }

    #[test]
    fn extreme_zoom_is_clamped_and_never_overflows_the_surface() {
        let mut scene = Scene::new();
        scene.bar(0.0, 0.0, 0.5, 2.0, default_light(), false);
        for zoom in [f32::NAN, -5.0, 0.0, 1000.0] {
            let canvas = scene.render(cam(), 32, 16, zoom);
            assert_eq!(canvas.to_rows().len(), 16);
            assert!(canvas.to_rows().iter().all(|r| r.chars().count() <= 32));
        }
    }

    #[test]
    fn every_glyph_occupies_exactly_one_cell() {
        // The raster is only geometrically sound if one scalar is one cell.
        for glyph in RAMP.iter().chain([&SOLID, &FLOOR]) {
            assert_eq!(sleipnir_widget::cell_cols(&glyph.to_string()), 1);
        }
    }
}
