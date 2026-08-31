//! Ergonomic constructors for the closed ADR-0017 widget set.
//!
//! A plugin author writes Rust, not raw enum literals and not JSON. Colour is
//! [`Tone`] only — no hex, no RGB — so a theme switch cannot break a tree
//! (ADR-0017 constraint 1, ADR-0002).

use plugin_protocol::v2::{Tone, Widget};

/// Start a column. Convert with [`Into::into`] or pass to
/// [`crate::v2::Context::render`].
pub fn col() -> Col {
    Col {
        gap: 0,
        children: Vec::new(),
    }
}

/// Start a row.
pub fn row() -> Row {
    Row {
        gap: 0,
        children: Vec::new(),
    }
}

/// Body text. Chain [`Text::tone`] / [`Text::bold`].
pub fn text(s: impl Into<String>) -> Text {
    Text {
        s: s.into(),
        fg: Tone::Fg,
        bold: false,
    }
}

/// The only interactive node (ADR-0017 constraint 3).
pub fn btn(s: impl Into<String>, action: impl Into<String>) -> Btn {
    Btn {
        s: s.into(),
        action: action.into(),
        arg: None,
    }
}

pub fn badge(s: impl Into<String>, tone: Tone) -> Widget {
    Widget::Badge { s: s.into(), tone }
}

pub fn code(s: impl Into<String>) -> Widget {
    Widget::Code {
        lang: None,
        s: s.into(),
    }
}

pub fn code_lang(lang: impl Into<String>, s: impl Into<String>) -> Widget {
    Widget::Code {
        lang: Some(lang.into()),
        s: s.into(),
    }
}

pub fn bar(v: f32) -> Widget {
    Widget::Bar { v }
}

pub fn spark(vs: impl Into<Vec<f32>>) -> Widget {
    Widget::Spark { vs: vs.into() }
}

pub fn sep() -> Widget {
    Widget::Sep
}

#[derive(Clone, Debug)]
pub struct Col {
    gap: u16,
    children: Vec<Widget>,
}

impl Col {
    pub fn gap(mut self, gap: u16) -> Self {
        self.gap = gap;
        self
    }

    pub fn child(mut self, child: impl Into<Widget>) -> Self {
        self.children.push(child.into());
        self
    }
}

impl From<Col> for Widget {
    fn from(value: Col) -> Self {
        Widget::Col {
            gap: value.gap,
            children: value.children,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Row {
    gap: u16,
    children: Vec<Widget>,
}

impl Row {
    pub fn gap(mut self, gap: u16) -> Self {
        self.gap = gap;
        self
    }

    pub fn child(mut self, child: impl Into<Widget>) -> Self {
        self.children.push(child.into());
        self
    }
}

impl From<Row> for Widget {
    fn from(value: Row) -> Self {
        Widget::Row {
            gap: value.gap,
            children: value.children,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Text {
    s: String,
    fg: Tone,
    bold: bool,
}

impl Text {
    pub fn tone(mut self, fg: Tone) -> Self {
        self.fg = fg;
        self
    }

    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }
}

impl From<Text> for Widget {
    fn from(value: Text) -> Self {
        Widget::Text {
            s: value.s,
            fg: value.fg,
            bold: value.bold,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Btn {
    s: String,
    action: String,
    arg: Option<String>,
}

impl Btn {
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.arg = Some(arg.into());
        self
    }
}

impl From<Btn> for Widget {
    fn from(value: Btn) -> Self {
        Widget::Btn {
            s: value.s,
            action: value.action,
            arg: value.arg,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_builders_produce_the_closed_widget_set() {
        let tree: Widget = col()
            .gap(1)
            .child(text("hello").tone(Tone::Err).bold())
            .child(
                row()
                    .child(badge("1", Tone::Warn))
                    .child(btn("Retry", "retry").arg("run-1")),
            )
            .child(sep())
            .into();
        let Widget::Col { gap, children } = tree else {
            panic!("expected col");
        };
        assert_eq!(gap, 1);
        assert_eq!(children.len(), 3);
        assert_eq!(
            children[0],
            Widget::Text {
                s: "hello".into(),
                fg: Tone::Err,
                bold: true,
            }
        );
        assert!(matches!(children[2], Widget::Sep));
    }
}
