//! Integration tests for the dedicated `resolver` module (Phase 1 of the 1.0
//! roadmap).
//!
//! These tests exercise the [`crate::resolver`] public surface — both the
//! `resolve` function (which returns a structured [`Resolution`]) and the
//! `resolve_modules` shim (which preserves the legacy `Err`-on-unresolved
//! contract).
//!
//! The tests verify:
//!
//! 1. **Empty program** — `resolve` on a program with no `use` declarations
//!    returns an empty `Resolution`.
//! 2. **Single-use local item** — `use foo` where `foo` is a function in the
//!    same module resolves to that function's `DefId`, and the alias is
//!    registered in the module scope.
//! 3. **Unknown single segment** — `use ghost` where `ghost` does not exist
//!    produces a `Resolution` with `unresolved_count() == 1` (caller can
//!    decide how to react). The legacy shim still surfaces this as an error.
//! 4. **Empty path** — `use ;` (empty path) is a hard error in both the new
//!    and legacy surfaces.
//! 5. **`mod ghost` (no child)** — a `mod ghost;` declaration whose target
//!    module was not discovered leaves the `Resolution.mod_resolutions`
//!    entry as `None`.
//! 6. **Idempotence** — calling `resolve` twice on the same program does
//!    not grow or shrink `module_scopes`.
//! 7. **Resolution::default** — the default value is empty.
//! 8. **Duplicate function** — two `fn foo` in the same module produce a
//!    "duplicate definition" error.
//! 9. **Duplicate struct** — two `struct Point` in the same module produce
//!    a "duplicate definition" error.
//! 10. **Duplicate function shim** — the legacy `resolve_modules` shim also
//!     surfaces the duplicate as an error.

use stnx::ast::Program;
use stnx::hir::lower::lower;
use stnx::hir::{HirProgram, HirUseDecl, SymbolId, Visibility};
use stnx::lexer::Lexer;
use stnx::module::ModuleId;
use stnx::parser;
use stnx::resolver::{resolve, resolve_modules, Resolution};

/// Lex + parse + lower a source string into a `HirProgram`.
fn lex_parse_lower(src: &str) -> HirProgram {
    let tokens: Vec<_> = Lexer::new(src)
        .collect::<Result<Vec<_>, _>>()
        .expect("lexing should succeed");
    let program: Program = parser::parse(src, tokens).expect("parsing should succeed");
    lower(&program).expect("lowering should succeed")
}

/// Push a synthetic `use foo;` declaration onto an existing `HirProgram` so
/// the resolver has work to do. The function is for tests that don't have
/// a real `use` syntax path in the source string.
fn push_use(hir: &mut HirProgram, name: &str) {
    let sym = hir.symbols.intern(name);
    let synthetic = hir
        .symbols
        .intern(&format!("__def_use_{}", hir.use_decls.len()));
    hir.use_decls.push(HirUseDecl {
        def_id: stnx::DefId(synthetic.0),
        path: vec![sym],
        alias: sym,
        module: ModuleId::ROOT,
        visibility: Visibility::Private,
        span: miette::SourceSpan::new(0.into(), 0),
    });
}

// ---------------------------------------------------------------------------
// 1. Empty program
// ---------------------------------------------------------------------------

#[test]
fn test_resolver_empty_program_returns_empty_resolution() {
    let mut hir = lex_parse_lower("fn main() -> i64 { 0 }");
    let res = resolve(&mut hir).expect("resolve should succeed");
    assert!(
        res.imports.is_empty(),
        "no use declarations → no resolutions"
    );
    assert!(
        res.unresolved.is_empty(),
        "no use declarations → no unresolved"
    );
    assert_eq!(res.resolved_count(), 0);
    assert_eq!(res.unresolved_count(), 0);
    assert!(
        res.mod_resolutions.is_empty(),
        "no mod declarations → no mod resolutions"
    );
}

// ---------------------------------------------------------------------------
// 2. Single-use local item
// ---------------------------------------------------------------------------

#[test]
fn test_resolver_single_use_local_item_succeeds() {
    // `fn foo` defined in the same module; `use foo` should resolve to it.
    let src = "fn foo() -> i64 { 1 } fn main() -> i64 { foo() }";
    let mut hir = lex_parse_lower(src);
    let foo_def = hir
        .function_by_name("foo")
        .expect("foo function exists after lowering")
        .def_id;
    let foo_sym: SymbolId = hir.symbols.intern("foo");
    push_use(&mut hir, "foo");

    let res = resolve(&mut hir).expect("resolve should succeed");
    assert_eq!(res.imports.len(), 1, "one use → one resolution");
    assert_eq!(
        res.imports[0],
        Some(foo_def),
        "use foo should resolve to foo's DefId"
    );
    assert_eq!(res.resolved_count(), 1);
    assert_eq!(res.unresolved_count(), 0);

    // The alias should be registered in the root module scope's imports.
    let root_scope = &hir.module_scopes[0];
    assert_eq!(
        root_scope.imports.get(&foo_sym).copied(),
        Some(foo_def),
        "alias should be registered in the root module scope's imports table"
    );
}

// ---------------------------------------------------------------------------
// 3. Unknown single segment
// ---------------------------------------------------------------------------

#[test]
fn test_resolver_unknown_single_segment_marks_unresolved() {
    let mut hir = lex_parse_lower("fn main() -> i64 { 0 }");
    push_use(&mut hir, "ghost");

    let res = resolve(&mut hir).expect("resolve returns Ok with unresolved list");
    assert_eq!(res.imports.len(), 1);
    assert_eq!(res.imports[0], None, "unresolved use has None target");
    assert_eq!(res.resolved_count(), 0);
    assert_eq!(
        res.unresolved_count(),
        1,
        "the unknown use should appear in `unresolved`"
    );
    assert_eq!(res.unresolved, vec![0]);

    // The legacy shim must surface this as an error so callers that
    // only inspect the `Result` still get the old behavior.
    let err = resolve_modules(&mut hir).expect_err("legacy shim should error");
    let msg = err.to_string();
    assert!(
        msg.contains("unresolved import"),
        "error should mention 'unresolved import', got: {}",
        msg
    );
}

// ---------------------------------------------------------------------------
// 4. Empty path
// ---------------------------------------------------------------------------

#[test]
fn test_resolver_empty_path_is_hard_error() {
    let mut hir = lex_parse_lower("fn main() -> i64 { 0 }");
    // Push a use with an empty path.
    let synthetic = hir
        .symbols
        .intern(&format!("__def_use_{}", hir.use_decls.len()));
    hir.use_decls.push(HirUseDecl {
        def_id: stnx::DefId(synthetic.0),
        path: vec![],
        alias: hir.symbols.intern(""),
        module: ModuleId::ROOT,
        visibility: Visibility::Private,
        span: miette::SourceSpan::new(0.into(), 0),
    });

    let err = resolve(&mut hir).expect_err("empty path should error");
    assert!(err.to_string().contains("empty path"));

    // Legacy shim also errors.
    let err2 = resolve_modules(&mut hir).expect_err("legacy shim should also error");
    assert!(err2.to_string().contains("empty path"));
}

// ---------------------------------------------------------------------------
// 5. mod ghost; (no child discovered)
// ---------------------------------------------------------------------------

#[test]
fn test_resolver_mod_decls_resolution_field() {
    // `mod ghost` declares a child module that isn't actually present in the
    // single-file program; the resolution entry should be None.
    let mut hir = lex_parse_lower("mod ghost fn main() -> i64 { 0 }");
    assert_eq!(hir.mod_decls.len(), 1);
    assert_eq!(
        hir.mod_decls[0].module_id, None,
        "child module not discovered → module_id is None"
    );
    let res = resolve(&mut hir).expect("resolve should succeed");
    assert_eq!(res.mod_resolutions.len(), 1);
    assert_eq!(res.mod_resolutions[0], None);
}

// ---------------------------------------------------------------------------
// 6. Idempotence
// ---------------------------------------------------------------------------

#[test]
fn test_resolver_idempotent_module_scopes() {
    let mut hir = lex_parse_lower("fn main() -> i64 { 0 }");
    let before = hir.module_scopes.len();
    let _ = resolve(&mut hir).expect("first resolve succeeds");
    assert_eq!(hir.module_scopes.len(), before);
    let _ = resolve(&mut hir).expect("second resolve succeeds");
    assert_eq!(
        hir.module_scopes.len(),
        before,
        "second resolve must not grow module_scopes"
    );
}

// ---------------------------------------------------------------------------
// 7. Resolution::default
// ---------------------------------------------------------------------------

#[test]
fn test_resolution_default_is_empty() {
    let r = Resolution::default();
    assert!(r.imports.is_empty());
    assert!(r.unresolved.is_empty());
    assert!(r.mod_resolutions.is_empty());
    assert_eq!(r.resolved_count(), 0);
    assert_eq!(r.unresolved_count(), 0);
}

// ---------------------------------------------------------------------------
// 8. Duplicate function definitions in the same module
// ---------------------------------------------------------------------------

#[test]
fn test_resolver_detects_duplicate_function_in_same_module() {
    // Two `fn foo` in the same (root) module — the resolver's Phase 0
    // should detect this and return an error.
    let mut hir =
        lex_parse_lower("fn foo() -> i64 { 1 } fn foo() -> i64 { 2 } fn main() -> i64 { 0 }");
    let err = resolve(&mut hir).expect_err("duplicate fn should error");
    let msg = err.to_string();
    assert!(
        msg.contains("duplicate definition") && msg.contains("foo"),
        "error should mention 'duplicate definition' and 'foo', got: {}",
        msg
    );
    // The legacy shim also errors.
    let err2 = resolve_modules(&mut hir).expect_err("legacy shim should also error");
    assert!(err2.to_string().contains("duplicate definition"));
}

// ---------------------------------------------------------------------------
// 9. Duplicate struct definitions in the same module
// ---------------------------------------------------------------------------

#[test]
fn test_resolver_detects_duplicate_struct_in_same_module() {
    let mut hir =
        lex_parse_lower("struct Point { x: i64 } struct Point { y: i64 } fn main() -> i64 { 0 }");
    let err = resolve(&mut hir).expect_err("duplicate struct should error");
    let msg = err.to_string();
    assert!(
        msg.contains("duplicate definition") && msg.contains("Point"),
        "error should mention 'duplicate definition' and 'Point', got: {}",
        msg
    );
}
