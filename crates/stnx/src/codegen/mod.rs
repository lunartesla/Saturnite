//! Code generation infrastructure for the Saturnite compiler.
//!
//! This module provides the LLVM backend *seams* — the object-emission and
//! linking stages that the MIR→LLVM backend (see [`crate::mir::codegen`])
//! delegates to.
//!
//! - [`ObjectEmitter`]: Emits an object file (or IR text) from an LLVM module.
//! - [`Linker`]: Links object files into a final executable using a
//!   platform-appropriate linker.
//!
//! A [`TargetConfig`] (re-exported from [`crate::target`]) holds target-specific
//! configuration used for native target initialization and cross-compilation via
//! the `TargetTriple` concept.

pub mod emitter;
pub mod linker;

pub use emitter::ObjectEmitter;
pub use linker::Linker;

pub use crate::target::TargetConfig;

use crate::error::CompilerResult;

// ---------------------------------------------------------------------------
// Convenience free functions
// ---------------------------------------------------------------------------

pub fn check_linker(target_config: &TargetConfig) -> CompilerResult<()> {
    linker::check_linker_available(target_config)
}

pub fn host_triple() -> CompilerResult<String> {
    let config = TargetConfig::host()?;
    Ok(config.triple_str())
}
