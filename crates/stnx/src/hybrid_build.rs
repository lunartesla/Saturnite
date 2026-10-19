//! Hybrid backend build orchestration.
//!
//! When the hybrid backend is selected, the Saturnite compiler must:
//!
//! 1. Emit LLVM IR/object from the MIR program (LLVM backend).
//! 2. Generate Rust source from the HIR program (Rust backend).
//! 3. Compile the generated Rust source into a `staticlib` via `cargo build`.
//! 4. Link the LLVM object, Rust staticlib, and Saturnite runtime support
//!    into a single executable.
//!
//! This module provides [`build_hybrid`] which orchestrates all four steps
//! in the correct sequence. All subprocess invocations use
//! `std::process::Command` (process API, not shell strings).

use crate::error::{CompilerError, CompilerResult};
use crate::hir::HirProgram;
use crate::mir::codegen::emit_object_from_mir;
use crate::rust_codegen::{
    build_rust_crate, generate_cargo_toml, generate_rust_source, CodegenOptions, RustSource,
};
use crate::target::TargetConfig;
use std::path::{Path, PathBuf};

/// The result of a hybrid build: paths to all produced artifacts.
#[derive(Debug, Clone)]
pub struct HybridBuildResult {
    /// The final executable path.
    pub output: PathBuf,
    /// The LLVM object file (intermediate).
    pub llvm_object: PathBuf,
    /// The Rust static library (intermediate, may not exist if no Rust deps).
    pub rust_staticlib: Option<PathBuf>,
    /// The generated Rust source directory (intermediate).
    pub rust_source_dir: PathBuf,
    /// Build time in milliseconds.
    pub elapsed_ms: u64,
}

/// Orchestrate the full hybrid build pipeline.
///
/// This is the primary entry point for the hybrid backend. It:
///
/// 1. Emits an LLVM object file from the MIR program.
/// 2. Generates Rust source from the HIR program.
/// 3. Writes the generated source and `Cargo.toml` to a build directory.
/// 4. Runs `cargo build --release` to produce a `staticlib`.
/// 5. Links the LLVM object + Rust staticlib + runtime into the final executable.
///
/// # Arguments
/// * `hir` — The HIR program (used for Rust source generation).
/// * `mir` — The MIR program (used for LLVM object emission).
/// * `output_path` — Path for the final executable.
/// * `target_config` — Target configuration (triple, profile, output kind).
/// * `project_root` — Project root (used for the build directory and wrapper cache).
/// * `crate_name` — Name for the generated Rust crate.
///
/// # Returns
/// Build artifacts and timing information.
pub fn build_hybrid(
    hir: &HirProgram,
    mir: &crate::mir::MirProgram,
    output_path: &str,
    target_config: &TargetConfig,
    project_root: &Path,
    crate_name: &str,
) -> CompilerResult<HybridBuildResult> {
    let start = std::time::Instant::now();
    let output_path = Path::new(output_path);

    // --- Step 1: Emit LLVM object from MIR ---
    let llvm_obj_path = output_path.with_extension("o");
    let obj_path_str = llvm_obj_path.to_string_lossy().to_string();
    emit_object_from_mir(mir, &obj_path_str, target_config)?;

    // --- Step 2: Generate Rust source from HIR ---
    // Use as_binary=false (staticlib mode) so we can link the Rust code
    // together with the LLVM object.
    let options = CodegenOptions {
        as_binary: false,
    };
    let rust_source: RustSource = generate_rust_source(hir, crate_name, &options)?;

    // --- Step 3: Write generated source and Cargo.toml to build directory ---
    let rust_build_dir = project_root
        .join("target")
        .join("saturnite")
        .join("rust_backend")
        .join(crate_name);

    // Clean previous build.
    if rust_build_dir.exists() {
        std::fs::remove_dir_all(&rust_build_dir)
            .map_err(|e| CompilerError::codegen(format!("failed to clean rust build dir: {}", e)))?;
    }

    let src_dir = rust_build_dir.join("src");
    std::fs::create_dir_all(&src_dir)
        .map_err(|e| CompilerError::codegen(format!("failed to create rust src dir: {}", e)))?;

    // Write Cargo.toml.
    // Dependencies come from the RustSource (collected from `use rust:` imports).
    let deps: Vec<(String, String)> = rust_source.dependencies.clone();
    let cargo_toml = generate_cargo_toml(&rust_source.crate_name, &rust_source.edition, &deps, false);
    std::fs::write(rust_build_dir.join("Cargo.toml"), cargo_toml)
        .map_err(|e| CompilerError::codegen(format!("failed to write Cargo.toml: {}", e)))?;

    // Write src/lib.rs.
    std::fs::write(src_dir.join("lib.rs"), &rust_source.source)
        .map_err(|e| CompilerError::codegen(format!("failed to write lib.rs: {}", e)))?;

    // --- Step 4: Build Rust crate with cargo ---
    let target_triple = target_config.triple().to_string();
    let rust_staticlib: PathBuf = build_rust_crate(&rust_build_dir, Some(&target_triple), false)?;

    // --- Step 5: Link LLVM object + Rust staticlib + runtime ---
    let rust_staticlib_opt = if rust_staticlib.exists() {
        Some(rust_staticlib.clone())
    } else {
        None
    };

    link_hybrid(target_config, &llvm_obj_path, rust_staticlib_opt.as_deref(), output_path)?;

    // Clean up intermediates unless save_temps is set.
    // (The caller controls save_temps, but for now we always clean.)
    let _ = std::fs::remove_file(&llvm_obj_path);

    let elapsed = start.elapsed();

    Ok(HybridBuildResult {
        output: output_path.to_path_buf(),
        llvm_object: llvm_obj_path,
        rust_staticlib: rust_staticlib_opt,
        rust_source_dir: rust_build_dir,
        elapsed_ms: elapsed.as_millis() as u64,
    })
}

/// Link the LLVM object, Rust staticlib, and Saturnite runtime into an
/// executable.
///
/// This uses the standard [`Linker`](crate::codegen::Linker) infrastructure, adding the Rust staticlib
/// to the link line alongside the LLVM object and runtime support library.
pub fn link_hybrid(
    target_config: &TargetConfig,
    llvm_obj: &Path,
    rust_staticlib: Option<&Path>,
    output_path: &Path,
) -> CompilerResult<()> {
    use crate::codegen::Linker;
    use std::process::Command;

    let linker = Linker::new(target_config);
    let os = target_config.os();
    let env = target_config.environment();

    // Build base linker args (object, output, runtime, platform flags).
    let base_args = linker.build_linker_args(os, env, llvm_obj, output_path);

    // Insert the Rust staticlib on the link line.
    let args = if let Some(lib) = rust_staticlib {
        let lib_str = lib.display().to_string();
        // Insert before the output flag so the staticlib symbols are available
        // when resolving the object's references.
        let insert_at = base_args
            .iter()
            .position(|a| a == "-o" || a.starts_with("/OUT:"))
            .unwrap_or(base_args.len().saturating_sub(1));
        let mut args = Vec::with_capacity(base_args.len() + 1);
        args.extend(base_args[..insert_at].iter().cloned());
        args.push(lib_str);
        args.extend(base_args[insert_at..].iter().cloned());
        args
    } else {
        base_args
    };

    let linker_cmd = linker.select_linker(os, env);
    let linker_name = linker_cmd
        .first()
        .map(|s| s.as_str())
        .unwrap_or("cc");

    let linker_path = crate::codegen::linker::resolve_linker_path(linker_name, os, env).map_err(
        |_| {
            crate::error::LinkError::linker_not_found(format!(
                "{} (searched PATH for {:?})\n\
                 Target platform: {} with {} environment.\n\
                 A C compiler/linker is required to link executables.",
                linker_name,
                linker_name,
                crate::codegen::linker::describe_os(os),
                crate::codegen::linker::describe_env(env)
            ))
        },
    )?;

    let output = Command::new(&linker_path)
        .args(&linker_cmd[1..])
        .args(&args)
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                crate::error::LinkError::linker_not_found(linker_name.to_string())
            } else {
                crate::error::LinkError::linking_failed(
                    output_path.display().to_string(),
                    Some(e.to_string()),
                )
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let details = if stderr.is_empty() && !stdout.is_empty() {
            Some(stdout)
        } else if stderr.is_empty() {
            None
        } else {
            Some(stderr)
        };
        return Err(crate::error::LinkError::linking_failed(
            output_path.display().to_string(),
            details,
        )
        .into());
    }

    Ok(())
}

/// Check whether the hybrid backend is needed based on backend selection.
///
/// Returns the list of Rust crate names that need to be built (i.e. the
/// reachable Rust imports from the HIR program).
pub fn rust_crates_to_build(
    selection: &crate::backend::BackendSelection,
    hir: &HirProgram,
) -> CompilerResult<Vec<(String, String)>> {
    if selection.backend != crate::backend::SelectedBackend::Hybrid {
        return Ok(Vec::new());
    }

    let mut crates = Vec::new();
    let symbols = &hir.symbols;

    for ud in &hir.use_decls {
        if ud.ecosystem != Some(crate::ast::ExternalKind::Rust) {
            continue;
        }
        let pkg = ud
            .path
            .first()
            .copied()
            .unwrap_or(crate::hir::symbol::SymbolId(0));
        let alias = ud.alias;

        let crate_name = symbols.lookup(pkg).unwrap_or("");
        let alias_str = symbols.lookup(alias).unwrap_or("");

        if !crate_name.is_empty() && !alias_str.is_empty() {
            crates.push((crate_name.to_string(), alias_str.to_string()));
        }
    }

    Ok(crates)
}

/// Verify that the hybrid build prerequisites are available.
///
/// Checks that `cargo` and `rustc` are on PATH, and that a C linker is
/// available for the target platform.
pub fn verify_hybrid_prerequisites(target_config: &TargetConfig) -> CompilerResult<()> {
    // Check cargo.
    if which::which("cargo").is_err() {
        return Err(CompilerError::codegen(
            "cargo not found on PATH — the hybrid backend requires the Rust \
             toolchain. Install Rust from https://rustup.rs.",
        ));
    }

    // Check rustc.
    if which::which("rustc").is_err() {
        return Err(CompilerError::codegen(
            "rustc not found on PATH — the hybrid backend requires the Rust \
             toolchain. Install Rust from https://rustup.rs.",
        ));
    }

    // Check C linker.
    crate::codegen::check_linker(target_config)?;

    Ok(())
}

/// Check whether a given crate name corresponds to a Rust ecosystem dependency.
///
/// This is a simple predicate used by the orchestrator to decide whether to
/// route a crate through the Rust wrapper build path.
pub fn is_rust_ecosystem_crate(name: &str, kind: crate::interop::DependencyKind) -> bool {
    kind == crate::interop::DependencyKind::Rust && !name.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{BackendSelection, SelectedBackend};

    #[test]
    fn test_rust_crates_to_build_no_hybrid() {
        // An LLVM-only backend selection should return no crates to build.
        let selection = BackendSelection {
            backend: SelectedBackend::Llvm,
            reason: "no rust imports".to_string(),
        };
        let program = empty_program();
        let result = rust_crates_to_build(&selection, &program).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_rust_crates_to_build_hybrid_no_imports() {
        // Hybrid backend with no `use rust:` imports should return empty.
        let selection = BackendSelection {
            backend: SelectedBackend::Hybrid,
            reason: "forced hybrid".to_string(),
        };
        let program = empty_program();
        let result = rust_crates_to_build(&selection, &program).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_is_rust_ecosystem_crate() {
        assert!(is_rust_ecosystem_crate(
            "serde",
            crate::interop::DependencyKind::Rust
        ));
        assert!(!is_rust_ecosystem_crate(
            "serde",
            crate::interop::DependencyKind::Python
        ));
        assert!(!is_rust_ecosystem_crate("", crate::interop::DependencyKind::Rust));
    }

    fn empty_program() -> HirProgram {
        use crate::hir::symbol::SymbolInterner;
        use crate::module::ModuleId;
        use std::collections::HashMap;

        let symbols = SymbolInterner::default();
        HirProgram {
            functions: Vec::new(),
            structs: Vec::new(),
            enums: Vec::new(),
            symbols,
            external_functions: Vec::new(),
            modules: Vec::new(),
            root_module: ModuleId::ROOT,
            module_paths: HashMap::new(),
            def_table: crate::hir::symbol::DefTable::default(),
            module_scopes: Vec::new(),
            use_decls: Vec::new(),
            mod_decls: Vec::new(),
            python_search_path: String::new(),
        }
    }
}
