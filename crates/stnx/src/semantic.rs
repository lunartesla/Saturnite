use crate::ast::Program;
use crate::error::CompilerResult;
use crate::hir;

/// Semantic analysis entry point.
///
/// In Saturnite 0.3, this delegates to the HIR lowering pass
/// (`hir::lower`), which performs name resolution, type checking,
/// and mutability enforcement as a single unified pass that produces
/// a typed `HirProgram`.
///
/// This function preserves the 0.2 signature (`CompilerResult<&Program>`)
/// for backward compatibility with callers that only need a pass/fail
/// result.  Callers that need the resolved HIR should use
/// [`hir::lower`] directly.
pub fn analyze(program: &Program) -> CompilerResult<()> {
    hir::lower::lower_unit(program)
}

/// Analyze a program and return the fully lowered HIR.
///
/// This is the preferred entry point for the 0.3 pipeline:
/// `lex → parse → lower → codegen`.
pub fn analyze_and_lower(program: &Program) -> CompilerResult<hir::HirProgram> {
    hir::lower::lower(program)
}

/// Re-export the HIR types so callers can use `stnx::semantic::Hir*`
/// or `stnx::hir::Hir*`.
pub use hir::*;
