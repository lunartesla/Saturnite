//! Token preparation for the 0.5 native syntax.
//!
//! Runs the lexer, then the indent pre-pass, then a token-level rewrite
//! that desugars colon-indented blocks into brace blocks:
//!
//! ```text
//! fn f():        =>   fn f() {
//!     x = 1          x = 1
//!                =>   }
//! ```
//!
//! This keeps the brace-based parser untouched while accepting native
//! colon-indented syntax. Newline/Indent/Dedent synthetic tokens are
//! consumed here; the parser never sees them.

use crate::error::{CompilerError, CompilerResult, ParseError};
use crate::lexer::indent;
use crate::lexer::{Lexer, Token, TokenKind};

/// Lex, run the indent pre-pass, and desugar native colon-blocks into
/// brace blocks. This is the token entry point used by the real compile
/// pipeline (`module::parse_source`).
pub fn prepare(src: &str) -> CompilerResult<Vec<Token>> {
    let mut raw: Vec<Token> = Vec::new();
    for tok in Lexer::new(src) {
        match tok {
            Ok(t) => raw.push(t),
            Err(e) => return Err(CompilerError::Lexer(e)),
        }
    }

    let indented = indent::run(src, raw).map_err(|errs| {
        CompilerError::Parse(ParseError {
            src: src.to_string(),
            span: (0, 0).into(),
            message: errs
                .iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("; "),
        })
    })?;

    desugar_blocks(src, indented.tokens)
}

/// One open colon-block being tracked by the desugar pass.
struct OpenBlock {
    /// Whether lines inside this block are struct/enum fields
    /// (a Comma is inserted between lines instead of dropping the newline).
    field_mode: bool,
    /// Indent depth at which the block body sits (base + 1).
    body_depth: usize,
}

/// Rewrite the token stream: colon-blocks become braces, synthetic
/// Newline/Indent/Dedent tokens are consumed, and struct/enum field
/// lines get separators.
fn desugar_blocks(src: &str, tokens: Vec<Token>) -> CompilerResult<Vec<Token>> {
    let mut out: Vec<Token> = Vec::with_capacity(tokens.len());
    let mut open: Vec<OpenBlock> = Vec::new();
    let mut depth: usize = 0;
    // Set after a block-opening colon, until the body's Indent arrives.
    let mut awaiting_body: Option<OpenBlock> = None;
    // The last keyword token seen (for struct/enum field-mode detection).
    let mut last_kw: Option<TokenKind> = None;
    let eof_span = src.len().saturating_sub(1)..src.len();

    for (i, tok) in tokens.iter().cloned().enumerate() {
        match &tok.kind {
            TokenKind::Newline => {
                if awaiting_body.is_some() {
                    // Colon followed by Newline: body starts on next line.
                } else if let Some(inner) = open.last_mut() {
                    if inner.field_mode {
                        // Insert a Comma between field lines, unless the
                        // block is about to close (look ahead for Dedents
                        // that pop back to the block's base depth).
                        if !block_closes_after(&tokens, i, inner.body_depth, depth) {
                            out.push(Token {
                                kind: TokenKind::Comma,
                                span: tok.span.clone(),
                            });
                        }
                    }
                }
                // Top-level and statement-mode newlines are dropped.
                last_kw = None;
            }
            TokenKind::Indent => {
                if let Some(block) = awaiting_body.take() {
                    open.push(block);
                }
                depth += 1;
                last_kw = None;
            }
            TokenKind::Dedent => {
                if awaiting_body.is_some() {
                    return Err(CompilerError::Parse(ParseError {
                        src: src.to_string(),
                        span: (tok.span.start, 1).into(),
                        message: "expected an indented block after ':'".to_string(),
                    }));
                }
                depth = depth.saturating_sub(1);
                while let Some(inner) = open.last() {
                    if depth < inner.body_depth {
                        open.pop();
                        out.push(Token {
                            kind: TokenKind::RBrace,
                            span: tok.span.clone(),
                        });
                    } else {
                        break;
                    }
                }
                last_kw = None;
            }
            TokenKind::Colon => {
                // Block-opening colon: the next token is a Newline (body on
                // the next line). Type-annotation colons (`x: i64`) are
                // followed by a token on the same line and stay untouched.
                let next_is_newline = tokens
                    .get(i + 1)
                    .map(|t| t.kind == TokenKind::Newline)
                    .unwrap_or(false);
                if next_is_newline {
                    let field_mode =
                        matches!(last_kw, Some(TokenKind::Struct) | Some(TokenKind::Enum));
                    // `main:` desugars to `fn main() -> i64 {`.
                    let prev_is_main = open.is_empty()
                        && !field_mode
                        && out.last().is_some_and(
                            |t: &Token| matches!(&t.kind, TokenKind::Ident(s) if s == "main"),
                        );
                    if prev_is_main {
                        // Replace the bare `main` identifier with the
                        // equivalent legacy function header.
                        out.pop();
                        let span = tok.span.clone();
                        for kind in [
                            TokenKind::Fn,
                            TokenKind::Ident("main".to_string()),
                            TokenKind::LParen,
                            TokenKind::RParen,
                            TokenKind::RArrow,
                            TokenKind::I64,
                        ] {
                            out.push(Token {
                                kind,
                                span: span.clone(),
                            });
                        }
                    }
                    out.push(Token {
                        kind: TokenKind::LBrace,
                        span: tok.span.clone(),
                    });
                    awaiting_body = Some(OpenBlock {
                        field_mode,
                        body_depth: depth + 1,
                    });
                } else {
                    out.push(tok);
                }
                last_kw = None;
            }
            kind => {
                // Track the last keyword for field-mode detection
                // (`struct Item:` — Struct must survive the Ident and any
                // generic parameters). Identifiers/commas keep it; anything
                // else clears it.
                if matches!(
                    kind,
                    TokenKind::Ident(_) | TokenKind::Comma | TokenKind::Lt | TokenKind::Gt
                ) {
                    // sticky
                } else if matches!(
                    kind,
                    TokenKind::Fn
                        | TokenKind::Struct
                        | TokenKind::Enum
                        | TokenKind::If
                        | TokenKind::Elif
                        | TokenKind::Else
                        | TokenKind::For
                        | TokenKind::While
                        | TokenKind::Let
                        | TokenKind::Return
                        | TokenKind::Give
                        | TokenKind::Say
                        | TokenKind::Raise
                        | TokenKind::Mod
                        | TokenKind::Module
                        | TokenKind::Use
                        | TokenKind::Pub
                        | TokenKind::Println
                ) {
                    last_kw = Some(kind.clone());
                } else {
                    last_kw = None;
                }
                out.push(tok);
            }
        }
    }

    if awaiting_body.is_some() {
        return Err(CompilerError::Parse(ParseError {
            src: src.to_string(),
            span: eof_span.into(),
            message: "expected an indented block after ':'".to_string(),
        }));
    }
    if !open.is_empty() {
        return Err(CompilerError::Parse(ParseError {
            src: src.to_string(),
            span: eof_span.into(),
            message: "unclosed indented block: expected the block body to end before EOF"
                .to_string(),
        }));
    }

    Ok(out)
}

/// Look ahead from a Newline at `idx`: do the Dedent(s) that follow pop the
/// indent depth back to (or below) `body_depth - 1`, closing the block?
fn block_closes_after(tokens: &[Token], idx: usize, body_depth: usize, depth: usize) -> bool {
    let mut d = depth;
    for t in &tokens[idx + 1..] {
        match t.kind {
            TokenKind::Dedent => {
                d = d.saturating_sub(1);
                if d < body_depth {
                    return true;
                }
            }
            TokenKind::Indent => d += 1,
            TokenKind::Newline => continue,
            _ => return false,
        }
    }
    // EOF with dedents outstanding: the block closes at EOF.
    d < body_depth
}
