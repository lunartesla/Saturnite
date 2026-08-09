//! Code generation module for the Saturnite compiler.
//!
//! This module orchestrates LLVM IR generation, object emission, and linking.
//! It exposes three primary public types alongside convenience functions:
//!
//! - [`CodeGenerator`]: Generates an LLVM module from a typed [`HirProgram`].
//! - [`ObjectEmitter`]: Emits an object file (or IR) from an LLVM module.
//! - [`Linker`]: Links object files into a final executable using a platform-appropriate linker.
//!
//! A [`TargetConfig`] (re-exported from [`crate::target`]) holds target-specific
//! configuration used for native target initialization and cross-compilation via
//! the `TargetTriple` concept.

pub mod context;
pub mod emitter;
pub mod linker;

// Re-export the three core types
pub use context::CodeGenContext;
pub use emitter::ObjectEmitter;
pub use linker::Linker;

pub use crate::target::{
    Architecture, DebugInfo, Environment, OperatingSystem, OptimizationLevel, OutputKind,
    TargetConfig,
};

use crate::error::{CompilerError, CompilerResult};
use crate::hir::HirProgram;
use inkwell::context::Context as LLVMContext;
use inkwell::passes::PassBuilderOptions;
use std::path::Path;

// ---------------------------------------------------------------------------
// CodeGenerator
// ---------------------------------------------------------------------------

pub struct CodeGenerator {
    target_config: TargetConfig,
}

impl CodeGenerator {
    pub fn new(target_config: TargetConfig) -> Self {
        Self { target_config }
    }

    pub fn target_config(&self) -> &TargetConfig {
        &self.target_config
    }

    pub fn generate_ir_string(program: &HirProgram) -> CompilerResult<String> {
        let context = LLVMContext::create();
        let mut ctx = CodeGenContext::new(&context);
        ctx.declare_builtin_functions();

        for func in &program.functions {
            ctx.declare_function(func, &program.symbols)?;
        }

        for func in &program.functions {
            ctx.generate_function(func, program)?;
        }

        let ir = ctx.module.print_to_string();
        Ok(ir.to_string())
    }

    pub fn compile(&self, program: &HirProgram, output_path: &str) -> CompilerResult<()> {
        self.emit(program, output_path, OutputKind::Exe, false)
    }

    pub fn emit(
        &self,
        program: &HirProgram,
        output_path: &str,
        output_kind: OutputKind,
        save_temps: bool,
    ) -> CompilerResult<()> {
        let context = LLVMContext::create();
        let mut ctx = CodeGenContext::new(&context);

        ctx.declare_builtin_functions();

        for func in &program.functions {
            ctx.declare_function(func, &program.symbols)?;
        }

        for func in &program.functions {
            ctx.generate_function(func, program)?;
        }

        // Set target triple
        let triple = self.target_config.triple();
        ctx.module.set_triple(triple);

        let output_path = Path::new(output_path);

        // Ensure the output directory exists so emitting to a nested path like
        // `target/debug/example.o` succeeds even on a clean tree.
        if let Some(parent) = output_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    CompilerError::codegen(format!(
                        "failed to create output directory '{}': {}",
                        parent.display(),
                        e
                    ))
                })?;
            }
        }

        // Run optimization passes if configured (requires a target machine)
        if *self.target_config.opt_level() != OptimizationLevel::None {
            let target_machine = self
                .target_config
                .create_target_machine()
                .map_err(CompilerError::Target)?;
            let opt_passes = match self.target_config.opt_level() {
                OptimizationLevel::Less => "default<O1>",
                OptimizationLevel::Default => "default<O2>",
                OptimizationLevel::Aggressive => "default<O3>",
                _ => "default<O0>",
            };
            let options = PassBuilderOptions::create();
            ctx.module
                .run_passes(opt_passes, &target_machine, options)
                .map_err(|e| {
                    CompilerError::codegen(format!("optimization passes failed: {}", e))
                })?;
        }

        match output_kind {
            OutputKind::Ir => {
                let emitter = ObjectEmitter::new(ctx.module, &self.target_config)?;
                emitter.emit_ir_to_file(output_path)?;
            }
            OutputKind::Object => {
                let emitter = ObjectEmitter::new(ctx.module, &self.target_config)?;
                emitter.emit_object(output_path)?;
            }
            OutputKind::Exe => {
                let obj_path = output_path.with_extension("o");
                let emitter = ObjectEmitter::new(ctx.module, &self.target_config)?;
                emitter.emit_object(&obj_path)?;

                let lk = Linker::new(&self.target_config);
                lk.link(&obj_path, output_path)
                    .map_err(CompilerError::Link)?;

                if !save_temps {
                    let _ = std::fs::remove_file(&obj_path);
                }
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Convenience free functions
// ---------------------------------------------------------------------------

pub fn generate_ir(program: &HirProgram) -> CompilerResult<String> {
    CodeGenerator::generate_ir_string(program)
}

pub fn compile_to_executable(program: &HirProgram, output_path: &str) -> CompilerResult<()> {
    let config = TargetConfig::host()?;
    let gen = CodeGenerator::new(config);
    gen.compile(program, output_path)
}

pub fn compile_with_target(
    program: &HirProgram,
    output_path: &str,
    target_config: TargetConfig,
) -> CompilerResult<()> {
    compile_with_target_ext(program, output_path, target_config, false)
}

pub fn compile_with_target_ext(
    program: &HirProgram,
    output_path: &str,
    target_config: TargetConfig,
    save_temps: bool,
) -> CompilerResult<()> {
    let gen = CodeGenerator::new(target_config);
    let mode = *gen.target_config().output_kind();
    gen.emit(program, output_path, mode, save_temps)
}

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
