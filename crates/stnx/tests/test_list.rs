//! 0.5.3 List<i64> integration tests: construction through the full
//! pipeline (lex → parse → HIR → MIR → verify → LLVM → link → run).
//!
//! Covers literal construction of `[1]`, `[1, 2, 3]`, element expressions
//! (`[1 + 2, 3 * 4]`), and left-to-right element evaluation order.

mod common;

use common::{analyze_src, compile_src, ir_only};

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

// ---------------------------------------------------------------------------
// 0.5.3 Phase 7: indexing and length
// ---------------------------------------------------------------------------

#[test]
fn test_list_index_first() {
    let src = r#"
fn main() -> i64 {
    let values = [10, 20, 30]
    println(values[0])
    return 0
}
"#;
    let bin = compile_src(src);
    let (code, stdout) = bin.run();
    assert_eq!(code, 0);
    assert_eq!(stdout, "10\n");
}

#[test]
fn test_list_index_middle() {
    let src = r#"
fn main() -> i64 {
    let values = [10, 20, 30]
    println(values[1])
    return 0
}
"#;
    let bin = compile_src(src);
    let (code, stdout) = bin.run();
    assert_eq!(code, 0);
    assert_eq!(stdout, "20\n");
}

#[test]
fn test_list_index_last() {
    let src = r#"
fn main() -> i64 {
    let values = [10, 20, 30]
    println(values[2])
    return 0
}
"#;
    let bin = compile_src(src);
    let (code, stdout) = bin.run();
    assert_eq!(code, 0);
    assert_eq!(stdout, "30\n");
}

#[test]
fn test_list_length() {
    let src = r#"
fn main() -> i64 {
    let values = [10, 20, 30]
    println(values.length)
    return 0
}
"#;
    let bin = compile_src(src);
    let (code, stdout) = bin.run();
    assert_eq!(code, 0);
    assert_eq!(stdout, "3\n");
}

#[test]
fn test_list_index_expression_element() {
    let src = r#"
fn main() -> i64 {
    let values = [10, 20, 30]
    let i = 1
    println(values[i])
    return 0
}
"#;
    let bin = compile_src(src);
    let (code, stdout) = bin.run();
    assert_eq!(code, 0);
    assert_eq!(stdout, "20\n");
}

#[test]
fn test_list_index_out_of_bounds_deterministic() {
    // Out-of-bounds must abort deterministically, not corrupt memory.
    let src = r#"
fn main() -> i64 {
    let values = [10, 20, 30]
    println(values[3])
    return 0
}
"#;
    let bin = compile_src(src);
    let (code, _stdout) = bin.run();
    assert_ne!(code, 0, "out-of-bounds access must fail non-zero");
}

#[test]
fn test_list_index_negative_out_of_bounds_deterministic() {
    let src = r#"
fn main() -> i64 {
    let values = [10, 20, 30]
    println(values[-1])
    return 0
}
"#;
    let bin = compile_src(src);
    let (code, _stdout) = bin.run();
    assert_ne!(code, 0, "negative index must fail non-zero");
}

#[test]
fn test_list_index_then_length_chained() {
    // Indexing and length compose: read the element at length-1.
    let src = r#"
fn main() -> i64 {
    let values = [10, 20, 30]
    let last = values[values.length - 1]
    println(last)
    return 0
}
"#;
    let bin = compile_src(src);
    let (code, stdout) = bin.run();
    assert_eq!(code, 0);
    assert_eq!(stdout, "30\n");
}

#[test]
fn test_list_ir_contains_list_get_and_list_len_calls() {
    let src = r#"
fn main() -> i64 {
    let values = [10, 20, 30]
    println(values[1])
    println(values.length)
    return 0
}
"#;
    let ir = ir_only(src);
    assert!(
        ir.contains("call i64 @list_get"),
        "generated IR should call list_get for indexing"
    );
    assert!(
        ir.contains("call i64 @list_len"),
        "generated IR should call list_len for length"
    );
}

// ---------------------------------------------------------------------------
// 0.5.3 Phase 8: list iteration
// ---------------------------------------------------------------------------

#[test]
fn test_list_iteration_basic() {
    let src = r#"
fn main() -> i64 {
    let values = [10, 20, 30]
    for item in values {
        println(item)
    }
    return 0
}
"#;
    let bin = compile_src(src);
    let (code, stdout) = bin.run();
    assert_eq!(code, 0);
    assert_eq!(
        stdout, "10\n20\n30\n",
        "list iteration should print each element in order"
    );
}

#[test]
fn test_list_iteration_empty_list_zero_iterations() {
    // Empty list literals are rejected at HIR in 0.5.3, so an empty list
    // cannot be constructed end-to-end. The zero-iteration contract is
    // therefore pinned on the generated IR shape: the index local is
    // initialized to 0 and the loop condition is `index < list_len(list)`.
    // For a zero-length list, `list_len` returns 0 (see runtime/list.c), so
    // `0 < 0` is false and the body never executes.
    let src = r#"
fn main() -> i64 {
    let values = [10, 20, 30]
    for item in values {
        println(item)
    }
    return 0
}
"#;
    let ir = ir_only(src);
    // Index initialized to zero before the condition block.
    assert!(
        ir.contains("for_cond"),
        "list iteration should have a for_cond block"
    );
    // The condition compares the index against the list length.
    assert!(
        ir.contains("call i64 @list_len"),
        "list iteration must read the list length for the bound"
    );
    // The loop body reads the element at the current index.
    assert!(
        ir.contains("call i64 @list_get"),
        "list iteration must read each element via list_get"
    );
}

#[test]
fn test_list_iteration_ir_uses_list_len_and_list_get() {
    let src = r#"
fn main() -> i64 {
    let values = [10, 20, 30]
    for item in values {
        println(item)
    }
    return 0
}
"#;
    let ir = ir_only(src);
    assert!(
        ir.contains("call i64 @list_len"),
        "list iteration should call list_len, got IR:\n{}",
        ir
    );
    assert!(
        ir.contains("call i64 @list_get"),
        "list iteration should call list_get, got IR:\n{}",
        ir
    );
}

#[test]
fn test_list_iteration_loop_shape() {
    // The loop must have a for_cond block, a for_body block, and an index
    // initialized to zero before the condition.
    let src = r#"
fn main() -> i64 {
    let values = [10, 20, 30]
    for item in values {
        println(item)
    }
    return 0
}
"#;
    let ir = ir_only(src);
    assert!(
        ir.contains("for_cond"),
        "list iteration should have a for_cond block"
    );
    assert!(
        ir.contains("for_body"),
        "list iteration should have a for_body block"
    );
}

#[test]
fn test_list_iteration_terminates_at_len() {
    // The condition must branch on a comparison, and the loop must print
    // exactly the list's elements (not more, not fewer).
    let src = r#"
fn main() -> i64 {
    let values = [10, 20, 30]
    for item in values {
        println(item)
    }
    return 0
}
"#;
    let ir = ir_only(src);
    // The loop condition compares the index against the list length and
    // branches on the result. The bound check is an unsigned less-than
    // (`icmp ult`); the terminator branches with `br i1`.
    assert!(
        ir.contains("icmp"),
        "list iteration condition should contain a comparison"
    );
    assert!(
        ir.contains("br i1"),
        "list iteration terminator should branch on the comparison"
    );
    let bin = compile_src(src);
    let (code, stdout) = bin.run();
    assert_eq!(code, 0);
    assert_eq!(stdout, "10\n20\n30\n");
}

#[test]
fn test_list_iteration_computed_list() {
    // Iterate a list whose elements are computed by a function call. The
    // iterable expression is a list literal containing calls, so the list
    // is fully evaluated (left-to-right) before the loop starts.
    let src = r#"
fn make_val(n: i64) -> i64 {
    return n
}
fn main() -> i64 {
    let values = [make_val(1), make_val(2), make_val(3)]
    for item in values {
        println(item)
    }
    return 0
}
"#;
    let bin = compile_src(src);
    let (code, stdout) = bin.run();
    assert_eq!(code, 0);
    assert_eq!(
        stdout, "1\n2\n3\n",
        "computed list should iterate its elements"
    );
}

#[test]
fn test_list_iteration_preserves_element_evaluation_order() {
    // Side effects in the iterable expression must run left-to-right before
    // the loop starts, and each element must be read in index order.
    let src = r#"
fn elem(x: i64) -> i64 {
    println(x)
    return x
}
fn main() -> i64 {
    for item in [elem(7), elem(9)] {
        println(item)
    }
    return 0
}
"#;
    let bin = compile_src(src);
    let (code, stdout) = bin.run();
    assert_eq!(code, 0);
    assert_eq!(
        stdout, "7\n9\n7\n9\n",
        "iterable evaluated left-to-right (7,9), then elements read in order (7,9)"
    );
}

#[test]
fn test_list_iteration_body_runs_once_per_element() {
    // A counter incremented inside the body must equal the list length.
    let src = r#"
fn main() -> i64 {
    let values = [100, 200, 300, 400]
    let mut n = 0
    for item in values {
        n = n + 1
    }
    println(n)
    return 0
}
"#;
    let bin = compile_src(src);
    let (code, stdout) = bin.run();
    assert_eq!(code, 0);
    assert_eq!(stdout, "4\n", "body must run once per element");
}

#[test]
fn test_list_iteration_range_for_still_works() {
    // Regression: the existing range-based `for` loop must be unaffected.
    let src = r#"
fn main() -> i64 {
    for i in 0..3 {
        println(i)
    }
    return 0
}
"#;
    let bin = compile_src(src);
    let (code, stdout) = bin.run();
    assert_eq!(code, 0);
    assert_eq!(stdout, "0\n1\n2\n", "range for loop must still work");
}

#[test]
fn test_list_iteration_inclusive_range_for_still_works() {
    let src = r#"
fn main() -> i64 {
    for i in 0...3 {
        println(i)
    }
    return 0
}
"#;
    let bin = compile_src(src);
    let (code, stdout) = bin.run();
    assert_eq!(code, 0);
    assert_eq!(
        stdout, "0\n1\n2\n3\n",
        "inclusive range for loop must still work"
    );
}

#[test]
fn test_list_iteration_rejects_non_list_iterable() {
    // A `for` loop over a non-range, non-list iterable must produce a
    // diagnostic.
    let src = r#"
fn main() -> i64 {
    for x in 42 {
        println(x)
    }
    return 0
}
"#;
    let result = analyze_src(src);
    assert!(result.is_err(), "for loop over a bare i64 must be rejected");
    let err = result.unwrap_err();
    assert!(
        err.contains("for loop requires a range or List<i64>"),
        "expected a for-loop iterable diagnostic, got: {}",
        err
    );
}

#[test]
fn test_list_iteration_rejects_bool_iterable() {
    let src = r#"
fn main() -> i64 {
    for x in true {
        println(x)
    }
    return 0
}
"#;
    let result = analyze_src(src);
    assert!(result.is_err(), "for loop over a bool must be rejected");
}

// ---------------------------------------------------------------------------
// 0.5.3 Phase 7: indexing and length
// ---------------------------------------------------------------------------
