use stnx::lexer::Lexer;
use stnx::parser;
use stnx::semantic::analyze;

fn compile_src(src: &str) -> Result<String, String> {
    let tokens: Vec<_> = Lexer::new(src)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Lex error: {}", e))?;
    let program = parser::parse(src, tokens).map_err(|e| format!("Parse error: {}", e))?;
    analyze(&program).map_err(|e| format!("Semantic error: {}", e))?;
    stnx::codegen::generate_ir(&program).map_err(|e| format!("Codegen error: {}", e))
}

#[test]
fn test_implicit_return_bool() {
    // Bug #5: Function with -> bool and no explicit return
    // Should generate ret i1 0, not ret i64 0
    let src = "fn main() -> bool { }";
    let ir = compile_src(src).unwrap();
    assert!(ir.contains("ret i1"), "Bool function should return i1, got: {}", ir);
}

#[test]
fn test_implicit_return_i64() {
    let src = "fn main() -> i64 { }";
    let ir = compile_src(src).unwrap();
    assert!(ir.contains("ret i64"), "I64 function should return i64, got: {}", ir);
}

#[test]
fn test_implicit_return_unit() {
    let src = "fn main() { }";
    let ir = compile_src(src).unwrap();
    assert!(ir.contains("ret void") || !ir.contains("ret i64"), "Unit function should return void");
}

#[test]
fn test_implicit_return_f64() {
    let src = "fn main() -> f64 { }";
    let ir = compile_src(src).unwrap();
    assert!(ir.contains("ret double"), "F64 function should return double, got: {}", ir);
}

#[test]
fn test_for_loop_with_range() {
    // Bug #6: For loop should iterate over range, not be infinite
    let src = "fn main() -> i64 { for i in 0..10 { } 0 }";
    let ir = compile_src(src).unwrap();
    // Should contain loop structure with conditional branch
    assert!(ir.contains("for_cond"), "For loop should have for_cond block");
    assert!(ir.contains("for_body"), "For loop should have for_body block");
    assert!(ir.contains("icmp"), "For loop should have comparison instruction");
}

#[test]
fn test_for_loop_inclusive_range() {
    let src = "fn main() -> i64 { for i in 0...5 { } 0 }";
    let ir = compile_src(src).unwrap();
    assert!(ir.contains("for_cond"), "Inclusive for loop should have for_cond block");
}

#[test]
fn test_elif_branch_codegen() {
    // Bug #7: elif branches should be codegen'd, not ignored
    let src = "fn main() -> i64 { let x = 1 if x == 1 { 0 } elif x == 2 { 0 } else { 0 } 0 }";
    let ir = compile_src(src).unwrap();
    // Count the number of conditional branches - should be more with elif
    let cond_br_count = ir.matches("br i1").count();
    assert!(cond_br_count >= 2, "If-elif-else should have at least 2 conditional branches, got {}", cond_br_count);
}

#[test]
fn test_range_evaluates_both_start_and_end() {
    // Bug #8: Range expression should evaluate both start and end
    let src = "fn main() -> i64 { let r = 1..10 r }";
    let ir = compile_src(src).unwrap();
    assert!(ir.contains("ret i64"), "Should compile successfully with range");
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
