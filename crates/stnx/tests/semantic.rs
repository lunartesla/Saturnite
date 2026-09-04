use stnx::lexer::Lexer;
use stnx::parser;
use stnx::semantic::analyze_and_lower;

fn analyze_src(src: &str) -> Result<(), String> {
    let tokens: Vec<_> = Lexer::new(src)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Lex error: {}", e))?;
    let program = parser::parse(src, tokens).map_err(|e| format!("Parse error: {}", e))?;
    analyze_and_lower(&program).map_err(|e| format!("Semantic error: {}", e))?;
    Ok(())
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

#[test]
fn test_assign_to_immutable_variable_rejected() {
    // Bug #1 (Phase 1): assignment to an immutable variable must be a semantic error.
    let src = "fn main() -> i64 { let x = 1 x = 2 return x }";
    let result = analyze_src(src);
    assert!(
        result.is_err(),
        "mutating an immutable variable should fail"
    );
    assert!(
        result.unwrap_err().contains("immutable"),
        "error should mention immutability"
    );
}

#[test]
fn test_assign_to_mutable_variable_allowed() {
    // The counterpart: `let mut x` allows reassignment.
    let src = "fn main() -> i64 { let mut x = 1 x = 2 return x }";
    assert!(
        analyze_src(src).is_ok(),
        "mutating a mutable variable should succeed"
    );
}

#[test]
fn test_struct_construction_type_check() {
    let src =
        "fn main() -> i64 { struct Point { x: i64, y: i64 } let p = Point { x: 10, y: 20 } 0 }";
    assert!(
        analyze_src(src).is_ok(),
        "struct construction with correct types should pass"
    );
}

#[test]
fn test_struct_field_type_mismatch() {
    let src =
        "fn main() -> i64 { struct Point { x: i64, y: i64 } let p = Point { x: true, y: 20 } 0 }";
    let result = analyze_src(src);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("expects"));
}

#[test]
fn test_undefined_struct_literal() {
    let src = "fn main() -> i64 { let p = Point { x: 10, y: 20 } 0 }";
    let result = analyze_src(src);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("undefined struct"));
}

#[test]
fn test_undefined_struct_field() {
    let src =
        "fn main() -> i64 { struct Point { x: i64, y: i64 } let p = Point { x: 10, z: 20 } 0 }";
    let result = analyze_src(src);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("no field"));
}

#[test]
fn test_field_access_on_non_struct() {
    let src = "fn main() -> i64 { let x = 42 x.foo }";
    let result = analyze_src(src);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("non-struct"));
}

#[test]
fn test_undefined_field_access() {
    let src =
        "fn main() -> i64 { struct Point { x: i64, y: i64 } let p = Point { x: 10, y: 20 } p.z }";
    let result = analyze_src(src);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("no field"));
}

#[test]
fn test_enum_construction_type_check() {
    let src = "fn main() -> i64 { enum Color { Red, Green, Blue } let c = Color::Red 0 }";
    assert!(analyze_src(src).is_ok(), "enum construction should pass");
}

#[test]
fn test_undefined_enum_constructor() {
    let src = "fn main() -> i64 { let c = Color::Red 0 }";
    let result = analyze_src(src);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("undefined enum"));
}

#[test]
fn test_undefined_enum_variant() {
    let src = "fn main() -> i64 { enum Color { Red, Green, Blue } let c = Color::Purple 0 }";
    let result = analyze_src(src);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("no variant"));
}

#[test]
fn test_struct_with_enum_field_type() {
    let src = "fn main() -> i64 { enum Status { Active, Inactive } struct Item { s: Status, n: i64 } let i = Item { s: Status::Active, n: 5 } i.n }";
    assert!(
        analyze_src(src).is_ok(),
        "struct with enum-typed field should pass"
    );
}

#[test]
fn test_struct_field_type_mismatch_with_enum() {
    let src = "fn main() -> i64 { enum Status { Active, Inactive } struct Item { s: Status, n: i64 } let i = Item { s: 5, n: 5 } 0 }";
    let result = analyze_src(src);
    assert!(result.is_err());
}

// --- Unary NOT semantics ---

#[test]
fn test_not_on_bool_allowed() {
    let src = "fn main() -> i64 { let x = true let y = !x 0 }";
    assert!(analyze_src(src).is_ok(), "! on bool should be allowed");
}

#[test]
fn test_not_on_integer_rejected() {
    let src = "fn main() -> i64 { let y = !42 0 }";
    let result = analyze_src(src);
    assert!(result.is_err(), "! on i64 should be rejected");
    assert!(
        result.unwrap_err().contains("only bool"),
        "error should mention only bool"
    );
}

#[test]
fn test_not_on_float_rejected() {
    let src = "fn main() -> i64 { let y = !3.14 0 }";
    let result = analyze_src(src);
    assert!(result.is_err(), "! on f64 should be rejected");
    assert!(
        result.unwrap_err().contains("only bool"),
        "error should mention only bool"
    );
}

#[test]
fn test_not_on_string_rejected() {
    let src = "fn main() -> i64 { let y = !\"hello\" 0 }";
    let result = analyze_src(src);
    assert!(result.is_err(), "! on str should be rejected");
}

// --- Modulo semantics ---

#[test]
fn test_mod_on_int_allowed() {
    let src = "fn main() -> i64 { let x = 7 % 3 x }";
    assert!(analyze_src(src).is_ok(), "mod on i64 should be allowed");
}

#[test]
fn test_mod_on_float_rejected() {
    let src = "fn main() -> i64 { let x = 1.5 % 0.5 0 }";
    let result = analyze_src(src);
    assert!(
        result.is_err(),
        "mod on f64 should be rejected by semantic analysis"
    );
    let err = result.unwrap_err();
    assert!(
        err.contains("modulo is only supported for i64"),
        "error should mention modulo is only for i64, got: {}",
        err
    );
}

#[test]
fn test_mod_on_bool_rejected() {
    let src = "fn main() -> i64 { let x = true % false 0 }";
    let result = analyze_src(src);
    assert!(result.is_err(), "mod on bool should be rejected");
}

// ---------------------------------------------------------------------------
// 0.5.3 Phase 7: list indexing and length semantics
// ---------------------------------------------------------------------------

#[test]
fn test_index_on_non_list_rejected() {
    let src = "fn main() -> i64 { let x = 5 let y = x[0] 0 }";
    let result = analyze_src(src);
    assert!(
        result.is_err(),
        "indexing a non-list should be rejected"
    );
    let err = result.unwrap_err();
    assert!(
        err.contains("List<T>"),
        "error should mention List<T>, got: {}",
        err
    );
}

#[test]
fn test_length_on_non_list_rejected() {
    let src = "fn main() -> i64 { let x = 5 let y = x.length 0 }";
    let result = analyze_src(src);
    assert!(
        result.is_err(),
        "length on a non-list should be rejected"
    );
    let err = result.unwrap_err();
    assert!(
        err.contains("non-struct type") || err.contains("List<T>"),
        "error should mention the type mismatch, got: {}",
        err
    );
}

#[test]
fn test_index_valid_list_passes() {
    let src = "fn main() -> i64 { let values = [10, 20, 30] let x = values[0] 0 }";
    let result = analyze_src(src);
    assert!(result.is_ok(), "indexing a List<i64> should pass");
}

#[test]
fn test_length_valid_list_passes() {
    let src = "fn main() -> i64 { let values = [10, 20, 30] let x = values.length 0 }";
    let result = analyze_src(src);
    assert!(result.is_ok(), "length on a List<i64> should pass");
}

