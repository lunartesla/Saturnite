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
