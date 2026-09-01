//! HIR statements — the compiler-level statement representation.
//!
//! [`HirStmt`] wraps a [`HirStmtKind`] with a [`SourceSpan`], preserving
//! the 0.2 diagnostic infrastructure while carrying resolved symbol IDs.

use crate::hir::expr::HirExpr;
use crate::hir::symbol::SymbolId;
use crate::hir::types::HirType;
use miette::SourceSpan;

/// A compiler-level statement.
#[derive(Debug, Clone)]
pub struct HirStmt {
    pub kind: HirStmtKind,
    pub span: SourceSpan,
}

/// Statement variants. Variable names are [`SymbolId`]s (resolved
/// during lowering), types are [`HirType`]s.
#[derive(Debug, Clone)]
pub enum HirStmtKind {
    /// `let name: Type = expr;` or `let mut name: Type = expr;`
    Let {
        name: SymbolId,
        mutable: bool,
        ty: Option<HirType>,
        value: HirExpr,
    },

    /// A bare expression statement: `expr;`
    Expr(HirExpr),

    /// `return expr;` or `return;`
    Return(Option<HirExpr>),

    /// `println(expr);` — builtin call to `println_i64`.
    Println(HirExpr),

    /// 0.5: `raise expr;` — lowers to a stub that prints the message and
    /// aborts the process. Real error semantics are deferred to a later
    /// phase. The MIR→LLVM backend lowers this to a `println` followed by
    /// `llvm.trap`.
    Raise(HirExpr),

    /// A struct definition: `struct Point { x: i64, y: i64 }`.
    /// `name` is the struct definition [`SymbolId`].
    /// `fields` maps interned field-name → [`HirType`].
    StructDef {
        name: SymbolId,
        fields: Vec<(SymbolId, HirType)>,
    },

    /// An enum definition: `enum Result { Ok, Error }`.
    /// `name` is the enum definition [`SymbolId`].
    /// `variants` is a list of interned variant names.
    EnumDef {
        name: SymbolId,
        variants: Vec<SymbolId>,
    },
}
