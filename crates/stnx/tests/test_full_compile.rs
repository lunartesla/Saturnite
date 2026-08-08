//! Tests the `compile_to_executable` convenience function end-to-end with an
//! isolated temp directory.

mod common;

use common::compile_src;

#[test]
fn test_full_compile() {
    // compiler success: the program builds; runtime success: exit code 42
    let bin = compile_src("fn main() -> i64 { return 42 }");
    assert!(bin.path().exists(), "executable should be created");

    let (exit_code, _) = bin.run();
    assert_eq!(exit_code, 42, "exit code should be 42");
}
