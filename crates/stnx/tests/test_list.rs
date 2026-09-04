//! 0.5.3 List<i64> integration tests: construction through the full
//! pipeline (lex → parse → HIR → MIR → verify → LLVM → link → run).
//!
//! Covers literal construction of `[1]`, `[1, 2, 3]`, element expressions
//! (`[1 + 2, 3 * 4]`), and left-to-right element evaluation order.

mod common;

use common::{compile_src, ir_only};

#[test]
fn test_list_single_element_compiles_and_runs() {
    let src = r#"
fn main() -> i64 {
    let a = [1]
    return 0
}
"#;
    let bin = compile_src(src);
    let (code, _) = bin.run();
    assert_eq!(code, 0, "list construction should not crash");
}

#[test]
fn test_list_multiple_elements_compiles_and_runs() {
    let src = r#"
fn main() -> i64 {
    let a = [1, 2, 3]
    return 0
}
"#;
    let bin = compile_src(src);
    let (code, _) = bin.run();
    assert_eq!(code, 0, "list construction should not crash");
}

#[test]
fn test_list_element_expressions_compiles_and_runs() {
    let src = r#"
fn main() -> i64 {
    let a = [1 + 2, 3 * 4]
    return 0
}
"#;
    let bin = compile_src(src);
    let (code, _) = bin.run();
    assert_eq!(code, 0, "list with element expressions should not crash");
}

#[test]
fn test_list_ir_contains_list_new_from_call() {
    let src = r#"
fn main() -> i64 {
    let a = [1, 2, 3]
    return 0
}
"#;
    let ir = ir_only(src);
    assert!(
        ir.contains("call ptr @list_new_from"),
        "generated IR should call the list_new_from runtime constructor"
    );
}

#[test]
fn test_list_evaluation_order_left_to_right() {
    // Use side effects (println) to pin evaluation order: 7 then 9.
    let src = r#"
fn elem(x: i64) -> i64 {
    println(x)
    return x
}
fn main() -> i64 {
    let a = [elem(7), elem(9)]
    return 0
}
"#;
    let bin = compile_src(src);
    let (code, stdout) = bin.run();
    assert_eq!(code, 0);
    assert_eq!(
        stdout, "7\n9\n",
        "list elements must evaluate left-to-right"
    );
}

#[test]
fn test_list_construction_repeated_no_crash() {
    // Repeated construction must not corrupt memory (each list is a fresh
    // malloc'd sat_list; process-lifetime model).
    let src = r#"
fn main() -> i64 {
    let a = [1, 2, 3]
    let b = [4, 5]
    let c = [6]
    return 0
}
"#;
    let bin = compile_src(src);
    let (code, _) = bin.run();
    assert_eq!(code, 0);
}
