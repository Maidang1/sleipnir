//! Spec tests for the pure mapping. Assertions name the invariant
//! they guard; helpers keep geometries short.

use super::*;

fn geom(lh: Px, line_count: i32) -> RowGeometry {
    let mut g = RowGeometry::new(lh);
    g.set_line_count(line_count);
    g
}

fn assert_sub_in_range(pos: ViewportPosition, geom: &RowGeometry) {
    let h = geom.height_of(i32::try_from(pos.row).unwrap_or(i32::MAX));
    assert!(
        pos.sub.is_finite() && pos.sub >= 0.0,
        "sub must be finite and >= 0, got {}",
        pos.sub
    );
    if h <= 0.0 {
        assert_eq!(pos.sub, 0.0, "zero-height row must keep sub at 0");
    } else {
        assert!(
            pos.sub < h,
            "sub {} must be < height_of(row) {}",
            pos.sub,
            h
        );
    }
}

// ---------------------------------------------------------------------------
// y_for is linear
// ---------------------------------------------------------------------------

#[test]
fn y_for_is_linear() {
    let g = geom(16.0, 40);
    for n in -4..40 {
        assert_eq!(g.y_for(n), n as Px * 16.0);
        assert_eq!(g.height_of(n), 16.0);
    }
}

// ---------------------------------------------------------------------------
// hit: y → row
// ---------------------------------------------------------------------------

#[test]
fn hit_of_y_for_is_that_line() {
    let lh = 16.0;
    let g = geom(lh, 30);
    for n in -5..30 {
        assert_eq!(g.hit(g.y_for(n)), n);
    }
}

#[test]
fn hit_of_interior_y_identifies_the_same_line() {
    let lh = 17.0;
    let g = geom(lh, 30);
    for line in 0..30 {
        let top = g.y_for(line);
        for &frac in &[0.0, 0.25, 0.5, 0.75] {
            let y = top + lh * frac;
            // The last pixel of the span belongs to this line; `lh` itself
            // is the next line, so stay strictly inside.
            let y = if frac == 0.0 {
                y
            } else {
                y.min(top + lh - lh * 0.01)
            };
            assert_eq!(g.hit(y), line);
        }
    }
}

#[test]
fn hit_clamps_non_finite_y() {
    let g = geom(16.0, 30);
    assert_eq!(g.hit(Px::NAN), 0);
    assert_eq!(g.hit(Px::INFINITY), i32::MAX);
    assert_eq!(g.hit(Px::NEG_INFINITY), i32::MIN);
}

// ---------------------------------------------------------------------------
// y_for / hit round-trip, property style
// ---------------------------------------------------------------------------

fn sample_geometries() -> Vec<RowGeometry> {
    let mut out = Vec::new();
    for &lh in &[1.0, 2.0, 7.0, 16.0, 17.0, 18.5, 21.0, 32.0] {
        out.push(geom(lh, 0));
        out.push(geom(lh, 1));
        out.push(geom(lh, 40));
        out.push(geom(lh, 80));
        out.push(geom(lh, 200));
    }
    out
}

#[test]
fn y_for_and_hit_round_trip_on_cell_rows() {
    for g in sample_geometries() {
        let lo = -5;
        let hi = g.line_count().max(20) + 5;
        for line in lo..hi {
            let y = g.y_for(line);
            assert_eq!(g.hit(y), line, "lh={} line={line} y={y}", g.line_height());
        }
    }
}

// ---------------------------------------------------------------------------
// ViewportPosition: sub-row normalisation
// ---------------------------------------------------------------------------

#[test]
fn sub_stays_in_half_open_range_for_positive_delta() {
    let g = geom(16.0, 40);
    let mut pos = ViewportPosition::new(5);
    let spilled = pos.apply_pixel_delta(40.0, &g);
    assert_eq!(spilled, 2);
    assert_eq!(pos.row, 7);
    assert_eq!(pos.sub, 8.0);
    assert_sub_in_range(pos, &g);
}

#[test]
fn sub_stays_in_half_open_range_for_negative_delta() {
    let g = geom(16.0, 40);
    let mut pos = ViewportPosition { row: 8, sub: 4.0 };
    let spilled = pos.apply_pixel_delta(-20.0, &g);
    assert_eq!(spilled, -1);
    assert_eq!(pos.row, 7);
    assert_eq!(pos.sub, 0.0);
    assert_sub_in_range(pos, &g);
}

#[test]
fn sub_normalises_deltas_larger_than_several_rows() {
    let g = geom(10.0, 80);
    let mut pos = ViewportPosition::new(0);
    let spilled = pos.apply_pixel_delta(10.0 * 12.0 + 3.0, &g);
    assert_eq!(spilled, 12);
    assert_eq!(pos.row, 12);
    assert_eq!(pos.sub, 3.0);
    assert_sub_in_range(pos, &g);

    let spilled = pos.apply_pixel_delta(-10.0 * 5.0 - 1.0, &g);
    assert_eq!(spilled, -5);
    assert_eq!(pos.row, 7);
    assert_eq!(pos.sub, 2.0);
    assert_sub_in_range(pos, &g);
}

#[test]
fn apply_pixel_delta_clamps_before_line_zero() {
    let g = geom(16.0, 20);
    let mut pos = ViewportPosition { row: 2, sub: 4.0 };
    let spilled = pos.apply_pixel_delta(-1000.0, &g);
    assert_eq!(spilled, -2);
    assert_eq!(pos.row, 0);
    assert_eq!(pos.sub, 0.0);
}

#[test]
fn apply_pixel_delta_zero_and_non_finite_are_noops() {
    let g = geom(16.0, 20);
    let mut pos = ViewportPosition { row: 3, sub: 2.0 };
    assert_eq!(pos.apply_pixel_delta(0.0, &g), 0);
    assert_eq!(pos.row, 3);
    assert_eq!(pos.sub, 2.0);
    assert_eq!(pos.apply_pixel_delta(Px::NAN, &g), 0);
    assert_eq!(pos.apply_pixel_delta(Px::INFINITY, &g), 0);
    assert_eq!(pos.row, 3);
}

// ---------------------------------------------------------------------------
// scroll-to-anchor lands flush
// ---------------------------------------------------------------------------

#[test]
fn scroll_to_anchor_sets_sub_zero() {
    let g = geom(16.0, 40);
    let mut pos = ViewportPosition { row: 20, sub: 11.0 };
    pos.jump_to_anchor(12);
    assert_eq!(pos.row, 12);
    assert_eq!(pos.sub, 0.0);
    assert_eq!(g.hit(g.y_for(pos.row as i32) + pos.sub), 12);
}

#[test]
fn scroll_to_anchor_clamps_negative_lines() {
    let pos = ViewportPosition::scroll_to_anchor(-4);
    assert_eq!(pos.row, 0);
    assert_eq!(pos.sub, 0.0);
}

// ---------------------------------------------------------------------------
// Degenerate inputs: never panic
// ---------------------------------------------------------------------------

#[test]
fn empty_geometry_is_linear_and_zero_extent() {
    let g = RowGeometry::new(16.0);
    assert_eq!(g.total_height(), 0.0);
    assert_eq!(g.y_for(0), 0.0);
    assert_eq!(g.hit(0.0), 0);
    assert_eq!(g.hit(32.0), 2);
}

#[test]
fn zero_and_non_finite_line_height_do_not_panic() {
    for h in [0.0, -4.0, Px::NAN, Px::INFINITY, Px::NEG_INFINITY] {
        let mut g = RowGeometry::new(h);
        g.set_line_count(10);
        let _ = g.y_for(0);
        let _ = g.y_for(-3);
        let _ = g.y_for(i32::MAX);
        let _ = g.hit(0.0);
        let _ = g.hit(Px::NAN);
        let _ = g.hit(Px::INFINITY);
        let _ = g.total_height();
        let _ = g.height_of(0);
        let mut pos = ViewportPosition::new(0);
        let _ = pos.apply_pixel_delta(12.0, &g);
    }
}

#[test]
fn absurd_lines_do_not_panic() {
    let mut g = RowGeometry::new(16.0);
    g.set_line_count(i32::MAX);
    let _ = g.y_for(0);
    let _ = g.y_for(i32::MAX);
    let _ = g.y_for(i32::MIN);
    let _ = g.hit(0.0);
    let _ = g.hit(Px::MAX);
    let _ = g.total_height();
}
