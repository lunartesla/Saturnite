//! Hybrid backend build orchestration.
//!
//! When the hybrid backend is selected, the Saturnite compiler emits an LLVM
//! object from the MIR program (LLVM backend), then links it together with
//! pre-built Rust wrapper static libraries. The Rust wrappers are thin
//! `#[no_mangle] extern "C"` adapter crates that forward to real Rust crates;
//! they are built during `saturn install` by [`crate::rust_wrapper`] and stored
//! under `target/saturnite/wrappers/<crate>_wrapper/target/release/`.
//!
//! This module provides [`build_hybrid`] which orchestrates:
//!
//! 1. LLVM object emission from MIR.
//! 2. Location of pre-built Rust wrapper staticlibs for reachable Rust imports.
//! 3. Linking the LLVM object + Rust staticlibs + Saturnite runtime into the
//!    final executable.
//!
//! All subprocess invocations use [`std::process::Command`] (process API, not
//! shell strings).

use crate::error::{CompilerError, CompilerResult};
use crate::hir::HirProgram;
use crate::mir::codegen::emit_object_from_mir;
use crate::target::TargetConfig;
use std::path::{Path, PathBuf};

/// The result of a hybrid build: paths to all produced artifacts.
#[derive(Debug, Clone)]
pub struct HybridBuildResult {
    /// The final executable path.
    pub output: PathBuf,
    /// The LLVM object file (intermediate, may be cleaned up).
    pub llvm_object: PathBuf,
    /// The Rust wrapper static libraries that were linked (may be empty).
    pub rust_staticlibs: Vec<PathBuf>,
    /// Build time in milliseconds.
    pub elapsed_ms: u64,
}

/// Orchestrate the hybrid build pipeline.
///
/// This is the primary entry point for the hybrid backend. It:
///
/// 1. Emits an LLVM object file from the MIR program.
/// 2. Locates pre-built Rust wrapper static libraries for all reachable
///    `use rust:` imports in the HIR program.
/// 3. Links the LLVM object + Rust staticlibs + runtime into the final
///    executable.
///
/// # Arguments
/// * `hir` — The HIR program (used to find Rust wrapper staticlibs).
/// * `mir` — The MIR program (used for LLVM object emission).
/// * `output_path` — Path for the final executable.
/// * `target_config` — Target configuration (triple, profile, output kind).
/// * `project_root` — Project root (used for the wrappers directory and runtime).
///
/// # Returns
/// Build artifacts and timing information.
pub fn build_hybrid(
    hir: &HirProgram,
    mir: &crate::mir::MirProgram,
    output_path: &str,
    target_config: &TargetConfig,
    project_root: &Path,
) -> CompilerResult<HybridBuildResult> {
    let start = std::time::Instant::now();
    let output_path = Path::new(output_path);

    // --- Step 1: Emit LLVM object from MIR ---
    // Saturnite source compiles entirely through HIR → MIR → LLVM.
    // No Saturnite code is translated to Rust.
    let llvm_obj_path = output_path.with_extension("o");
    let obj_path_str = llvm_obj_path.to_string_lossy().to_string();
    emit_object_from_mir(mir, &obj_path_str, target_config)?;

    // --- Step 2: Locate pre-built Rust wrapper staticlibs ---
    // These were built during `saturn install` via rust_wrapper::build_rust_wrapper.
    let wrappers_dir = project_root
        .join("target")
        .join("saturnite")
        .join("wrappers");

    let rust_staticlibs = find_rust_staticlibs(hir, &wrappers_dir, target_config)?;

    // --- Step 3: Link LLVM object + Rust staticlibs + runtime ---
    link_hybrid(
        target_config,
        &llvm_obj_path,
        &rust_staticlibs,
        output_path,
    )?;

    // Clean up the intermediate object file.
    let _ = std::fs::remove_file(&llvm_obj_path);

    let elapsed = start.elapsed();

    Ok(HybridBuildResult {
        output: output_path.to_path_buf(),
        llvm_object: llvm_obj_path,
        rust_staticlibs,
        elapsed_ms: elapsed.as_millis() as u64,
    })
}

/// Locate pre-built Rust wrapper static libraries for all reachable
/// `use rust:` imports in the HIR program.
///
/// For each Rust ecosystem import, this looks in
/// `wrappers_dir/<crate>_wrapper/target/release/` for the static library
/// produced by `cargo build --release` during `saturn install`.
fn find_rust_staticlibs(
    hir: &HirProgram,
    wrappers_dir: &Path,
    target_config: &TargetConfig,
) -> CompilerResult<Vec<PathBuf>> {
    let mut staticlibs = Vec::new();

    // Collect unique Rust crate names from use_decls.
    let rust_crates: Vec<String> = hir
        .use_decls
        .iter()
        .filter_map(|ud| {
            if ud.ecosystem != Some(crate::ast::ExternalKind::Rust) {
                return None;
            }
            let crate_name = hir
                .symbols
                .lookup(ud.path.first().copied().unwrap_or(crate::hir::symbol::SymbolId(0)))
                .unwrap_or("");
            if crate_name.is_empty() {
                None
            } else {
                Some(crate_name.to_string())
            }
        })
        .collect::<std::collections::HashSet<String>>()
        .into_iter()
        .collect();

    let os = target_config.os();
    let env = target_config.environment();

    for crate_name in &rust_crates {
        let sanitized = crate::rust_discovery::sanitize_crate_name(crate_name);
        let wrapper_dir_name = format!("{}_wrapper", crate_name);
        let release_dir = wrappers_dir
            .join(&wrapper_dir_name)
            .join("target")
            .join("release");

        if !release_dir.is_dir() {
            return Err(CompilerError::codegen(format!(
                "Rust wrapper for crate '{}' not found. Run `saturn install` first. \
                 Expected static library in {}",
                crate_name,
                release_dir.display()
            )));
        }

        // Staticlib naming: lib<name>.a on Unix, <name>.lib on MSVC.
        let lib_base = format!("{}-saturnite-wrapper", sanitized);
        let candidates = match (os, env) {
            (crate::target::OperatingSystem::Linux, _)
            | (crate::target::OperatingSystem::Darwin, _)
            | (crate::target::OperatingSystem::Windows, crate::target::Environment::Gnu) => {
                vec![
                    format!("lib{}.a", lib_base),
                    format!("lib{}.so", lib_base), // in case of cdylib fallback
                ]
            }
            (crate::target::OperatingSystem::Windows, crate::target::Environment::Msvc) => {
                vec![
                    format!("{}.lib", lib_base),
                    format!("lib{}.lib", lib_base),
                ]
            }
            _ => vec![
                format!("lib{}.a", lib_base),
                format!("{}.lib", lib_base),
            ],
        };

        let mut found = None;
        for candidate in &candidates {
            let path = release_dir.join(candidate);
            if path.is_file() {
                found = Some(path);
                break;
            }
        }

        if found.is_none() {
            // Search in deps subdirectory.
            let deps_dir = release_dir.join("deps");
            if deps_dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&deps_dir) {
                    for entry in entries.flatten() {
                        let name = entry.file_name();
                        let name = name.to_string_lossy();
                        if name.starts_with(&format!("lib{}", lib_base))
                            && (name.ends_with(".a") || name.ends_with(".lib"))
                        {
                            found = Some(entry.path());
                            break;
                        }
                    }
                }
            }
        }

        let lib_path = found.ok_or_else(|| {
            CompilerError::codegen(format!(
                "Rust wrapper static library for crate '{}' not found in {}. \
                 Run `saturn install` to build it.",
                crate_name,
                release_dir.display()
            ))
        })?;

        if std::env::var("SATURNITE_VERBOSE").is_ok() {
            eprintln!("rust wrapper: {} → {}", crate_name, lib_path.display());
        }

        staticlibs.push(lib_path);
    }

    Ok(staticlibs)
}

/// Link the LLVM object, Rust wrapper staticlibs, and Saturnite runtime into
/// an executable.
///
/// This uses the standard [`crate::codegen::Linker`] infrastructure, inserting
/// each Rust staticlib on the link line alongside the LLVM object and runtime
/// support library. The staticlibs are placed before the output flag so their
/// symbols resolve the LLVM object's external references.
pub fn link_hybrid(
    target_config: &TargetConfig,
    llvm_obj: &Path,
    rust_staticlibs: &[PathBuf],
    output_path: &Path,
) -> CompilerResult<()> {
    use crate::codegen::Linker;
    use std::process::Command;

    let linker = Linker::new(target_config);
    let os = target_config.os();
    let env = target_config.environment();

    // Build base linker args (object, output, runtime, platform flags).
    let base_args = linker.build_linker_args(os, env, llvm_obj, output_path);

    // Insert all Rust staticlibs before the output flag so their symbols are
    // available when resolving the object's external references.
    let args = if rust_staticlibs.is_empty() {
        base_args
    } else {
        let insert_at = base_args
            .iter()
            .position(|a| a == "-o" || a.starts_with("/OUT:"))
            .unwrap_or(base_args.len().saturating_sub(1));

        let mut args = Vec::with_capacity(base_args.len() + rust_staticlibs.len());
        args.extend(base_args[..insert_at].iter().cloned());
        for lib in rust_staticlibs {
            args.push(lib.display().to_string());
        }
        args.extend(base_args[insert_at..].iter().cloned());
        args
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
pub fn is_rust_ecosystem_crate(
    name: &str,
    kind: crate::interop::DependencyKind,
) -> bool {
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
