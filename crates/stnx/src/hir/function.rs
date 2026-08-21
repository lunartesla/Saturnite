//! HIR functions and the top-level program.
//!
//! [`HirFunction`] replaces `ast::Function` with resolved types,
//! [`SymbolId`] parameter names, and [`HirStmt`] bodies.
//! [`HirProgram`] owns the symbol table so later stages (codegen) can
//! resolve names without re-interning.
//!
//! Struct and enum definitions are stored at the program level so that
//! the lowering pass and codegen can both reference them by [`DefId`].

use crate::hir::stmt::HirStmt;
use crate::hir::symbol::{DefId, SymbolId, SymbolInterner};
use crate::hir::types::HirType;
use miette::SourceSpan;

/// A lowered function with resolved types and symbol references.
#[derive(Debug)]
pub struct HirFunction {
    pub def_id: DefId,
    pub name: SymbolId,
    pub params: Vec<(SymbolId, HirType)>,
    pub return_type: HirType,
    pub body: Vec<HirStmt>,
    pub span: SourceSpan,
}

/// A lowered struct definition: name + typed fields.
#[derive(Debug, Clone)]
pub struct StructDef {
    pub def_id: DefId,
    pub name: SymbolId,
    pub fields: Vec<(SymbolId, HirType)>,
    pub span: SourceSpan,
}

/// A lowered enum definition: name + variant names.
/// Each variant is represented as an `i64` tag at the LLVM level.
#[derive(Debug, Clone)]
pub struct EnumDef {
    pub def_id: DefId,
    pub name: SymbolId,
    pub variants: Vec<SymbolId>,
    pub span: SourceSpan,
}

/// The top-level HIR program. Contains all lowered functions, struct
/// definitions, enum definitions, and the shared symbol table that
/// maps [`SymbolId`] → `&str`.
#[derive(Debug)]
pub struct HirProgram {
    pub functions: Vec<HirFunction>,
    pub structs: Vec<StructDef>,
    pub enums: Vec<EnumDef>,
    pub symbols: SymbolInterner,
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
}
