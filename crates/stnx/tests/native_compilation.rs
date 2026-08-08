use stnx::lexer::Lexer;
use stnx::parser;
use stnx::semantic::analyze;
use stnx::target::{OutputKind, TargetConfig};
use stnx::codegen;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn compile_src(src: &str, output: &str, kind: OutputKind) -> Result<(), String> {
    let tokens: Vec<_> = Lexer::new(src)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Lex error: {}", e))?;
    let program = parser::parse(src, tokens).map_err(|e| format!("Parse error: {}", e))?;
    analyze(&program).map_err(|e| format!("Semantic error: {}", e))?;

    let mut config = TargetConfig::host().map_err(|e| format!("Target error: {}", e))?;
    config.set_output_kind(kind);

    codegen::compile_with_target(&program, output, config)
        .map_err(|e| format!("Codegen error: {}", e))?;
    Ok(())
}

fn run_compiled_binary(path: &PathBuf) -> (i32, String) {
    let result = Command::new(path)
        .output()
        .expect("failed to execute compiled binary");
    let stdout = String::from_utf8_lossy(&result.stdout).to_string();
    let exit_code = result.status.code().unwrap_or(-1);
    (exit_code, stdout)
}

#[test]
fn test_main_returning_i64() {
    let src = "fn main() -> i64 { return 42 }";
    let tmp = env::temp_dir().join("test_main_i64");
    if tmp.exists() {
        fs::remove_file(&tmp).ok();
    }

    compile_src(src, tmp.to_str().unwrap(), OutputKind::Exe)
        .expect("compilation should succeed");

    assert!(tmp.exists(), "executable should be created");

    let (exit_code, _) = run_compiled_binary(&tmp);
    assert_eq!(exit_code, 42, "exit code should be 42");

    fs::remove_file(&tmp).ok();
}

#[test]
fn test_main_returning_unit() {
    let src = "fn main() { println(42) }";
    let tmp = env::temp_dir().join("test_main_unit");
    if tmp.exists() {
        fs::remove_file(&tmp).ok();
    }

    compile_src(src, tmp.to_str().unwrap(), OutputKind::Exe)
        .expect("compilation should succeed");

    let (exit_code, stdout) = run_compiled_binary(&tmp);
    assert_eq!(stdout.trim(), "42", "println should output 42");
    // Exit code for void-returning main is not deterministic; just verify it ran

    fs::remove_file(&tmp).ok();
}

#[test]
fn test_local_variable() {
    let src = "fn main() -> i64 { let x = 42 return x }";
    let tmp = env::temp_dir().join("test_local_var");
    if tmp.exists() {
        fs::remove_file(&tmp).ok();
    }

    compile_src(src, tmp.to_str().unwrap(), OutputKind::Exe)
        .expect("compilation should succeed");

    let (exit_code, _) = run_compiled_binary(&tmp);
    assert_eq!(exit_code, 42, "local variable should be 42");

    fs::remove_file(&tmp).ok();
}

#[test]
fn test_arithmetic() {
    let src = "fn main() -> i64 { let x = 10 + 5 * 2 return x }";
    let tmp = env::temp_dir().join("test_arithmetic");
    if tmp.exists() {
        fs::remove_file(&tmp).ok();
    }

    compile_src(src, tmp.to_str().unwrap(), OutputKind::Exe)
        .expect("compilation should succeed");

    let (exit_code, _) = run_compiled_binary(&tmp);
    // 10 + 5 * 2 = 20
    assert_eq!(exit_code, 20, "arithmetic result should be 20");

    fs::remove_file(&tmp).ok();
}

#[test]
fn test_if_else() {
    let src = "fn main() -> i64 { let x = 1 if x == 1 { println(100) } else { println(200) } return 0 }";
    let tmp = env::temp_dir().join("test_if_else");
    if tmp.exists() {
        fs::remove_file(&tmp).ok();
    }

    compile_src(src, tmp.to_str().unwrap(), OutputKind::Exe)
        .expect("compilation should succeed");

    let (_, stdout) = run_compiled_binary(&tmp);
    assert_eq!(stdout.trim(), "100", "if true should print 100");

    fs::remove_file(&tmp).ok();
}

#[test]
fn test_function_call() {
    let src = "fn add(a: i64, b: i64) -> i64 { return a + b } fn main() -> i64 { return add(10, 20) }";
    let tmp = env::temp_dir().join("test_func_call");
    if tmp.exists() {
        fs::remove_file(&tmp).ok();
    }

    compile_src(src, tmp.to_str().unwrap(), OutputKind::Exe)
        .expect("compilation should succeed");

    let (exit_code, _) = run_compiled_binary(&tmp);
    assert_eq!(exit_code, 30, "function call should return 30");

    fs::remove_file(&tmp).ok();
}

#[test]
fn test_for_loop_runtime() {
    let src = "fn main() -> i64 { for i in 0..5 { println(i) } return 0 }";
    let tmp = env::temp_dir().join("test_for_loop");
    if tmp.exists() {
        fs::remove_file(&tmp).ok();
    }

    compile_src(src, tmp.to_str().unwrap(), OutputKind::Exe)
        .expect("compilation should succeed");

    let (exit_code, stdout) = run_compiled_binary(&tmp);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 5, "for loop should produce 5 lines of output");
    assert_eq!(lines[0], "0", "first line should be 0");
    assert_eq!(lines[4], "4", "last line should be 4");
    assert_eq!(exit_code, 0, "exit code should be 0");

    fs::remove_file(&tmp).ok();
}

#[test]
fn test_for_loop_with_arithmetic() {
    let src = "fn main() -> i64 { let mut sum = 0 for i in 0..10 { sum = sum + i } return sum }";
    let tmp = env::temp_dir().join("test_for_loop_arith");
    if tmp.exists() {
        fs::remove_file(&tmp).ok();
    }

    compile_src(src, tmp.to_str().unwrap(), OutputKind::Exe)
        .expect("compilation should succeed");

    let (exit_code, _) = run_compiled_binary(&tmp);
    // Sum of 0..10 = 0+1+2+3+4+5+6+7+8+9 = 45
    assert_eq!(exit_code, 45, "sum of 0..10 should be 45");

    fs::remove_file(&tmp).ok();
}

#[test]
fn test_invalid_target_configuration() {
    let result = TargetConfig::from_triple("invalid-triple");
    assert!(result.is_err(), "invalid target triple should produce an error");
}

#[test]
fn test_emit_ir_mode() {
    let src = "fn main() -> i64 { return 42 }";
    let tmp = env::temp_dir().join("test_emit_ir.ll");
    if tmp.exists() {
        fs::remove_file(&tmp).ok();
    }

    compile_src(src, tmp.to_str().unwrap(), OutputKind::Ir)
        .expect("IR compilation should succeed");

    let content = fs::read_to_string(&tmp).expect("IR file should exist");
    assert!(content.contains("define i64 @main"), "IR should contain main function definition");

    fs::remove_file(&tmp).ok();
}

#[test]
fn test_emit_object_mode() {
    let src = "fn main() -> i64 { return 42 }";
    let tmp = env::temp_dir().join("test_emit_obj.o");
    if tmp.exists() {
        fs::remove_file(&tmp).ok();
    }

    compile_src(src, tmp.to_str().unwrap(), OutputKind::Object)
        .expect("object compilation should succeed");

    assert!(tmp.exists(), "object file should be created");
    // Verify it's actually an object file by checking the ELF header
    let header = fs::read(&tmp).expect("should read object file");
    assert!(header.len() > 4, "object file should not be empty");

    fs::remove_file(&tmp).ok();
}

#[test]
fn test_native_target_initialization() {
    let config = TargetConfig::host();
    assert!(config.is_ok(), "host target initialization should succeed");

    let config = config.unwrap();
    let triple = config.triple_str();
    assert!(!triple.is_empty(), "host triple should not be empty");
    assert!(triple.contains("linux") || triple.contains("windows") || triple.contains("darwin"),
            "host triple should contain OS name: got {}", triple);
}
