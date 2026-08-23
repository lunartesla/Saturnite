//! Integration tests for the `resolve_modules` function (Phase 6B).
//!
//! These tests verify use-import resolution across modules:
//!
//! 1. **Unresolved import error** — `use bar::foo` where module `bar` does not
//!    exist produces a `CompilerError::Semantic` with the expected message.
//! 2. **Single-segment local import** — `use foo` where `foo` is a function in
//!    the same (root) module resolves successfully and registers an import in
//!    the root module scope.
//! 3. **Multi-module path resolution** — `use child::some_func` where `child`
//!    is a declared submodule resolves successfully and registers an import
//!    in the root module's scope.
//!
//! All tests use `tempfile::TempDir` for filesystem isolation and exercise the
//! full pipeline: `ModuleGraph::discover_modules` → `analyze_and_lower_with_graph`
//! (which internally calls `lower_with_graph` + `resolve_modules`).

mod common;

use std::fs;
use stnx::module::ModuleGraph;
use stnx::semantic::analyze_and_lower_with_graph;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Write a file at `dir/<rel>` with the given contents, creating parent dirs.
fn write_file(dir: &TempDir, rel: &str, contents: &str) {
    let path = dir.path().join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("failed to create parent dir");
    }
    fs::write(&path, contents)
        .unwrap_or_else(|e| panic!("failed to write {}: {}", path.display(), e));
}

/// Write a minimal `saturn.toml` for project discovery.
fn write_saturn_toml(dir: &TempDir, name: &str) {
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

// ---------------------------------------------------------------------------
// Test 1: Unresolved import error
// ---------------------------------------------------------------------------

/// `use bar::foo` where module `bar` does not exist should produce:
/// `CompilerError::Semantic("unresolved import: module 'bar' not found")`
#[test]
fn test_unresolved_import_error() {
    let tmp = TempDir::new().unwrap();

    write_saturn_toml(&tmp, "unresolved_test");
    // The source declares a use of `bar::foo`, but there is no `mod bar`
    // declaration, so module `bar` does not exist in the graph.
    let src = "use bar::foo\nfn main() -> i64 {\n    return 0\n}\n";
    write_file(&tmp, "src/main.stnx", src);

    // Build the module graph from the source file — this creates a root
    // module only (no child module `bar` is discovered).
    let main_path = tmp.path().join("src").join("main.stnx");
    let graph = ModuleGraph::discover_modules(main_path).expect("discovery should succeed");

    // The root AST is the entry program for HIR lowering.
    let root_ast = graph
        .root_module()
        .ast
        .as_ref()
        .expect("root module AST should be available");

    let result = analyze_and_lower_with_graph(root_ast, &graph);

    assert!(result.is_err(), "unresolved import should produce an error");

    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("unresolved import"),
        "error should mention 'unresolved import', got: {}",
        err
    );
    assert!(
        err.contains("bar"),
        "error should mention the missing module name 'bar', got: {}",
        err
    );
}

// ---------------------------------------------------------------------------
// Test 2: Single-segment local import
// ---------------------------------------------------------------------------

/// `use foo` where `foo` is a function in the same (root) module — should
/// resolve successfully and register an import in the root module scope.
#[test]
fn test_single_segment_local_import() {
    let tmp = TempDir::new().unwrap();

    write_saturn_toml(&tmp, "local_import_test");
    // `foo` is a function defined in the root module.
    // `use foo` imports it (single-segment path, same-module item).
    let src = "use foo\nfn foo() -> i64 {\n    return 42\n}\nfn main() -> i64 {\n    return 0\n}\n";
    write_file(&tmp, "src/main.stnx", src);

    let main_path = tmp.path().join("src").join("main.stnx");
    let graph = ModuleGraph::discover_modules(main_path).expect("discovery should succeed");

    let root_ast = graph
        .root_module()
        .ast
        .as_ref()
        .expect("root module AST should be available");

    let hir = analyze_and_lower_with_graph(root_ast, &graph).expect("resolution should succeed");

    // The import should be registered in the root module scope (ModuleId::ROOT = 0).
    let root_scope = hir
        .module_scope(stnx::ModuleId::ROOT)
        .expect("root module scope must exist");

    // Find the use declaration for `use foo` and use its alias SymbolId
    // (which was interned by the HIR's own SymbolInterner during lowering).
    let use_decl = hir
        .use_decls
        .iter()
        .find(|ud| {
            // The path is [foo] — a single segment.
            ud.path.len() == 1 && hir.symbols.lookup(ud.path[0]) == Some("foo")
        })
        .expect("should find use decl for `foo` in HIR");

    let alias = use_decl.alias;

    // The import should be registered: the alias maps to a DefId in the
    // root module scope's imports table.
    assert!(
        root_scope.imports.contains_key(&alias),
        "import for 'foo' should be registered in root module scope; \
         scope imports: {:?}",
        root_scope.imports
    );

    // The imported DefId should resolve to the function `foo`.
    let imported_def_id = root_scope.imports.get(&alias).copied().unwrap();
    let imported_func = hir
        .function(imported_def_id)
        .expect("imported DefId should resolve to a function");
    let func_name = hir
        .symbol_name(imported_func.name)
        .expect("function should have a name");
    assert_eq!(func_name, "foo", "imported function should be named 'foo'");

    // The imported function should belong to the root module (same-module import).
    let func_module = hir
        .module_of(imported_def_id)
        .expect("imported function should have an owning module");
    assert_eq!(
        func_module,
        stnx::ModuleId::ROOT,
        "single-segment local import should resolve within the root module"
    );
}

// ---------------------------------------------------------------------------
// Test 3: Multi-module path resolution (basic)
// ---------------------------------------------------------------------------

/// Set up a root module that declares `mod child` and imports `use child::some_func`.
/// The child module defines `some_func`. Verify resolution succeeds and the import
/// is registered in the root module's scope.
#[test]
fn test_multi_module_path_resolution() {
    let tmp = TempDir::new().unwrap();

    write_saturn_toml(&tmp, "multimodule_test");

    // Root module: declares `mod child` and imports `use child::some_func`.
    let root_src = "mod child\nuse child::some_func\nfn main() -> i64 {\n    return 0\n}\n";
    write_file(&tmp, "src/main.stnx", root_src);

    // Child module: defines `some_func`.
    let child_src = "fn some_func() -> i64 {\n    return 99\n}\n";
    write_file(&tmp, "src/child.stnx", child_src);

    // Discover modules from the root source file.
    let main_path = tmp.path().join("src").join("main.stnx");
    let mut graph =
        ModuleGraph::discover_modules(main_path).expect("module discovery should succeed");

    // We should have 2 modules: root + child.
    assert_eq!(graph.len(), 2, "graph should contain root + child modules");

    let root_ast = graph
        .root_module()
        .ast
        .as_ref()
        .expect("root module AST should be available");

    let hir = analyze_and_lower_with_graph(root_ast, &graph).expect("resolution should succeed");

    // Verify the child module exists in the graph, using the graph's own interner
    // so SymbolIds are consistent.
    let child_seg = graph.symbol_interner.intern("child");
    let child_path = stnx::ModulePath::from_segments(vec![child_seg]);
    let child_id = graph
        .find_module(&child_path)
        .expect("child module should exist in the graph");
    let _child_mod = graph
        .get_module(child_id)
        .expect("child module should be retrievable from the graph");

    // The import `use child::some_func` should be registered in the root module scope.
    let root_scope = hir
        .module_scope(stnx::ModuleId::ROOT)
        .expect("root module scope must exist");

    // Find the use declaration for `use child::some_func` and use its alias
    // SymbolId (interned by the HIR's SymbolInterner during lowering).
    let use_decl = hir
        .use_decls
        .iter()
        .find(|ud| {
            ud.path.len() == 2
                && hir.symbols.lookup(ud.path[0]) == Some("child")
                && hir.symbols.lookup(ud.path[1]) == Some("some_func")
        })
        .expect("should find use decl for `child::some_func` in HIR");

    let alias = use_decl.alias;

    assert!(
        root_scope.imports.contains_key(&alias),
        "import for 'some_func' should be registered in root module scope; \
         scope imports: {:?}",
        root_scope.imports
    );

    let imported_def_id = root_scope.imports.get(&alias).copied().unwrap();

    // The imported DefId should resolve to a function named "some_func".
    let imported_func = hir
        .function(imported_def_id)
        .expect("imported DefId should resolve to a function");
    let func_name = hir
        .symbol_name(imported_func.name)
        .expect("function should have a name");
    assert_eq!(
        func_name, "some_func",
        "imported function should be named 'some_func'"
    );

    // The imported function's DefId should be the same one registered as an
    // item in the child module's scope — confirming cross-module resolution
    // reached into the child module to find `some_func`.
    let child_scope = hir
        .module_scope(child_id)
        .expect("child module scope must exist");
    // Look up "some_func" by name in the child module scope.
    let some_func_in_child = child_scope
        .items
        .values()
        .find(|def_id| {
            hir.function(**def_id)
                .map(|f| hir.symbol_name(f.name) == Some("some_func"))
                .unwrap_or(false)
        })
        .copied()
        .expect("child module scope should contain a function named 'some_func'");
    assert_eq!(
        imported_def_id, some_func_in_child,
        "imported DefId should match the function definition in the child module scope"
    );
}
