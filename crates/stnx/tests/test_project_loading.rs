//! Integration tests for the CLI compilation path through `Project::discover`.
//!
//! Phase 4B — tests only.  These tests verify that the end-to-end pipeline
//! used by `main.rs` (`Project::discover` + `load_from` + `analyze_and_lower`
//! + `lower_program` + `compile_from_mir_ext`) works correctly when driven
//!   from a real on-disk project layout.
//!
//! They complement `test_module_graph.rs` (which tests the module subsystem
//! in isolation) by exercising the **full** compile path: the `Program` AST
//!   returned by `load_from` flows through semantic analysis, MIR lowering,
//!   verification + optimization, and LLVM codegen back to a working executable.
//!
//! Every test builds its own isolated `tempfile::TempDir` so parallel test
//! execution never collides on fixed filenames.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use stnx::mir::lower::lower_program;
use stnx::mir::opt::optimize;
use stnx::module::Project;
use stnx::target::{OutputKind, TargetConfig};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers (mirror the helpers in test_module_graph.rs / common/mod.rs)
// ---------------------------------------------------------------------------

/// Write a file at `dir/<rel>` with the given contents, creating parent dirs.
fn write_file(dir: &Path, rel: &str, contents: &str) -> PathBuf {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("failed to create parent dir");
    }
    fs::write(&path, contents)
        .unwrap_or_else(|e| panic!("failed to write {}: {}", path.display(), e));
    path
}

/// Write a minimal `saturn.toml` into the given project directory.
fn write_saturn_toml(dir: &Path, name: &str, version: &str, edition: &str) {
    let toml = format!(
        r#"[package]
name = "{name}"
version = "{version}"
edition = "{edition}"
"#
    );
    write_file(dir, "saturn.toml", &toml);
}

/// A minimal valid Saturnite program that calls `println(42)` and returns 0.
///
/// Note: Saturnite uses space/newline as the statement separator (no semicolons),
/// matching the format used throughout the existing test suite.
fn println_program() -> &'static str {
    "fn main() -> i64 { println(42) return 0 }"
}

/// Drive the full production compilation seam starting from a `Program` AST.
///
/// This mirrors the exact sequence that `main.rs` performs after
/// `Project::load_from` returns its `Program`:
///
/// 1. `analyze_and_lower`  — AST → HIR (semantic analysis + typed lowering)
/// 2. `lower_program`       — HIR → MIR (CFG construction)
/// 3. `mir.verify()`         — MIR CFG sanity checks
/// 4. `optimize`            — MIR-level optimization
/// 5. `compile_from_mir_ext` — MIR → LLVM → object → link → executable
fn compile_program_to_exe(program: &stnx::ast::Program, exe_path: &Path) {
    let hir = stnx::semantic::analyze_and_lower(program).expect("semantic analysis failed");

    let mut mir = lower_program(&hir).expect("MIR lowering failed");

    if let Err(errs) = mir.verify() {
        let msgs: Vec<String> = errs.iter().map(|e| e.to_string()).collect();
        panic!("MIR verification failed: {}", msgs.join(", "));
    }
    optimize(&mut mir);

    let mut config = TargetConfig::host().expect("target init failed");
    config.set_output_kind(OutputKind::Exe);
    stnx::mir::codegen::compile_from_mir_ext(&mir, exe_path.to_str().unwrap(), config, false)
        .expect("codegen/linking failed");
}

// ---------------------------------------------------------------------------
// Test 1 — Single-file compilation via the Project path
// ---------------------------------------------------------------------------

#[test]
fn test_project_loading_single_file_executable() {
    let tmp = TempDir::new().expect("failed to create isolated temp dir");
    let root = tmp.path();

    // Lay out a real project: saturn.toml + src/main.stnx.
    write_saturn_toml(root, "proj_single", "0.1.0", "2026");
    let main_file = write_file(root, "src/main.stnx", println_program());

    // Mirror the main.rs driver exactly: discover then load_from.
    let mut project = Project::discover(&main_file).expect("Project::discover should succeed");
    let program = project
        .load_from(&main_file)
        .expect("Project::load_from should succeed");

    // The returned AST should contain the `main` function.
    let has_main = program.functions.iter().any(|f| f.name.as_str() == "main");
    assert!(has_main, "loaded program should contain a `main` function");

    // Compile through the full production seam and run the resulting binary.
    let exe_path = root.join("program");
    compile_program_to_exe(&program, &exe_path);

    let result = std::process::Command::new(&exe_path)
        .output()
        .expect("failed to execute compiled binary");
    let stdout = String::from_utf8_lossy(&result.stdout);

    assert_eq!(result.status.code(), Some(0), "executable should exit 0");
    assert_eq!(stdout.trim(), "42", "println(42) should print 42");
}

// ---------------------------------------------------------------------------
// Test 2 — Project discovery from a subdirectory finds the real root
// ---------------------------------------------------------------------------

#[test]
fn test_project_discovery_from_subdirectory_full_pipeline() {
    let tmp = TempDir::new().expect("failed to create isolated temp dir");
    let root = tmp.path();

    write_saturn_toml(root, "subdir_project", "0.2.0", "2026");
    let main_file = write_file(root, "src/main.stnx", println_program());

    // Create a file located in a subdirectory of the source root so that
    // Project::discover must walk upward to locate the real project root.
    let subdir = root.join("tests").join("sub");
    fs::create_dir_all(&subdir).expect("failed to create subdir");

    let deep_file = subdir.join("file.stnx");
    fs::write(&deep_file, println_program()).expect("failed to write deep file");

    // discover() should walk up from deep_file and stop at `root`
    // (where saturn.toml lives).
    let project = Project::discover(&deep_file).expect("discovery from subdir should succeed");
    assert_eq!(
        project.root, root,
        "project root should be the dir containing saturn.toml"
    );
    assert_eq!(project.config.package.name, "subdir_project");

    // load_from on the *main* entry (passed as an absolute path) should
    // load the root module AST.
    let mut project = project;
    let program = project
        .load_from(&main_file)
        .expect("load_from on main entry should succeed");

    let exe_path = root.join("subdir_program");
    compile_program_to_exe(&program, &exe_path);

    let result = std::process::Command::new(&exe_path)
        .output()
        .expect("failed to execute compiled binary");
    let stdout = String::from_utf8_lossy(&result.stdout);

    assert_eq!(result.status.code(), Some(0), "executable should exit 0");
    assert_eq!(stdout.trim(), "42", "println(42) should print 42");
}

// ---------------------------------------------------------------------------
// Test 3 — Project discovery without saturn.toml synthesizes a config
// ---------------------------------------------------------------------------

#[test]
fn test_project_discovery_no_saturn_toml_full_pipeline() {
    let tmp = TempDir::new().expect("failed to create isolated temp dir");
    let root = tmp.path();

    // No saturn.toml — discover() should synthesize a config from the starting
    // directory name and still work end-to-end.  We place main.stnx directly in
    // `root` (not in `src/`) so that `discover` from the file path uses `root`
    // as the start directory and synthesizes the package name from the temp
    // dir's random name.
    let main_file = write_file(root, "main.stnx", println_program());

    let mut project = Project::discover(&main_file).expect("synthesized discovery should succeed");
    // The synthesized name comes from the directory name (TempDir's random name).
    let dir_name = root.file_name().unwrap().to_str().unwrap();
    assert_eq!(
        project.config.package.name, dir_name,
        "synthesized config name should match directory name"
    );
    assert_eq!(project.config.package.version, "0.1.0");
    assert_eq!(project.config.package.edition, "2026");

    let program = project
        .load_from(&main_file)
        .expect("load_from should succeed with synthesized config");

    let exe_path = root.join("synth_program");
    compile_program_to_exe(&program, &exe_path);

    let result = std::process::Command::new(&exe_path)
        .output()
        .expect("failed to execute compiled binary");
    let stdout = String::from_utf8_lossy(&result.stdout);

    assert_eq!(result.status.code(), Some(0), "executable should exit 0");
    assert_eq!(stdout.trim(), "42", "println(42) should print 42");
}
