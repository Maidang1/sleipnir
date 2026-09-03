//! Crate-level layout tests. Helpers keep widget construction short; assertions
//! name the ADR constraint they guard.

use super::*;
use plugin_protocol::v2::Tone;

fn text(s: &str) -> Widget {
    Widget::Text {
        s: s.into(),
        fg: Tone::Fg,
        bold: false,
    }
}

fn btn(s: &str, action: &str) -> Widget {
    Widget::Btn {
        s: s.into(),
        action: action.into(),
        arg: None,
    }
}

fn col(gap: u16, children: Vec<Widget>) -> Widget {
    Widget::Col { gap, children }
}

fn row(gap: u16, children: Vec<Widget>) -> Widget {
    Widget::Row { gap, children }
}

fn lay(tree: &Widget) -> Layout {
    layout(tree, 20, "demo")
}

fn content(layout: &Layout) -> Vec<&LaidOut> {
    layout.walk().collect()
}

fn kinds(layout: &Layout) -> Vec<&LaidOutKind> {
    content(layout).into_iter().map(|n| &n.kind).collect()
}

fn first_kind<F>(layout: &Layout, pred: F) -> &LaidOut
where
    F: Fn(&LaidOutKind) -> bool,
{
    content(layout)
        .into_iter()
        .find(|n| pred(&n.kind))
        .expect("missing kind")
}

// ---------------------------------------------------------------------------
// Exact row counts per widget kind (content, excluding attribution).
// ---------------------------------------------------------------------------

#[test]
fn text_row_count_is_wrapped_lines() {
    assert_eq!(lay(&text("hi")).content_height(), 1);
    assert_eq!(layout(&text("hello"), 2, "p").content_height(), 3); // he / ll / o
    assert_eq!(lay(&text("")).content_height(), 1);
    assert_eq!(lay(&text("a\nb")).content_height(), 2);
}

#[test]
fn code_row_count_is_line_count_not_wrap() {
    let short = Widget::Code {
        lang: None,
        s: "fn\nmain".into(),
    };
    assert_eq!(lay(&short).content_height(), 2);

    let long = Widget::Code {
        lang: None,
        s: "abcdefghijklmnopqrstuvwxyz".into(),
    };
    let laid = layout(&long, 8, "p");
    assert_eq!(
        laid.content_height(),
        1,
        "code must not wrap; a long line is one truncated row"
    );
    let LaidOutKind::Code { lines } =
        &first_kind(&laid, |k| matches!(k, LaidOutKind::Code { .. })).kind
    else {
        panic!("expected code");
    };
    assert_eq!(lines.len(), 1);
    assert!(lines[0].truncated);
    assert_eq!(cell_cols(&lines[0].text), 8);
    assert!(lines[0].text.ends_with(ELLIPSIS));
}

#[test]
fn fixed_leaves_are_one_row() {
    assert_eq!(
        lay(&Widget::Badge {
            s: "3000".into(),
            tone: Tone::Ok,
        })
        .content_height(),
        1
    );
    assert_eq!(lay(&Widget::Bar { v: 0.6 }).content_height(), 1);
    assert_eq!(
        lay(&Widget::Spark {
            vs: vec![1.0, 2.0, 3.0],
        })
        .content_height(),
        1
    );
    assert_eq!(lay(&Widget::Sep).content_height(), 1);
    assert_eq!(lay(&btn("Retry", "retry")).content_height(), 1);
    assert_eq!(lay(&Widget::Unknown).content_height(), 1);
}

#[test]
fn col_and_row_row_counts() {
    assert_eq!(lay(&col(0, vec![text("a"), text("b")])).content_height(), 2);
    assert_eq!(lay(&col(1, vec![text("a"), text("b")])).content_height(), 3);
    assert_eq!(
        lay(&row(1, vec![text("a"), text("b")])).content_height(),
        1,
        "side-by-side texts share a row"
    );
    let mixed = row(
        0,
        vec![
            Widget::Code {
                lang: None,
                s: "a\nb".into(),
            },
            Widget::Badge {
                s: "ok".into(),
                tone: Tone::Ok,
            },
        ],
    );
    assert_eq!(lay(&mixed).content_height(), 2);
}

#[test]
fn empty_col_is_zero_content_plus_attribution() {
    let laid = lay(&col(0, vec![]));
    assert_eq!(laid.content_height(), 0);
    assert_eq!(laid.height, ATTRIBUTION_ROWS);
    assert_eq!(laid.attribution.rect.height, ATTRIBUTION_ROWS);
}

#[test]
fn total_height_is_content_plus_attribution() {
    let laid = lay(&text("hi"));
    assert_eq!(laid.height, laid.content_height() + ATTRIBUTION_ROWS);
}

// ---------------------------------------------------------------------------
// Text wrap boundaries
// ---------------------------------------------------------------------------

#[test]
fn text_wraps_exactly_at_available_width() {
    let laid = layout(&text("abcdef"), 4, "p");
    let LaidOutKind::Text { lines, .. } =
        &first_kind(&laid, |k| matches!(k, LaidOutKind::Text { .. })).kind
    else {
        panic!("expected text");
    };
    assert_eq!(lines, &["abcd".to_string(), "ef".to_string()]);
    assert_eq!(laid.content_height(), 2);

    let exact = layout(&text("abcd"), 4, "p");
    let LaidOutKind::Text { lines, .. } =
        &first_kind(&exact, |k| matches!(k, LaidOutKind::Text { .. })).kind
    else {
        panic!("expected text");
    };
    assert_eq!(lines, &["abcd".to_string()]);
    assert_eq!(exact.content_height(), 1);
}

// ---------------------------------------------------------------------------
// Budget truncation
// ---------------------------------------------------------------------------

#[test]
fn over_depth_tree_is_truncated_with_visible_marker() {
    let mut tree = btn("x", "deep");
    for _ in 0..MAX_WIDGET_DEPTH + 5 {
        tree = col(0, vec![tree]);
    }
    assert!(!measure(&tree).within_budget());
    let laid = layout(&tree, 20, "p");
    assert!(laid.truncated);
    assert!(
        kinds(&laid)
            .iter()
            .any(|k| matches!(k, LaidOutKind::Truncated)),
        "budget cut must leave a visible marker, not a silent drop"
    );
    assert!(
        !kinds(&laid)
            .iter()
            .any(|k| matches!(k, LaidOutKind::Btn { .. })),
        "the buried button is past MAX_WIDGET_DEPTH and must not render"
    );
    assert_eq!(hit_test(&laid, CellPos::new(0, 0)), Hit::Miss);
}

#[test]
fn over_node_tree_is_truncated_with_visible_marker() {
    let tree = col(0, (0..MAX_WIDGET_NODES + 50).map(|_| Widget::Sep).collect());
    assert!(!measure(&tree).within_budget());
    let laid = layout(&tree, 20, "p");
    assert!(laid.truncated);
    let seps = kinds(&laid)
        .iter()
        .filter(|k| matches!(k, LaidOutKind::Sep))
        .count();
    assert!(seps <= MAX_WIDGET_NODES);
    assert!(
        kinds(&laid)
            .iter()
            .any(|k| matches!(k, LaidOutKind::Truncated))
    );
}

#[test]
fn within_budget_tree_is_not_truncated() {
    let tree = col(1, vec![text("a"), btn("b", "b")]);
    let laid = lay(&tree);
    assert!(laid.stats.within_budget());
    assert!(!laid.truncated);
    assert!(
        !kinds(&laid)
            .iter()
            .any(|k| matches!(k, LaidOutKind::Truncated))
    );
}

// ---------------------------------------------------------------------------
// Hit-testing (also covered in hit.rs; this checks arg + nested col/row)
// ---------------------------------------------------------------------------

#[test]
fn hit_test_nested_btn_action_and_arg() {
    let tree = col(
        0,
        vec![row(
            0,
            vec![
                Widget::Btn {
                    s: "Go".into(),
                    action: "retry".into(),
                    arg: Some("run-42".into()),
                },
                text("no"),
            ],
        )],
    );
    let laid = lay(&tree);
    match hit_test(&laid, CellPos::new(0, 0)) {
        Hit::Btn { action, arg } => {
            assert_eq!(action, "retry");
            assert_eq!(arg, Some("run-42"));
        }
        Hit::Miss => panic!("expected btn hit"),
    }
}

// ---------------------------------------------------------------------------
// Attribution: reserved, renderer-owned, unspoofable
// ---------------------------------------------------------------------------

#[test]
fn attribution_band_is_reserved_and_unspoofable() {
    let tree = col(
        0,
        vec![
            text("plugin:evil"),
            Widget::Btn {
                s: "steal".into(),
                action: "steal".into(),
                arg: None,
            },
        ],
    );
    let laid = layout(&tree, 20, "honest");

    let LaidOutKind::Attribution { plugin_id, label } = &laid.attribution.kind else {
        panic!("attribution must be a renderer-owned node, not a widget");
    };
    assert_eq!(plugin_id, "honest");
    assert!(label.contains("honest"));
    assert!(!label.contains("evil"));

    assert_eq!(laid.attribution.rect.row, laid.content_height());
    assert_eq!(laid.attribution.rect.height, ATTRIBUTION_ROWS);
    assert_eq!(laid.attribution.rect.width, laid.width);
    assert_eq!(laid.attribution.rect.col, 0);

    for node in laid.walk() {
        assert!(
            !node.rect.intersects(laid.attribution.rect),
            "tree content {node:?} occupied the attribution band"
        );
        assert!(
            !matches!(node.kind, LaidOutKind::Attribution { .. }),
            "the plugin tree must not be able to emit Attribution"
        );
    }

    let attr_pos = CellPos::new(0, laid.attribution.rect.row);
    assert_eq!(hit_test(&laid, attr_pos), Hit::Miss);
}

#[test]
fn attribution_survives_a_tree_that_looks_like_the_marker() {
    let tree = text("plugin:honest");
    let laid = layout(&tree, 20, "honest");
    let attr_hits: Vec<_> = std::iter::once(&laid.attribution)
        .chain(laid.walk())
        .filter(|n| matches!(n.kind, LaidOutKind::Attribution { .. }))
        .collect();
    assert_eq!(
        attr_hits.len(),
        1,
        "only the renderer band is Attribution, even if the tree copies the label"
    );
    assert!(std::ptr::eq(attr_hits[0], &laid.attribution));
}

#[test]
fn empty_plugin_id_still_gets_a_marker() {
    let laid = layout(&text("x"), 10, "");
    let LaidOutKind::Attribution { plugin_id, label } = &laid.attribution.kind else {
        panic!("expected attribution");
    };
    assert!(!plugin_id.is_empty());
    assert!(!label.is_empty());
    assert_eq!(laid.attribution.rect.height, ATTRIBUTION_ROWS);
}

// ---------------------------------------------------------------------------
// Unknown is visible and inert
// ---------------------------------------------------------------------------

#[test]
fn unknown_renders_visibly_and_is_not_a_hit() {
    let laid = lay(&Widget::Unknown);
    let node = first_kind(&laid, |k| matches!(k, LaidOutKind::Unknown));
    assert!(
        node.rect.width >= 1 && node.rect.height >= 1,
        "Unknown must never be zero-size"
    );
    assert_eq!(hit_test(&laid, node.rect.pos()), Hit::Miss);
}

#[test]
fn unknown_inside_a_tree_does_not_drop_siblings() {
    let tree = col(0, vec![text("before"), Widget::Unknown, text("after")]);
    let laid = lay(&tree);
    let ks = kinds(&laid);
    assert!(ks.iter().any(|k| matches!(k, LaidOutKind::Unknown)));
    let texts: Vec<_> = ks
        .iter()
        .filter_map(|k| match k {
            LaidOutKind::Text { lines, .. } => Some(lines.as_slice()),
            _ => None,
        })
        .collect();
    assert_eq!(texts.len(), 2);
}

// ---------------------------------------------------------------------------
// Degenerate inputs must not panic
// ---------------------------------------------------------------------------

#[test]
fn degenerate_inputs_do_not_panic() {
    let huge: String = std::iter::repeat_n('a', 100_000).collect();
    let deep = {
        let mut t = Widget::Sep;
        for _ in 0..64 {
            t = col(0, vec![t]);
        }
        t
    };
    let wide = col(u16::MAX, vec![text("a"), text("b")]);
    let trees = [
        text(""),
        text(&huge),
        Widget::Code {
            lang: None,
            s: String::new(),
        },
        Widget::Code {
            lang: Some("rust".into()),
            s: huge.clone(),
        },
        Widget::Code {
            lang: Some("nope".into()),
            s: "x".into(),
        },
        Widget::Badge {
            s: String::new(),
            tone: Tone::Err,
        },
        Widget::Bar { v: f32::NAN },
        Widget::Bar { v: f32::INFINITY },
        Widget::Bar {
            v: f32::NEG_INFINITY,
        },
        Widget::Bar { v: -12.0 },
        Widget::Bar { v: 99.0 },
        Widget::Spark { vs: vec![] },
        Widget::Spark {
            vs: vec![f32::NAN, f32::INFINITY, f32::NEG_INFINITY],
        },
        Widget::Sep,
        Widget::Unknown,
        btn("", ""),
        col(0, vec![]),
        row(u16::MAX, vec![btn("a", "a"), btn("b", "b")]),
        deep,
        wide,
        col(0, (0..600).map(|_| Widget::Sep).collect()),
    ];
    for tree in &trees {
        let a = layout(tree, 0, "p");
        let b = layout(tree, 1, "");
        let c = layout(tree, 80, "p\nwith\nnewlines");
        assert!(a.width >= 1);
        assert!(b.height >= ATTRIBUTION_ROWS);
        assert_eq!(c.attribution.rect.height, ATTRIBUTION_ROWS);
    }
}

// ---------------------------------------------------------------------------
// Determinism
// ---------------------------------------------------------------------------

#[test]
fn same_tree_and_width_produce_identical_layout() {
    let tree = col(
        1,
        vec![
            text("hello world"),
            row(
                1,
                vec![
                    btn("Retry", "retry"),
                    Widget::Badge {
                        s: "3000".into(),
                        tone: Tone::Warn,
                    },
                    Widget::Sep,
                ],
            ),
            Widget::Code {
                lang: Some("rust".into()),
                s: "fn main() {\n    let x = 1;\n}".into(),
            },
            Widget::Bar { v: 0.25 },
            Widget::Spark {
                vs: vec![1.0, 2.0, 3.0, 5.0, 8.0],
            },
            Widget::Unknown,
        ],
    );
    let a = layout(&tree, 40, "plug");
    let b = layout(&tree, 40, "plug");
    assert_eq!(a, b);
}

#[test]
fn code_layout_is_deterministic() {
    let tree = Widget::Code {
        lang: Some("rust".into()),
        s: "fn main() {}".into(),
    };
    let a = layout(&tree, 40, "p");
    let b = layout(&tree, 40, "p");
    assert_eq!(a, b);
}

// ---------------------------------------------------------------------------
// Tones pass through to the mounts
// ---------------------------------------------------------------------------

#[test]
fn text_and_badge_carry_semantic_tones() {
    let tree = col(
        0,
        vec![
            Widget::Text {
                s: "a".into(),
                fg: Tone::Accent,
                bold: true,
            },
            Widget::Badge {
                s: "ok".into(),
                tone: Tone::Ok,
            },
        ],
    );
    let laid = lay(&tree);
    let mut saw_accent = false;
    let mut saw_ok = false;
    for node in laid.walk() {
        match &node.kind {
            LaidOutKind::Text { tone, bold, .. } => {
                assert_eq!(*tone, Tone::Accent);
                assert!(*bold);
                saw_accent = true;
            }
            LaidOutKind::Badge { tone, .. } => {
                assert_eq!(*tone, Tone::Ok);
                saw_ok = true;
            }
            _ => {}
        }
    }
    assert!(saw_accent && saw_ok);
}

#[test]
fn bar_and_spark_have_predictable_footprints() {
    let bar = lay(&Widget::Bar { v: 0.5 });
    let node = first_kind(&bar, |k| matches!(k, LaidOutKind::Bar { .. }));
    assert_eq!(node.rect.height, 1);
    assert_eq!(node.rect.width, BAR_COLS.min(20));
    let LaidOutKind::Bar { filled, width } = &node.kind else {
        panic!();
    };
    assert_eq!(*filled, width / 2);

    let spark = lay(&Widget::Spark {
        vs: vec![1.0, 2.0, 3.0],
    });
    let node = first_kind(&spark, |k| matches!(k, LaidOutKind::Spark { .. }));
    assert_eq!(node.rect.height, 1);
    assert_eq!(node.rect.width, 3);
}

#[test]
fn zero_available_width_is_treated_as_one_column() {
    let laid = layout(&text("ab"), 0, "p");
    assert_eq!(laid.width, 1);
    assert!(laid.content_height() >= 1);
    assert_eq!(laid.attribution.rect.width, 1);
}
