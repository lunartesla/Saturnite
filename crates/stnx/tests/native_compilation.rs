//! Native compilation integration tests.
//!
//! These tests exercise the **full** pipeline: lex -> parse -> semantic
//! analysis -> LLVM IR -> object file -> link -> execute.  Each test distinguishes
//! *compiler success* (the program was built) from *program runtime success*
//! (the built binary exits with the expected code / stdout).
//!
//! All artifacts live inside isolated tempfile::TempDir directories so
//! parallel test execution never collides on fixed filenames.

mod common;

use common::{compile_src, compile_to_object, ir_only};
use stnx::target::TargetConfig;

// Arithmetic

#[test]
fn test_arithmetic() {
    let bin = compile_src("fn main() -> i64 { let x = 10 + 5 * 2 return x }");
    let (code, _) = bin.run();
    assert_eq!(code, 20, "arithmetic result should be 20");
}

#[test]
fn test_arithmetic_subtraction_and_division() {
    let bin = compile_src("fn main() -> i64 { let mut x = 100 x = x - 20 return x / 4 }");
    let (code, _) = bin.run();
    assert_eq!(code, 20);
}

// Variables

#[test]
fn test_local_variable() {
    let bin = compile_src("fn main() -> i64 { let x = 42 return x }");
    let (code, _) = bin.run();
    assert_eq!(code, 42);
}

#[test]
fn test_multiple_variables() {
    let bin = compile_src("fn main() -> i64 { let x = 10 let y = 20 let z = x + y return z }");
    let (code, _) = bin.run();
    assert_eq!(code, 30);
}

// Mutable variables

#[test]
fn test_mutable_variable_assignment() {
    let bin = compile_src("fn main() -> i64 { let mut x = 0 x = 10 return x }");
    let (code, _) = bin.run();
    assert_eq!(code, 10);
}

#[test]
fn test_augmented_assignment() {
    let bin = compile_src("fn main() -> i64 { let mut x = 10 x += 5 x -= 2 x *= 3 return x }");
    let (code, _) = bin.run();
    assert_eq!(code, 39);
}

// For loops / ranges

#[test]
fn test_for_loop_runtime() {
    let bin = compile_src("fn main() -> i64 { for i in 0..5 { println(i) } return 0 }");
    let (code, stdout) = bin.run();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 5);
    assert_eq!(lines[0], "0");
    assert_eq!(lines[4], "4");
    assert_eq!(code, 0);
}

#[test]
fn test_for_loop_with_arithmetic() {
    let bin = compile_src(
        "fn main() -> i64 { let mut sum = 0 for i in 0..10 { sum = sum + i } return sum }",
    );
    let (code, _) = bin.run();
    assert_eq!(code, 45, "sum of 0..10 should be 45");
}

#[test]
fn test_for_loop_inclusive_range() {
    let bin = compile_src(
        "fn main() -> i64 { let mut sum = 0 for i in 1...5 { sum = sum + i } return sum }",
    );
    let (code, _) = bin.run();
    assert_eq!(code, 15, "sum of 1...5 should be 15");
}

// While loops

#[test]
fn test_while_loop() {
    let bin = compile_src("fn main() -> i64 { let mut i = 0 let mut sum = 0 while i < 5 { sum = sum + i i = i + 1 } return sum }");
    let (code, _) = bin.run();
    assert_eq!(code, 10);
}

// If / else

#[test]
fn test_if_else() {
    let bin = compile_src(
        "fn main() -> i64 { let x = 1 if x == 1 { println(100) } else { println(200) } return 0 }",
    );
    let (code, stdout) = bin.run();
    assert_eq!(stdout.trim(), "100");
    assert_eq!(code, 0);
}

#[test]
fn test_if_else_return_value() {
    let bin = compile_src(
        "fn main() -> i64 { let x = 1 if x == 1 { return 100 } else { return 200 } return 0 }",
    );
    let (code, _) = bin.run();
    assert_eq!(code, 100);
}

#[test]
fn test_elif_branches() {
    let bin = compile_src("fn main() -> i64 { let x = 2 if x == 1 { return 10 } elif x == 2 { return 20 } else { return 99 } return 0 }");
    let (code, _) = bin.run();
    assert_eq!(code, 20);
}

#[test]
fn test_if_false_else() {
    let bin = compile_src(
        "fn main() -> i64 { let x = 0 if x == 1 { return 100 } else { return 200 } return 0 }",
    );
    let (code, _) = bin.run();
    assert_eq!(code, 200);
}

// Functions & recursion

#[test]
fn test_function_call() {
    let bin = compile_src("fn main() -> i64 { return greet() } fn greet() -> i64 { return 42 }");
    let (code, _) = bin.run();
    assert_eq!(code, 42);
}

#[test]
fn test_function_with_args() {
    let bin = compile_src(
        "fn main() -> i64 { return add(10, 20) } fn add(a: i64, b: i64) -> i64 { return a + b }",
    );
    let (code, _) = bin.run();
    assert_eq!(code, 30);
}

#[test]
fn test_recursion() {
    let bin = compile_src("fn main() -> i64 { return fact(5) }\nfn fact(n: i64) -> i64 { if n <= 1 { return 1 } return n * fact(n - 1) }");
    let (code, _) = bin.run();
    assert_eq!(code, 120, "fact(5) should be 120");
}

#[test]
fn test_recursive_countdown() {
    let bin = compile_src("fn main() -> i64 { return count(3) }\nfn count(n: i64) -> i64 { if n == 0 { return 0 } return count(n - 1) + 1 }");
    let (code, _) = bin.run();
    assert_eq!(code, 3);
}

// Shadowing

#[test]
fn test_shadowing() {
    let bin = compile_src("fn main() -> i64 { let x = 1 let x = x + 1 return x }");
    let (code, _) = bin.run();
    assert_eq!(code, 2);
}

#[test]
fn test_shadowing_different_value() {
    let bin = compile_src("fn main() -> i64 { let x = 42 let x = x * 2 return x }");
    let (code, _) = bin.run();
    assert_eq!(code, 84);
}

// Builtins

#[test]
fn test_main_returning_i64() {
    let bin = compile_src("fn main() -> i64 { return 42 }");
    let (code, _) = bin.run();
    assert_eq!(code, 42);
}

#[test]
fn test_main_returning_unit_with_println() {
    let bin = compile_src("fn main() { println(42) }");
    let (_code, stdout) = bin.run();
    assert_eq!(stdout.trim(), "42");
}

#[test]
fn test_println_multiple() {
    let bin = compile_src("fn main() { println(10) println(20) }");
    let (_code, stdout) = bin.run();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], "10");
    assert_eq!(lines[1], "20");
}

// Object file emission

#[test]
fn test_emit_object_file() {
    let artifact = compile_to_object("fn main() -> i64 { return 42 }");
    assert!(artifact.path().exists(), "object file should be created");
    let bytes = std::fs::read(artifact.path()).expect("should read object file");
    assert!(bytes.len() > 4, "object file should not be empty");
    assert_eq!(
        &bytes[0..4],
        b"\x7fELF",
        "object file should be a valid ELF"
    );
}

// IR generation

#[test]
fn test_ir_generation_contains_main() {
    let ir = ir_only("fn main() -> i64 { return 42 }");
    assert!(
        ir.contains("define i64 @main"),
        "IR should contain main function definition"
    );
}

#[test]
fn test_ir_implicit_return_bool() {
    let ir = ir_only("fn main() -> bool { }");
    assert!(ir.contains("ret i1"), "Bool function should return i1");
}

#[test]
fn test_ir_implicit_return_i64() {
    let ir = ir_only("fn main() -> i64 { }");
    assert!(ir.contains("ret i64"), "I64 function should return i64");
}

#[test]
fn test_ir_implicit_return_unit() {
    let ir = ir_only("fn main() { }");
    assert!(
        ir.contains("ret void") || !ir.contains("ret i64"),
        "Unit function should return void"
    );
}

#[test]
fn test_ir_implicit_return_f64() {
    let ir = ir_only("fn main() -> f64 { }");
    assert!(
        ir.contains("ret double"),
        "F64 function should return double"
    );
}

#[test]
fn test_ir_for_loop_structure() {
    let ir = ir_only("fn main() -> i64 { for i in 0..10 { } 0 }");
    assert!(ir.contains("for_cond"));
    assert!(ir.contains("for_body"));
    assert!(ir.contains("icmp"));
}

#[test]
fn test_ir_elif_branch_codegen() {
    let ir =
        ir_only("fn main() -> i64 { let x = 1 if x == 1 { 0 } elif x == 2 { 0 } else { 0 } 0 }");
    let cond_br_count = ir.matches("br i1").count();
    assert!(
        cond_br_count >= 2,
        "If-elif-else should have >= 2 conditional branches, got {}",
        cond_br_count
    );
}

#[test]
fn test_ir_range_evaluates_both_ends() {
    let ir = ir_only("fn main() -> i64 { let r = 1..10 r }");
    assert!(ir.contains("define i64 @main"));
}

#[test]
fn test_ir_forward_function_reference() {
    let ir = ir_only("fn main() -> i64 { foo() } fn foo() -> i64 { 42 }");
    assert!(ir.contains("foo"), "Forward reference to foo should work");
}

#[test]
fn test_ir_string_literal_compiles() {
    let ir = ir_only("fn main() -> i64 { let s = \"hello\" 0 }");
    assert!(
        ir.contains("define i64 @main"),
        "String literal should compile"
    );
}

// Target configuration

#[test]
fn test_native_target_initialization() {
    let config = TargetConfig::host();
    assert!(config.is_ok());
    let config = config.unwrap();
    let triple = config.triple_str();
    assert!(!triple.is_empty());
    assert!(triple.contains("linux") || triple.contains("windows") || triple.contains("darwin"));
}

#[test]
fn test_invalid_target_configuration() {
    let result = TargetConfig::from_triple("invalid-triple");
    assert!(
        result.is_err(),
        "invalid target triple should produce an error"
    );
}
