use std::ops::Range;

#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    // --- Legacy keywords ---
    Fn,
    Let,
    Mut,
    If,
    Elif,
    Else,
    For,
    While,
    In,
    Return,
    I64,
    F64,
    Bool,
    Str,
    Unit,
    True,
    False,
    Println,
    Struct,
    Enum,
    Mod,
    Use,
    Pub,
    As,

    // --- 0.5 native syntax additions ---
    /// `module name` — full-word module declaration alias for `mod`.
    Module,
    /// `give expr` — synonym for `return`.
    Give,
    /// `say expr` — synonym for `println`.
    Say,
    /// `raise expr` — error raise (lowered to abort in 0.5).
    Raise,
    /// `text` — type alias for `str`.
    Text,
    /// `number` — type alias for `i64`.
    Number,

    // --- Literals ---
    Ident(String),
    Integer(i64),
    Float(f64),
    StrLit(String),

    // --- Operators ---
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Assign,
    PlusAssign,
    MinusAssign,
    StarAssign,
    SlashAssign,
    EqEq,
    NotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    And,
    Or,
    Bang,
    DotDot,
    DotDotEllipsis,
    Dot,
    DoubleColon,
    /// `|>` pipeline operator.
    Pipe,

    // --- Punctuation ---
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Colon,
    RArrow,

    // --- Synthetic indentation tokens (emitted by the indent pre-pass) ---
    /// One indent level deeper than the previous line's indent.
    Indent,
    /// One indent level shallower than the previous line's indent.
    Dedent,
    /// Logical newline (a non-blank line boundary).
    Newline,

    // --- Meta ---
    Error,
    Eof,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Range<usize>,
}
