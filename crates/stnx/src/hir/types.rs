//! Compiler-internal type representation for HIR.
//!
//! `ast::Type` and `HirType` currently map 1:1 (I64, F64, Bool, Str, Unit),
//! but `HirType` is a distinct compiler-internal type that is independent
//! from parser syntax. Future language features (enums, generics, user types)
//! can extend `HirType` without touching the parser.
//!
//! For structs and enums, the compiler-internal type is a `SymbolId`
//! referencing the definition. This allows the type system to express
//! user-defined types without string lookups in later stages.

use crate::hir::symbol::SymbolId;
use serde::{Deserialize, Serialize};

/// Compiler-internal type used throughout HIR and consumed by codegen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HirType {
    I64,
    F64,
    Bool,
    Str,
    Unit,
    /// A named struct type. `SymbolId` references the struct definition
    /// stored in [`HirProgram::structs`](crate::hir::HirProgram).
    Struct(SymbolId),
    /// A named enum type. At the LLVM level this is represented as an
    /// `i64` tag so codegen is straightforward; the `SymbolId` enables
    /// future pattern-matching / variant discrimination.
    Enum(SymbolId),
}

impl HirType {
    /// The default / "empty" type for statements that produce no value.
    pub fn unit() -> Self {
        HirType::Unit
    }

    /// Whether this type is `Unit` (i.e., `()`).
    pub fn is_unit(&self) -> bool {
        matches!(self, HirType::Unit)
    }
}

impl From<crate::ast::Type> for HirType {
    fn from(ty: crate::ast::Type) -> Self {
        match ty {
            crate::ast::Type::I64 => HirType::I64,
            crate::ast::Type::F64 => HirType::F64,
            crate::ast::Type::Bool => HirType::Bool,
            crate::ast::Type::Str => HirType::Str,
            crate::ast::Type::Unit => HirType::Unit,
            crate::ast::Type::Struct(_) => HirType::Struct(SymbolId(0)),
            crate::ast::Type::Enum(_) => HirType::Enum(SymbolId(0)),
        }
    }
}
