//! HIR expressions — the compiler-level expression representation.
//!
//! Every [`HirExpr`] carries its resolved [`HirType`] and a [`SourceSpan`].
//! String identifiers have been replaced with [`SymbolId`] / [`DefId`], so
//! later stages (MIR, LLVM codegen) never need to perform string lookups.

use crate::ast::{AugOp, BinOp, UnOp};
use crate::hir::symbol::{DefId, SymbolId};
use crate::hir::types::HirType;
use miette::SourceSpan;

/// A compiler-level expression with resolved type and source span.
#[derive(Debug, Clone)]
pub struct HirExpr {
    pub kind: HirExprKind,
    /// The fully-resolved type of this expression (no re-derivation needed).
    pub ty: HirType,
    /// Source location preserved from the AST.
    pub span: SourceSpan,
}

/// Expression variants. All identifiers are resolved:
/// - [`HirExprKind::Variable`] uses a [`SymbolId`] (not a `String`)
/// - [`HirExprKind::Call`] uses a [`DefId`] (not a `String`)
#[derive(Debug, Clone)]
pub enum HirExprKind {
    Integer(i64),
    Float(f64),
    Bool(bool),
    StrLit(SymbolId),
    Unit,

    /// A variable reference. `symbol` resolves to a local or parameter SlotId.
    Variable {
        symbol: SymbolId,
    },

    /// Assignment to a mutable variable (validated during lowering).
    Assign {
        symbol: SymbolId,
        value: Box<HirExpr>,
    },

    /// Augmented assignment (`+=`, `-=`, etc.) to a mutable variable.
    AugAssign {
        symbol: SymbolId,
        op: AugOp,
        value: Box<HirExpr>,
    },

    Binary {
        op: BinOp,
        lhs: Box<HirExpr>,
        rhs: Box<HirExpr>,
    },
    Unary {
        op: UnOp,
        expr: Box<HirExpr>,
    },

    /// Function call. `func` is a [`DefId`] referencing a declared function.
    /// `type_args` carries the explicit type arguments from a turbofish
    /// (`f::<T>(x)`); empty when none were supplied. Generic monomorphization
    /// uses this to drive substitution; non-generic callees must receive
    /// an empty vec (validated during lowering).
    Call {
        func: DefId,
        args: Vec<HirExpr>,
        type_args: Vec<HirType>,
    },

    If {
        condition: Box<HirExpr>,
        then_branch: Vec<crate::hir::stmt::HirStmt>,
        elif_branches: Vec<(HirExpr, Vec<crate::hir::stmt::HirStmt>)>,
        else_branch: Option<Vec<crate::hir::stmt::HirStmt>>,
    },

    /// `for var in range` — `iter` must be a [`HirExprKind::Range`]
    /// with resolved `is_inclusive` semantics.
    For {
        var: SymbolId,
        iter: Box<HirExpr>,
        body: Vec<crate::hir::stmt::HirStmt>,
    },

    /// `while condition` loop.
    While {
        condition: Box<HirExpr>,
        body: Vec<crate::hir::stmt::HirStmt>,
    },

    /// `start..end` (exclusive) or `start...end` (inclusive).
    Range {
        start: Box<HirExpr>,
        end: Box<HirExpr>,
        is_inclusive: bool,
    },

    /// Struct construction: `Point { x: 10, y: 20 }`.
    /// `name` is the struct definition [`SymbolId`].
    /// `fields` maps interned field-name → value expression.
    /// `type_args` carries explicit turbofish for generic structs
    /// (`Box::<i64> { value: 21 }`); empty otherwise.
    StructLiteral {
        name: SymbolId,
        fields: Vec<(SymbolId, Box<HirExpr>)>,
        type_args: Vec<HirType>,
    },

    /// Field access: `p.x`.
    /// `field` is the interned field-name [`SymbolId`].
    /// The resolved type is on the enclosing [`HirExpr`].
    FieldAccess {
        expr: Box<HirExpr>,
        field: SymbolId,
    },

    /// List literal: `[1, 2, 3]`. Elements must have uniform type.
    ListLiteral {
        elements: Vec<HirExpr>,
    },

    /// Enum variant construction: `Result::Ok`.
    /// `name` is the enum definition [`SymbolId`].
    /// `variant` is the variant [`SymbolId`].
    EnumConstructor {
        name: SymbolId,
        variant: SymbolId,
    },
}
