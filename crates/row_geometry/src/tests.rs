//! Spec tests for ADR-0018's pure mapping. Assertions name the invariant
//! they guard; helpers keep geometries short.

use super::*;

fn bid(n: u128) -> BlockId {
    BlockId::from_u128(n)
}

fn rid(n: u128) -> RunId {
    RunId::from_u128(n)
}

fn block(n: u128, line: i32, height: u16) -> Block {
    Block {
        id: bid(n),
        run_id: rid(n),
        anchor: Anchor { line, column: 0 },
        height,
    }
}

fn geom(lh: Px, line_count: i32, blocks: &[(u128, i32, u16)]) -> RowGeometry {
    let mut g = RowGeometry::new(lh);
    g.set_line_count(line_count);
    for &(n, line, height) in blocks {
        g.upsert(block(n, line, height));
    }
    g
}

fn cell(line: i32) -> HitTarget {
    HitTarget::Cell { line }
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
// y_for is a step function across and between blocks
// ---------------------------------------------------------------------------

#[test]
fn y_for_is_linear_without_blocks() {
    let g = geom(16.0, 40, &[]);
    for n in -4..40 {
        assert_eq!(g.y_for(n), n as Px * 16.0);
        assert_eq!(g.height_of(n), 16.0);
    }
}

#[test]
fn y_for_steps_by_block_height_at_anchor_and_by_one_row_elsewhere() {
    let lh = 16.0;
    let g = geom(lh, 30, &[(1, 10, 5)]);
    for n in 0..10 {
        assert_eq!(g.y_for(n), n as Px * lh, "before the block, still linear");
        assert_eq!(g.y_for(n + 1) - g.y_for(n), lh);
    }
    assert_eq!(g.y_for(10), 10.0 * lh);
    assert_eq!(g.y_for(11) - g.y_for(10), 5.0 * lh, "across the block");
    assert_eq!(g.height_of(10), 5.0 * lh);
    for n in 11..20 {
        assert_eq!(
            g.y_for(n + 1) - g.y_for(n),
            lh,
            "after the block, one row again"
        );
        assert_eq!(g.y_for(n), (n + 4) as Px * lh, "shifted by extra 4 rows");
    }
}

#[test]
fn y_for_stacks_two_blocks_at_the_same_line() {
    let lh = 8.0;
    let g = geom(lh, 20, &[(2, 4, 3), (1, 4, 2)]);
    // Sorted by id: 1 then 2. Heights 2 + 3, extra = 4.
    assert_eq!(g.height_of(4), 5.0 * lh);
    assert_eq!(g.y_for(5) - g.y_for(4), 5.0 * lh);
    assert_eq!(g.y_for(5), (5 + 4) as Px * lh);
}

// ---------------------------------------------------------------------------
// hit: Block with local_y inside, Cell outside
// ---------------------------------------------------------------------------

#[test]
fn hit_returns_block_inside_and_cell_outside() {
    let lh = 16.0;
    let g = geom(lh, 30, &[(1, 10, 5)]);
    let top = g.y_for(10);
    let bottom = g.y_for(11);

    assert_eq!(
        g.hit(top),
        HitTarget::Block {
            id: bid(1),
            local_y: 0.0
        }
    );
    match g.hit(top + 2.0 * lh) {
        HitTarget::Block { id, local_y } => {
            assert_eq!(id, bid(1));
            assert_eq!(local_y, 2.0 * lh);
        }
        other => panic!("expected Block, got {other:?}"),
    }
    match g.hit(bottom - 0.5) {
        HitTarget::Block { id, local_y } => {
            assert_eq!(id, bid(1));
            assert!(local_y > 0.0 && local_y < 5.0 * lh);
        }
        other => panic!("expected Block just inside, got {other:?}"),
    }

    assert_eq!(g.hit(g.y_for(9)), cell(9));
    assert_eq!(g.hit(g.y_for(9) + lh / 2.0), cell(9));
    assert_eq!(g.hit(bottom), cell(11));
    assert_eq!(g.hit(0.0), cell(0));
}

#[test]
fn hit_stacked_blocks_use_local_y_of_the_hit_block() {
    let lh = 10.0;
    let g = geom(lh, 20, &[(1, 3, 2), (2, 3, 3)]);
    let top = g.y_for(3);
    assert_eq!(
        g.hit(top),
        HitTarget::Block {
            id: bid(1),
            local_y: 0.0
        }
    );
    assert_eq!(
        g.hit(top + 2.0 * lh),
        HitTarget::Block {
            id: bid(2),
            local_y: 0.0
        }
    );
    match g.hit(top + 2.0 * lh + 5.0) {
        HitTarget::Block { id, local_y } => {
            assert_eq!(id, bid(2));
            assert_eq!(local_y, 5.0);
        }
        other => panic!("{other:?}"),
    }
}

// ---------------------------------------------------------------------------
// y_for / hit round-trip, property style
// ---------------------------------------------------------------------------

fn sample_geometries() -> Vec<RowGeometry> {
    let mut out = Vec::new();
    for &lh in &[1.0, 2.0, 7.0, 16.0, 17.0, 18.5, 21.0, 32.0] {
        out.push(geom(lh, 0, &[]));
        out.push(geom(lh, 1, &[]));
        out.push(geom(lh, 40, &[]));
        out.push(geom(lh, 80, &[(1, 10, 5)]));
        out.push(geom(lh, 80, &[(1, 0, 4)]));
        out.push(geom(lh, 80, &[(1, 0, 4), (2, 0, 2)]));
        out.push(geom(lh, 80, &[(1, 10, 1), (2, 20, 8), (3, 50, 3)]));
        out.push(geom(
            lh,
            200,
            &[(9, 3, 2), (1, 40, 7), (5, 12, 1), (2, 40, 2), (8, 0, 3)],
        ));
        let mut many = RowGeometry::new(lh);
        many.set_line_count(400);
        for i in 0..80u128 {
            many.upsert(block(i + 100, (i as i32) * 3, (i % 7) as u16 + 1));
        }
        out.push(many);
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
            match g.hit(y) {
                HitTarget::Cell { line: got } => {
                    assert_eq!(
                        got,
                        line,
                        "lh={} line={line} y={y} expected Cell",
                        g.line_height()
                    );
                }
                HitTarget::Block { id, local_y } => {
                    let b = g.get(id).expect("hit a Block that is not stored");
                    assert_eq!(
                        b.anchor.line,
                        line,
                        "lh={} line={line} y={y} Block at {}",
                        g.line_height(),
                        b.anchor.line
                    );
                    assert_eq!(
                        local_y, 0.0,
                        "y_for is the top of the line, so local_y must be 0"
                    );
                }
            }
        }
    }
}

#[test]
fn hit_of_interior_y_identifies_the_same_line() {
    for g in sample_geometries() {
        if !g.line_height().is_finite() || g.line_height() <= 0.0 {
            continue;
        }
        let hi = g.line_count().max(8);
        for line in 0..hi {
            let top = g.y_for(line);
            let h = g.height_of(line);
            if h <= 0.0 {
                continue;
            }
            for &frac in &[0.0, 0.25, 0.5, 0.75] {
                let y = top + h * frac;
                // The last pixel of the span belongs to this line; `h` itself
                // is the next line, so stay strictly inside.
                let y = if frac == 0.0 {
                    y
                } else {
                    y.min(top + h - h * 0.01)
                };
                match g.hit(y) {
                    HitTarget::Cell { line: got } => assert_eq!(got, line),
                    HitTarget::Block { id, .. } => {
                        assert_eq!(g.get(id).unwrap().anchor.line, line);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Alt-screen: bit-identical to the linear mapping
// ---------------------------------------------------------------------------

#[test]
fn alt_screen_is_bit_identical_to_linear_mapping() {
    let mut g = geom(17.0, 100, &[(1, 10, 20), (2, 0, 8), (3, 50, 4)]);
    g.set_alt_screen(true);
    let lh = 17.0;
    for n in -8..120 {
        let linear = n as Px * lh;
        assert_eq!(g.y_for(n), linear, "y_for({n})");
        assert_eq!(g.height_of(n), lh);
        assert_eq!(g.hit(linear), cell(n), "hit(y_for({n}))");
    }
    // Interior y: still a Cell, never a Block.
    assert_eq!(g.hit(10.0 * lh + 3.0), cell(10));
    assert_eq!(g.total_height(), 100.0 * lh);

    g.set_alt_screen(false);
    assert!(matches!(g.hit(g.y_for(10)), HitTarget::Block { .. }));
}

#[test]
fn alt_screen_with_no_blocks_matches_empty_geometry() {
    let lh = 18.5;
    let empty = geom(lh, 50, &[]);
    let mut hidden = geom(lh, 50, &[(1, 4, 9)]);
    hidden.set_alt_screen(true);
    for n in -2..60 {
        assert_eq!(hidden.y_for(n), empty.y_for(n));
        assert_eq!(hidden.hit(hidden.y_for(n)), empty.hit(empty.y_for(n)));
    }
}

// ---------------------------------------------------------------------------
// ViewportPosition: sub-row normalisation
// ---------------------------------------------------------------------------

#[test]
fn sub_stays_in_half_open_range_for_positive_delta() {
    let g = geom(16.0, 40, &[]);
    let mut pos = ViewportPosition::new(5);
    let spilled = pos.apply_pixel_delta(40.0, &g);
    assert_eq!(spilled, 2);
    assert_eq!(pos.row, 7);
    assert_eq!(pos.sub, 8.0);
    assert_sub_in_range(pos, &g);
}

#[test]
fn sub_stays_in_half_open_range_for_negative_delta() {
    let g = geom(16.0, 40, &[]);
    let mut pos = ViewportPosition { row: 8, sub: 4.0 };
    let spilled = pos.apply_pixel_delta(-20.0, &g);
    assert_eq!(spilled, -1);
    assert_eq!(pos.row, 7);
    assert_eq!(pos.sub, 0.0);
    assert_sub_in_range(pos, &g);
}

#[test]
fn sub_normalises_deltas_larger_than_several_rows() {
    let g = geom(10.0, 80, &[]);
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
fn sub_walks_a_tall_block_without_spilling_until_it_is_consumed() {
    let lh = 16.0;
    let g = geom(lh, 40, &[(1, 10, 5)]);
    let mut pos = ViewportPosition::new(10);
    let spilled = pos.apply_pixel_delta(3.0 * lh, &g);
    assert_eq!(spilled, 0, "still inside the block");
    assert_eq!(pos.row, 10);
    assert_eq!(pos.sub, 3.0 * lh);
    assert_sub_in_range(pos, &g);

    let spilled = pos.apply_pixel_delta(2.0 * lh + 4.0, &g);
    assert_eq!(spilled, 1, "exited the block into the next cell");
    assert_eq!(pos.row, 11);
    assert_eq!(pos.sub, 4.0);
    assert_sub_in_range(pos, &g);
}

#[test]
fn negative_delta_into_a_tall_block_lands_inside_it() {
    let lh = 16.0;
    let g = geom(lh, 40, &[(1, 10, 5)]);
    let mut pos = ViewportPosition::new(11);
    let spilled = pos.apply_pixel_delta(-2.0 * lh, &g);
    assert_eq!(spilled, -1);
    assert_eq!(pos.row, 10);
    assert_eq!(pos.sub, 3.0 * lh);
    assert_sub_in_range(pos, &g);
}

#[test]
fn apply_pixel_delta_clamps_before_line_zero() {
    let g = geom(16.0, 20, &[]);
    let mut pos = ViewportPosition { row: 2, sub: 4.0 };
    let spilled = pos.apply_pixel_delta(-1000.0, &g);
    assert_eq!(spilled, -2);
    assert_eq!(pos.row, 0);
    assert_eq!(pos.sub, 0.0);
}

#[test]
fn apply_pixel_delta_zero_and_non_finite_are_noops() {
    let g = geom(16.0, 20, &[]);
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
fn scroll_to_anchor_sets_sub_zero_on_a_block() {
    let g = geom(16.0, 40, &[(1, 12, 6)]);
    let mut pos = ViewportPosition { row: 20, sub: 11.0 };
    pos.jump_to_anchor(12);
    assert_eq!(pos.row, 12);
    assert_eq!(pos.sub, 0.0);
    assert_eq!(
        g.hit(g.y_for(pos.row as i32) + pos.sub).block_id(),
        Some(bid(1))
    );
}

#[test]
fn scroll_to_anchor_clamps_negative_lines() {
    let pos = ViewportPosition::scroll_to_anchor(-4);
    assert_eq!(pos.row, 0);
    assert_eq!(pos.sub, 0.0);
}

trait BlockIdOf {
    fn block_id(self) -> Option<BlockId>;
}

impl BlockIdOf for HitTarget {
    fn block_id(self) -> Option<BlockId> {
        match self {
            HitTarget::Block { id, .. } => Some(id),
            HitTarget::Cell { .. } => None,
        }
    }
}

// ---------------------------------------------------------------------------
// History-shrink rebase: keep, shift, drop
// ---------------------------------------------------------------------------

#[test]
fn history_shrink_keeps_shifts_and_drops_anchors() {
    let mut g = geom(16.0, 40, &[(1, 2, 1), (2, 5, 2), (3, 12, 3)]);
    g.rebase_after_history_shrink(5);
    assert!(
        g.get(bid(1)).is_none(),
        "line 2 fell inside the removed region"
    );
    let kept = g.get(bid(2)).expect("line 5 survives at 0");
    assert_eq!(kept.anchor.line, 0);
    let shifted = g.get(bid(3)).expect("line 12 shifts to 7");
    assert_eq!(shifted.anchor.line, 7);
    assert_eq!(g.y_for(0), 0.0);
    assert!(matches!(g.hit(0.0), HitTarget::Block { id, .. } if id == bid(2)));
}

#[test]
fn history_shrink_zero_or_negative_is_noop() {
    let mut g = geom(16.0, 20, &[(1, 3, 2)]);
    g.rebase_after_history_shrink(0);
    g.rebase_after_history_shrink(-4);
    assert_eq!(g.get(bid(1)).unwrap().anchor.line, 3);
}

#[test]
fn scrollback_eviction_drops_the_block_with_its_anchor() {
    let mut g = geom(16.0, 40, &[(1, 2, 1), (2, 8, 2)]);
    g.evict_before(5);
    assert!(g.get(bid(1)).is_none());
    assert_eq!(g.get(bid(2)).unwrap().anchor.line, 8);
}

// ---------------------------------------------------------------------------
// Frozen mode pins heights
// ---------------------------------------------------------------------------

#[test]
fn frozen_mode_returns_stable_heights_while_pinned() {
    let mut g = geom(16.0, 30, &[(1, 6, 3)]);
    let before = g.y_for(7);
    assert_eq!(g.height_of(6), 3.0 * 16.0);

    g.set_frozen(true);
    g.upsert(block(1, 6, 12));
    assert_eq!(g.get(bid(1)).unwrap().height, 3, "height pinned");
    assert_eq!(g.y_for(7), before, "mapping unchanged");
    assert_eq!(g.height_of(6), 3.0 * 16.0);

    g.set_frozen(false);
    g.upsert(block(1, 6, 12));
    assert_eq!(g.get(bid(1)).unwrap().height, 12);
    assert_eq!(g.height_of(6), 12.0 * 16.0);
    assert_ne!(g.y_for(7), before);
}

#[test]
fn frozen_mode_still_rebases_anchors() {
    let mut g = geom(16.0, 30, &[(1, 8, 4)]);
    g.set_frozen(true);
    g.rebase_after_history_shrink(3);
    let b = g.get(bid(1)).unwrap();
    assert_eq!(b.anchor.line, 5);
    assert_eq!(b.height, 4);
}

#[test]
fn frozen_mode_accepts_a_new_block_at_its_first_height() {
    let mut g = geom(16.0, 30, &[]);
    g.set_frozen(true);
    g.upsert(block(1, 2, 5));
    assert_eq!(g.get(bid(1)).unwrap().height, 5);
}

// ---------------------------------------------------------------------------
// Binary search with many blocks
// ---------------------------------------------------------------------------

#[test]
fn binary_search_agrees_with_linear_scan_on_many_blocks() {
    let lh = 13.0;
    let mut g = RowGeometry::new(lh);
    g.set_line_count(2000);
    for i in 0..400u128 {
        g.upsert(block(i + 1, (i as i32) * 4 + 1, (i % 11) as u16 + 1));
    }
    for line in 0..500 {
        let y = g.y_for(line);
        let hit = g.hit(y);
        let expected = naive_line_at(&g, line);
        match (hit, expected) {
            (HitTarget::Cell { line: got }, None) => assert_eq!(got, line),
            (HitTarget::Block { id, local_y }, Some(want)) => {
                assert_eq!(id, want);
                assert_eq!(local_y, 0.0);
            }
            other => panic!("line {line}: {other:?}"),
        }
        // Interior of this row, if it is a block, must still resolve via
        // binary search to a Block at this line.
        let mid = y + g.height_of(line) * 0.5;
        match g.hit(mid) {
            HitTarget::Cell { line: got } => assert_eq!(got, line),
            HitTarget::Block { id, .. } => {
                assert_eq!(g.get(id).unwrap().anchor.line, line);
            }
        }
    }
}

fn naive_line_at(g: &RowGeometry, line: i32) -> Option<BlockId> {
    g.blocks()
        .find(|b| b.anchor.line == line && b.height > 0)
        .map(|b| b.id)
}

// ---------------------------------------------------------------------------
// Degenerate inputs: never panic
// ---------------------------------------------------------------------------

#[test]
fn empty_geometry_is_linear_and_zero_extent() {
    let g = RowGeometry::new(16.0);
    assert_eq!(g.total_height(), 0.0);
    assert_eq!(g.y_for(0), 0.0);
    assert_eq!(g.hit(0.0), cell(0));
    assert_eq!(g.hit(32.0), cell(2));
}

#[test]
fn zero_and_non_finite_line_height_do_not_panic() {
    for h in [0.0, -4.0, Px::NAN, Px::INFINITY, Px::NEG_INFINITY] {
        let mut g = RowGeometry::new(h);
        g.set_line_count(10);
        g.upsert(block(1, 0, 8));
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
fn block_at_line_zero_is_hittable() {
    let g = geom(16.0, 10, &[(1, 0, 3)]);
    assert_eq!(g.y_for(0), 0.0);
    assert_eq!(
        g.hit(0.0),
        HitTarget::Block {
            id: bid(1),
            local_y: 0.0
        }
    );
    assert_eq!(g.y_for(1), 3.0 * 16.0);
}

#[test]
fn unsorted_and_overlapping_input_is_normalised() {
    let mut g = RowGeometry::new(16.0);
    g.set_line_count(30);
    g.upsert(block(3, 20, 2));
    g.upsert(block(1, 5, 4));
    g.upsert(block(2, 5, 1));
    g.upsert(block(4, 5, 0));
    let ids: Vec<_> = g.blocks().map(|b| b.id).collect();
    assert_eq!(ids, vec![bid(1), bid(2), bid(4), bid(3)]);
    assert_eq!(g.height_of(5), 5.0 * 16.0);
}

#[test]
fn anchors_beyond_scrollback_do_not_panic() {
    let mut g = geom(16.0, 10, &[(1, 10_000, 4), (2, -20, 2)]);
    let _ = g.y_for(10_000);
    let _ = g.hit(g.y_for(10_000));
    let _ = g.total_height();
    g.evict_before(0);
    assert!(g.get(bid(2)).is_none());
    assert!(g.get(bid(1)).is_some());
}

#[test]
fn absurd_heights_and_extreme_lines_do_not_panic() {
    let mut g = RowGeometry::new(16.0);
    g.set_line_count(i32::MAX);
    g.upsert(block(1, i32::MAX, u16::MAX));
    g.upsert(block(2, i32::MIN, u16::MAX));
    g.upsert(block(3, 0, 0));
    let _ = g.y_for(0);
    let _ = g.y_for(i32::MAX);
    let _ = g.y_for(i32::MIN);
    let _ = g.hit(0.0);
    let _ = g.hit(Px::MAX);
    let _ = g.total_height();
    g.rebase_after_history_shrink(i32::MAX);
    g.clear_blocks();
    assert_eq!(g.blocks().count(), 0);
}

#[test]
fn zero_height_block_does_not_steal_the_cell() {
    let g = geom(16.0, 10, &[(1, 4, 0)]);
    assert_eq!(g.height_of(4), 16.0);
    assert_eq!(g.hit(g.y_for(4)), cell(4));
    assert_eq!(g.get(bid(1)).unwrap().height, 0);
}

#[test]
fn remove_and_get_missing_ids_are_none() {
    let mut g = geom(16.0, 10, &[(1, 2, 2)]);
    assert!(g.remove(bid(99)).is_none());
    assert!(g.get(bid(99)).is_none());
    assert!(g.remove(bid(1)).is_some());
    assert!(g.get(bid(1)).is_none());
    assert_eq!(g.y_for(3) - g.y_for(2), 16.0);
}

#[test]
fn column_is_preserved_and_ignored_by_the_mapping() {
    let mut g = RowGeometry::new(16.0);
    g.upsert(Block {
        id: bid(1),
        run_id: rid(1),
        anchor: Anchor {
            line: 4,
            column: 17,
        },
        height: 2,
    });
    assert_eq!(g.get(bid(1)).unwrap().anchor.column, 17);
    assert_eq!(g.y_for(4), 4.0 * 16.0);
}
