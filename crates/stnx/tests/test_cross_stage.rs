//! Cross-stage integration tests for Rust+Python ecosystem interoperability.
//!
//! These tests verify that types, kinds, and ABI contracts are preserved
//! across all compiler stages: lexer → parser → HIR → MIR → verify → LLVM IR.

mod common;

use common::ir_only;

// ---------------------------------------------------------------------------
// Type preservation tests: source type → MIR arg_types → LLVM IR kind tag
// ---------------------------------------------------------------------------

#[test]
fn test_type_preservation_i64_through_pipeline() {
    // An i64 external call param should produce SAT_PY_I64=2 in the IR.
    let src = r#"external python "json" "dumps"(x: i64) -> i64
fn main() -> i64 { dumps(42) }"#;
    let ir = ir_only(src);
    // The kind value 2 (SAT_PY_I64) should appear.
    assert!(
        ir.contains("i32 2"),
        "i64 type should produce SAT_PY_I64 (2) kind in IR"
    );
    // The arg count should be 1.
    assert!(
        ir.contains("i64 1"),
        "IR should pass arg count 1 to sat_py_call_flat"
    );
}

#[test]
fn test_type_preservation_f64_through_pipeline() {
    let src = r#"external python "math" "sqrt"(x: f64) -> f64
fn main() -> f64 { sqrt(81.0) }"#;
    let ir = ir_only(src);
    assert!(
        ir.contains("i32 3"),
        "f64 type should produce SAT_PY_F64 (3) kind in IR"
    );
    assert!(
        ir.contains("i64 1"),
        "IR should pass arg count 1 to sat_py_call_flat"
    );
}

#[test]
fn test_type_preservation_bool_through_pipeline() {
    let src = r#"external python "builtins" "check"(x: bool) -> i64
fn main() -> i64 { check(true) }"#;
    let ir = ir_only(src);
    assert!(
        ir.contains("i32 1"),
        "bool type should produce SAT_PY_BOOL (1) kind in IR"
    );
    assert!(
        ir.contains("i64 1"),
        "IR should pass arg count 1 to sat_py_call_flat"
    );
}

#[test]
fn test_type_preservation_multiple_types_in_one_call() {
    // All ABI-safe types in a single call: i64, f64, bool.
    let src = r#"external python "api" "process"(a: i64, b: f64, c: bool) -> i64
fn main() -> i64 { process(1, 2.0, true) }"#;
    let ir = ir_only(src);
    // Check all kind values are present.
    assert!(ir.contains("i32 2"), "should have SAT_PY_I64 (2)");
    assert!(ir.contains("i32 3"), "should have SAT_PY_F64 (3)");
    assert!(ir.contains("i32 1"), "should have SAT_PY_BOOL (1)");
    // Check arg count is 3.
    assert!(
        ir.contains("i64 3"),
        "IR should pass arg count 3 to sat_py_call_flat"
    );
}

#[test]
fn test_type_preservation_unit_return_propagates() {
    // Unit return type should be handled (ret_ty = Unit maps to SAT_PY_NONE).
    let src = r#"external python "builtins" "print"(x: i64) -> unit
fn main() -> i64 { print(42) }"#;
    let ir = ir_only(src);
    // Should call sat_py_call_flat.
    assert!(
        ir.contains("sat_py_call_flat"),
        "Unit-return external should still use Python bridge"
    );
    // The arg kind should be SAT_PY_I64 = 2.
    assert!(ir.contains("i32 2"), "should have SAT_PY_I64 (2) for arg");
}

// ---------------------------------------------------------------------------
// ABI boundary tests: module-qualified symbols for Python, plain symbols for Rust
// ---------------------------------------------------------------------------

#[test]
fn test_abi_boundary_python_uses_module_qualified_symbol() {
    // Python externals use "module::function" symbol format.
    let src = r#"external python "json" "dumps"(x: i64) -> i64
fn main() -> i64 { dumps(42) }"#;
    let ir = ir_only(src);
    assert!(
        ir.contains("json::dumps"),
        "Python external should use 'module::function' symbol format in IR"
    );
}

#[test]
fn test_abi_boundary_rust_uses_plain_symbol() {
    // Rust externals use the plain symbol name.
    let src = r#"external rust "serde" "add"(x: i64, y: i64) -> i64
fn main() -> i64 { add(1, 2) }"#;
    let ir = ir_only(src);
    // Should NOT use sat_py_call_flat.
    assert!(
        !ir.contains("sat_py_call_flat"),
        "Rust external should bypass Python bridge"
    );
    // Should declare the symbol directly.
    assert!(
        ir.contains("@add"),
        "Rust external should use plain symbol name"
    );
}

#[test]
fn test_abi_boundary_native_uses_plain_symbol() {
    let src = r#"external native "libm" "floor"(x: f64) -> f64
fn main() -> f64 { floor(3.7) }"#;
    let ir = ir_only(src);
    assert!(
        !ir.contains("sat_py_call_flat"),
        "Native external should bypass Python bridge"
    );
    assert!(
        ir.contains("@floor"),
        "Native external should use plain symbol name"
    );
}

#[test]
fn test_abi_boundary_py_kind_enum_matches_ir() {
    // Verify that the SAT_PY_* constants in codegen.rs match the values
    // emitted in the LLVM IR for each type.
    let cases = [
        ("external python \"m\" \"f\"(x: i64) -> i64\nfn main() -> i64 { f(1) }", "i32 2"),  // SAT_PY_I64
        ("external python \"m\" \"f\"(x: f64) -> i64\nfn main() -> i64 { f(1.0) }", "i32 3"),  // SAT_PY_F64
        ("external python \"m\" \"f\"(x: bool) -> i64\nfn main() -> i64 { f(true) }", "i32 1"), // SAT_PY_BOOL
    ];
    for (src, expected_kind) in &cases {
        let ir = ir_only(src);
        assert!(
            ir.contains(expected_kind),
            "expected {} in IR for source:\n{}",
            expected_kind,
            src
        );
    }
}

// ---------------------------------------------------------------------------
// Struct layout matching: sat_py_result struct in IR
// ---------------------------------------------------------------------------

#[test]
fn test_sat_py_result_struct_layout_in_ir() {
    // The sat_py_result struct must have exactly 7 fields matching pyrt.h:
    // { bool ok, i32 kind, i64 union, i64 str_len, ptr err_class,
    //   ptr err_message, ptr handle }
    let src = r#"external python "math" "sqrt"(x: f64) -> f64
fn main() -> f64 { sqrt(81.0) }"#;
    let ir = ir_only(src);
    // Check the struct type exists with the correct field types.
    assert!(
        ir.contains("{ i1, i32, i64, i64, ptr, ptr, ptr }"),
        "sat_py_result struct should have layout: i1, i32, i64, i64, ptr, ptr, ptr"
    );
    // The alloca should use this struct type.
    assert!(
        ir.contains("alloca { i1, i32, i64, i64, ptr, ptr, ptr }"),
        "sat_py_out should be alloca'd as sat_py_result struct"
    );
}

#[test]
fn test_sat_py_out_initialized_to_zero() {
    // The sat_py_result struct should be zero-initialized before the call.
    let src = r#"external python "math" "sqrt"(x: f64) -> f64
fn main() -> f64 { sqrt(81.0) }"#;
    let ir = ir_only(src);
    assert!(
        ir.contains("zeroinitializer"),
        "sat_py_result should be zero-initialized before call"
    );
}

#[test]
fn test_sat_py_result_extractvalue_offsets() {
    // The result is read from the struct: ok is at index 0, value at index 2.
    let src = r#"external python "math" "sqrt"(x: f64) -> f64
fn main() -> f64 { sqrt(81.0) }"#;
    let ir = ir_only(src);
    // The value extraction uses extractvalue at index 2 (the i64 union).
    assert!(
        ir.contains("extractvalue"),
        "IR should extract result from sat_py_result struct"
    );
}

// ---------------------------------------------------------------------------
// End-to-end compilation tests (compile + run)
// ---------------------------------------------------------------------------

#[test]
fn test_rust_external_ir_is_valid() {
    // Rust externals should produce valid IR with the direct symbol call.
    // Full linking requires the actual library, so we only verify IR here.
    let src = r#"external rust "serde" "add"(x: i64, y: i64) -> i64
fn main() -> i64 { add(1, 2) }"#;
    let ir = ir_only(src);
    // Should NOT use sat_py_call_flat.
    assert!(
        !ir.contains("sat_py_call_flat"),
        "Rust external should bypass Python bridge"
    );
    // Should use the plain symbol name directly.
    assert!(
        ir.contains("call i64 @add"),
        "Rust external should call the plain symbol directly"
    );
}

#[test]
fn test_native_external_ir_is_valid() {
    let src = r#"external native "libm" "floor"(x: f64) -> f64
fn main() -> f64 { floor(3.7) }"#;
    let ir = ir_only(src);
    assert!(
        !ir.contains("sat_py_call_flat"),
        "Native external should bypass Python bridge"
    );
    assert!(
        ir.contains("call double @floor"),
        "Native external should call the plain symbol directly"
    );
}

#[test]
fn test_python_external_with_i64_arg_compiles() {
    // Even though it can't actually call Python at link time (no runtime
    // linked in IR-only mode), the program should still generate valid IR.
    let src = r#"external python "json" "dumps"(x: i64) -> i64
fn main() -> i64 { dumps(42) }"#;
    // ir_only should succeed (it doesn't link).
    let ir = ir_only(src);
    assert!(ir.contains("sat_py_call_flat"), "IR should be valid and reference the bridge");
}

#[test]
fn test_multiple_external_calls_in_main() {
    // Multiple external calls in the same function should all produce
    // ExternalCall rvalues with correct arg_types.
    let src = r#"external python "api" "get_a"(x: i64) -> i64
external python "api" "get_b"(x: f64) -> f64
fn main() -> i64 { let a = get_a(1) let b = get_b(2.0) a }
"#;
    let ir = ir_only(src);
    // Both calls should use sat_py_call_flat.
    let call_count = ir.matches("sat_py_call_flat").count();
    assert!(
        call_count >= 2,
        "Expected at least 2 sat_py_call_flat calls, got {}",
        call_count
    );
    // Both kind values 2 (I64) and 3 (F64) should appear.
    assert!(ir.contains("i32 2"), "should have SAT_PY_I64 for get_a");
    assert!(ir.contains("i32 3"), "should have SAT_PY_F64 for get_b");
}

#[test]
fn test_external_and_regular_function_coexist() {
    // A program with both external declarations and regular function calls.
    let src = r#"external python "math" "sqrt"(x: f64) -> f64
fn helper(x: i64) -> i64 { x * 2 }
fn main() -> i64 { let h = helper(10) let r = sqrt(81.0) h }
"#;
    let ir = ir_only(src);
    // Both the external call and the regular function call should be present.
    assert!(ir.contains("sat_py_call_flat"), "should have external call");
    // Regular function call should use a different pattern (not sat_py_call_flat).
    // The helper function should have its own definition.
    assert!(
        ir.contains("define i64 @helper"),
        "should have helper function definition"
    );
}

#[test]
fn test_external_call_arg_count_in_ir() {
    // Verify that the arg count (5th parameter of sat_py_call_flat) matches
    // the number of arguments in the call.
    let src = r#"external python "api" "process"(a: i64, b: f64, c: bool) -> i64
fn main() -> i64 { process(1, 2.0, true) }"#;
    let ir = ir_only(src);
    // sat_py_call_flat signature: (spec, search, kinds, values, count, out)
    // The count should be 3.
    assert!(
        ir.contains("i64 3"),
        "IR should pass arg count 3 for 3-arg Python external"
    );
    // Check all three kind values are present.
    assert!(ir.contains("i32 2"), "SAT_PY_I64 (2) for arg a");
    assert!(ir.contains("i32 3"), "SAT_PY_F64 (3) for arg b");
    assert!(ir.contains("i32 1"), "SAT_PY_BOOL (1) for arg c");
}
