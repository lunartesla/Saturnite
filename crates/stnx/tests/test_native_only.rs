//! Tests native target initialization + executable generation with an
//! isolated temp directory.

mod common;

use common::compile_src;

#[test]
fn test_native_compile_only() {
    let bin = compile_src("fn main() -> i64 { return 42 }");
    assert!(bin.path().exists(), "executable should be created");

    let (exit_code, _) = bin.run();
    assert_eq!(exit_code, 42);
}
