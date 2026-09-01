use bytemuck::{Pod, Zeroable};

use crate::view::{self, View};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 3],
}

const PALETTE: &[[f32; 3]] = &[
    [0.40, 0.70, 0.95],
    [0.95, 0.55, 0.35],
    [0.45, 0.85, 0.50],
    [0.90, 0.45, 0.65],
    [0.70, 0.55, 0.90],
    [0.95, 0.80, 0.35],
    [0.50, 0.80, 0.80],
    [0.85, 0.60, 0.45],
];

const HIGHLIGHT: [f32; 3] = [1.0, 1.0, 0.6];

const BAR_HALF: f32 = 0.34;
const BAR_PITCH: f32 = 1.0;
const MAX_BAR_HEIGHT: f32 = 4.5;
const MIN_BAR_SHARE: f32 = 0.05;

pub struct GpuScene {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl GpuScene {
    pub fn from_view(view: &View) -> Self {
        let entries = &view.scan.entries;
        let mut vertices = Vec::new();
        let mut indices = Vec::new();

        if entries.is_empty() {
            return Self { vertices, indices };
        }

        let largest = view.scan.largest_bytes().max(1);
        let cols = view::grid_cols(entries.len());
        let rows = entries.len().div_ceil(cols);
        let x0 = -((cols as f32 - 1.0) * BAR_PITCH) * 0.5;
        let z0 = -((rows as f32 - 1.0) * BAR_PITCH) * 0.5;

        // Floor quad
        {
            let hw = (cols as f32 * BAR_PITCH) * 0.5 + 0.5;
            let hd = (rows as f32 * BAR_PITCH) * 0.5 + 0.5;
            let color = [0.18, 0.18, 0.22];
            let n = [0.0, 1.0, 0.0];
            let base = vertices.len() as u32;
            vertices.push(Vertex { position: [-hw, 0.0, -hd], normal: n, color });
            vertices.push(Vertex { position: [ hw, 0.0, -hd], normal: n, color });
            vertices.push(Vertex { position: [ hw, 0.0,  hd], normal: n, color });
            vertices.push(Vertex { position: [-hw, 0.0,  hd], normal: n, color });
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }

        for (i, entry) in entries.iter().enumerate() {
            let gx = i % cols;
            let gz = i / cols;
            let cx = x0 + gx as f32 * BAR_PITCH;
            let cz = z0 + gz as f32 * BAR_PITCH;
            let share = entry.bytes as f32 / largest as f32;
            let height = share.max(MIN_BAR_SHARE) * MAX_BAR_HEIGHT;

            let color = if i == view.selected {
                HIGHLIGHT
            } else {
                PALETTE[i % PALETTE.len()]
            };

            push_box(
                &mut vertices,
                &mut indices,
                cx,
                cz,
                BAR_HALF,
                height,
                color,
            );
        }

        Self { vertices, indices }
    }
}

fn push_box(
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    cx: f32,
    cz: f32,
    half: f32,
    height: f32,
    color: [f32; 3],
) {
    let x0 = cx - half;
    let x1 = cx + half;
    let z0 = cz - half;
    let z1 = cz + half;
    let y0 = 0.0f32;
    let y1 = height;

    let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
        // +Y (top)
        ([0.0, 1.0, 0.0], [[x0, y1, z0], [x1, y1, z0], [x1, y1, z1], [x0, y1, z1]]),
        // -Y (bottom)
        ([0.0, -1.0, 0.0], [[x0, y0, z1], [x1, y0, z1], [x1, y0, z0], [x0, y0, z0]]),
        // +X (right)
        ([1.0, 0.0, 0.0], [[x1, y0, z0], [x1, y0, z1], [x1, y1, z1], [x1, y1, z0]]),
        // -X (left)
        ([-1.0, 0.0, 0.0], [[x0, y0, z1], [x0, y0, z0], [x0, y1, z0], [x0, y1, z1]]),
        // +Z (front)
        ([0.0, 0.0, 1.0], [[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]]),
        // -Z (back)
        ([0.0, 0.0, -1.0], [[x1, y0, z0], [x0, y0, z0], [x0, y1, z0], [x1, y1, z0]]),
    ];

    for (normal, corners) in &faces {
        let base = vertices.len() as u32;
        for &pos in corners {
            vertices.push(Vertex {
                position: pos,
                normal: *normal,
                color,
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}
