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
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    /// A free type variable — the name of a generic parameter as it appears
    /// in the source (e.g. `T` in `fn id<T>(x: T) -> T`). Resolved at
    /// monomorphization time by substitution. Not produced by `From<ast::Type>`.
    Generic(SymbolId),
    /// An applied (instantiated) generic type: `Pair<i64, bool>` is
    /// `Apply { base: Pair, args: [I64, Bool] }`. `base` is the
    /// `SymbolId` of the generic struct/enum. Not produced by
    /// `From<ast::Type>` (the parser keeps generics at the item level).
    Apply {
        base: SymbolId,
        args: Vec<HirType>,
    },
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

    /// Whether this type contains a free generic parameter, either
    /// directly (`Generic(_)`) or nested inside an `Apply`'s args.
    /// Used to detect generic callees whose return type cannot be
    /// concretely resolved until monomorphization.
    pub fn contains_generic(&self) -> bool {
        match self {
            HirType::Generic(_) => true,
            HirType::Apply { args, .. } => args.iter().any(|a| a.contains_generic()),
            _ => false,
        }
    }

    /// Substitute each `Generic(s)` according to `subst`. Used to compute
    /// the concrete return type of a generic call given its turbofish type
    /// arguments (e.g. `id::<i64>(x)` produces `I64` from `T`).
    pub fn substitute(&self, subst: &std::collections::HashMap<SymbolId, HirType>) -> HirType {
        match self {
            HirType::Generic(sym) => subst.get(sym).cloned().unwrap_or_else(|| self.clone()),
            HirType::Apply { base, args } => HirType::Apply {
                base: *base,
                args: args.iter().map(|a| a.substitute(subst)).collect(),
            },
            _ => self.clone(),
        }
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
