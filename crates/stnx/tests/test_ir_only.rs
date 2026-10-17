//! Tests IR-only generation (no linking, no file output).

mod common;

use common::ir_only;

#[test]
fn test_ir_generation_only() {
    let ir = ir_only("fn main() -> i64 { return 42 }");
    assert!(
        ir.contains("define i64 @main"),
        "IR should contain main function definition"
    );
    println!("IR: {}", ir);
}

// ---------------------------------------------------------------------------
// 0.5.4 Phase 5B: Code generation tests for SAT_PY_* constant emission
// ---------------------------------------------------------------------------

/// Check that the IR for a Python external call with an i64 argument emits
/// the `SAT_PY_I64` kind value (2).
#[test]
fn test_codegen_sat_py_i64_constant_for_i64_arg() {
    let src = r#"external python "json" "dumps"(x: i64) -> i64
fn main() -> i64 { dumps(42) }"#;
    let ir = ir_only(src);
    // SAT_PY_I64 = 2 — the i32 kind value stored should be 2.
    assert!(
        ir.contains("store i32 2"),
        "IR should emit SAT_PY_I64 (2) for i64 arg, got:\n{}",
        ir
    );
}

/// Check that the IR for a Python external call with an f64 argument emits
/// `SAT_PY_F64` (3).
#[test]
fn test_codegen_sat_py_f64_constant_for_f64_arg() {
    let src = r#"external python "math" "sqrt"(x: f64) -> f64
fn main() -> f64 { sqrt(81.0) }"#;
    let ir = ir_only(src);
    assert!(
        ir.contains("store i32 3"),
        "IR should emit SAT_PY_F64 (3) for f64 arg, got:\n{}",
        ir
    );
}

/// Check that the IR for a Python external call with a bool argument emits
/// `SAT_PY_BOOL` (1).
#[test]
fn test_codegen_sat_py_bool_constant_for_bool_arg() {
    let src = r#"external python "builtins" "check"(x: bool) -> i64
fn main() -> i64 { check(true) }"#;
    let ir = ir_only(src);
    assert!(
        ir.contains("store i32 1"),
        "IR should emit SAT_PY_BOOL (1) for bool arg, got:\n{}",
        ir
    );
}

/// Check that Python external calls route through the sat_py_call_flat bridge
/// with the sat_py_result struct alloca, confirming the Python codegen path
/// is taken for all Python ecosystem calls.
#[test]
fn test_codegen_python_call_uses_sat_py_bridge() {
    let src = r#"external python "math" "sqrt"(x: f64) -> f64
fn main() -> f64 { sqrt(81.0) }"#;
    let ir = ir_only(src);
    // The Python bridge uses the sat_py_result struct type.
    assert!(
        ir.contains("{ i1, i32, i64, i64, ptr, ptr, ptr }"),
        "Python external call should use sat_py_result struct"
    );
    // The call should go through sat_py_call_flat.
    assert!(
        ir.contains("call i1 @sat_py_call_flat"),
        "Python external call should call sat_py_call_flat"
    );
}

/// Check that the IR for a Python external call with a Unit return emits
/// `SAT_PY_NONE` (0) as the kind for no args (zero-arg call has no kinds array,
/// but we verify the call to sat_py_call_flat with count 0).
#[test]
fn test_codegen_sat_py_none_constant_for_unit_return() {
    let src = r#"external python "random" "seed"() -> unit
fn main() -> i64 { seed() }"#;
    let ir = ir_only(src);
    // The call should reference sat_py_call_flat with args count 0.
    assert!(
        ir.contains("sat_py_call_flat"),
        "IR should call sat_py_call_flat for Python externals"
    );
    // The count argument should be 0 for no args.
    assert!(
        ir.contains("i64 0"),
        "IR should pass arg count 0 for zero-arg Python external"
    );
}

/// Check that Python external calls use the `module::function` symbol format
/// in the IR global string.
#[test]
fn test_codegen_python_symbol_uses_module_qualified_format() {
    let src = r#"external python "json" "dumps"(x: i64) -> i64
fn main() -> i64 { dumps(42) }"#;
    let ir = ir_only(src);
    assert!(
        ir.contains("json::dumps"),
        "IR should contain module-qualified Python symbol 'json::dumps'"
    );
}

/// Check that the sat_py_call_flat function declaration appears in IR with
/// the correct signature (returns i1/bool, takes 6 pointer-sized args).
#[test]
fn test_codegen_sat_py_call_flat_declaration() {
    let src = r#"external python "math" "sqrt"(x: f64) -> f64
fn main() -> f64 { sqrt(81.0) }"#;
    let ir = ir_only(src);
    // sat_py_call_flat returns i1 (bool) and takes (ptr, ptr, ptr, ptr, i64, ptr)
    assert!(
        ir.contains("declare i1 @sat_py_call_flat(ptr, ptr, ptr, ptr, i64, ptr)"),
        "IR should declare sat_py_call_flat with bool return and correct params:\n{}",
        ir
    );
}

/// Check that sat_py_abort is declared with void return type, matching the C
/// declaration `void sat_py_abort(const char*, const char*, bool)`.
#[test]
fn test_codegen_sat_py_abort_void_return() {
    let src = r#"external python "math" "sqrt"(x: f64) -> f64
fn main() -> f64 { sqrt(81.0) }"#;
    let ir = ir_only(src);
    // sat_py_abort returns void, not ptr — matching the C declaration.
    assert!(
        ir.contains("declare void @sat_py_abort"),
        "IR should declare sat_py_abort with void return type (matching C declaration):\n{}",
        ir
    );
}

/// Check that the sat_py_result struct type appears in IR for Python calls.
#[test]
fn test_codegen_sat_py_result_struct_type() {
    let src = r#"external python "math" "sqrt"(x: f64) -> f64
fn main() -> f64 { sqrt(81.0) }"#;
    let ir = ir_only(src);
    assert!(
        ir.contains("{ i1, i32, i64, i64, ptr, ptr, ptr }"),
        "IR should contain sat_py_result struct type"
    );
}

/// Check that multiple argument types get correct kind values.
/// `process(a: i64, b: f64, c: bool)` should emit kind values 2, 3, 1.
#[test]
fn test_codegen_sat_py_constants_for_mixed_args() {
    let src = r#"external python "api" "process"(a: i64, b: f64, c: bool) -> i64
fn main() -> i64 { process(1, 2.0, true) }"#;
    let ir = ir_only(src);
    assert!(
        ir.contains("store i32 2"),
        "IR should emit SAT_PY_I64 (2) for first arg"
    );
    assert!(
        ir.contains("store i32 3"),
        "IR should emit SAT_PY_F64 (3) for second arg"
    );
    assert!(
        ir.contains("store i32 1"),
        "IR should emit SAT_PY_BOOL (1) for third arg"
    );
}

/// Check that Rust external calls use a direct extern declaration, not
/// the Python bridge (no sat_py_call_flat).
#[test]
fn test_codegen_rust_external_uses_direct_call() {
    let src = r#"external rust "serde" "add"(x: i64, y: i64) -> i64
fn main() -> i64 { add(1, 2) }"#;
    let ir = ir_only(src);
    // Rust externals should NOT go through sat_py_call_flat.
    assert!(
        !ir.contains("sat_py_call_flat"),
        "Rust external should not use Python bridge"
    );
    // Rust externals should declare the symbol directly.
    assert!(
        ir.contains("@add"),
        "Rust external should declare the link-time symbol directly (@add)"
    );
}

/// Check that Rust externals declare the symbol as an external function.
#[test]
fn test_codegen_rust_external_decl_symbol() {
    let src = r#"external rust "crate" "func"(x: i64) -> i64
fn main() -> i64 { func(42) }"#;
    let ir = ir_only(src);
    // For Rust externals, the symbol is the foreign symbol name directly.
    assert!(
        ir.contains("@func"),
        "IR should reference the Rust external symbol 'func'"
    );
}
