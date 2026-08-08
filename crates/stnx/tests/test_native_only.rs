use stnx::lexer::Lexer;
use stnx::parser;
use stnx::semantic::analyze;
use stnx::codegen::compile_to_executable;
use stnx::target::TargetConfig;
use std::env;
use std::path::PathBuf;
use std::process::Command;
use std::fs;

#[test]
fn test_native_compile_only() {
    let src = "fn main() -> i64 { return 42 }";
    let tokens: Vec<_> = Lexer::new(src).collect::<Result<Vec<_>, _>>().unwrap();
    let program = parser::parse(src, tokens).unwrap();
    analyze(&program).unwrap();

    // Initialize native target first
    let config = TargetConfig::host().unwrap();
    println!("Triple: {}", config.triple_str());

    let tmp = env::temp_dir().join("test_native_compile");
    if tmp.exists() {
        fs::remove_file(&tmp).ok();
    }

    compile_to_executable(&program, tmp.to_str().unwrap()).expect("compile should succeed");

    assert!(tmp.exists(), "executable should be created");

    let result = Command::new(&tmp).output().expect("failed to execute");
    println!("Exit code: {:?}", result.status.code());
    fs::remove_file(&tmp).ok();
}
