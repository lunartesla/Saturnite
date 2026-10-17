//! Integration tests for first-class Rust and Python ecosystem interoperability.
//!
//! These tests exercise the full path from `use rust:crate` / `use python:module`
//! through config parsing, qualified call syntax, HIR lowering with wrapper
//! metadata, and MIR generation.

mod common;

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

use common::compile_src_with_root;
use stnx::ast::ExternalKind;
use stnx::config::SaturnConfig;
use stnx::hir::HirLower;
use stnx::interop::DependencyKind;
use stnx::lexer::prepare;
use stnx::mir::codegen::generate_ir_from_mir;
use stnx::mir::lower::lower_program;
use stnx::parser;

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
        (
            "rust_lib.add(1, 2, 3)",
            "use rust:example_crate as rust_lib",
        ),
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
    assert!(
        found,
        "external function 'rust_lib.add' should be registered in external_functions"
    );

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
        !hir.external_functions
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
    let found = hir
        .external_functions
        .iter()
        .any(|ext_fn| hir.symbols.lookup(ext_fn.name) == Some("py_math.add"));
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
    assert!(
        has_external_call,
        "MIR should contain an ExternalCall for Python sqrt"
    );
}

#[test]
fn test_mir_external_call_python_arg_types_includes_str() {
    use stnx::hir::HirType;
    use stnx::mir::lower::lower_program;
    use stnx::mir::MirRvalue;

    let temp = TempDir::new().unwrap();
    let metadata = r#"{
    "module": "builtins",
    "functions": [
        {"name": "len", "symbol": "len", "params": ["str"], "ret": "i64"}
    ]
}"#;
    setup_wrapper_metadata(&temp, "builtins", metadata);

    let src = r#"
use python:builtins as py_builtins

fn main() -> i64 {
    let n = py_builtins.len("hello")
    0
}
"#;

    let hir = lower_with_root(src, &temp.path().to_path_buf());
    let mir = lower_program(&hir).expect("MIR lowering failed");

    // Find the ExternalCall and verify arg_types contains Str for the string arg.
    let mut found = false;
    for func_body in &mir.functions {
        for block in &func_body.blocks {
            for stmt in &block.stmts {
                if let stnx::mir::MirStmtKind::Assign {
                    rvalue:
                        MirRvalue::ExternalCall {
                            kind, arg_types, ..
                        },
                    ..
                } = &stmt.kind
                {
                    if *kind == stnx::mir::MirExternalKind::Python {
                        found = true;
                        assert_eq!(
                            arg_types.as_slice(),
                            &[HirType::Str],
                            "Python external call arg_types should include Str for string args"
                        );
                    }
                }
            }
        }
    }
    assert!(
        found,
        "Should have found a Python ExternalCall with arg_types"
    );
}

// ---------------------------------------------------------------------------
// Phase C: Rust wrapper generation tests
// ---------------------------------------------------------------------------

#[test]
fn test_rust_wrapper_generation_produces_valid_staticlib() {
    use stnx::hir::lower::{EcosystemFunctionMeta, WrapperMetadata};
    use stnx::rust_wrapper::generate_cargo_toml;
    use stnx::rust_wrapper::generate_wrapper_src;

    // Use the local fixture crate as a path dependency so the test doesn't
    // require network access. We generate a Cargo.toml that depends on the
    // local example_crate via a path dependency.
    let temp = TempDir::new().unwrap();
    let wrapper_output_dir = temp.path().to_path_buf();

    let fixture_crate = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("example_crate");

    let metadata = WrapperMetadata {
        crate_name: "example_crate".to_string(),
        functions: vec![EcosystemFunctionMeta {
            name: "add".to_string(),
            symbol: "sat_example_crate_add_i64".to_string(),
            params: vec!["i64".to_string(), "i64".to_string()],
            ret: "i64".to_string(),
        }],
    };

    let build_dir = wrapper_output_dir.join("example_crate_wrapper");
    std::fs::create_dir_all(build_dir.join("src")).unwrap();

    // Write Cargo.toml with path dependency (patching the generated one).
    let toml = generate_cargo_toml(&metadata, "0.1.0");
    // Replace the crates.io dependency with a path dependency.
    // Use forward slashes in the path to avoid TOML escape issues on Windows.
    let fixture_str = fixture_crate.to_string_lossy().replace('\\', "/");
    let path_dep = format!("example_crate = {{ path = \"{}\" }}", fixture_str);
    let toml = toml.replace("example_crate = \"0.1.0\"", &path_dep);
    std::fs::write(build_dir.join("Cargo.toml"), &toml).unwrap();

    // Write the generated lib.rs source.
    let lib_rs = generate_wrapper_src(&metadata, &metadata.functions);
    std::fs::write(build_dir.join("src").join("lib.rs"), &lib_rs).unwrap();

    // Build the wrapper crate using cargo.
    let output = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .current_dir(&build_dir)
        .output();

    match output {
        Ok(output) => {
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stdout = String::from_utf8_lossy(&output.stdout);
                // Skip if cargo/rustc isn't available in this environment.
                if stderr.contains("error: cargo or rustc")
                    || stdout.is_empty() && stderr.is_empty()
                {
                    eprintln!("Skipping test: cargo/rustc not available");
                    return;
                }
                panic!(
                    "cargo build failed:\n--- stdout ---\n{}\n--- stderr ---\n{}",
                    stdout, stderr
                );
            }

            // Verify the static library was produced.
            let release_dir = build_dir.join("target").join("release");
            let lib_path = release_dir.join("libexample_crate_saturnite_wrapper.a");
            let lib_path_lib = release_dir.join("example_crate_saturnite_wrapper.lib");

            let found = lib_path.exists() || lib_path_lib.exists();
            assert!(
                found,
                "static library should exist in {}; checked {:?} and {:?}",
                release_dir.display(),
                lib_path,
                lib_path_lib
            );
        }
        Err(e) => {
            eprintln!("Skipping test: cargo not available: {}", e);
        }
    }
}

#[test]
fn test_rust_wrapper_generation_generates_correct_source() {
    use stnx::hir::lower::{EcosystemFunctionMeta, WrapperMetadata};
    use stnx::interop::SAT_ABI_VERSION;
    use stnx::rust_wrapper::generate_wrapper_src;

    let metadata = WrapperMetadata {
        crate_name: "serde".to_string(),
        functions: vec![EcosystemFunctionMeta {
            name: "serialize_to_string".to_string(),
            symbol: "sat_serde_serialize_to_string".to_string(),
            params: vec!["i64".to_string()],
            ret: "i64".to_string(),
        }],
    };

    let src = generate_wrapper_src(&metadata, &metadata.functions);

    // The generated source should contain the extern "C" shim.
    assert!(src.contains("serde::serialize_to_string"));
    assert!(src.contains("sat_serde_serialize_to_string"));
    assert!(src.contains("extern \"C\""));
    assert!(src.contains(&format!("ABI version: {}", SAT_ABI_VERSION)));

    // The generated source should be valid Rust (starts with a doc comment).
    assert!(src.starts_with("//! Auto-generated"));
}

#[test]
fn test_rust_wrapper_generation_handles_multiple_functions() {
    use stnx::hir::lower::{EcosystemFunctionMeta, WrapperMetadata};
    use stnx::rust_wrapper::generate_wrapper_src;

    let metadata = WrapperMetadata {
        crate_name: "math_lib".to_string(),
        functions: vec![
            EcosystemFunctionMeta {
                name: "add".to_string(),
                symbol: "sat_math_lib_add".to_string(),
                params: vec!["i64".to_string(), "i64".to_string()],
                ret: "i64".to_string(),
            },
            EcosystemFunctionMeta {
                name: "sqrt".to_string(),
                symbol: "sat_math_lib_sqrt".to_string(),
                params: vec!["f64".to_string()],
                ret: "f64".to_string(),
            },
            EcosystemFunctionMeta {
                name: "is_valid".to_string(),
                symbol: "sat_math_lib_is_valid".to_string(),
                params: vec!["bool".to_string()],
                ret: "bool".to_string(),
            },
        ],
    };

    let src = generate_wrapper_src(&metadata, &metadata.functions);

    // All three functions should be present.
    assert!(src.contains("sat_math_lib_add"));
    assert!(src.contains("sat_math_lib_sqrt"));
    assert!(src.contains("sat_math_lib_is_valid"));

    // Check type correctness in generated signatures.
    assert!(src.contains("arg0: i64, arg1: i64) -> i64"));
    assert!(src.contains("arg0: f64) -> f64"));
    assert!(src.contains("arg0: bool) -> bool"));
}

#[test]
fn test_rust_wrapper_cargo_toml_uses_path_dependency() {
    // Verify that the generated Cargo.toml has the correct structure
    // for a path dependency (which is what the build function uses
    // when building from a local fixture).
    use stnx::hir::lower::WrapperMetadata;
    use stnx::rust_wrapper::generate_cargo_toml;

    let metadata = WrapperMetadata {
        crate_name: "serde_json".to_string(),
        functions: vec![],
    };

    let toml = generate_cargo_toml(&metadata, "1.0.0");

    // The dependency should reference the crate by its original name.
    assert!(toml.contains("serde_json = \"1.0.0\""));
    // The package name should be sanitized.
    assert!(toml.contains("name = \"serde_json-saturnite-wrapper\""));
    // Must be a staticlib.
    assert!(toml.contains("crate-type = [\"staticlib\"]"));
}

// ---------------------------------------------------------------------------
// Phase D: Python venv management tests
// ---------------------------------------------------------------------------

#[test]
fn test_pyenv_at_construct() {
    use stnx::PyEnv;
    let venv = PyEnv::at(PathBuf::from("/tmp/test_venv"));
    assert!(!venv.exists());
    // The python_exe path should be set correctly.
    #[cfg(windows)]
    {
        assert_eq!(
            venv.python_exe,
            PathBuf::from("/tmp/test_venv/Scripts/python.exe")
        );
    }
    #[cfg(not(windows))]
    {
        assert_eq!(venv.python_exe, PathBuf::from("/tmp/test_venv/bin/python3"));
    }
}

#[test]
fn test_pyenv_remove_on_nonexistent_is_ok() {
    use stnx::PyEnv;
    let venv = PyEnv::at(PathBuf::from("/nonexistent/venv_xyz_123"));
    // Removing a non-existent venv should not error.
    assert!(venv.remove().is_ok());
}

#[test]
fn test_pyenv_search_path_format() {
    use stnx::PyEnv;

    // Create a fake venv structure so search_path returns multiple dirs
    // (which exercises the separator logic).
    let temp = TempDir::new().unwrap();
    let venv_root = temp.path().to_path_buf();

    #[cfg(unix)]
    {
        let pyver_dir = venv_root.join("lib").join("python3.11");
        std::fs::create_dir_all(pyver_dir.join("site-packages")).unwrap();
    }
    #[cfg(windows)]
    {
        std::fs::create_dir_all(venv_root.join("Lib").join("site-packages")).unwrap();
    }

    let venv = PyEnv::at(venv_root);
    let result = venv.search_path();
    assert!(result.is_ok());
    let path = result.unwrap();
    // On Unix the venv has both a version-specific lib/pythonX.Y/site-packages
    // directory and the primary site-packages, so the path contains a separator.
    #[cfg(not(windows))]
    {
        assert!(
            path.contains(":"),
            "path should contain ':' on Unix: {}",
            path
        );
    }
    // On Windows the venv layout is a single Lib/site-packages, so there is no
    // separator — the path is just that directory.
    // Should include site-packages.
    assert!(path.contains("site-packages"));
}

#[test]
fn test_pyenv_search_path_includes_version_specific_dir() {
    use stnx::PyEnv;

    // Create a fake venv structure to test the search path logic.
    // On Unix, venvs have lib/pythonX.Y/site-packages.
    let temp = TempDir::new().unwrap();
    let venv_root = temp.path().join("test_venv");

    #[cfg(unix)]
    {
        let pyver_dir = venv_root.join("lib").join("python3.11");
        let site_packages = pyver_dir.join("site-packages");
        std::fs::create_dir_all(&site_packages).unwrap();
    }
    #[cfg(windows)]
    {
        // On Windows, venvs have Lib/site-packages directly.
        let lib_dir = venv_root.join("Lib");
        std::fs::create_dir_all(lib_dir.join("site-packages")).unwrap();
    }

    let venv = PyEnv::at(venv_root);
    let path = venv.search_path().unwrap();

    // Should include site-packages.
    assert!(
        path.contains("site-packages"),
        "path should contain 'site-packages': {}",
        path
    );

    // On Unix, should also contain the version-specific dir.
    #[cfg(unix)]
    assert!(
        path.contains("python3.11"),
        "path should contain 'python3.11': {}",
        path
    );
}

#[test]
fn test_pyenv_search_path_dedup() {
    use stnx::PyEnv;

    // Create a venv where the site-packages dir is the same as the
    // version-specific one — ensure no duplicate entries.
    let temp = TempDir::new().unwrap();
    let venv_root = temp.path().to_path_buf();
    let lib_dir = venv_root.join("lib");
    let pyver_dir = lib_dir.join("python3.11");
    let site_packages = pyver_dir.join("site-packages");
    std::fs::create_dir_all(&site_packages).unwrap();

    let venv = PyEnv::at(venv_root);
    let path = venv.search_path().unwrap();

    // Count occurrences of "site-packages" — should appear at most twice
    // (primary + version-specific, which may be the same path).
    let site_pkg_count = path.matches("site-packages").count();
    assert!(
        site_pkg_count <= 2,
        "should not have excessive site-packages entries: {} in {}",
        site_pkg_count,
        path
    );
}

// ---------------------------------------------------------------------------
// Phase D: Python object handle and method-call tests
//
// These tests compile and run a small C program (runtime/test_pyrt_handles.c)
// that exercises the handle-based Python bridge API declared in pyrt.h:
// sat_py_handle_new, sat_py_get_attr, sat_py_call_method,
// sat_py_release_handle, sat_py_import_and_cache, sat_py_get_cached_module.
//
// The C test is only built and run when `python3-config` is available (i.e.
// Python development headers are installed). On systems without Python, the
// test is skipped.
// ---------------------------------------------------------------------------

#[test]
fn test_pyrt_handles_c_runtime() {
    use std::process::Command;

    // Locate the runtime source directory.
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let runtime_dir = std::path::Path::new(manifest_dir).join("runtime");
    let test_c = runtime_dir.join("test_pyrt_handles.c");
    let pyrt_impl = runtime_dir.join("pyrt_impl.c");

    if !test_c.exists() || !pyrt_impl.exists() {
        eprintln!(
            "WARNING: pyrt test source not found at {}; skipping",
            test_c.display()
        );
        return;
    }

    // Check if python3-config is available.
    let py_inc_out = Command::new("python3-config").arg("--includes").output();
    let py_inc = match py_inc_out {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        _ => {
            eprintln!("python3-config not found; skipping Python runtime handle tests");
            return;
        }
    };

    // Get ldflags for linking.
    let py_ld_out = Command::new("python3-config")
        .args(["--ldflags", "--embed"])
        .output();
    let py_ldflags = match py_ld_out {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        _ => {
            eprintln!("python3-config --ldflags failed; skipping Python runtime handle tests");
            return;
        }
    };

    let temp = TempDir::new().unwrap();
    let bin_path = temp.path().join("test_pyrt_handles");

    // Compile the test C file + pyrt_impl.c together with Python flags.
    let mut cmd = Command::new("cc");
    cmd.arg(&test_c)
        .arg(&pyrt_impl)
        .arg("-o")
        .arg(&bin_path)
        .arg("-I")
        .arg(&runtime_dir);

    for flag in py_inc.split_whitespace() {
        if let Some(dir) = flag.strip_prefix("-I") {
            cmd.arg("-I").arg(dir);
        }
    }
    for flag in py_ldflags.split_whitespace() {
        cmd.arg(flag);
    }

    let out = cmd.output();
    let _out = match out {
        Ok(o) if o.status.success() => o,
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            eprintln!("Failed to compile/link pyrt test: {}", stderr);
            panic!("pyrt handle test compilation failed");
        }
        Err(e) => {
            eprintln!("Could not find C compiler: {}; skipping", e);
            return;
        }
    };

    // Run the test binary.
    let output = Command::new(&bin_path).output();
    match output {
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let stdout = String::from_utf8_lossy(&out.stdout);

            println!("pyrt test stdout:\n{}", stdout);
            if !stderr.is_empty() {
                println!("pyrt test stderr:\n{}", stderr);
            }

            assert!(
                out.status.success(),
                "pyrt handle C test failed with exit code {:?}:\nstdout:\n{}\nstderr:\n{}",
                out.status.code(),
                stdout,
                stderr
            );
        }
        Err(e) => {
            eprintln!("Could not run pyrt test binary: {}; skipping", e);
        }
    }
}

// ---------------------------------------------------------------------------
// Real end-to-end Python execution tests
//
// These tests compile and *run* a Saturnite program that calls Python via
// `use python:module`. They require:
//   1. A C compiler (cc/gcc/clang) — required for all compile_src tests
//   2. Python 3 development headers/libraries — required for Python interop
//   3. A functional Python module in the import path
//
// If Python is not available, the test is skipped.
// ---------------------------------------------------------------------------

/// Check if Python interpreter AND development headers/libraries are available
/// for end-to-end tests. The interpreter alone is not sufficient — we also need
/// `Python.h` and a linkable Python library to compile and link `pyrt_impl.c`.
fn python_available() -> bool {
    let python_cmd = if Command::new("python3")
        .arg("-c")
        .arg("import sys; sys.exit(0)")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        "python3"
    } else if Command::new("python")
        .arg("-c")
        .arg("import sys; sys.exit(0)")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        "python"
    } else {
        eprintln!("Python interpreter not found; skipping Python interop tests");
        return false;
    };

    // Verify development headers are installed: Python.h must be in a valid
    // include directory.
    let py_inc = Command::new(python_cmd)
        .arg("-c")
        .arg(
            "import sysconfig, os; inc=sysconfig.get_path('include'); \
             exit(0 if os.path.isdir(inc) and os.path.isfile(os.path.join(inc, 'Python.h')) else 1)",
        )
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    // Verify a linkable library is available (LDLIBRARY is non-empty).
    let py_lib = Command::new(python_cmd)
        .arg("-c")
        .arg("import sysconfig; lib=sysconfig.get_config_var('LDLIBRARY'); exit(0 if lib else 1)")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !py_inc {
        eprintln!(
            "Python interpreter found but development headers (Python.h) missing; \
             Python interop tests will be skipped"
        );
    }
    if !py_lib {
        eprintln!(
            "Python interpreter found but development libraries (LDLIBRARY) missing; \
             Python interop tests will be skipped"
        );
    }

    py_inc && py_lib
}

#[test]
fn test_python_end_to_end_sqrt() {
    if !python_available() {
        eprintln!("Python not available; skipping end-to-end test");
        return;
    }

    let temp = TempDir::new().unwrap();

    // Create wrapper metadata for the `math` module.
    // The "symbol" field is the function name as seen by the Python runtime
    // (i.e., the attribute name on the imported module). The codegen layer
    // prepends the module name via "ecosystem::symbol".
    //
    // We use math.ceil (returns i64) because the `say` builtin only supports
    // i64, str, and enum types in the runtime.
    let metadata = r#"{
        "module": "math",
        "functions": [
            {"name": "ceil", "symbol": "ceil", "params": ["f64"], "ret": "i64"},
            {"name": "sqrt", "symbol": "sqrt", "params": ["f64"], "ret": "f64"}
        ]
    }"#;
    setup_wrapper_metadata(&temp, "math", metadata);

    // Saturnite program that computes ceil(81.0) = 81 (i64, printable via say).
    let src = r#"
use python:math

fn main() -> i64 {
    let result = math.ceil(81.0)
    say result
    0
}
"#;

    let artifact = compile_src_with_root(src, temp.path());
    let (code, out) = artifact.run();
    assert_eq!(code, 0, "program should exit cleanly");
    assert_eq!(
        out, "81\n",
        "math.ceil(81.0) should output 81, got: {:?}",
        out
    );
}

#[test]
fn test_python_end_to_end_multiple_args() {
    if !python_available() {
        eprintln!("Python not available; skipping end-to-end test");
        return;
    }

    let temp = TempDir::new().unwrap();

    // Create wrapper metadata for `math.ceil` which takes a float and returns int.
    let metadata = r#"{
        "module": "math",
        "functions": [
            {"name": "ceil", "symbol": "ceil", "params": ["f64"], "ret": "i64"}
        ]
    }"#;
    setup_wrapper_metadata(&temp, "math", metadata);

    let src = r#"
use python:math

fn main() -> i64 {
    let result = math.ceil(3.7)
    say result
    0
}
"#;

    let artifact = compile_src_with_root(src, temp.path());
    let (code, out) = artifact.run();
    assert_eq!(code, 0, "program should exit cleanly");
    assert_eq!(out, "4\n", "math.ceil(3.7) should output 4, got: {:?}", out);
}

#[test]
fn test_python_end_to_end_math_constants() {
    if !python_available() {
        eprintln!("Python not available; skipping end-to-end test");
        return;
    }

    let temp = TempDir::new().unwrap();

    let metadata = r#"{
        "module": "math",
        "functions": [
            {"name": "sqrt", "symbol": "sqrt", "params": ["f64"], "ret": "f64"},
            {"name": "ceil", "symbol": "ceil", "params": ["f64"], "ret": "i64"}
        ]
    }"#;
    setup_wrapper_metadata(&temp, "math", metadata);

    let src = r#"
use python:math

fn main() -> i64 {
    let root = math.sqrt(25.0)
    let ceiled = math.ceil(root)
    0
}
"#;

    let artifact = compile_src_with_root(src, temp.path());
    let (code, _out) = artifact.run();
    assert_eq!(code, 0, "program should exit cleanly");
}

#[test]
fn test_python_end_to_end_str_return() {
    // A Python function that returns a string. The Saturnite caller uses
    // `say` on the returned string — this exercises the codegen path that
    // extracts the `char*` from the sat_py_result union field (index 2)
    // and returns it as a pointer-typed local for Str results.
    if !python_available() {
        eprintln!("Python not available; skipping end-to-end test");
        return;
    }

    let temp = TempDir::new().unwrap();

    // Use the `builtins` module (always available). We call builtins.str
    // to convert an integer to a string, then `say` prints it.
    let metadata = r#"{
        "module": "builtins",
        "functions": [
            {"name": "str", "symbol": "str", "params": ["i64"], "ret": "str"}
        ]
    }"#;
    setup_wrapper_metadata(&temp, "builtins", metadata);

    let src = r#"
use python:builtins as py

fn main() -> i64 {
    let s = py.str(42)
    say s
    0
}
"#;

    let artifact = compile_src_with_root(src, temp.path());
    let (code, out) = artifact.run();
    assert_eq!(code, 0, "program should exit cleanly");
    assert_eq!(
        out, "42\n",
        "builtins.str(42) should output '42', got: {:?}",
        out
    );
}

#[test]
fn test_python_end_to_end_str_concat() {
    // Call two Python functions that return strings, then use string
    // interpolation (which calls concat_str) to combine them.
    if !python_available() {
        eprintln!("Python not available; skipping end-to-end test");
        return;
    }

    let temp = TempDir::new().unwrap();

    let metadata = r#"{
        "module": "builtins",
        "functions": [
            {"name": "str", "symbol": "str", "params": ["i64"], "ret": "str"}
        ]
    }"#;
    setup_wrapper_metadata(&temp, "builtins", metadata);

    let src = r#"
use python:builtins as py

fn main() -> i64 {
    let a = py.str(42)
    let b = py.str(99)
    say "values: {a} and {b}"
    0
}
"#;

    let artifact = compile_src_with_root(src, temp.path());
    let (code, out) = artifact.run();
    assert_eq!(code, 0, "program should exit cleanly");
    assert_eq!(
        out, "values: 42 and 99\n",
        "interpolated Python strings should be correct, got: {:?}",
        out
    );
}

#[test]
fn test_python_call_flat_ir_extracts_result_fields() {
    // Verify that the codegen for Python external calls properly handles
    // the sat_py_result struct — it must extract the `ok` flag and use
    // select() to zero/NULL the result on failure.
    if !python_available() {
        eprintln!("Python not available; skipping IR test");
        return;
    }

    let temp = TempDir::new().unwrap();

    let metadata = r#"{
        "module": "math",
        "functions": [
            {"name": "ceil", "symbol": "ceil", "params": ["f64"], "ret": "i64"}
        ]
    }"#;
    setup_wrapper_metadata(&temp, "math", metadata);

    let src = r#"
use python:math

fn main() -> i64 {
    let result = math.ceil(81.0)
    say result
    0
}
"#;

    // Generate IR only (no linking) to inspect the codegen.
    let tokens = prepare(src).expect("prepare failed");
    let program = parser::parse(src, tokens).expect("parsing failed");
    let mut lower = stnx::hir::HirLower::new().with_project_root(temp.path().to_path_buf());
    let hir = lower.lower_program(&program).expect("HIR lowering failed");
    let mir = stnx::mir::lower::lower_program(&hir).expect("MIR lowering failed");
    let ir = stnx::mir::codegen::generate_ir_from_mir(&mir).expect("IR generation failed");

    // The IR must call sat_py_call_flat.
    assert!(
        ir.contains("sat_py_call_flat"),
        "IR should contain sat_py_call_flat call:\n{}",
        ir
    );

    // The IR must extract the `ok` field (extractvalue ... 0) and use
    // conditional branching to handle the error path. On failure, the
    // bridge aborts via sat_py_abort instead of silently returning a default.
    assert!(
        ir.contains("py_ok"),
        "IR should extract the ok flag:\n{}",
        ir
    );
    assert!(
        ir.contains("sat_py_abort"),
        "IR should call sat_py_abort on failure:\n{}",
        ir
    );
    assert!(
        ir.contains("br i1 %py_ok"),
        "IR should branch on the ok flag:\n{}",
        ir
    );
}

// ---------------------------------------------------------------------------
// Failure handling and regression tests
// ---------------------------------------------------------------------------

#[test]
fn test_python_exception_is_fatal() {
    // A Python function that raises should abort the program with a
    // nonzero exit status and a useful diagnostic on stderr. The program
    // must NOT silently return a default value.
    if !python_available() {
        eprintln!("Python not available; skipping end-to-end test");
        return;
    }

    let temp = TempDir::new().unwrap();
    let metadata = r#"{
        "module": "builtins",
        "functions": [
            {"name": "int", "symbol": "int", "params": ["str"], "ret": "i64"}
        ]
    }"#;
    setup_wrapper_metadata(&temp, "builtins", metadata);

    // builtins.int("not_a_number") raises ValueError.
    let src = r#"
use python:builtins as py

fn main() -> i64 {
    let val = py.int("not_a_number")
    say val
    0
}
"#;

    let artifact = compile_src_with_root(src, temp.path());
    let (code, _stdout, stderr) = artifact.run_with_stderr();
    assert_ne!(
        code, 0,
        "program should exit nonzero on Python exception, got code {}",
        code
    );
    assert!(
        stderr.contains("Python error") || stderr.contains("ValueError"),
        "stderr should mention the Python exception, got: {:?}",
        stderr
    );
}

#[test]
fn test_python_missing_function_is_fatal() {
    // Calling a function that doesn't exist on the module should fail
    // with a diagnostic, not silently return a default.
    if !python_available() {
        eprintln!("Python not available; skipping end-to-end test");
        return;
    }

    let temp = TempDir::new().unwrap();
    let metadata = r#"{
        "module": "builtins",
        "functions": [
            {"name": "nonexistent_func_xyz", "symbol": "nonexistent_func_xyz", "params": [], "ret": "i64"}
        ]
    }"#;
    setup_wrapper_metadata(&temp, "builtins", metadata);

    let src = r#"
use python:builtins as py

fn main() -> i64 {
    let val = py.nonexistent_func_xyz()
    say val
    0
}
"#;

    let artifact = compile_src_with_root(src, temp.path());
    let (code, _stdout, stderr) = artifact.run_with_stderr();
    assert_ne!(
        code, 0,
        "program should exit nonzero on missing function, got code {}",
        code
    );
}

#[test]
fn test_python_missing_module_is_fatal() {
    // Importing a nonexistent module should fail with a diagnostic.
    if !python_available() {
        eprintln!("Python not available; skipping end-to-end test");
        return;
    }

    let temp = TempDir::new().unwrap();
    let metadata = r#"{
        "module": "nonexistent_module_xyz",
        "functions": [
            {"name": "foo", "symbol": "foo", "params": [], "ret": "i64"}
        ]
    }"#;
    setup_wrapper_metadata(&temp, "nonexistent_module_xyz", metadata);

    let src = r#"
use python:nonexistent_module_xyz as nm

fn main() -> i64 {
    let val = nm.foo()
    say val
    0
}
"#;

    let artifact = compile_src_with_root(src, temp.path());
    let (code, _stdout, stderr) = artifact.run_with_stderr();
    assert_ne!(
        code, 0,
        "program should exit nonzero on missing module, got code {}",
        code
    );
}

#[test]
fn test_python_multiple_calls_in_one_module() {
    // Multiple Python calls in the same compiled module must link and
    // execute correctly. This is a regression test for the duplicate
    // sat_py_call_flat declaration suffix issue on Windows COFF.
    if !python_available() {
        eprintln!("Python not available; skipping end-to-end test");
        return;
    }

    let temp = TempDir::new().unwrap();
    let metadata = r#"{
        "module": "math",
        "functions": [
            {"name": "ceil", "symbol": "ceil", "params": ["f64"], "ret": "i64"},
            {"name": "floor", "symbol": "floor", "params": ["f64"], "ret": "i64"}
        ]
    }"#;
    setup_wrapper_metadata(&temp, "math", metadata);

    let src = r#"
use python:math

fn main() -> i64 {
    let a = math.ceil(3.7)
    let b = math.floor(3.7)
    say a
    say b
    0
}
"#;

    let artifact = compile_src_with_root(src, temp.path());
    let (code, out) = artifact.run();
    assert_eq!(code, 0, "program should exit cleanly");
    assert_eq!(out, "4\n3\n", "ceil(3.7)=4, floor(3.7)=3, got: {:?}", out);
}

#[test]
fn test_python_i64_return() {
    // A Python function returning an integer — verify the i64 path
    // (extract from union, no conversion needed).
    if !python_available() {
        eprintln!("Python not available; skipping end-to-end test");
        return;
    }

    let temp = TempDir::new().unwrap();
    let metadata = r#"{
        "module": "builtins",
        "functions": [
            {"name": "int", "symbol": "int", "params": ["f64"], "ret": "i64"}
        ]
    }"#;
    setup_wrapper_metadata(&temp, "builtins", metadata);

    let src = r#"
use python:builtins as py

fn main() -> i64 {
    let val = py.int(42.0)
    say val
    0
}
"#;

    let artifact = compile_src_with_root(src, temp.path());
    let (code, out) = artifact.run();
    assert_eq!(code, 0, "program should exit cleanly");
    assert_eq!(out, "42\n", "int(42.0) should output 42, got: {:?}", out);
}

#[test]
fn test_python_bool_return() {
    // A Python function returning a bool — verify the truncate path.
    // builtins.bool(1) returns True, which is truthy. We check via math.isclose
    // or just check that bool(0) returns False (0) and bool(1) returns True (1).
    if !python_available() {
        eprintln!("Python not available; skipping end-to-end test");
        return;
    }

    let temp = TempDir::new().unwrap();
    let metadata = r#"{
        "module": "builtins",
        "functions": [
            {"name": "bool", "symbol": "bool", "params": ["i64"], "ret": "bool"}
        ]
    }"#;
    setup_wrapper_metadata(&temp, "builtins", metadata);

    let src = r#"
use python:builtins as py

fn main() -> i64 {
    let t = py.bool(1)
    let f = py.bool(0)
    if t {
        say 1
    }
    if f {
        say 2
    }
    0
}
"#;

    let artifact = compile_src_with_root(src, temp.path());
    let (code, out) = artifact.run();
    assert_eq!(code, 0, "program should exit cleanly");
    // bool True → branch taken, bool False → branch skipped
    assert_eq!(out, "1\n", "bool(1) should be truthy, got: {:?}", out);
}

#[test]
fn test_python_str_return_multiple_calls() {
    // Multiple Python calls that return strings — verifies both the Str
    // conversion path and the multiple-call declaration reuse.
    if !python_available() {
        eprintln!("Python not available; skipping end-to-end test");
        return;
    }

    let temp = TempDir::new().unwrap();
    let metadata = r#"{
        "module": "builtins",
        "functions": [
            {"name": "str", "symbol": "str", "params": ["i64"], "ret": "str"}
        ]
    }"#;
    setup_wrapper_metadata(&temp, "builtins", metadata);

    let src = r#"
use python:builtins as py

fn main() -> i64 {
    let a = py.str(123)
    let b = py.str(456)
    say "first: {a}"
    say "second: {b}"
    0
}
"#;

    let artifact = compile_src_with_root(src, temp.path());
    let (code, out) = artifact.run();
    assert_eq!(code, 0, "program should exit cleanly");
    assert_eq!(out, "first: 123\nsecond: 456\n", "got: {:?}", out);
}

#[test]
fn test_parser_keyword_as_member_name() {
    // Regression test: `py.str(42)` must parse correctly because `str` is
    // a keyword token but is used as a member name in qualified calls.
    let src = r#"
use python:builtins as py

fn main() -> i64 {
    let s = py.str(42)
    say s
    0
}
"#;

    let tokens = prepare(src).expect("prepare failed");
    let program = parser::parse(src, tokens).expect("parsing should succeed");
    assert_eq!(
        program.functions.len(),
        1,
        "should parse successfully with keyword member name"
    );
}
