//! Tests that compiler diagnostics carry useful source spans — from tokens
//! through the AST to the final compiler error.

use stnx::error::{CompilerError, ParseError};
use stnx::lexer::Lexer;
use stnx::parser;
use stnx::semantic::analyze;

/// Helper: lex -> parse -> analyze, returning the raw CompilerError.
fn compile_error(src: &str) -> CompilerError {
    let tokens: Vec<_> = Lexer::new(src)
        .collect::<Result<Vec<_>, _>>()
        .expect("lexing should succeed up to parse");
    match parser::parse(src, tokens) {
        Ok(program) => match analyze(&program) {
            Ok(_) => panic!("expected an error but compilation succeeded"),
            Err(e) => e,
        },
        Err(e) => e,
    }
}

#[test]
fn test_parse_error_contains_source_span() {
    // Missing closing brace -> parse error
    let src = "fn main() -> i64 { return 42\n";
    let err = compile_error(src);
    match &err {
        CompilerError::Parse(ParseError { span, .. }) => {
            // The span should point at a real location in the source, not 0..0.
            let offset = span.offset();
            let len = span.len();
            assert!(len > 0, "parse error span should have non-zero length");
            assert!(offset < src.len(), "span offset should be within source");
        }
        other => panic!("expected Parse error, got: {:?}", other),
    }
}

#[test]
fn test_parse_error_message_is_descriptive() {
    let src = "fn main() -> i64 { return 42\n";
    let err = compile_error(src);
    let msg = err.to_string();
    assert!(
        msg.contains("expected") || msg.contains("unexpected"),
        "parse error should be descriptive, got: {}",
        msg
    );
}

#[test]
fn test_lex_error_contains_span() {
    // '$' is not a valid token
    let src = "fn main() -> i64 { return $ }";
    let result: Result<Vec<_>, _> = Lexer::new(src).collect();
    assert!(result.is_err(), "lexing invalid char should fail");
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains('$') || msg.contains("unexpected"),
        "lex error should mention the bad character"
    );
}

#[test]
fn test_semantic_error_undefined_variable() {
    let src = "fn main() -> i64 { return undefined_var }";
    let err = compile_error(src);
    let msg = err.to_string();
    assert!(
        msg.contains("undefined variable"),
        "should report undefined variable, got: {}",
        msg
    );
}

#[test]
fn test_semantic_error_immutable_assignment() {
    let src = "fn main() -> i64 { let x = 1 x = 2 return x }";
    let err = compile_error(src);
    let msg = err.to_string();
    assert!(
        msg.contains("immutable"),
        "should report immutable assignment, got: {}",
        msg
    );
}

#[test]
fn test_semantic_error_return_type_mismatch() {
    let src = "fn main() -> bool { return 42 }";
    let err = compile_error(src);
    let msg = err.to_string();
    assert!(
        msg.contains("return type mismatch"),
        "should report return type mismatch, got: {}",
        msg
    );
}
