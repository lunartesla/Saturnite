//! Integration tests for first-class Rust and Python ecosystem interoperability.
//!
//! These tests exercise the full path from `use rust:crate` / `use python:module`
//! through config parsing, qualified call syntax, HIR lowering with wrapper
//! metadata, and MIR generation.

mod common;

use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

use stnx::interop::DependencyKind;
use stnx::config::SaturnConfig;
use stnx::parser;
use stnx::lexer::prepare;
use stnx::ast::ExternalKind;

/// Write a wrapper metadata JSON file into `target/saturnite/wrappers/`
/// inside the given temp directory.
fn setup_wrapper_metadata(temp: &TempDir, crate_or_module: &str, metadata_json: &str) {
    let wrapper_dir = temp
        .path()
        .join("target")
        .join("saturnite")
        .join("wrappers");
    fs::create_dir_all(&wrapper_dir).unwrap();
    fs::write(
        wrapper_dir.join(format!("{}_saturnite.json", crate_or_module)),
        metadata_json,
    )
    .unwrap();
}

/// Run the full lex → parse → HIR lowering pipeline with a project root
/// so ecosystem imports pick up wrapper metadata.
fn lower_with_root(src: &str, project_root: &PathBuf) -> stnx::hir::HirProgram {
    let tokens = prepare(src).expect("prepare failed");
    let program = parser::parse(src, tokens).expect("parsing failed");
    let mut lower = stnx::hir::HirLower::new().with_project_root(project_root.clone());
    lower.lower_program(&program).expect("HIR lowering failed")
}

// ---------------------------------------------------------------------------
// Config parsing tests
// ---------------------------------------------------------------------------

#[test]
fn test_config_ecosystem_dependency_parsing() {
    let toml_content = r#"
[package]
name = "myapp"
version = "0.1.0"

[dependencies]
serde = { kind = "rust", version = "1.0" }
numpy = { kind = "python", version = "1.24" }
sqlite = { kind = "native", version = "3.40" }
myutil = "0.1"
"#;

    let config = SaturnConfig::from_toml_str(toml_content).unwrap();

    assert_eq!(config.dependencies.len(), 4);

    let serde_dep = config.dependencies.get("serde").unwrap();
    assert_eq!(serde_dep.kind(), DependencyKind::Rust);
    assert_eq!(serde_dep.version(), "1.0");

    let numpy_dep = config.dependencies.get("numpy").unwrap();
    assert_eq!(numpy_dep.kind(), DependencyKind::Python);
    assert_eq!(numpy_dep.version(), "1.24");

    let sqlite_dep = config.dependencies.get("sqlite").unwrap();
    assert_eq!(sqlite_dep.kind(), DependencyKind::Native);
    assert_eq!(sqlite_dep.version(), "3.40");

    let myutil_dep = config.dependencies.get("myutil").unwrap();
    assert_eq!(myutil_dep.kind(), DependencyKind::Saturnite);
    assert_eq!(myutil_dep.version(), "0.1");
}

#[test]
fn test_config_dependency_struct_with_target() {
    let toml_content = r#"
[package]
name = "tgt"

[dependencies]
foo = { kind = "rust", version = "1.0", target = "linux" }
"#;

    let config = SaturnConfig::from_toml_str(toml_content).unwrap();
    let dep = config.dependencies.get("foo").unwrap();
    assert_eq!(dep.kind(), DependencyKind::Rust);
    assert_eq!(dep.version(), "1.0");
    assert_eq!(dep.target(), Some("linux"));
}

#[test]
fn test_config_simple_dependency_defaults_to_saturnite() {
    let toml_content = r#"
[package]
name = "simple"

[dependencies]
mydep = "0.1"
"#;

    let config = SaturnConfig::from_toml_str(toml_content).unwrap();
    let dep = config.dependencies.get("mydep").unwrap();
    assert_eq!(dep.kind(), DependencyKind::Saturnite);
    assert_eq!(dep.version(), "0.1");
    assert_eq!(dep.target(), None);
}

// ---------------------------------------------------------------------------
// Qualified call parsing tests
// ---------------------------------------------------------------------------

#[test]
fn test_qualified_call_parses_without_errors() {
    let src = r#"
use rust:example_crate as rust_lib

fn main() -> i64 {
    let x = rust_lib.add(1, 2)
    0
}
"#;
    let tokens = prepare(src).expect("prepare failed");
    let _ = parser::parse(src, tokens).expect("qualified call should parse without errors");
}

#[test]
fn test_qualified_call_various_arg_counts() {
    let test_cases = vec![
        ("rust_lib.add(1)", "use rust:example_crate as rust_lib"),
        ("rust_lib.add(1, 2, 3)", "use rust:example_crate as rust_lib"),
        ("py_math.sqrt(81.0)", "use python:math as py_math"),
        ("ns.func()", "use rust:crate as ns"),
    ];

    for (call_src, use_decl) in test_cases {
        let src = format!(
            r#"
{}
fn main() -> i64 {{
    {}
    0
}}
"#,
            use_decl, call_src
        );
        let tokens = prepare(&src).expect("prepare failed");
        let _program = parser::parse(&src, tokens).unwrap_or_else(|e| {
            panic!("failed to parse qualified call '{}': {}", call_src, e);
        });
    }
}

#[test]
fn test_qualified_access_without_parens_is_field_access() {
    let src = r#"
use rust:example_crate as rust_lib

fn main() -> i64 {
    let x = rust_lib.value
    0
}
"#;
    let tokens = prepare(src).expect("prepare failed");
    let _program = parser::parse(src, tokens).expect("field access should parse without errors");
}

// ---------------------------------------------------------------------------
// HIR lowering with ecosystem metadata
// ---------------------------------------------------------------------------

#[test]
fn test_hir_lowering_registers_rust_ecosystem_functions() {
    let temp = TempDir::new().unwrap();
    let metadata = r#"{
    "crate_name": "example_crate",
    "functions": [
        {"name": "add", "symbol": "sat_example_crate_add_i64", "params": ["i64", "i64"], "ret": "i64"},
        {"name": "greet", "symbol": "sat_example_crate_greet", "params": ["str"], "ret": "str"}
    ]
}"#;
    setup_wrapper_metadata(&temp, "example_crate", metadata);

    let src = r#"
use rust:example_crate as rust_lib

fn main() -> i64 {
    let x = rust_lib.add(1, 2)
    0
}
"#;

    let hir = lower_with_root(src, &temp.path().to_path_buf());

    // Verify that external_functions contains an entry whose interned name
    // is "rust_lib.add" (the qualified call-site lookup key).
    let found = hir.external_functions.iter().any(|ext_fn| {
        let name_str = hir.symbols.lookup(ext_fn.name);
        name_str == Some("rust_lib.add")
            && ext_fn.kind == ExternalKind::Rust
            && ext_fn.ecosystem == "example_crate"
            && ext_fn.symbol == "sat_example_crate_add_i64"
    });
    assert!(found, "external function 'rust_lib.add' should be registered in external_functions");

    // Check the function that was found for correct param/return types.
    let ext_fn = hir
        .external_functions
        .iter()
        .find(|e| hir.symbols.lookup(e.name) == Some("rust_lib.greet"))
        .expect("greet should be registered");
    assert_eq!(ext_fn.param_types.len(), 1);
}

#[test]
fn test_hir_lowering_registers_python_ecosystem_functions() {
    let temp = TempDir::new().unwrap();
    let metadata = r#"{
    "module": "math",
    "functions": [
        {"name": "sqrt", "symbol": "sqrt", "params": ["f64"], "ret": "f64"},
        {"name": "add", "symbol": "add", "params": ["i64", "i64"], "ret": "i64"}
    ]
}"#;
    setup_wrapper_metadata(&temp, "math", metadata);

    let src = r#"
use python:math as py_math

fn main() -> i64 {
    let root = py_math.sqrt(81.0)
    let sum = py_math.add(20, 22)
    0
}
"#;

    let hir = lower_with_root(src, &temp.path().to_path_buf());

    let found_sqrt = hir.external_functions.iter().any(|ext_fn| {
        hir.symbols.lookup(ext_fn.name) == Some("py_math.sqrt")
            && ext_fn.kind == ExternalKind::Python
            && ext_fn.ecosystem == "math"
    });
    assert!(found_sqrt, "py_math.sqrt should be registered");

    let found_add = hir.external_functions.iter().any(|ext_fn| {
        hir.symbols.lookup(ext_fn.name) == Some("py_math.add")
            && ext_fn.kind == ExternalKind::Python
    });
    assert!(found_add, "py_math.add should be registered");
}

#[test]
fn test_hir_lowering_without_metadata_does_not_crash() {
    // Without wrapper metadata, ecosystem imports should still lower
    // (qualified calls will fail at resolution, but lowering itself shouldn't panic).
    let temp = TempDir::new().unwrap();

    let src = r#"
use rust:example_crate as rust_lib

fn main() -> i64 {
    0
}
"#;

    let hir = lower_with_root(src, &temp.path().to_path_buf());
    // No external functions should have been auto-generated.
    assert!(
        !hir
            .external_functions
            .iter()
            .any(|e| hir.symbols.lookup(e.name) == Some("rust_lib.add")),
        "no external functions should be registered without metadata"
    );
}

#[test]
fn test_hir_lowering_python_with_module_alias() {
    let temp = TempDir::new().unwrap();
    let metadata = r#"{
    "module": "math",
    "functions": [
        {"name": "add", "symbol": "add", "params": ["i64", "i64"], "ret": "i64"}
    ]
}"#;
    setup_wrapper_metadata(&temp, "math", metadata);

    let src = r#"
use python:math as py_math

fn main() -> i64 {
    let result = py_math.add(20, 22)
    result
}
"#;

    let hir = lower_with_root(src, &temp.path().to_path_buf());
    let found = hir.external_functions.iter().any(|ext_fn| {
        hir.symbols.lookup(ext_fn.name) == Some("py_math.add")
    });
    assert!(found, "py_math.add should be registered");
}

#[test]
fn test_hir_lowering_multiple_aliases_for_same_crate() {
    let temp = TempDir::new().unwrap();
    let metadata = r#"{
    "crate_name": "example_crate",
    "functions": [
        {"name": "add", "symbol": "sat_example_crate_add_i64", "params": ["i64", "i64"], "ret": "i64"}
    ]
}"#;
    setup_wrapper_metadata(&temp, "example_crate", metadata);

    let src = r#"
use rust:example_crate as rust_lib_a
use rust:example_crate as rust_lib_b

fn main() -> i64 {
    let a = rust_lib_a.add(1, 2)
    let b = rust_lib_b.add(3, 4)
    0
}
"#;

    let hir = lower_with_root(src, &temp.path().to_path_buf());

    // Both aliases should generate external functions under qualified names.
    let found_a = hir
        .external_functions
        .iter()
        .any(|e| hir.symbols.lookup(e.name) == Some("rust_lib_a.add"));
    assert!(found_a, "rust_lib_a.add should be registered");

    let found_b = hir
        .external_functions
        .iter()
        .any(|e| hir.symbols.lookup(e.name) == Some("rust_lib_b.add"));
    assert!(found_b, "rust_lib_b.add should be registered");
}

// ---------------------------------------------------------------------------
// MIR generation tests
// ---------------------------------------------------------------------------

#[test]
fn test_mir_external_call_for_rust_ecosystem() {
    use stnx::mir::lower::lower_program;
    use stnx::mir::MirRvalue;

    let temp = TempDir::new().unwrap();
    let metadata = r#"{
    "crate_name": "example_crate",
    "functions": [
        {"name": "add", "symbol": "sat_example_crate_add_i64", "params": ["i64", "i64"], "ret": "i64"}
    ]
}"#;
    setup_wrapper_metadata(&temp, "example_crate", metadata);

    let src = r#"
use rust:example_crate as rust_lib

fn main() -> i64 {
    let result = rust_lib.add(20, 22)
    result
}
"#;

    let hir = lower_with_root(src, &temp.path().to_path_buf());
    let mir = lower_program(&hir).expect("MIR lowering failed");

    // Check that at least one ExternalCall exists in the MIR.
    let has_external_call = mir.functions.iter().any(|func_body| {
        func_body.blocks.iter().any(|block| {
            block.stmts.iter().any(|stmt| {
                matches!(
                    &stmt.kind,
                    stnx::mir::MirStmtKind::Assign {
                        rvalue: MirRvalue::ExternalCall { .. },
                        ..
                    }
                )
            })
        })
    });
    assert!(
        has_external_call,
        "MIR should contain an ExternalCall for the ecosystem function"
    );
}

#[test]
fn test_mir_external_call_for_python_ecosystem() {
    use stnx::mir::lower::lower_program;
    use stnx::mir::MirRvalue;

    let temp = TempDir::new().unwrap();
    let metadata = r#"{
    "module": "math",
    "functions": [
        {"name": "sqrt", "symbol": "sqrt", "params": ["f64"], "ret": "f64"}
    ]
}"#;
    setup_wrapper_metadata(&temp, "math", metadata);

    let src = r#"
use python:math as py_math

fn main() -> i64 {
    let root = py_math.sqrt(81.0)
    0
}
"#;

    let hir = lower_with_root(src, &temp.path().to_path_buf());
    let mir = lower_program(&hir).expect("MIR lowering failed");

    let has_external_call = mir.functions.iter().any(|func_body| {
        func_body.blocks.iter().any(|block| {
            block.stmts.iter().any(|stmt| {
                matches!(
                    &stmt.kind,
                    stnx::mir::MirStmtKind::Assign {
                        rvalue: MirRvalue::ExternalCall { .. },
                        ..
                    }
                )
            })
        })
    });
    assert!(has_external_call, "MIR should contain an ExternalCall for Python sqrt");
}
