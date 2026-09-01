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
}
