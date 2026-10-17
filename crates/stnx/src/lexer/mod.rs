mod indent;
mod prepare;
mod token;

pub use indent::{run as run_indent, IndentedTokens};
pub use prepare::prepare;
pub use token::{Token, TokenKind};

use crate::error::LexError;
use logos::Logos;

#[derive(Logos, Debug, PartialEq)]
#[logos(skip(r"[ \t\n\f]+|//[^\n]*", allow_greedy = true))]
pub enum LexicalToken {
    #[token("fn")]
    Fn,
    #[token("let")]
    Let,
    #[token("mut")]
    Mut,
    #[token("if")]
    If,
    #[token("elif")]
    Elif,
    #[token("else")]
    Else,
    #[token("for")]
    For,
    #[token("while")]
    While,
    #[token("in")]
    In,
    #[token("return")]
    Return,
    #[token("i64")]
    I64,
    #[token("f64")]
    F64,
    #[token("bool")]
    Bool,
    #[token("str")]
    Str,
    #[token("unit")]
    Unit,
    #[token("true")]
    True,
    #[token("false")]
    False,
    #[token("println")]
    Println,
    #[token("struct")]
    Struct,
    #[token("enum")]
    Enum,
    #[token("mod")]
    Mod,
    #[token("use")]
    Use,
    #[token("pub")]
    Pub,
    #[token("as")]
    As,

    // --- 0.5 native syntax additions ---
    #[token("module")]
    Module,
    #[token("give")]
    Give,
    #[token("say")]
    Say,
    #[token("raise")]
    Raise,
    #[token("text")]
    Text,
    #[token("number")]
    Number,
    #[token("external")]
    External,

    #[regex(r"[a-zA-Z_][a-zA-Z0-9_]*", |lex| lex.slice().to_string())]
    Ident(String),

    #[regex(r"[0-9]+", |lex| lex.slice().to_string())]
    Integer(String),
    #[regex(r"[0-9]+\.[0-9]+", |lex| lex.slice().to_string())]
    Float(String),
    #[regex(r#""([^"\\]|\\.)*""#, |lex| {
        let s = lex.slice();
        s.trim_start_matches('"').trim_end_matches('"').to_string()
    })]
    StrLit(String),

    #[token("+")]
    Plus,
    #[token("-")]
    Minus,
    #[token("*")]
    Star,
    #[token("/")]
    Slash,
    #[token("%")]
    Percent,
    #[token("=")]
    Assign,
    #[token("+=")]
    PlusAssign,
    #[token("-=")]
    MinusAssign,
    #[token("*=")]
    StarAssign,
    #[token("/=")]
    SlashAssign,
    #[token("==")]
    EqEq,
    #[token("!=")]
    NotEq,
    #[token("<")]
    Lt,
    #[token(">")]
    Gt,
    #[token("<=")]
    LtEq,
    #[token(">=")]
    GtEq,
    #[token("&&")]
    And,
    #[token("||")]
    Or,
    #[token("!")]
    Bang,
    #[token("..")]
    DotDot,
    #[token("...")]
    DotDotEllipsis,
    #[token(".")]
    Dot,
    #[token("::")]
    DoubleColon,

    #[token("|>")]
    Pipe,

    #[token("(")]
    LParen,
    #[token(")")]
    RParen,
    #[token("[")]
    LBracket,
    #[token("]")]
    RBracket,
    #[token("{")]
    LBrace,
    #[token("}")]
    RBrace,
    #[token(",")]
    Comma,
    #[token(":")]
    Colon,
    #[token("->")]
    RArrow,

    Error,
}

pub struct Lexer<'a> {
    inner: logos::Lexer<'a, LexicalToken>,
    src: &'a str,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self {
        Self {
            inner: LexicalToken::lexer(src),
            src,
        }
    }

    pub fn src(&self) -> &'a str {
        self.src
    }
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Result<Token, LexError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.inner.next() {
            Some(Ok(kind)) => {
                let span = self.inner.span();
                Some(Ok(Token {
                    kind: convert(kind),
                    span,
                }))
            }
            Some(Err(_)) => {
                let span = self.inner.span();
                let error_text = self.inner.slice().to_string();
                let offset = span.start;
                let len = span.end - span.start;
                Some(Err(LexError::new(
                    self.src,
                    offset,
                    len,
                    format!("unexpected character(s): '{}'", error_text),
                )))
            }
            None => None,
        }
    }
}

fn convert(lt: LexicalToken) -> TokenKind {
    match lt {
        LexicalToken::Fn => TokenKind::Fn,
        LexicalToken::Let => TokenKind::Let,
        LexicalToken::Mut => TokenKind::Mut,
        LexicalToken::If => TokenKind::If,
        LexicalToken::Elif => TokenKind::Elif,
        LexicalToken::Else => TokenKind::Else,
        LexicalToken::For => TokenKind::For,
        LexicalToken::While => TokenKind::While,
        LexicalToken::In => TokenKind::In,
        LexicalToken::Return => TokenKind::Return,
        LexicalToken::I64 => TokenKind::I64,
        LexicalToken::F64 => TokenKind::F64,
        LexicalToken::Bool => TokenKind::Bool,
        LexicalToken::Str => TokenKind::Str,
        LexicalToken::Unit => TokenKind::Unit,
        LexicalToken::True => TokenKind::True,
        LexicalToken::False => TokenKind::False,
        LexicalToken::Println => TokenKind::Println,
        LexicalToken::Struct => TokenKind::Struct,
        LexicalToken::Enum => TokenKind::Enum,
        LexicalToken::Mod => TokenKind::Mod,
        LexicalToken::Use => TokenKind::Use,
        LexicalToken::Pub => TokenKind::Pub,
        LexicalToken::As => TokenKind::As,
        LexicalToken::Module => TokenKind::Module,
        LexicalToken::Give => TokenKind::Give,
        LexicalToken::Say => TokenKind::Say,
        LexicalToken::Raise => TokenKind::Raise,
        LexicalToken::Text => TokenKind::Text,
        LexicalToken::Number => TokenKind::Number,
        LexicalToken::Ident(s) => TokenKind::Ident(s),
        LexicalToken::Integer(s) => match s.parse::<i64>() {
            Ok(n) => TokenKind::Integer(n),
            Err(_) => TokenKind::Error,
        },
        LexicalToken::Float(s) => match s.parse::<f64>() {
            Ok(f) => TokenKind::Float(f),
            Err(_) => TokenKind::Error,
        },
        LexicalToken::StrLit(s) => TokenKind::StrLit(s),
        LexicalToken::Plus => TokenKind::Plus,
        LexicalToken::Minus => TokenKind::Minus,
        LexicalToken::Star => TokenKind::Star,
        LexicalToken::Slash => TokenKind::Slash,
        LexicalToken::Percent => TokenKind::Percent,
        LexicalToken::Assign => TokenKind::Assign,
        LexicalToken::PlusAssign => TokenKind::PlusAssign,
        LexicalToken::MinusAssign => TokenKind::MinusAssign,
        LexicalToken::StarAssign => TokenKind::StarAssign,
        LexicalToken::SlashAssign => TokenKind::SlashAssign,
        LexicalToken::EqEq => TokenKind::EqEq,
        LexicalToken::NotEq => TokenKind::NotEq,
        LexicalToken::Lt => TokenKind::Lt,
        LexicalToken::Gt => TokenKind::Gt,
        LexicalToken::LtEq => TokenKind::LtEq,
        LexicalToken::GtEq => TokenKind::GtEq,
        LexicalToken::And => TokenKind::And,
        LexicalToken::Or => TokenKind::Or,
        LexicalToken::Bang => TokenKind::Bang,
        LexicalToken::DotDot => TokenKind::DotDot,
        LexicalToken::DotDotEllipsis => TokenKind::DotDotEllipsis,
        LexicalToken::Dot => TokenKind::Dot,
        LexicalToken::DoubleColon => TokenKind::DoubleColon,
        LexicalToken::Pipe => TokenKind::Pipe,
        LexicalToken::LParen => TokenKind::LParen,
        LexicalToken::RParen => TokenKind::RParen,
        LexicalToken::LBrace => TokenKind::LBrace,
        LexicalToken::RBrace => TokenKind::RBrace,
        LexicalToken::LBracket => TokenKind::LBracket,
        LexicalToken::RBracket => TokenKind::RBracket,
        LexicalToken::Comma => TokenKind::Comma,
        LexicalToken::Colon => TokenKind::Colon,
        LexicalToken::RArrow => TokenKind::RArrow,
        LexicalToken::External => TokenKind::External,
        LexicalToken::Error => TokenKind::Error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: lex a source string and collect the resulting `TokenKind`s.
    fn lex_kinds(src: &str) -> Vec<TokenKind> {
        Lexer::new(src)
            .collect::<Result<Vec<Token>, _>>()
            .map(|tokens| tokens.into_iter().map(|t| t.kind).collect())
            .expect("lexing should succeed")
    }

    // --- mod / use / pub / as keyword tests (Phase 5A) ---

    #[test]
    fn test_mod_keyword() {
        let tokens = lex_kinds("mod");
        assert_eq!(tokens, vec![TokenKind::Mod]);
    }

    #[test]
    fn test_use_keyword() {
        let tokens = lex_kinds("use");
        assert_eq!(tokens, vec![TokenKind::Use]);
    }

    #[test]
    fn test_pub_keyword() {
        let tokens = lex_kinds("pub");
        assert_eq!(tokens, vec![TokenKind::Pub]);
    }

    #[test]
    fn test_as_keyword_is_reserved() {
        // `as` is reserved for future rename syntax (Phase 6). It must lex as
        // the As keyword token, not as a plain identifier.
        let tokens = lex_kinds("as");
        assert_eq!(tokens, vec![TokenKind::As]);
    }

    #[test]
    fn test_mod_decl_tokens() {
        let tokens = lex_kinds("mod io");
        assert_eq!(
            tokens,
            vec![TokenKind::Mod, TokenKind::Ident("io".to_string())]
        );
    }

    #[test]
    fn test_pub_mod_tokens() {
        let tokens = lex_kinds("pub mod io");
        assert_eq!(
            tokens,
            vec![
                TokenKind::Pub,
                TokenKind::Mod,
                TokenKind::Ident("io".to_string()),
            ]
        );
    }

    #[test]
    fn test_use_path_tokens() {
        let tokens = lex_kinds("use io::println");
        assert_eq!(
            tokens,
            vec![
                TokenKind::Use,
                TokenKind::Ident("io".to_string()),
                TokenKind::DoubleColon,
                TokenKind::Println,
            ]
        );
    }

    #[test]
    fn test_all_module_keywords_are_distinct() {
        let src = "mod use pub as";
        let tokens = lex_kinds(src);
        assert_eq!(
            tokens,
            vec![
                TokenKind::Mod,
                TokenKind::Use,
                TokenKind::Pub,
                TokenKind::As,
            ]
        );
    }

    #[test]
    fn test_mod_not_confused_with_identifier() {
        // Identifier "modern" must not be split into Mod + Ident
        let tokens = lex_kinds("modern");
        assert_eq!(tokens, vec![TokenKind::Ident("modern".to_string())]);
    }

    // --- Ecosystem import token tests ---

    // Helper: collect tokens, stripping the trailing Newline/Eof for concise assertions.
    fn lex_kinds_trim(src: &str) -> Vec<TokenKind> {
        let all = lex_kinds(src);
        all.into_iter()
            .filter(|t| !matches!(t, TokenKind::Newline | TokenKind::Eof))
            .collect()
    }

    #[test]
    fn test_lex_use_python_colon() {
        // `use python:numpy` — python is an Ident, colon is a Colon
        let tokens = lex_kinds_trim("use python:numpy");
        assert_eq!(
            tokens,
            vec![
                TokenKind::Use,
                TokenKind::Ident("python".to_string()),
                TokenKind::Colon,
                TokenKind::Ident("numpy".to_string()),
            ]
        );
    }

    #[test]
    fn test_lex_use_rust_colon() {
        let tokens = lex_kinds_trim("use rust:serde");
        assert_eq!(
            tokens,
            vec![
                TokenKind::Use,
                TokenKind::Ident("rust".to_string()),
                TokenKind::Colon,
                TokenKind::Ident("serde".to_string()),
            ]
        );
    }

    #[test]
    fn test_lex_use_native_colon() {
        let tokens = lex_kinds_trim("use native:libc");
        assert_eq!(
            tokens,
            vec![
                TokenKind::Use,
                TokenKind::Ident("native".to_string()),
                TokenKind::Colon,
                TokenKind::Ident("libc".to_string()),
            ]
        );
    }

    #[test]
    fn test_lex_use_python_with_alias() {
        let tokens = lex_kinds_trim("use python:numpy as np");
        assert_eq!(
            tokens,
            vec![
                TokenKind::Use,
                TokenKind::Ident("python".to_string()),
                TokenKind::Colon,
                TokenKind::Ident("numpy".to_string()),
                TokenKind::As,
                TokenKind::Ident("np".to_string()),
            ]
        );
    }

    #[test]
    fn test_lex_use_rust_dotted_path() {
        // `use rust:serde` is distinct from `use serde::de` (no ecosystem)
        let tokens = lex_kinds_trim("use serde::de");
        assert_eq!(
            tokens,
            vec![
                TokenKind::Use,
                TokenKind::Ident("serde".to_string()),
                TokenKind::DoubleColon,
                TokenKind::Ident("de".to_string()),
            ]
        );
    }

    #[test]
    fn test_lex_external_keyword() {
        let tokens = lex_kinds("external");
        assert_eq!(tokens, vec![TokenKind::External]);
    }

    #[test]
    fn test_lex_external_python_decl() {
        let src = r#"external python "json" "dumps"(x: i64) -> i64"#;
        let tokens = lex_kinds_trim(src);
        assert_eq!(tokens[0], TokenKind::External);
        assert_eq!(tokens[1], TokenKind::Ident("python".to_string()));
        assert_eq!(tokens[2], TokenKind::StrLit("json".to_string()));
        assert_eq!(tokens[3], TokenKind::StrLit("dumps".to_string()));
    }

    #[test]
    fn test_lex_external_rust_decl() {
        let src = r#"external rust "libc" "malloc"(n: i64) -> i64"#;
        let tokens = lex_kinds_trim(src);
        assert_eq!(tokens[0], TokenKind::External);
        assert_eq!(tokens[1], TokenKind::Ident("rust".to_string()));
        assert_eq!(tokens[2], TokenKind::StrLit("libc".to_string()));
    }

    #[test]
    fn test_lex_external_native_decl() {
        let src = r#"external native "libm" "sin"(x: f64) -> f64"#;
        let tokens = lex_kinds_trim(src);
        assert_eq!(tokens[0], TokenKind::External);
        assert_eq!(tokens[1], TokenKind::Ident("native".to_string()));
        assert_eq!(tokens[2], TokenKind::StrLit("libm".to_string()));
    }

    #[test]
    fn test_lex_qualified_call_syntax() {
        // `math.sqrt(81.0)` — dot, call, parens
        let tokens = lex_kinds_trim("math.sqrt(81.0)");
        assert_eq!(
            tokens,
            vec![
                TokenKind::Ident("math".to_string()),
                TokenKind::Dot,
                TokenKind::Ident("sqrt".to_string()),
                TokenKind::LParen,
                TokenKind::Float(81.0),
                TokenKind::RParen,
            ]
        );
    }

    #[test]
    fn test_lex_qualified_call_no_parens_is_field_access() {
        let tokens = lex_kinds_trim("math.value");
        assert_eq!(
            tokens,
            vec![
                TokenKind::Ident("math".to_string()),
                TokenKind::Dot,
                TokenKind::Ident("value".to_string()),
            ]
        );
    }

    #[test]
    fn test_lex_ecosystem_keywords_are_identifiers() {
        // `python`, `rust`, `native` should NOT be keywords — they're
        // only special in the context of `use` and `external` declarations.
        for kw in ["python", "rust", "native"] {
            let tokens = lex_kinds(kw);
            // Strip trailing Eof/Newline
            let real: Vec<_> = tokens
                .into_iter()
                .filter(|t| !matches!(t, TokenKind::Newline | TokenKind::Eof))
                .collect();
            assert_eq!(
                real,
                vec![TokenKind::Ident(kw.to_string())],
                "'{}' should lex as a plain identifier",
                kw
            );
        }
    }

    #[test]
    fn test_lex_str_literal_argument() {
        // String literal argument to a qualified call
        let tokens = lex_kinds_trim(r#"py_builtins.len("hello")"#);
        assert_eq!(
            tokens,
            vec![
                TokenKind::Ident("py_builtins".to_string()),
                TokenKind::Dot,
                TokenKind::Ident("len".to_string()),
                TokenKind::LParen,
                TokenKind::StrLit("hello".to_string()),
                TokenKind::RParen,
            ]
        );
    }

    #[test]
    fn test_lex_multiple_ecosystem_imports() {
        // Multiple use statements in one file
        let tokens = lex_kinds_trim("use python:json\nuse rust:serde\n");
        assert_eq!(tokens.len(), 8);
        assert_eq!(tokens[0], TokenKind::Use);
        assert_eq!(tokens[1], TokenKind::Ident("python".to_string()));
        assert_eq!(tokens[3], TokenKind::Ident("json".to_string()));
        assert_eq!(tokens[4], TokenKind::Use);
        assert_eq!(tokens[5], TokenKind::Ident("rust".to_string()));
    }
}
