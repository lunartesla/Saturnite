use stnx::lexer::Lexer;
use stnx::parser;
use stnx::semantic::analyze;
use stnx::codegen::compile_to_executable;
use std::env;
use std::fs;
use std::process::Command;

#[test]
fn test_full_compile() {
    let src = "fn main() -> i64 { return 42 }";
    let tokens: Vec<_> = Lexer::new(src).collect::<Result<Vec<_>, _>>().unwrap();
    let program = parser::parse(src, tokens).unwrap();
    analyze(&program).unwrap();

    let tmp = env::temp_dir().join("test_full_compile");
    if tmp.exists() {
        fs::remove_file(&tmp).ok();
    }

    println!("About to compile...");
    compile_to_executable(&program, tmp.to_str().unwrap()).expect("compile should succeed");
    println!("Compilation done!");
    
    // Run in a subprocess to avoid segfault during LLVM cleanup
    let result = Command::new(&tmp).output().expect("failed to execute");
    println!("Exit code: {:?}", result.status.code());
    fs::remove_file(&tmp).ok();
}
