//! Pre-parse token filter that emits `Indent`/`Dedent`/`Newline` synthetic
//! tokens based on the column position of each non-whitespace token.

use crate::error::LexError;
use crate::lexer::{Token, TokenKind};

/// Result of running the indent pre-pass.
pub struct IndentedTokens {
    pub tokens: Vec<Token>,
    pub src: String,
}

/// Run the indent pre-pass over a token stream produced by `Lexer::new`.
pub fn run(src: &str, raw: Vec<Token>) -> Result<IndentedTokens, Vec<LexError>> {
    let mut out: Vec<Token> = Vec::with_capacity(raw.len() + 16);
    let mut indent_stack: Vec<usize> = vec![0];
    let mut errors: Vec<LexError> = Vec::new();
    let mut cursor: usize = 0;

    for tok in raw {
        let span_start = tok.span.start;
        let span_end = tok.span.end;

        // Compute the start of the line containing this token.
        let line_start = src[..span_start].rfind('\n').map(|p| p + 1).unwrap_or(0);

        // Did we cross any newlines between `cursor` and `line_start`?
        let crossed_newline = cursor <= line_start;

        if crossed_newline {
            // For each line that ended between `cursor` and `line_start`,
            // emit a Newline (and check whether it was meaningful for indent).
            let mut walk = cursor;
            while walk < line_start {
                let nl = src[walk..line_start].find('\n');
                match nl {
                    Some(off) => {
                        let line_end = walk + off;
                        let line_content = &src[walk..line_end];
                        // Emit Newline regardless; emit Indent/Dedent only
                        // for meaningful lines.
                        if !is_blank_or_comment_line(line_content) {
                            let indent = compute_indent(line_content);
                            apply_indent(indent, &mut indent_stack, &mut out, src, &mut errors);
                        }
                        out.push(Token {
                            kind: TokenKind::Newline,
                            span: line_end..line_end,
                        });
                        walk = line_end + 1;
                    }
                    None => break,
                }
            }

            // Now handle the line containing this token — emit Indent based
            // on the column of the token itself, regardless of whether the
            // line so far is blank (it might just be leading whitespace).
            let line_prefix = &src[line_start..span_start];
            // Compute the indent of THIS token's column. If the prefix is
            // all whitespace, compute_indent gives the count of spaces, which
            // is the column the token sits at.
            let indent = compute_indent(line_prefix);
            apply_indent(indent, &mut indent_stack, &mut out, src, &mut errors);
        }

        out.push(tok.clone());
        cursor = span_end;
    }

    // After the last token, emit dedents to unwind and a final newline.
    while indent_stack.len() > 1 {
        out.push(Token {
            kind: TokenKind::Dedent,
            span: cursor..cursor,
        });
        indent_stack.pop();
    }
    out.push(Token {
        kind: TokenKind::Newline,
        span: cursor..cursor,
    });

    if !errors.is_empty() {
        return Err(errors);
    }

    Ok(IndentedTokens {
        tokens: out,
        src: src.to_string(),
    })
}

fn is_blank_or_comment_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.is_empty() || trimmed.starts_with("//")
}

fn compute_indent(prefix: &str) -> usize {
    prefix
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .count()
}

fn apply_indent(
    indent: usize,
    indent_stack: &mut Vec<usize>,
    out: &mut Vec<Token>,
    src: &str,
    errors: &mut Vec<LexError>,
) {
    let top = *indent_stack.last().unwrap_or(&0);
    if indent > top {
        indent_stack.push(indent);
        out.push(Token {
            kind: TokenKind::Indent,
            span: 0..0,
        });
    } else if indent < top {
        while let Some(&top) = indent_stack.last() {
            if top <= indent {
                break;
            }
            indent_stack.pop();
            out.push(Token {
                kind: TokenKind::Dedent,
                span: 0..0,
            });
        }
        if *indent_stack.last().unwrap_or(&0) != indent {
            errors.push(LexError::new(
                src,
                0,
                1,
                format!(
                    "mismatched indentation: expected one of {:?}, found column {}",
                    indent_stack, indent
                ),
            ));
            indent_stack.push(indent);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

    fn lex(src: &str) -> Vec<Token> {
        Lexer::new(src).collect::<Result<Vec<_>, _>>().expect("lex")
    }

    fn kinds(toks: &[Token]) -> Vec<TokenKind> {
        toks.iter().map(|t| t.kind.clone()).collect()
    }

    #[test]
    fn test_no_indent_emits_no_synthetic_tokens() {
        let src = "fn f() -> i64 { 0 }";
        let toks = lex(src);
        let out = run(src, toks).unwrap().tokens;
        assert!(!out.iter().any(|t| matches!(t.kind, TokenKind::Indent)));
        assert!(!out.iter().any(|t| matches!(t.kind, TokenKind::Dedent)));
    }

    #[test]
    fn test_two_lines_emits_newline_and_indent() {
        let src = "fn a():\n    pass\n";
        let toks = lex(src);
        let out = run(src, toks).unwrap().tokens;
        assert!(out.iter().any(|t| matches!(t.kind, TokenKind::Newline)));
        assert!(out.iter().any(|t| matches!(t.kind, TokenKind::Indent)));
        assert!(out.iter().any(|t| matches!(t.kind, TokenKind::Dedent)));
    }

    #[test]
    fn test_balanced_indent_dedent() {
        let src = "fn a():\n    x = 1\n    y = 2\nfn b():\n    pass\n";
        let toks = lex(src);
        let out = run(src, toks).unwrap().tokens;
        let k = kinds(&out);
        let indent_count = k.iter().filter(|x| matches!(x, TokenKind::Indent)).count();
        let dedent_count = k.iter().filter(|x| matches!(x, TokenKind::Dedent)).count();
        assert_eq!(indent_count, 2);
        assert_eq!(dedent_count, 2);
    }

    #[test]
    fn test_blank_lines_ignored() {
        let src = "fn a():\n\n    x = 1\n";
        let toks = lex(src);
        let out = run(src, toks).unwrap().tokens;
        let k = kinds(&out);
        let indent_count = k.iter().filter(|x| matches!(x, TokenKind::Indent)).count();
        assert_eq!(indent_count, 1);
    }
}
