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

pub fn run_diagnostics() -> CompilerResult<()> {
    println!("Saturnite Compiler Diagnostics");
    println!("==============================");
    println!();

    match host_triple() {
        Ok(triple) => {
            println!("Host target triple: {}", triple);
            println!();
        }
        Err(e) => {
            println!("ERROR: Failed to determine host target triple: {}", e);
            println!();
            return Ok(());
        }
    }

    match TargetConfig::host() {
        Ok(config) => {
            println!("Host configuration:");
            println!("  Triple:      {}", config.triple_str());
            println!("  Architecture: {:?}", config.architecture());
            println!("  OS:          {:?}", config.os());
            println!("  Environment: {:?}", config.environment());
            println!("  Opt level:   {:?}", config.opt_level());
            println!();
        }
        Err(e) => {
            println!("ERROR: Failed to initialize target config: {}", e);
            println!();
            return Ok(());
        }
    }

    match TargetConfig::host() {
        Ok(config) => match check_linker(&config) {
            Ok(()) => {
                println!("Linker: available");
            }
            Err(e) => {
                println!("WARNING: Linker not available: {}", e);
            }
        },
        Err(e) => {
            println!("WARNING: Could not check linker: {}", e);
        }
    }
    println!();

    println!("inkwell 0.9 with LLVM 21.x (dynamic linking)");

    Ok(())
}
