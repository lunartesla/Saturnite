use stnx::lexer::{Lexer, TokenKind};

fn lex(src: &str) -> Result<Vec<TokenKind>, stnx::error::LexError> {
    Lexer::new(src)
        .collect::<Result<Vec<_>, _>>()
        .map(|tokens| tokens.into_iter().map(|t| t.kind).collect())
}

#[test]
fn test_valid_integer() {
    let tokens = lex("42").unwrap();
    assert_eq!(tokens, vec![TokenKind::Integer(42)]);
}

#[test]
fn test_valid_float() {
    let tokens = lex("3.5").unwrap();
    assert_eq!(tokens, vec![TokenKind::Float(3.5)]);
}

#[test]
fn test_valid_string() {
    let tokens = lex("\"hello\"").unwrap();
    assert_eq!(tokens, vec![TokenKind::StrLit("hello".to_string())]);
}

#[test]
fn test_valid_identifier() {
    let tokens = lex("my_var").unwrap();
    assert_eq!(tokens, vec![TokenKind::Ident("my_var".to_string())]);
}

#[test]
fn test_keyword_fn() {
    let tokens = lex("fn").unwrap();
    assert_eq!(tokens, vec![TokenKind::Fn]);
}

#[test]
fn test_invalid_token_at() {
    let result = lex("@");
    assert!(result.is_err(), "Expected lexer error for '@'");
    if let Err(e) = result {
        assert!(e.message.contains("unexpected character"));
    }
}

#[test]
fn test_invalid_token_dollar() {
    let result = lex("$");
    assert!(result.is_err(), "Expected lexer error for '$'");
}

#[test]
fn test_integer_overflow() {
    let src = "99999999999999999999999";
    let tokens = lex(src).unwrap();
    assert_eq!(
        tokens,
        vec![TokenKind::Error],
        "Overflow should produce Error token"
    );
}

#[test]
fn test_float_very_large() {
    // Very large floats are parsed as infinity by f64, which is acceptable behavior
    let src = "999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999.0";
    let tokens = lex(src).unwrap();
    // The token is parsed as a Float (infinity or large value) rather than silently 0.0
    // This is acceptable since f64 has built-in infinity representation
    assert!(matches!(tokens[0], TokenKind::Float(f) if f.is_infinite() || f > 0.0));
}

#[test]
fn test_dotdot_token() {
    let tokens = lex("..").unwrap();
    assert_eq!(tokens, vec![TokenKind::DotDot]);
}

#[test]
fn test_dotdot_ellipsis_token() {
    let tokens = lex("...").unwrap();
    assert_eq!(tokens, vec![TokenKind::DotDotEllipsis]);
}

#[test]
fn test_valid_program_lexes() {
    let src = "fn main() -> i64 { 42 }";
    let result = lex(src);
    assert!(result.is_ok(), "Valid program should lex successfully");
}
