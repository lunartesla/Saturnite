//! End-to-end tests for 0.5.1 runtime string interpolation.
//!
//! Covers: single/multiple interpolated segments, interpolation at the
//! start/middle/end of a string, numeric and text segments, no-interpolation
//! regression, IR-level verification that runtime concatenation really
//! happens, and the compile-time diagnostic for unsupported types.

mod common;

use common::{analyze_src, compile_src, ir_only};

/// Full pipeline: compile an interpolated program and run it, returning
/// (exit_code, stdout).
fn run_src(src: &str) -> (i32, String) {
    compile_src(src).run()
}

#[test]
fn test_interpolated_say_basic() {
    // STEP 5: the regression case from the spec.
    let (code, out) = run_src(
        "fn main() -> i64 { \
         let name = \"Saturnite\" \
         say \"Hello {name}!\" \
         return 0 }",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "Hello Saturnite!\n");
}

#[test]
fn test_interpolation_two_variables() {
    let (code, out) = run_src(
        "fn main() -> i64 { \
         let first = \"Ada\" \
         let last = \"Lovelace\" \
         say \"Hello {first} {last}!\" \
         return 0 }",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "Hello Ada Lovelace!\n");
}

#[test]
fn test_interpolation_prefix_name_age() {
    let (code, out) = run_src(
        "fn main() -> i64 { \
         let prefix = \"user\" \
         let name = \"Saturnite\" \
         let age = 15 \
         say \"{prefix}: {name} has {age} years\" \
         return 0 }",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "user: Saturnite has 15 years\n");
}

#[test]
fn test_interpolation_number_only_text() {
    let (code, out) = run_src(
        "fn main() -> i64 { \
         let age = 15 \
         say \"Age: {age}\" \
         return 0 }",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "Age: 15\n");
}

#[test]
fn test_no_interpolation_regression() {
    // Plain string literals must keep working unchanged.
    let (code, out) = run_src(
        "fn main() -> i64 { \
         say \"Hello world\" \
         return 0 }",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "Hello world\n");
}

#[test]
fn test_interpolation_only_expr() {
    let (code, out) = run_src(
        "fn main() -> i64 { \
         let name = \"world\" \
         say \"{name}\" \
         return 0 }",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "world\n");
}

#[test]
fn test_interpolation_at_beginning() {
    let (code, out) = run_src(
        "fn main() -> i64 { \
         let name = \"Ada\" \
         say \"{name} says hello\" \
         return 0 }",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "Ada says hello\n");
}

#[test]
fn test_interpolation_at_end() {
    let (code, out) = run_src(
        "fn main() -> i64 { \
         let name = \"Ada\" \
         say \"Hello {name}\" \
         return 0 }",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "Hello Ada\n");
}

#[test]
fn test_multiple_adjacent_interpolations() {
    let (code, out) = run_src(
        "fn main() -> i64 { \
         let a = \"a\" \
         let b = \"b\" \
         let c = \"c\" \
         say \"{a}{b}{c}\" \
         return 0 }",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "abc\n");
}

#[test]
fn test_multiple_literal_segments() {
    let (code, out) = run_src(
        "fn main() -> i64 { \
         let name = \"Kai\" \
         let place = \"Saturn\" \
         say \"hello {name}, welcome to {place}!\" \
         return 0 }",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "hello Kai, welcome to Saturn!\n");
}

#[test]
fn test_interpolated_raise_prints_message() {
    // `raise` in 0.5 prints the message and then hits an Unreachable terminator.
    // Real abort semantics are deferred; we only verify that the interpolated
    // string reaches the printer correctly.
    let (_code, out) = run_src(
        "fn main() -> i64 { \
         let x = \"value\" \
         raise \"bad {x}\" \
         return 0 }",
    );
    assert_eq!(out, "bad value\n");
}

#[test]
fn test_ir_contains_runtime_concatenation() {
    // The generated IR must actually call the runtime `concat_str`, and must
    // NOT hard-code the final interpolated output.
    let ir = ir_only(
        "fn main() -> i64 { \
         let name = \"Saturnite\" \
         say \"Hello {name}!\" \
         return 0 }",
    );
    assert!(
        ir.contains("concat_str"),
        "IR should call the runtime concat_str:\n{}",
        ir
    );
    // The output literal must not be baked in as a single combined string.
    assert!(
        !ir.contains("Hello Saturnite!"),
        "IR must not hard-code the interpolated result:\n{}",
        ir
    );
}

#[test]
fn test_unsupported_interpolation_type_is_a_diagnostic() {
    // A boolean has no Saturnite string conversion in 0.5.1; this must be a
    // compile-time error, never a silent miscompile.
    let result = analyze_src(
        "fn main() -> i64 { \
         let flag = true \
         say \"value {flag}\" \
         return 0 }",
    );
    assert!(result.is_err(), "bool interpolation should be rejected");
    if let Err(e) = result {
        assert!(
            e.contains("cannot render"),
            "expected a clear interpolation diagnostic, got: {}",
            e
        );
    }
}
