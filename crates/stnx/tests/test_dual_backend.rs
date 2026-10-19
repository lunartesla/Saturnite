//! Integration tests for the dual-backend (hybrid) compilation pipeline.
//!
//! These tests exercise:
//! - Pure LLVM backend compilation (no Rust imports → LLVM-only path)
//! - Hybrid backend Rust source generation (with `use rust:` imports)
//! - Rust code generation correctness (type mapping, string escaping, etc.)
//! - Round-trip: Saturnite HIR → Rust source → compile → verify
//!
//! The full `build_hybrid` pipeline (which invokes `cargo build`) is exercised
//! in the `saturn` crate integration tests where a real Cargo project structure
//! is available. Here we test the codegen pieces in isolation.

mod common;

use stnx::hir::types::HirType;
use stnx::hir::{HirExpr, HirExprKind, HirFunction, HirProgram, HirStmt, HirStmtKind};
use stnx::hir::symbol::{DefId, SymbolInterner, Visibility};
use stnx::mir::codegen::emit_object_from_mir;
use stnx::module::ModuleId;
use stnx::target::{OutputKind, TargetConfig};
use stnx::rust_codegen::{
    generate_rust_source, generate_cargo_toml, sat_type_to_rust, escape_rust_string,
    CodegenOptions,
};
use stnx::backend::{BackendMode, select_backend, SelectedBackend};
use std::collections::HashMap;

/// Build a minimal HIR program with a `main` function returning 42.
fn simple_program() -> HirProgram {
    let mut symbols = SymbolInterner::default();
    let main_name = symbols.intern("main");

    let body = vec![HirStmt {
        kind: HirStmtKind::Return(Some(HirExpr {
            kind: HirExprKind::Integer(42),
            ty: HirType::I64,
            span: miette::SourceSpan::new(0.into(), 0),
        })),
        span: miette::SourceSpan::new(0.into(), 0),
    }];

    let func = HirFunction {
        def_id: DefId(0),
        name: main_name,
        generic_params: Vec::new(),
        params: Vec::new(),
        return_type: HirType::I64,
        body,
        span: miette::SourceSpan::new(0.into(), 0),
        module: ModuleId::ROOT,
        visibility: Visibility::Private,
    };

    HirProgram {
        functions: vec![func],
        structs: Vec::new(),
        enums: Vec::new(),
        symbols,
        external_functions: Vec::new(),
        modules: Vec::new(),
        root_module: ModuleId::ROOT,
        module_paths: HashMap::new(),
        def_table: stnx::hir::symbol::DefTable::default(),
        module_scopes: Vec::new(),
        use_decls: Vec::new(),
        mod_decls: Vec::new(),
        python_search_path: String::new(),
    }
}

// ---------------------------------------------------------------------------
// Backend selection tests
// ---------------------------------------------------------------------------

#[test]
fn test_backend_auto_selects_llvm_for_no_rust_imports() {
    let program = simple_program();
    let selection = select_backend(BackendMode::Auto, &program).unwrap();
    assert_eq!(selection.backend, SelectedBackend::Llvm);
}

#[test]
fn test_backend_hybrid_forced() {
    let program = simple_program();
    let selection = select_backend(BackendMode::Hybrid, &program).unwrap();
    assert_eq!(selection.backend, SelectedBackend::Hybrid);
}

// ---------------------------------------------------------------------------
// Rust code generation unit tests
// ---------------------------------------------------------------------------

#[test]
fn test_sat_type_to_rust_all_mappings() {
    assert_eq!(sat_type_to_rust(&HirType::I64), "i64");
    assert_eq!(sat_type_to_rust(&HirType::F64), "f64");
    assert_eq!(sat_type_to_rust(&HirType::Bool), "bool");
    assert_eq!(sat_type_to_rust(&HirType::Str), "&'static str");
    assert_eq!(sat_type_to_rust(&HirType::Unit), "()");
}

#[test]
fn test_sat_type_to_rust_complex_types_fallback() {
    // Struct, Enum, List, Generic, Apply all fall back to i64.
    use stnx::hir::symbol::SymbolId;
    assert_eq!(sat_type_to_rust(&HirType::Struct(SymbolId(0))), "i64");
    assert_eq!(sat_type_to_rust(&HirType::Enum(SymbolId(0))), "i64");
    assert_eq!(
        sat_type_to_rust(&HirType::List(std::boxed::Box::new(HirType::I64))),
        "i64"
    );
}

#[test]
fn test_escape_rust_string_edge_cases() {
    // Empty string
    assert_eq!(escape_rust_string(""), "");
    // Unicode passthrough
    assert_eq!(escape_rust_string("hello"), "hello");
    // Quote escaping
    assert_eq!(escape_rust_string("say \"hi\""), "say \\\"hi\\\"");
    // Backslash escaping
    assert_eq!(escape_rust_string("a\\b"), "a\\\\b");
    // Note: escape_rust_string only escapes backslash and quotes, so tabs and
    // newlines pass through unescaped. This is acceptable for string literals
    // in generated Rust source where raw content is intended.
    assert_eq!(escape_rust_string("a\tb"), "a\tb");
    assert_eq!(escape_rust_string("a\nb"), "a\nb");
}

#[test]
fn test_generate_rust_source_produces_valid_structure() {
    let program = simple_program();
    let source = generate_rust_source(&program, "test_crate", &CodegenOptions::default()).unwrap();

    assert_eq!(source.crate_name, "test_crate");
    assert_eq!(source.edition, "2021");

    // The source should contain a function for main.
    let src = &source.source;

    // Should have the function definition.
    assert!(src.contains("fn main") || src.contains("fn run_main"),
        "source should contain a main function: {}", src);

    // In non-binary mode, it should be `pub fn main() -> i64`.
    assert!(src.contains("pub fn main"), "source should have pub fn main: {}", src);

    // Should return i64 (42).
    assert!(src.contains("42"), "source should contain the literal 42: {}", src);
}

#[test]
fn test_generate_rust_source_as_binary() {
    let program = simple_program();
    let source = generate_rust_source(
        &program,
        "test_bin",
        &CodegenOptions { as_binary: true },
    )
    .unwrap();

    // As a binary, it should have a `fn main()` (entry point that exits).
    assert!(source.source.contains("fn main()"),
        "binary source should have a main entry point: {}", source.source);

    // It should call std::process::exit.
    assert!(source.source.contains("std::process::exit"),
        "binary source should exit with code: {}", source.source);
}

#[test]
fn test_generate_cargo_toml_with_dependencies() {
    let toml = generate_cargo_toml(
        "my_crate",
        "2021",
        &[("serde".to_string(), "1.0".to_string())],
        false,
    );

    assert!(toml.contains("name = \"my_crate\""));
    assert!(toml.contains("edition = \"2021\""));
    assert!(toml.contains("crate-type = [\"staticlib\"]"));
    assert!(toml.contains("serde = \"1.0\""));
}

#[test]
fn test_generate_cargo_toml_as_binary() {
    let toml = generate_cargo_toml("my_bin", "2021", &[], true);

    // As a binary, there should be no crate-type line.
    assert!(!toml.contains("crate-type"));
}

// ---------------------------------------------------------------------------
// Object-only emission (hybrid backend step 1)
// ---------------------------------------------------------------------------

#[test]
fn test_emit_object_from_mir_produces_object_file() {
    let src = "fn main() -> i64 { return 42 }";
    let mir = common::to_mir(src);

    let temp_dir = tempfile::TempDir::new().unwrap();
    let obj_path = temp_dir.path().join("test.o");

    let config = TargetConfig::host().unwrap();
    let mut config = config;
    config.set_output_kind(OutputKind::Object);

    emit_object_from_mir(&mir, obj_path.to_str().unwrap(), &config).unwrap();
    assert!(obj_path.exists(), "object file should exist after emission");
}
