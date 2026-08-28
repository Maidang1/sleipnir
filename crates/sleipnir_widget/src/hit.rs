//! Hit-testing: cell → button action (ADR-0017 constraint 3).
//!
//! Only `btn` is interactive. No inputs, no drag, no focus — text entry is
//! what the command palette is for. The attribution band is never a hit, even
//! if a crafted tree tries to occupy the same cells (it cannot: layout
//! reserves that band after the tree).

use crate::geom::CellPos;
use crate::layout::{LaidOut, LaidOutKind, Layout};

/// Result of mapping a cell to an interactive node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hit<'a> {
    Miss,
    /// A `btn` occupies this cell. `action` / `arg` are the opaque strings
    /// the host forwards as `Action`.
    Btn {
        action: &'a str,
        arg: Option<&'a str>,
    },
}

/// Map `pos` to a hit. Later siblings and descendants win, so a button inside
/// a row inside a col is found rather than the container.
pub fn hit_test<'a>(layout: &'a Layout, pos: CellPos) -> Hit<'a> {
    // Attribution is renderer-owned and inert. Checked first so a programming
    // error that overlapped content into the band still cannot spoof a click.
    if layout.attribution.rect.contains(pos) {
        return Hit::Miss;
    }
    hit_node(&layout.root, pos)
}

fn hit_node<'a>(node: &'a LaidOut, pos: CellPos) -> Hit<'a> {
    if !node.rect.contains(pos) {
        return Hit::Miss;
    }
    for child in node.children.iter().rev() {
        match hit_node(child, pos) {
            Hit::Miss => {}
            hit => return hit,
        }
    }
    match &node.kind {
        LaidOutKind::Btn { action, arg, .. } => Hit::Btn {
            action,
            arg: arg.as_deref(),
        },
        _ => Hit::Miss,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::layout;
    use plugin_protocol::v2::{Tone, Widget};

    fn btn(s: &str, action: &str, arg: Option<&str>) -> Widget {
        Widget::Btn {
            s: s.into(),
            action: action.into(),
            arg: arg.map(str::to_string),
        }
    }

    #[test]
    fn nested_row_and_col_buttons() {
        // Col
        //   Row [ A ][ B ]
        //   [ C ]
        let tree = Widget::Col {
            gap: 1,
            children: vec![
                Widget::Row {
                    gap: 1,
                    children: vec![btn("A", "a", None), btn("B", "b", None)],
                },
                btn("C", "c", Some("x")),
            ],
        };
        let laid = layout(&tree, 20, "p");
        // "A" + pad 2 = 3 cols at (0,0); gap 1; "B" at col 4.
        assert_eq!(
            hit_test(&laid, CellPos::new(0, 0)),
            Hit::Btn {
                action: "a",
                arg: None
            }
        );
        assert_eq!(
            hit_test(&laid, CellPos::new(2, 0)),
            Hit::Btn {
                action: "a",
                arg: None
            }
        );
        assert_eq!(
            hit_test(&laid, CellPos::new(3, 0)),
            Hit::Miss,
            "gap between A and B"
        );
        assert_eq!(
            hit_test(&laid, CellPos::new(4, 0)),
            Hit::Btn {
                action: "b",
                arg: None
            }
        );
        assert_eq!(hit_test(&laid, CellPos::new(0, 1)), Hit::Miss, "col gap");
        assert_eq!(
            hit_test(&laid, CellPos::new(0, 2)),
            Hit::Btn {
                action: "c",
                arg: Some("x")
            }
        );
    }

    #[test]
    fn attribution_is_never_a_hit() {
        let tree = btn("X", "go", None);
        let laid = layout(&tree, 10, "p");
        let pos = CellPos::new(0, laid.attribution.rect.row);
        assert!(laid.attribution.rect.contains(pos));
        assert_eq!(hit_test(&laid, pos), Hit::Miss);
    }

    #[test]
    fn miss_outside_and_on_non_btn() {
        let tree = Widget::Text {
            s: "hello".into(),
            fg: Tone::Fg,
            bold: false,
        };
        let laid = layout(&tree, 10, "p");
        assert_eq!(hit_test(&laid, CellPos::new(0, 0)), Hit::Miss);
        assert_eq!(hit_test(&laid, CellPos::new(99, 99)), Hit::Miss);
    }
}
