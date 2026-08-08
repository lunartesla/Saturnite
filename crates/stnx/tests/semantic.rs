use stnx::lexer::Lexer;
use stnx::parser;
use stnx::semantic::analyze;

fn analyze_src(src: &str) -> Result<(), String> {
    let tokens: Vec<_> = Lexer::new(src)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Lex error: {}", e))?;
    let program = parser::parse(src, tokens).map_err(|e| format!("Parse error: {}", e))?;
    analyze(&program).map_err(|e| format!("Semantic error: {}", e))
}

#[test]
fn test_return_type_match_i64() {
    let src = "fn main() -> i64 { return 42 }";
    assert!(analyze_src(src).is_ok());
}

#[test]
fn test_return_type_mismatch_bool() {
    let src = "fn main() -> bool { return 42 }";
    let result = analyze_src(src);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("return type mismatch"));
}

#[test]
fn test_return_type_mismatch_i64_from_bool() {
    let src = "fn main() -> i64 { return true }";
    let result = analyze_src(src);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("return type mismatch"));
}

#[test]
fn test_no_return_with_unit_type() {
    let src = "fn main() { }";
    assert!(analyze_src(src).is_ok());
}

#[test]
fn test_no_return_with_i64_type() {
    // The language supports implicit returns, so this is valid
    let src = "fn main() -> i64 { }";
    assert!(analyze_src(src).is_ok());
}

#[test]
fn test_no_return_with_bool_type() {
    let src = "fn main() -> bool { }";
    assert!(analyze_src(src).is_ok());
}

#[test]
fn test_explicit_unit_return() {
    let src = "fn main() -> unit { return }";
    assert!(analyze_src(src).is_ok());
}

#[test]
fn test_println_with_correct_arg_type() {
    let src = "fn main() -> i64 { println(42) return 0 }";
    assert!(analyze_src(src).is_ok());
}

#[test]
fn test_println_with_wrong_arg_type_bool() {
    let src = "fn main() -> i64 { println(true) return 0 }";
    let result = analyze_src(src);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("println expects i64"));
}

#[test]
fn test_println_with_wrong_arg_type_float() {
    let src = "fn main() -> i64 { println(3.14) return 0 }";
    let result = analyze_src(src);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("println expects i64"));
}

#[test]
fn test_range_with_valid_types() {
    let src = "fn main() -> i64 { let x = 1..10 return 0 }";
    assert!(analyze_src(src).is_ok());
}

#[test]
fn test_range_with_mismatched_types() {
    let src = "fn main() -> i64 { let x = true..false return 0 }";
    let result = analyze_src(src);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("range start type mismatch"));
}

#[test]
fn test_forward_function_reference() {
    let src = "fn main() -> i64 { foo() } fn foo() -> i64 { 42 }";
    assert!(analyze_src(src).is_ok());
}

#[test]
fn test_function_arg_count_mismatch() {
    let src = "fn main() -> i64 { foo(1, 2) } fn foo(x: i64) -> i64 { x }";
    let result = analyze_src(src);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("expects 1 args"));
}

#[test]
fn test_function_arg_type_mismatch() {
    let src = "fn main() -> i64 { foo(true) } fn foo(x: i64) -> i64 { x }";
    let result = analyze_src(src);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("arg type mismatch"));
}
