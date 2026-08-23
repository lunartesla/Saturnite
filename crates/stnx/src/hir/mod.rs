//! HIR (High-level Intermediate Representation) module for Saturnite 0.3.
//!
//! The HIR is the compiler's single authoritative semantic representation.
//! It replaces the 0.2 design where `ast::Program` was the sole IR shared
//! by both `semantic.rs` and `codegen/context.rs`.
//!
//! ## Pipeline (0.3)
//!
//! ```text
//! Source → Lexer → Parser → AST → HIR Lowering → Typed HIR → LLVM Codegen
//! ```
//!
//! The AST preserves what the programmer wrote (syntax + spans). The HIR
//! represents what the compiler understands (resolved identifiers as
//! [`SymbolId`] / [`DefId`], resolved types on every expression).
//!
//! ## Sub-modules
//!
//! - [`symbol`] — `SymbolId`, `DefId`, `SymbolInterner`
//! - [`types`] — `HirType` (compiler-internal type enum)
//! - [`expr`] — `HirExpr`, `HirExprKind` (resolved expressions)
//! - [`stmt`] — `HirStmt`, `HirStmtKind` (resolved statements)
//! - [`function`] — `HirFunction`, `HirProgram` (top-level structures)
//! - [`lower`] — `lower()` function: AST → HIR (absorbs `semantic::analyze`)

pub mod expr;
pub mod function;
pub mod lower;
pub mod stmt;
pub mod symbol;
pub mod types;

// Re-export the most commonly used types at the module root.
pub use expr::{HirExpr, HirExprKind};
pub use function::{EnumDef, HirFunction, HirModDecl, HirProgram, HirUseDecl, StructDef};
pub use lower::{lower_unit_with_graph, lower_with_graph, resolve_modules, HirLower};
pub use stmt::{HirStmt, HirStmtKind};
pub use symbol::{DefEntry, DefId, DefKind, DefTable, SymbolId, SymbolInterner, Visibility};
pub use types::HirType;
