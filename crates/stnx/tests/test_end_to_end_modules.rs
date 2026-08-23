//! End-to-end module integration tests (Phase 9B).
//!
//! These tests verify the full compilation pipeline for multi-file Saturnite
//! projects that use the module system:
//!
//! 1. **Cross-module call via `use`** — the root module declares `mod child`,
//!    imports `use child::helper`, and calls `helper()` from `main()`. The
//!    helper function is defined in `src/child.stnx`.
//!
//! 2. **Mutual cross-module calls** — the root module calls a function in the
//!    child module, and the child module calls a function in the root module,
//!    exercising bidirectional cross-module references.
//!
//! Pipeline exercised (mirrors `main.rs` for the module path):
//!
//! 1. `Project::discover` — locate the project root and parse `saturn.toml`.
//! 2. `Project::load()` — discover the module graph and return the root `Program`.
//! 3. `analyze_and_lower_with_graph(&program, &graph)` — AST → HIR with
//!    multi-module support (resolves `mod` and `use` declarations).
//! 4. `lower_program(&hir)` — HIR → MIR (CFG construction).
//! 5. `mir.verify()` — MIR CFG sanity checks.
//! 6. `optimize` — MIR-level optimization (constant folding).
//! 7. `compile_from_mir_ext` — MIR → LLVM → object → link → executable.
//!
//! Each test writes its source into an isolated `TempDir`, compiles, runs the
//! resulting binary, and asserts that stdout matches the expected output.

use std::fs;
use std::path::{Path, PathBuf};
use stnx::mir::lower::lower_program;
use stnx::mir::opt::optimize;
use stnx::module::Project;
use stnx::target::{OutputKind, TargetConfig};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers (mirror the helpers in test_project_loading.rs)
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
fn write_saturn_toml(dir: &Path, name: &str) {
    let toml = format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2026"

[dependencies]
"#
    );
    write_file(dir, "saturn.toml", &toml);
}

/// Drive the full production compilation seam from a HIR program produced by
/// `analyze_and_lower_with_graph`.
///
/// This mirrors the exact sequence that `main.rs` performs after
/// `Project::load` returns its `Program` + `ModuleGraph`:
///
/// 1. `analyze_and_lower_with_graph`  — AST + graph → HIR (semantic analysis
///    + typed lowering + `use`/`mod` resolution).
/// 2. `lower_program`                  — HIR → MIR (CFG construction).
/// 3. `mir.verify()`                   — MIR CFG sanity checks.
/// 4. `optimize`                       — MIR-level optimization.
/// 5. `compile_from_mir_ext`           — MIR → LLVM → object → link → executable.
fn compile_with_graph(
    program: &stnx::ast::Program,
    graph: &stnx::module::ModuleGraph,
    exe_path: &Path,
) {
    let hir = stnx::semantic::analyze_and_lower_with_graph(program, graph)
        .expect("semantic analysis with graph failed");

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
// Test 1 — Root calls a function defined in a child module (via `use`)
// ---------------------------------------------------------------------------

/// Layout:
///
///   src/main.stnx:
///     mod child
///     use child::helper
///     fn main() -> i64 { println(helper(21)) return 0 }
///
///   src/child.stnx:
///     fn helper(x) -> i64 { x + 21 }
///
/// Expected output: `42` (21 + 21)
#[test]
fn test_end_to_end_cross_module_call_via_use() {
    let tmp = TempDir::new().expect("failed to create isolated temp dir");
    let root = tmp.path();

    // Project scaffolding.
    write_saturn_toml(root, "cross_module_use");
    let main_file = write_file(
        root,
        "src/main.stnx",
        "mod child\nuse child::helper\nfn main() -> i64 { println(helper(21)) return 0 }",
    );
    write_file(
        root,
        "src/child.stnx",
        "fn helper(x: i64) -> i64 { x + 21 }",
    );

    // Discover + load the project (mirrors main.rs driver).
    let mut project = Project::discover(&main_file).expect("Project::discover should succeed");
    let program = project.load().expect("Project::load should succeed");
    let graph = &project.graph;

    // The graph should contain two modules: root + child.
    assert_eq!(graph.len(), 2, "graph should contain root + child modules");

    // Compile through the full production seam and run the binary.
    let exe_path = root.join("cross_module_use");
    compile_with_graph(&program, graph, &exe_path);

    let result = std::process::Command::new(&exe_path)
        .output()
        .expect("failed to execute compiled binary");
    let stdout = String::from_utf8_lossy(&result.stdout);

    assert_eq!(result.status.code(), Some(0), "executable should exit 0");
    assert_eq!(stdout.trim(), "42", "helper(21) should print 42");
}

// ---------------------------------------------------------------------------
// Test 2 — Mutual cross-module calls (root → child → root)
// ---------------------------------------------------------------------------

/// Layout:
///
///   src/main.stnx:
///     mod child
///     use child::compute
///     fn root_value() -> i64 { 100 }
///     fn main() -> i64 { println(compute()) return 0 }
///
///   src/child.stnx:
///     fn compute() -> i64 { root_value() + 5 }
///
/// `root_value` is defined in the root module and called from the child module,
/// while `compute` is defined in the child and called from the root module
/// (imported via `use child::compute`). All function names are globally visible
/// through the HIR function-signature table, so the child can call `root_value`
/// by name directly.
///
/// Expected output: `105` (100 + 5)
#[test]
fn test_end_to_end_mutual_cross_module_calls() {
    let tmp = TempDir::new().expect("failed to create isolated temp dir");
    let root = tmp.path();

    write_saturn_toml(root, "mutual_calls");
    let main_file = write_file(
        root,
        "src/main.stnx",
        "mod child\nuse child::compute\nfn root_value() -> i64 { 100 }\nfn main() -> i64 { println(compute()) return 0 }",
    );
    write_file(
        root,
        "src/child.stnx",
        "fn compute() -> i64 { root_value() + 5 }",
    );

    // Discover + load the project.
    let mut project = Project::discover(&main_file).expect("Project::discover should succeed");
    let program = project.load().expect("Project::load should succeed");
    let graph = &project.graph;

    assert_eq!(graph.len(), 2, "graph should contain root + child modules");

    // Compile through the full production seam and run the binary.
    let exe_path = root.join("mutual_calls");
    compile_with_graph(&program, graph, &exe_path);

    let result = std::process::Command::new(&exe_path)
        .output()
        .expect("failed to execute compiled binary");
    let stdout = String::from_utf8_lossy(&result.stdout);

    assert_eq!(result.status.code(), Some(0), "executable should exit 0");
    assert_eq!(stdout.trim(), "105", "root_value() + 5 should print 105");
}
