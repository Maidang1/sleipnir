//! Token categories. Colors stay with the theme; this crate only names kinds.

/// The token category a highlighter paints with.
///
/// Field-for-field with a syntax palette so classification can name a kind
/// without knowing its color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightKind {
    Comment,
    Keyword,
    String,
    StringSpecial,
    Escape,
    Number,
    Boolean,
    TypeName,
    TypeBuiltin,
    Constructor,
    Function,
    FunctionBuiltin,
    MacroName,
    Property,
    Constant,
    Variable,
    VariableSpecial,
    Parameter,
    Operator,
    Punctuation,
    Tag,
    Attribute,
    Label,
    Invalid,
}
