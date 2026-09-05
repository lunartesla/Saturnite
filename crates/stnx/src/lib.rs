//! Saturnite compiler library.
//!
//! This crate exposes the core compilation pipeline for the Saturnite
//! language: lexing, parsing, semantic analysis, code generation, object
//! emission, and linking.
//!
//! # Modules
//!
//! - [`lexer`] — tokenizes source text into [`Token`](lexer::Token)s.
//! - [`parser`] — parses tokens into an [`ast::Program`] AST.
//! - [`semantic`] — semantic analysis (delegates to HIR lowering).
//! - [`resolver`] — dedicated name-resolution pass (consumes HIR, runs
//!   after lowering, before MIR).
//! - [`resolver`] — dedicated name-resolution pass (consumes HIR, runs
//!   after lowering, before MIR).
//! - [`codegen`] — object emission and linking seams (MIR→LLVM via [`mir::codegen`]).
//! - [`target`] — target configuration (triple, architecture, OS, etc.).
//! - [`error`] — structured error types for every compilation stage.
//! - [`ast`] — AST node definitions.
//! - [`config`] — `saturn.toml` project configuration representation.

pub mod ast;
pub mod codegen;
pub mod config;
pub mod error;
pub mod hir;
pub mod interop;
pub mod lexer;
pub mod mir;
pub mod module;
pub mod parser;
pub mod resolver;
pub mod semantic;
pub mod target;

// --- AST re-exports ---

pub use ast::Program;

// --- HIR re-exports ---
//
// The HIR is the compiler's single authoritative semantic representation,
// produced by the AST→HIR lowering pass (see [`hir::lower`]).
// Codegen consumes `HirProgram` directly — not raw AST.

pub use hir::lower::lower_with_graph;
pub use hir::{
    DefEntry, DefId, DefKind, DefTable, HirExpr, HirExprKind, HirFunction, HirLower, HirModDecl,
    HirProgram, HirStmt, HirStmtKind, HirType, HirUseDecl, SymbolId, SymbolInterner, Visibility,
};
pub use resolver::resolve_modules;

// --- Code generation re-exports ---
//
// The codegen module exposes the object-emission and linking seams that the
// MIR→LLVM backend (see `mir::codegen`) delegates to.
//
pub use codegen::{check_linker, host_triple, Linker, ObjectEmitter};

// --- MIR re-exports ---
pub use mir::codegen::{compile_from_mir_ext, generate_ir_from_mir};
pub use mir::lower::lower_program;
pub use mir::verify::{MirVerifyError, VerifyResult};

// --- Target configuration re-exports ---

pub use target::{
    Architecture, DebugInfo, Environment, OperatingSystem, OptimizationLevel, OutputKind, Profile,
    TargetConfig,
};

// --- Error type re-exports ---
//
// Every compilation stage has its own error type. They are all re-exported
// here so downstream consumers can `use stnx::{LexError, ParseError, ...}`.

pub use error::{
    CompilerError, CompilerResult, LexError, LinkError, ParseError, TargetError, TargetResult,
};

// --- Config re-exports ---
//
// `saturn.toml` configuration types and parsing logic.

pub use config::{DependencySpec, Package, SaturnConfig};

// --- Module system re-exports ---
//
// Module graph and project loading infrastructure for multi-file projects.

pub use module::{Module, ModuleGraph, ModuleId, ModulePath, ModuleScope, Project};
