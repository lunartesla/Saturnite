//! Shared helpers for integration tests.
//!
//! Every test that produces an on-disk artifact (executable, object file)
//! uses an isolated [`tempfile::TempDir`] so that parallel test execution
//! never collides on fixed filenames.  The [`TempDir`] is kept alive inside
//! the returned handle for the lifetime of the test.
//!
// Each integration test binary links this module but only uses a subset of the
// helpers, so suppress `dead_code` at the module level.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use stnx::codegen;
use stnx::lexer::Lexer;
use stnx::parser;
use stnx::semantic::analyze;
use stnx::target::{OutputKind, TargetConfig};
use tempfile::TempDir;

/// An on-disk artifact (executable or object file) living inside an isolated
/// temp directory.  The directory is kept alive so the file persists for the
/// duration of the test.
pub struct Artifact {
    pub path: PathBuf,
    _temp_dir: TempDir,
}

impl Artifact {
    /// Execute `self.path` as a fresh subprocess and capture (exit code, stdout).
    /// Only meaningful when the artifact is an executable.
    pub fn run(&self) -> (i32, String) {
        let result = Command::new(&self.path)
            .output()
            .unwrap_or_else(|e| panic!("failed to execute {}: {}", self.path.display(), e));
        let stdout = String::from_utf8_lossy(&result.stdout).to_string();
        let exit_code = result.status.code().unwrap_or(-1);
        (exit_code, stdout)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Full pipeline: lex -> parse -> analyze -> codegen -> link into an executable.
/// Everything happens inside an isolated temp directory.
pub fn compile_src(src: &str) -> Artifact {
    let temp_dir = TempDir::new().expect("failed to create isolated temp dir");
    let exe_path = temp_dir.path().join("program");

    let tokens: Vec<_> = Lexer::new(src)
        .collect::<Result<Vec<_>, _>>()
        .expect("lexing failed");
    let program = parser::parse(src, tokens).expect("parsing failed");
    analyze(&program).expect("semantic analysis failed");

    let mut config = TargetConfig::host().expect("target init failed");
    config.set_output_kind(OutputKind::Exe);
    codegen::compile_with_target(&program, exe_path.to_str().unwrap(), config)
        .expect("codegen/linking failed");

    Artifact {
        path: exe_path,
        _temp_dir: temp_dir,
    }
}

/// Compile to a relocatable object file (.o) in an isolated temp directory.
pub fn compile_to_object(src: &str) -> Artifact {
    let temp_dir = TempDir::new().expect("failed to create isolated temp dir");
    let obj_path = temp_dir.path().join("program.o");

    let tokens: Vec<_> = Lexer::new(src)
        .collect::<Result<Vec<_>, _>>()
        .expect("lexing failed");
    let program = parser::parse(src, tokens).expect("parsing failed");
    analyze(&program).expect("semantic analysis failed");

    let mut config = TargetConfig::host().expect("target init failed");
    config.set_output_kind(OutputKind::Object);
    codegen::compile_with_target(&program, obj_path.to_str().unwrap(), config)
        .expect("codegen failed");

    Artifact {
        path: obj_path,
        _temp_dir: temp_dir,
    }
}

/// Generate LLVM IR text only (no file I/O, no linking).
pub fn ir_only(src: &str) -> String {
    let tokens: Vec<_> = Lexer::new(src)
        .collect::<Result<Vec<_>, _>>()
        .expect("lexing failed");
    let program = parser::parse(src, tokens).expect("parsing failed");
    analyze(&program).expect("semantic analysis failed");
    codegen::generate_ir(&program).expect("IR generation failed")
}

/// Full analysis that may fail — used by diagnostics / negative tests.
pub type AnalysisResult = Result<(), String>;

/// Lex -> parse -> analyze, converting errors to plain strings.
pub fn analyze_src(src: &str) -> AnalysisResult {
    let tokens: Vec<_> = Lexer::new(src)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Lex error: {}", e))?;
    let program = parser::parse(src, tokens).map_err(|e| format!("Parse error: {}", e))?;
    analyze(&program).map_err(|e| format!("Semantic error: {}", e))
}

/// Read a file to a string, panicking with context on failure.
pub fn read_file(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", path.display(), e))
}

/// Assert that a file exists and is non-empty.
pub fn assert_file_exists(path: &Path) {
    assert!(
        path.exists(),
        "expected file does not exist: {}",
        path.display()
    );
    assert!(
        std::fs::metadata(path)
            .map(|m| m.len() > 0)
            .unwrap_or(false),
        "expected file to be non-empty: {}",
        path.display()
    );
}
