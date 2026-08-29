//! HIR functions and the top-level program.
//!
//! [`HirFunction`] replaces `ast::Function` with resolved types,
//! [`SymbolId`] parameter names, and [`HirStmt`] bodies.
//! [`HirProgram`] owns the symbol table so later stages (codegen) can
//! resolve names without re-interning.
//!
//! Struct and enum definitions are stored at the program level so that
//! the lowering pass and codegen can both reference them by [`DefId`].
//!
//! ## Module integration (Phase 5B)
//!
//! [`HirProgram`] now carries module metadata alongside the existing
//! `functions`, `structs`, `enums`, and `symbols` fields. These additions
//! are purely structural — `DefId` semantics and the existing fields are
//! unchanged, so single-file programs continue to work exactly as before.
//!
//! The field set mirrors the Phase 3 design doc (sections 9–11):
//!
//! - [`HirProgram::modules`] — one [`Module`](crate::module::Module) per
//!   discovered module, including its `ModuleId`, `ModulePath`, and source
//!   file path.
//! - [`HirProgram::root_module`] — the `ModuleId` of the crate root.
//! - [`HirProgram::module_paths`] — maps each item's `DefId` to its
//!   owning `ModuleId` (O(1) lookup via the [`DefTable`](crate::hir::symbol::DefTable)).
//! - [`HirProgram::def_table`] — the [`DefTable`] that maps every global
//!   `DefId` to `(ModuleId, local_index, DefKind)`.
//! - [`HirProgram::module_scopes`] — per-module name→`DefId` tables
//!   (items + imports), layered on top of the lexical `LowerScope`.
//! - [`HirProgram::use_decls`] / [`HirProgram::mod_decls`] — HIR
//!   representations of `use` and `mod` declarations, tracked for future
//!   Phase 6 resolution.

use crate::hir::stmt::HirStmt;
use crate::hir::symbol::{DefId, SymbolId, SymbolInterner, Visibility};
use crate::hir::types::HirType;
use crate::module::{Module, ModuleId, ModuleScope};
use miette::SourceSpan;
use std::collections::HashMap;

/// A lowered function with resolved types and symbol references.
#[derive(Debug, Clone)]
pub struct HirFunction {
    pub def_id: DefId,
    pub name: SymbolId,
    /// Generic parameter names, interned (`fn id<T>(...)` → `[SymbolId("T")]`).
    /// Empty for non-generic functions. Monomorphization uses this to
    /// substitute concrete types at instantiation time.
    pub generic_params: Vec<SymbolId>,
    pub params: Vec<(SymbolId, HirType)>,
    pub return_type: HirType,
    pub body: Vec<HirStmt>,
    pub span: SourceSpan,
    /// The module that owns this function.
    pub module: ModuleId,
    /// Whether this function was declared `pub`.
    pub visibility: Visibility,
}

/// A lowered struct definition: name + typed fields.
#[derive(Debug, Clone)]
pub struct StructDef {
    pub def_id: DefId,
    pub name: SymbolId,
    /// Generic parameter names, interned (`struct Pair<A, B>` → `[A, B]`).
    /// Empty for non-generic structs.
    pub generic_params: Vec<SymbolId>,
    pub fields: Vec<(SymbolId, HirType)>,
    pub span: SourceSpan,
    /// The module that owns this struct.
    pub module: ModuleId,
    /// Whether this struct was declared `pub`.
    pub visibility: Visibility,
}

/// A lowered enum definition: name + variant names.
/// Each variant is represented as an `i64` tag at the LLVM level.
#[derive(Debug, Clone)]
pub struct EnumDef {
    pub def_id: DefId,
    pub name: SymbolId,
    /// Generic parameter names, interned. Empty for non-generic enums.
    pub generic_params: Vec<SymbolId>,
    pub variants: Vec<SymbolId>,
    pub span: SourceSpan,
    /// The module that owns this enum.
    pub module: ModuleId,
    /// Whether this enum was declared `pub`.
    pub visibility: Visibility,
}

/// HIR representation of a `use foo::bar` declaration.
///
/// `path` is the fully-qualified path (each segment a `SymbolId`),
/// `alias` is the interned name introduced in this module (the last path
/// segment by default), and `module` is the `ModuleId` from which the
/// import was declared.
///
/// Actual path resolution to a target `DefId` is deferred to Phase 6.
#[derive(Debug, Clone)]
pub struct HirUseDecl {
    pub def_id: DefId,
    /// The full path as written, e.g. `[SymbolId("foo"), SymbolId("bar")]`.
    pub path: Vec<SymbolId>,
    /// The name introduced into this module's namespace.
    pub alias: SymbolId,
    /// The module in which this `use` was declared.
    pub module: ModuleId,
    /// Visibility of this `use` declaration.
    pub visibility: Visibility,
    pub span: SourceSpan,
}

/// HIR representation of a `mod foo;` declaration.
///
/// At this phase we record the module name, the resolved `ModuleId` (if the
/// child was discovered), and the owning module. Full dependency tracking
/// is a Phase 6 concern.
#[derive(Debug, Clone)]
pub struct HirModDecl {
    pub def_id: DefId,
    /// The last path segment of the module name (interned).
    pub name: SymbolId,
    /// The child module this declaration refers to, once discovered.
    pub module_id: Option<ModuleId>,
    /// The module in which this `mod` was declared.
    pub module: ModuleId,
    /// Visibility of this `mod` declaration.
    pub visibility: Visibility,
    pub span: SourceSpan,
}

/// The top-level HIR program. Contains all lowered functions, struct
/// definitions, enum definitions, the shared symbol table that
/// maps [`SymbolId`] → `&str`, and module metadata.
#[derive(Debug)]
pub struct HirProgram {
    pub functions: Vec<HirFunction>,
    pub structs: Vec<StructDef>,
    pub enums: Vec<EnumDef>,
    pub symbols: SymbolInterner,
    // --- new for modules ---
    /// All discovered modules (indexed by `ModuleId.0`).
    pub modules: Vec<Module>,
    /// The root (crate) module.
    pub root_module: ModuleId,
    /// Maps each item `DefId` to its owning `ModuleId`.
    /// (Redundant with `def_table` but provided for O(1) lookup without
    /// indexing into the def table's `DefEntry`.)
    pub module_paths: HashMap<DefId, ModuleId>,
    /// The `DefTable`: maps each global `DefId` to `(ModuleId, local_index, DefKind)`.
    pub def_table: crate::hir::symbol::DefTable,
    /// Per-module scope: name→`DefId` for items and imports.
    pub module_scopes: Vec<ModuleScope>,
    /// All `use` declarations encountered during lowering.
    pub use_decls: Vec<HirUseDecl>,
    /// All `mod` declarations encountered during lowering.
    pub mod_decls: Vec<HirModDecl>,
}

impl HirProgram {
    /// Look up a function by its [`DefId`].
    pub fn function(&self, id: DefId) -> Option<&HirFunction> {
        self.functions.get(id.0 as usize)
    }

    /// Resolve a [`SymbolId`] to its interned string.
    pub fn symbol_name(&self, id: SymbolId) -> Option<&str> {
        self.symbols.lookup(id)
    }

    /// Get the function with the given name (by string).
    pub fn function_by_name(&self, name: &str) -> Option<&HirFunction> {
        self.functions
            .iter()
            .find(|f| self.symbols.lookup(f.name) == Some(name))
    }

    /// Look up a struct definition by its [`SymbolId`] name.
    pub fn struct_by_name(&self, name: &str) -> Option<&StructDef> {
        self.structs
            .iter()
            .find(|s| self.symbols.lookup(s.name) == Some(name))
    }

    /// Look up an enum definition by its [`SymbolId`] name.
    pub fn enum_by_name(&self, name: &str) -> Option<&EnumDef> {
        self.enums
            .iter()
            .find(|e| self.symbols.lookup(e.name) == Some(name))
    }

    /// Get the struct definition referenced by a [`HirType::Struct`].
    pub fn struct_def(&self, sym: SymbolId) -> Option<&StructDef> {
        self.structs.iter().find(|s| s.name == sym)
    }

    /// Get the enum definition referenced by a [`HirType::Enum`].
    pub fn enum_def(&self, sym: SymbolId) -> Option<&EnumDef> {
        self.enums.iter().find(|e| e.name == sym)
    }

    // --- module-aware accessors (Phase 5B) ---

    /// Look up the [`ModuleId`] that owns a given [`DefId`].
    /// Returns `None` if the `DefId` is not in the `def_table`.
    pub fn module_of(&self, id: DefId) -> Option<ModuleId> {
        // Fast path: the parallel HashMap is checked first.
        if let Some(&mid) = self.module_paths.get(&id) {
            return Some(mid);
        }
        // Fallback: the DefTable (covers use/mod declarations too).
        self.def_table.lookup(id).map(|entry| entry.module)
    }

    /// Look up the [`DefEntry`] for a [`DefId`], if registered.
    pub fn def_entry(&self, id: DefId) -> Option<&crate::hir::symbol::DefEntry> {
        self.def_table.lookup(id)
    }

    /// Look up a module by its [`ModuleId`].
    pub fn module(&self, id: ModuleId) -> Option<&Module> {
        self.modules.get(id.0 as usize)
    }

    /// Returns the [`ModuleScope`] for a given module, if it exists.
    pub fn module_scope(&self, id: ModuleId) -> Option<&ModuleScope> {
        self.module_scopes.get(id.0 as usize)
    }
}
