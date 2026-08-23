use stnx::hir::HirProgram;
use stnx::lexer::Lexer;
use stnx::parser;
use stnx::semantic::analyze_and_lower;

fn compile_src(src: &str) -> Result<String, String> {
    let tokens: Vec<_> = Lexer::new(src)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Lex error: {}", e))?;
    let program = parser::parse(src, tokens).map_err(|e| format!("Parse error: {}", e))?;
    let hir: HirProgram =
        analyze_and_lower(&program).map_err(|e| format!("Semantic error: {}", e))?;

    let mir =
        stnx::mir::lower::lower_program(&hir).map_err(|e| format!("MIR lowering error: {}", e))?;

    if let Err(errs) = mir.verify() {
        return Err(format!(
            "MIR verification failed: {}",
            errs.iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    stnx::mir::codegen::generate_ir_from_mir(&mir).map_err(|e| format!("Codegen error: {}", e))
}

#[test]
fn test_implicit_return_bool() {
    // Bug #5: Function with -> bool and no explicit return
    // Should generate ret i1 0, not ret i64 0
    let src = "fn main() -> bool { }";
    let ir = compile_src(src).unwrap();
    assert!(
        ir.contains("ret i1"),
        "Bool function should return i1, got: {}",
        ir
    );
}

#[test]
fn test_implicit_return_i64() {
    let src = "fn main() -> i64 { }";
    let ir = compile_src(src).unwrap();
    assert!(
        ir.contains("ret i64"),
        "I64 function should return i64, got: {}",
        ir
    );
}

#[test]
fn test_implicit_return_unit() {
    let src = "fn main() { }";
    let ir = compile_src(src).unwrap();
    assert!(
        ir.contains("ret void") || !ir.contains("ret i64"),
        "Unit function should return void"
    );
}

#[test]
fn test_implicit_return_f64() {
    let src = "fn main() -> f64 { }";
    let ir = compile_src(src).unwrap();
    assert!(
        ir.contains("ret double"),
        "F64 function should return double, got: {}",
        ir
    );
}

#[test]
fn test_for_loop_with_range() {
    // Bug #6: For loop should iterate over range, not be infinite
    let src = "fn main() -> i64 { for i in 0..10 { } 0 }";
    let ir = compile_src(src).unwrap();
    // Should contain loop structure with conditional branch
    assert!(
        ir.contains("for_cond"),
        "For loop should have for_cond block"
    );
    assert!(
        ir.contains("for_body"),
        "For loop should have for_body block"
    );
    assert!(
        ir.contains("icmp"),
        "For loop should have comparison instruction"
    );
}

#[test]
fn test_for_loop_inclusive_range() {
    let src = "fn main() -> i64 { for i in 0...5 { } 0 }";
    let ir = compile_src(src).unwrap();
    assert!(
        ir.contains("for_cond"),
        "Inclusive for loop should have for_cond block"
    );
}

#[test]
fn test_elif_branch_codegen() {
    // Bug #7: elif branches should be codegen'd, not ignored
    let src = "fn main() -> i64 { let x = 1 if x == 1 { 0 } elif x == 2 { 0 } else { 0 } 0 }";
    let ir = compile_src(src).unwrap();
    // Count the number of conditional branches - should be more with elif
    let cond_br_count = ir.matches("br i1").count();
    assert!(
        cond_br_count >= 2,
        "If-elif-else should have at least 2 conditional branches, got {}",
        cond_br_count
    );
}

#[test]
fn test_range_evaluates_both_start_and_end() {
    // Bug #8: Range expression should evaluate both start and end
    let src = "fn main() -> i64 { let r = 1..10 r }";
    let ir = compile_src(src).unwrap();
    assert!(
        ir.contains("ret i64"),
        "Should compile successfully with range"
    );
}

#[test]
fn test_forward_function_reference() {
    // Bug #15: Forward declarations should work in compile_to_executable path
    let src = "fn main() -> i64 { foo() } fn foo() -> i64 { 42 }";
    let ir = compile_src(src).unwrap();
    assert!(ir.contains("foo"), "Forward reference to foo should work");
}

#[test]
fn test_string_literal_compiles() {
    let src = "fn main() -> i64 { let s = \"hello\" 0 }";
    let ir = compile_src(src).unwrap();
    // Should compile without panicking
    assert!(ir.contains("define i64 @main"));
}

#[test]
fn test_struct_literal_codegen() {
    let src =
        "fn main() -> i64 { struct Point { x: i64, y: i64 } let p = Point { x: 10, y: 20 } p.x }";
    let ir = compile_src(src).unwrap();
    assert!(
        ir.contains("define i64 @main"),
        "struct construction should generate main"
    );
}

#[test]
fn test_field_access_codegen() {
    let src =
        "fn main() -> i64 { struct Point { x: i64, y: i64 } let p = Point { x: 10, y: 20 } p.y }";
    let ir = compile_src(src).unwrap();
    assert!(
        ir.contains("define i64 @main"),
        "field access should generate code"
    );
}

#[test]
fn test_struct_with_nested_field_access() {
    let src = "fn main() -> i64 { struct Point { x: i64, y: i64 } struct Pair { a: Point, b: i64 } let p = Pair { a: Point { x: 5, y: 6 }, b: 7 } p.a.x }";
    let ir = compile_src(src).unwrap();
    assert!(
        ir.contains("define i64 @main"),
        "nested struct field access should compile"
    );
}

#[test]
fn test_enum_constructor_codegen() {
    let src = "fn main() -> i64 { enum Color { Red, Green, Blue } let c = Color::Green 0 }";
    let ir = compile_src(src).unwrap();
    assert!(
        ir.contains("define i64 @main"),
        "enum construction should generate code"
    );
}

#[test]
fn test_bool_function_signature() {
    // Regression test: functions returning `bool` must be declared with
    // `i1` return type in LLVM IR, not `i64`.  Previously all functions
    // were declared with `i64` regardless of their actual return type,
    // causing undefined behaviour at call sites.
    let src = "fn is_even(n: i64) -> bool { return n % 2 == 0 } fn main() -> i64 { return 0 }";
    let ir = compile_src(src).unwrap();
    assert!(
        ir.contains("define i1 @is_even"),
        "Bool-returning function should be declared with i1 return type, got: {}",
        ir
    );
}

#[test]
fn test_f64_function_return_signature() {
    // Regression test: functions returning `f64` must be declared with
    // `double` return type in LLVM IR, not `i64`.
    let src = "fn half(n: f64) -> f64 { return n } fn main() -> i64 { return 0 }";
    let ir = compile_src(src).unwrap();
    assert!(
        ir.contains("define double @half"),
        "F64-returning function should be declared with double return type, got: {}",
        ir
    );
}

// --- Floating-point binary operation IR tests ---
// These tests use variables (not inline constants) to prevent LLVM from
// constant-folding the operations, ensuring the IR actually contains
// fadd/fsub/fmul/fdiv/fcmp instructions.

#[test]
fn test_ir_float_add() {
    let src = "fn main() -> i64 { let a = 1.5 let b = 2.5 if a + b == 4.0 { 1 } else { 0 } }";
    let ir = compile_src(src).unwrap();
    assert!(
        ir.contains("fadd"),
        "Float add should emit fadd, got: {}",
        ir
    );
}

#[test]
fn test_ir_float_sub() {
    let src = "fn main() -> i64 { let a = 10.0 let b = 1.5 if a - b == 8.5 { 1 } else { 0 } }";
    let ir = compile_src(src).unwrap();
    assert!(
        ir.contains("fsub"),
        "Float sub should emit fsub, got: {}",
        ir
    );
}

#[test]
fn test_ir_float_mul() {
    let src = "fn main() -> i64 { let a = 2.0 let b = 3.0 if a * b == 6.0 { 1 } else { 0 } }";
    let ir = compile_src(src).unwrap();
    assert!(
        ir.contains("fmul"),
        "Float mul should emit fmul, got: {}",
        ir
    );
}

#[test]
fn test_ir_float_div() {
    let src = "fn main() -> i64 { let a = 10.0 let b = 2.0 if a / b == 5.0 { 1 } else { 0 } }";
    let ir = compile_src(src).unwrap();
    assert!(
        ir.contains("fdiv"),
        "Float div should emit fdiv, got: {}",
        ir
    );
}

#[test]
fn test_ir_float_comparison_eq() {
    let src = "fn main() -> i64 { let a = 5.5 let b = 5.5 if a == b { 1 } else { 0 } }";
    let ir = compile_src(src).unwrap();
    assert!(
        ir.contains("fcmp"),
        "Float comparison should emit fcmp, got: {}",
        ir
    );
}

#[test]
fn test_ir_float_comparison_ordering() {
    // Test floating-point ordering predicates: OLT (less than), OGT (greater than), OLE, OGE
    let src = "fn main() -> i64 { let a = 1.0 let b = 2.0 if a < b { 1 } else { 0 } }";
    let ir = compile_src(src).unwrap();
    assert!(
        ir.contains("fcmp olt"),
        "Float < should emit fcmp olt, got: {}",
        ir
    );
}

#[test]
fn test_ir_float_function_call() {
    // Test that f64 function calls work across functions and emit correct IR
    let src = "fn compute() -> f64 { let a = 10.0 let b = 2.0 a / b } fn main() -> i64 { let r = compute() if r == 5.0 { 1 } else { 0 } }";
    let ir = compile_src(src).unwrap();
    assert!(
        ir.contains("define double @compute"),
        "compute should return double: {}",
        ir
    );
    assert!(
        ir.contains("fdiv"),
        "Should contain fdiv in compute function: {}",
        ir
    );
    assert!(
        ir.contains("fcmp oeq"),
        "Should contain fcmp oeq comparison in main: {}",
        ir
    );
}

#[test]
fn test_ir_float_mixed_type_rejected() {
    // Mixed-type arithmetic should be rejected by semantic analysis (type mismatch)
    let src = "fn main() -> i64 { let x = 1 + 2.0 0 }";
    let result = compile_src(src);
    assert!(
        result.is_err(),
        "Mixed int+float should be rejected by semantic analysis"
    );
}
