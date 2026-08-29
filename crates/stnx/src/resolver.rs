//! Dedicated name resolution pass for Saturnite.
//!
//! This module is the authoritative owner of the resolution phase that
//! runs after HIR lowering. It is intentionally separate from
//! `hir::lower` so that:
//!
//! * name resolution is testable in isolation,
//! * future privacy / visibility / use-glob / use-alias work has a
//!   single home,
//! * the resolver can be re-run after incremental changes without
//!   re-lowering the AST.
//!
//! ## Pipeline position
//!
//! ```text
//! AST
//!  ↓
//! ModuleGraph
//!  ↓
//! HIR lowering (hir::lower)
//!  ↓
//! Resolution (resolver::resolve)        ← this module
//!  ↓
//! MIR lowering
//! ```
//!
//! ## What this pass does
//!
//! For every [`HirUseDecl`](crate::hir::HirUseDecl) in the program:
//!
//! 1. The first path segment is treated as a module name. We look it up
//!    in the [`ModuleGraph`] by comparing the segment's interned
//!    `SymbolId` against each module's path's last segment.
//! 2. Subsequent segments are walked through the target module's
//!    [`ModuleScope`](crate::module::ModuleScope), using
//!    `lookup_with_parent` so that items inherited from a parent module
//!    are reachable (Rust 2018 style).
//! 3. The final segment's `DefId` is recorded as the import target and
//!    registered in the declaring module's scope under the use-decl's
//!    alias (which defaults to the last path segment).
//!
//! The pass also defensively re-registers structs, enums, and
//! `mod` declarations in their owning module's scope so that resolution
//! succeeds even if lowering did not pre-populate the scope.
//!
//! ## What this pass does NOT do (yet)
//!
//! * Privacy / visibility enforcement — tracked but not gated.
//! * Glob imports (`use foo::*`).
//! * Renaming imports that collide with local items.
//! * Re-exports (`pub use foo::Bar`).
//! * Cycle detection across use edges.
//!
//! These are queued for follow-up milestones (see
//! `docs/SATURNITE_1_0_ROADMAP.md`).

use crate::error::{CompilerError, CompilerResult};
use crate::hir::symbol::{DefId, DefKind, SymbolId};
use crate::hir::HirProgram;

// ---------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------

/// The output of the resolution pass.
///
/// This struct is the bridge between name resolution and downstream
/// stages. The body is currently side-effect-free: the resolver mutates
/// `HirProgram` in place to register imports, and the `Resolution` value
/// records *what was resolved* so callers (tests, diagnostics, future
/// incremental layers) can inspect the result without re-scanning the
/// program.
///
/// ## Fields
///
/// * `imports` — for each `use` declaration (in declaration order), the
///   `DefId` it resolves to. The order matches `HirProgram::use_decls`.
/// * `unresolved` — indices into `HirProgram::use_decls` that could not
///   be resolved. Normally the resolver returns an `Err` on the first
///   unresolved use, but collecting the list is useful for tests and
///   for future "report-all-errors" modes.
/// * `mod_resolutions` — for each `mod` declaration, the `ModuleId` it
///   resolves to (already populated by lowering, but copied here so the
///   resolver owns a self-contained view of all module bindings).
#[derive(Debug, Default, Clone)]
pub struct Resolution {
    /// Per-use-decl resolution results, in declaration order.
    /// The length equals `HirProgram::use_decls.len()`.
    pub imports: Vec<Option<DefId>>,
    /// Indices of unresolved use declarations.
    pub unresolved: Vec<usize>,
    /// Per-mod-decl resolved `ModuleId`s, in declaration order.
    /// `None` means the child module was not discovered.
    pub mod_resolutions: Vec<Option<crate::module::ModuleId>>,
}

impl Resolution {
    /// Number of use declarations that were successfully resolved.
    pub fn resolved_count(&self) -> usize {
        self.imports.iter().filter(|d| d.is_some()).count()
    }

    /// Number of use declarations that failed to resolve.
    pub fn unresolved_count(&self) -> usize {
        self.unresolved.len()
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Run the dedicated resolution pass over a freshly lowered HIR program.
///
/// On success, every `use` declaration is registered in its declaring
/// module's [`ModuleScope`](crate::module::ModuleScope) under the
/// declared alias, and the returned [`Resolution`] reports what was
/// resolved.
///
/// On failure, returns the first resolution error and leaves the
/// program's `module_scopes` in a partially-updated state (this is
/// intentional: the program is unusable past this point anyway).
pub fn resolve(hir: &mut HirProgram) -> CompilerResult<Resolution> {
    let mut out = Resolution {
        imports: vec![None; hir.use_decls.len()],
        unresolved: Vec::new(),
        mod_resolutions: hir.mod_decls.iter().map(|md| md.module_id).collect(),
    };

    // --- Phase 0: detect duplicate definitions within each module ---
    //
    // We walk every kind of item (function, struct, enum, mod decl) and
    // group by (module, name). If any group has more than one entry, the
    // program contains a duplicate definition and we surface a semantic
    // error pointing at the duplicated name.
    //
    // This catches cases that lowering's HashMap-insert silently overwrites,
    // e.g. two `fn foo` in the same module, or a `fn foo` colliding with a
    // `struct foo` in the same module.
    {
        use std::collections::HashMap;
        let mut seen: HashMap<(crate::module::ModuleId, SymbolId), DefId> = HashMap::new();
        let mut check = |module: crate::module::ModuleId,
                         name: SymbolId,
                         def_id: DefId|
         -> Result<(), CompilerError> {
            if let Some(prev) = seen.insert((module, name), def_id) {
                if prev != def_id {
                    let n = hir.symbols.lookup(name).unwrap_or("<unknown>");
                    return Err(CompilerError::semantic(format!(
                        "duplicate definition: '{}' is already defined in this module",
                        n
                    )));
                }
            }
            Ok(())
        };
        for f in &hir.functions {
            check(f.module, f.name, f.def_id)?;
        }
        for s in &hir.structs {
            check(s.module, s.name, s.def_id)?;
        }
        for e in &hir.enums {
            check(e.module, e.name, e.def_id)?;
        }
        for md in &hir.mod_decls {
            check(md.module, md.name, md.def_id)?;
        }
    }

    // --- Phase 0: detect duplicate definitions within each module ---
    //
    // We walk every kind of item (function, struct, enum, mod decl) and
    // group by (module, name). If any group has more than one entry, the
    // program contains a duplicate definition and we surface a semantic
    // error pointing at the duplicated name.
    //
    // This catches cases that lowering's HashMap-insert silently overwrites,
    // e.g. two `fn foo` in the same module, or a `fn foo` colliding with a
    // `struct foo` in the same module.
    {
        use std::collections::HashMap;
        let mut seen: HashMap<(crate::module::ModuleId, SymbolId), DefId> = HashMap::new();
        let mut check = |module: crate::module::ModuleId,
                         name: SymbolId,
                         def_id: DefId|
         -> Result<(), CompilerError> {
            if let Some(prev) = seen.insert((module, name), def_id) {
                if prev != def_id {
                    let n = hir.symbols.lookup(name).unwrap_or("<unknown>");
                    return Err(CompilerError::semantic(format!(
                        "duplicate definition: '{}' is already defined in this module",
                        n
                    )));
                }
            }
            Ok(())
        };
        for f in &hir.functions {
            check(f.module, f.name, f.def_id)?;
        }
        for s in &hir.structs {
            check(s.module, s.name, s.def_id)?;
        }
        for e in &hir.enums {
            check(e.module, e.name, e.def_id)?;
        }
        for md in &hir.mod_decls {
            check(md.module, md.name, md.def_id)?;
        }
    }

    // --- Phase 1: defensive re-registration of items in their scopes ---
    //
    // Lowering already populates module_scopes, but we re-register here
    // as a safety net so the resolver is self-contained and never
    // depends on lowering's internal ordering. Duplicate definitions are
    // already detected by Phase 0, so this phase is a no-op when no
    // duplicates exist; it just ensures every item has an entry in the
    // owning module's scope even if lowering skipped one.
    for s in &hir.structs {
        if let Some(scope) = hir.module_scopes.get_mut(s.module.0 as usize) {
            scope.define_item(s.name, s.def_id);
        }
    }
    for e in &hir.enums {
        if let Some(scope) = hir.module_scopes.get_mut(e.module.0 as usize) {
            scope.define_item(e.name, e.def_id);
        }
    }
    for md in &hir.mod_decls {
        if let Some(scope) = hir.module_scopes.get_mut(md.module.0 as usize) {
            scope.define_item(md.name, md.def_id);
        }
    }

    // --- Phase 2: resolve each use declaration ---
    //
    // We collect the resolutions into a side buffer first to avoid
    // borrow-checker conflicts between iterating `use_decls` and
    // mutating `module_scopes`.
    let mut pending: Vec<(usize, SymbolId, DefId)> = Vec::new();

    for (use_idx, _use_decl) in hir.use_decls.iter().enumerate() {
        match resolve_one_use(hir, use_idx) {
            Ok(Some((alias, def_id))) => {
                out.imports[use_idx] = Some(def_id);
                pending.push((use_idx, alias, def_id));
            }
            Ok(None) => {
                out.unresolved.push(use_idx);
            }
            Err(e) => return Err(e),
        }
    }

    // --- Phase 3: apply resolutions to module_scopes ---
    for (use_idx, alias, def_id) in &pending {
        let module_id = hir.use_decls[*use_idx].module;
        if let Some(scope) = hir.module_scopes.get_mut(module_id.0 as usize) {
            scope.define_import(*alias, *def_id);
        }
    }

    Ok(out)
}

/// Resolve a single `use` declaration by index.
///
/// Returns:
/// * `Ok(Some((alias, def_id)))` on success.
/// * `Ok(None)` if the path could not be resolved to an item
///   (caller decides whether that's an error).
/// * `Err(_)` for hard errors (empty path, interned symbol missing,
///   invalid module name).
fn resolve_one_use(hir: &HirProgram, use_idx: usize) -> CompilerResult<Option<(SymbolId, DefId)>> {
    let use_decl = &hir.use_decls[use_idx];

    if use_decl.path.is_empty() {
        return Err(CompilerError::semantic(format!(
            "unresolved import: empty path in use declaration at {:?}",
            use_decl.span
        )));
    }

    let mut path_iter = use_decl.path.iter().copied();
    let first_segment = path_iter.next().unwrap();

    let first_name = hir.symbols.lookup(first_segment).ok_or_else(|| {
        CompilerError::semantic(format!(
            "unresolved import: cannot look up symbol {:?}",
            first_segment
        ))
    })?;

    // Step 1: find the target module by matching the first segment
    // against each module's last path segment.
    let target_module = hir
        .modules
        .iter()
        .find(|m| m.path.name(&hir.symbols) == Some(first_name))
        .filter(|m| !m.is_root() || hir.modules.len() == 1);

    if let Some(target_mod) = target_module {
        // Step 2: walk remaining segments through the target module's scope.
        let mut current_module_id = target_mod.id;
        let mut resolved_def_id: Option<DefId> = None;

        for segment in path_iter {
            let segment_name = hir.symbols.lookup(segment).ok_or_else(|| {
                CompilerError::semantic(format!(
                    "unresolved import: cannot look up path segment {:?}",
                    segment
                ))
            })?;

            let Some(target_scope) = hir.module_scope(current_module_id) else {
                return Err(CompilerError::semantic(format!(
                    "unresolved import: no scope for module {:?}",
                    current_module_id
                )));
            };

            match target_scope.lookup_with_parent(&segment, &hir.module_scopes) {
                Some(def_id) => {
                    resolved_def_id = Some(def_id);
                    // For module declarations, module_of returns the
                    // parent module (where `mod` was declared). We need
                    // to follow into the child module instead. For other
                    // items, module_of returns the owning module, which
                    // is correct for continuing the search.
                    current_module_id = if let Some(entry) = hir.def_table.lookup(def_id) {
                        if entry.kind == DefKind::Module {
                            hir.mod_decls
                                .iter()
                                .find(|md| md.def_id == def_id)
                                .and_then(|md| md.module_id)
                                .unwrap_or(current_module_id)
                        } else {
                            hir.module_of(def_id).unwrap_or(current_module_id)
                        }
                    } else {
                        hir.module_of(def_id).unwrap_or(current_module_id)
                    };
                }
                None => {
                    let mod_name = hir
                        .module(current_module_id)
                        .and_then(|m| m.path.name(&hir.symbols))
                        .unwrap_or("<root>");
                    return Err(CompilerError::semantic(format!(
                        "unresolved import: '{}' not found in module '{}'",
                        segment_name, mod_name
                    )));
                }
            }
        }

        // No remaining segments: the path was just the module name.
        // The first segment did not name an item, so this use has no
        // resolvable target.
        let Some(target_def_id) = resolved_def_id else {
            return Err(CompilerError::semantic(format!(
                "unresolved import: '{}' is a module, not an item. \
                 Use 'mod {};' to declare the module instead of importing it.",
                first_name, first_name
            )));
        };

        Ok(Some((use_decl.alias, target_def_id)))
    } else if use_decl.path.len() == 1 {
        // Single-segment path that didn't match any module: treat as
        // an item in the current module's scope.
        let Some(current_scope) = hir.module_scope(use_decl.module) else {
            return Err(CompilerError::semantic(format!(
                "unresolved import: no scope for current module {:?}",
                use_decl.module
            )));
        };

        match current_scope.lookup_with_parent(&first_segment, &hir.module_scopes) {
            Some(def_id) => Ok(Some((use_decl.alias, def_id))),
            None => Ok(None),
        }
    } else {
        Err(CompilerError::semantic(format!(
            "unresolved import: module '{}' not found",
            first_name
        )))
    }
}

// ---------------------------------------------------------------------------
// Backward-compatible thin wrapper
// ---------------------------------------------------------------------------

/// Backward-compatible wrapper around [`resolve`].
///
/// Existing callers (`semantic::analyze_and_lower_with_graph` and tests)
/// still use the `hir::lower::resolve_modules` symbol. This wrapper
/// preserves that surface area while delegating to the new dedicated
/// resolver.
///
/// On a non-empty `unresolved` list this returns an error so that the
/// legacy `Err`-on-unresolved contract is preserved for callers that
/// only inspect the `Result`. The detailed `Resolution` value (with
/// per-decl status) is still available by calling [`resolve`] directly.
pub fn resolve_modules(hir: &mut HirProgram) -> CompilerResult<()> {
    let res = resolve(hir)?;
    if let Some(&idx) = res.unresolved.first() {
        let use_decl = &hir.use_decls[idx];
        let name = hir
            .symbols
            .lookup(use_decl.path.first().copied().unwrap_or(SymbolId(0)))
            .unwrap_or("<unknown>");
        return Err(CompilerError::semantic(format!(
            "unresolved import: '{}' not found in current module",
            name
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Program;
    use crate::hir::lower;
    use crate::lexer::Lexer;
    use crate::parser;
    use crate::semantic::analyze_and_lower_with_graph;

    fn lex_parse(src: &str) -> Program {
        let tokens: Vec<_> = Lexer::new(src)
            .collect::<Result<Vec<_>, _>>()
            .expect("lexing should succeed");
        parser::parse(src, tokens).expect("parsing should succeed")
    }

    fn lower_src(src: &str) -> HirProgram {
        let program = lex_parse(src);
        lower::lower(&program).expect("lowering should succeed")
    }

    #[test]
    fn test_resolve_empty_program_returns_empty_resolution() {
        let mut hir = lower_src("fn main() -> i64 { 0 }");
        let res = resolve(&mut hir).expect("resolve should succeed");
        assert!(res.imports.is_empty());
        assert!(res.unresolved.is_empty());
        assert_eq!(res.resolved_count(), 0);
        assert_eq!(res.unresolved_count(), 0);
    }

    #[test]
    fn test_resolution_default_is_empty() {
        let r = Resolution::default();
        assert!(r.imports.is_empty());
        assert!(r.unresolved.is_empty());
        assert_eq!(r.resolved_count(), 0);
    }

    fn make_use(hir: &mut HirProgram, name: &str) {
        let sym = hir.symbols.intern(name);
        // Allocate a fresh use DefId by inserting a synthetic
        // "__def_N" symbol (mirrors `HirLower::next_def_id`).
        let synthetic = hir
            .symbols
            .intern(&format!("__def_use_{}", hir.use_decls.len()));
        hir.use_decls.push(crate::hir::HirUseDecl {
            def_id: DefId(synthetic.0),
            path: vec![sym],
            alias: sym,
            module: crate::module::ModuleId::ROOT,
            visibility: crate::hir::Visibility::Private,
            span: miette::SourceSpan::new(0.into(), 0),
        });
    }

    #[test]
    fn test_resolve_single_use_local_item_succeeds() {
        // `use foo` where `foo` is a function in the same module.
        let src = "fn foo() -> i64 { 1 } fn main() -> i64 { foo() }";
        let mut hir = lower_src(src);
        let foo_def = hir.function_by_name("foo").expect("foo function").def_id;
        let foo_sym = hir.symbols.intern("foo");
        make_use(&mut hir, "foo");
        let res = resolve(&mut hir).expect("resolve should succeed");
        assert_eq!(res.imports.len(), 1);
        assert_eq!(res.imports[0], Some(foo_def));
        assert_eq!(res.resolved_count(), 1);
        assert_eq!(res.unresolved_count(), 0);
        // The alias should now be resolvable in the root scope.
        let root_scope = &hir.module_scopes[0];
        assert_eq!(root_scope.imports.get(&foo_sym).copied(), Some(foo_def));
    }

    #[test]
    fn test_resolve_unknown_single_segment_marks_unresolved() {
        let src = "fn main() -> i64 { 0 }";
        let mut hir = lower_src(src);
        make_use(&mut hir, "ghost");
        let res = resolve(&mut hir).expect("resolve should succeed with None");
        assert_eq!(res.imports.len(), 1);
        assert_eq!(res.imports[0], None);
        assert_eq!(res.unresolved_count(), 1);
    }

    #[test]
    fn test_resolve_empty_path_is_hard_error() {
        let src = "fn main() -> i64 { 0 }";
        let mut hir = lower_src(src);
        // Push a use with an empty path.
        let synthetic = hir
            .symbols
            .intern(&format!("__def_use_{}", hir.use_decls.len()));
        hir.use_decls.push(crate::hir::HirUseDecl {
            def_id: DefId(synthetic.0),
            path: vec![],
            alias: hir.symbols.intern(""),
            module: crate::module::ModuleId::ROOT,
            visibility: crate::hir::Visibility::Private,
            span: miette::SourceSpan::new(0.into(), 0),
        });
        let err = resolve(&mut hir).expect_err("empty path should error");
        let msg = format!("{:?}", err);
        assert!(msg.contains("empty path"), "msg was: {}", msg);
    }

    #[test]
    fn test_resolve_mod_decls_resolution_field() {
        // A program with a mod decl (the child module does not exist
        // since we use a single-file program) — module_id should be
        // None and the Resolution's mod_resolutions should reflect that.
        let src = "mod ghost fn main() -> i64 { 0 }";
        let mut hir = lower_src(src);
        // Lowering records the mod decl, but module_id is None because
        // the child was not discovered.
        assert_eq!(hir.mod_decls.len(), 1);
        assert_eq!(hir.mod_decls[0].module_id, None);
        let res = resolve(&mut hir).expect("resolve should succeed");
        assert_eq!(res.mod_resolutions.len(), 1);
        assert_eq!(res.mod_resolutions[0], None);
    }

    #[test]
    fn test_resolve_preserves_existing_module_scopes() {
        // A fresh program with no use decls should leave module_scopes
        // unchanged in count and basic shape.
        let mut hir = lower_src("fn main() -> i64 { 0 }");
        let before = hir.module_scopes.len();
        let _ = resolve(&mut hir).expect("resolve should succeed");
        assert_eq!(hir.module_scopes.len(), before);
    }

    #[test]
    fn test_resolve_through_analyze_and_lower_with_graph() {
        // Smoke test: the wrapper at semantic::analyze_and_lower_with_graph
        // still works end-to-end after the extraction.
        use crate::module::Project;
        use std::io::Write;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path();
        std::fs::write(
            path.join("saturn.toml"),
            "[package]\nname = \"res_smoke\"\nversion = \"0.0.1\"\nedition = \"2024\"\n",
        )
        .unwrap();
        let mut main = std::fs::File::create(path.join("main.stnx")).unwrap();
        main.write_all(b"fn main() -> i64 { 0 }").unwrap();
        let mut project = Project::discover(path).expect("project discover");
        let entry = path.join("main.stnx");
        let _ = project.load_from(&entry).expect("project load_from");
        let program = project
            .graph
            .root_module()
            .ast
            .clone()
            .expect("root module AST present after load_from");
        let mut hir = analyze_and_lower_with_graph(&program, &project.graph)
            .expect("analyze_and_lower_with_graph should succeed");
        let res = resolve(&mut hir).expect("resolve should succeed");
        assert!(res.unresolved.is_empty());
    }
}
