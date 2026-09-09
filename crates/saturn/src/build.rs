//! Build orchestration — ties the toolchain to the compiler library.
//!
//! This module owns the build directory layout, profile selection, and the
//! sequence of compiler calls needed to produce an executable.

use crate::manifest::Manifest;
use crate::profile::BuildProfile;
use anyhow::Result;
use std::path::{Path, PathBuf};
use stnx::mir::codegen::compile_from_mir_ext;
use stnx::mir::monomorphize::monomorphize;
use stnx::mir::opt::optimize;
use stnx::module::Project;
use stnx::target::{OutputKind, TargetConfig};

/// Context for a single build invocation.
pub struct BuildContext {
    /// The project root (contains `saturn.toml`).
    pub project_root: PathBuf,
    /// The parsed manifest.
    pub manifest: Manifest,
    /// The build profile (debug / release).
    pub profile: BuildProfile,
    /// The target/ output directory (`<root>/target/<profile>/`).
    pub target_dir: PathBuf,
    /// The resolved entry point source file.
    pub entry: PathBuf,
    /// The output executable path.
    pub output: PathBuf,
    /// Whether to skip the linking stage (emit object only).
    pub no_link: bool,
    /// Whether to keep intermediate object files.
    pub save_temps: bool,
}

impl BuildContext {
    /// Create a new build context from a project root and profile.
    pub fn new(project_root: PathBuf, profile: BuildProfile) -> Result<Self> {
        let manifest = Manifest::from_dir(&project_root)?;
        let entry = manifest.resolve_entry(&project_root);
        let target_dir = project_root.join("target").join(profile.dir_name());

        let package_name = &manifest.package.name;
        let exe_name = crate::profile::executable_name(package_name);
        let output = target_dir.join(&exe_name);

        Ok(Self {
            project_root,
            manifest,
            profile,
            target_dir,
            entry,
            output,
            no_link: false,
            save_temps: false,
        })
    }

    /// Create a build context with an explicitly provided entry file
    /// (used when a user passes a file argument).
    pub fn with_entry(
        project_root: PathBuf,
        entry: PathBuf,
        profile: BuildProfile,
    ) -> Result<Self> {
        let manifest = Manifest::from_dir(&project_root)?;
        let target_dir = project_root.join("target").join(profile.dir_name());

        // Derive the output name from the entry file stem.
        let stem = entry
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("out")
            .to_string();
        let exe_name = crate::profile::executable_name(&stem);
        let output = target_dir.join(&exe_name);

        Ok(Self {
            project_root,
            manifest,
            profile,
            target_dir,
            entry,
            output,
            no_link: false,
            save_temps: false,
        })
    }

    /// Ensure the target directory exists.
    pub fn ensure_target_dir(&self) -> Result<()> {
        std::fs::create_dir_all(&self.target_dir)
            .map_err(|e| anyhow::anyhow!("failed to create target dir: {}", e))?;
        Ok(())
    }

    /// Build a [`TargetConfig`] with the profile applied.
    fn target_config(&self) -> Result<TargetConfig> {
        let mut config = TargetConfig::host()
            .map_err(|e| anyhow::anyhow!("failed to initialize native target: {}", e))?;
        // Use the compiler's own Profile enum and apply_profile for consistency
        // with the stnx CLI.
        config.apply_profile(self.profile.to_compiler_profile());
        if self.no_link {
            config.set_output_kind(OutputKind::Object);
        } else {
            config.set_output_kind(OutputKind::Exe);
        }
        Ok(config)
    }

    /// Run the full compilation pipeline: discover project → load modules →
    /// HIR lowering → MIR lowering → verify → optimize → LLVM codegen → link.
    pub fn build(&self) -> Result<BuildResult> {
        self.ensure_target_dir()?;

        let entry = &self.entry;
        if !entry.is_file() {
            anyhow::bail!(
                "no entry point found: expected {} (create a saturn.toml project with src/main.stn or pass a file explicitly)",
                entry.display()
            );
        }

        // Use the compiler's Project discovery and module loading.
        let mut project = Project::discover(entry)?;
        let program = project.load()?;

        // Semantic analysis: AST → HIR (graph-aware for multi-module).
        let hir = stnx::semantic::analyze_and_lower_with_graph(&program, &project.graph)
            .map_err(|e| anyhow::anyhow!("Semantic error: {}", e))?;

        // HIR → MIR (with monomorphization for generics).
        let mut mir = monomorphize(&hir)
            .map_err(|e| anyhow::anyhow!("monomorphization failed: {}", e))?;

        // Verify MIR CFG.
        if let Err(errs) = mir.verify() {
            let msgs: Vec<String> = errs.iter().map(|e| e.to_string()).collect();
            anyhow::bail!("MIR verification failed: {}", msgs.join(", "));
        }

        // MIR-level optimizations.
        optimize(&mut mir);

        // Configure target and codegen.
        let config = self.target_config()?;

        let start = std::time::Instant::now();

        compile_from_mir_ext(&mir, self.output.to_str().unwrap(), config, self.save_temps)
            .map_err(|e| anyhow::anyhow!("Compilation failed: {}", e))?;

        let elapsed = start.elapsed();

        Ok(BuildResult {
            profile: self.profile,
            output: self.output.clone(),
            elapsed_ms: elapsed.as_millis() as u64,
        })
    }

    /// Run the type-check / semantic analysis pipeline without codegen.
    pub fn check(&self) -> Result<()> {
        let entry = &self.entry;
        if !entry.is_file() {
            anyhow::bail!(
                "no entry point found: expected {}",
                entry.display()
            );
        }

        let mut project = Project::discover(entry)?;
        let program = project.load()?;

        // This runs lexing, parsing, HIR lowering, and module resolution —
        // but does NOT generate code or emit an object/executable.

        stnx::semantic::analyze_and_lower_with_graph(&program, &project.graph)
            .map_err(|e| anyhow::anyhow!("Semantic error: {}", e))?;

        Ok(())
    }

    /// Run (build if needed, then execute) the project.
    pub fn run(&self) -> Result<i32> {
        // Build first.
        let result = self.build()?;

        // Execute the resulting binary.
        let status = std::process::Command::new(&result.output)
            .status()
            .map_err(|e| {
                anyhow::anyhow!("failed to execute {}: {}", result.output.display(), e)
            })?;

        Ok(status.code().unwrap_or(1))
    }

    /// Remove the build artifacts for this profile.
    pub fn clean_profile(&self) -> Result<()> {
        if self.target_dir.exists() {
            std::fs::remove_dir_all(&self.target_dir)
                .map_err(|e| anyhow::anyhow!("failed to remove target dir: {}", e))?;
        }
        Ok(())
    }

    /// Remove the entire target/ directory (all profiles).
    pub fn clean_all(&self) -> Result<()> {
        let target = self.project_root.join("target");
        if target.exists() {
            std::fs::remove_dir_all(&target)
                .map_err(|e| anyhow::anyhow!("failed to remove target dir: {}", e))?;
        }
        Ok(())
    }
}

/// Result of a successful build.
pub struct BuildResult {
    pub profile: BuildProfile,
    pub output: PathBuf,
    pub elapsed_ms: u64,
}

/// Check if a file needs rebuilding by comparing mtime (stubs for now;
/// full incremental compilation is a future feature).
pub fn needs_rebuild(source: &Path, output: &Path) -> bool {
    if !output.exists() {
        return true;
    }
    let src_mtime = std::fs::metadata(source)
        .and_then(|m| m.modified())
        .ok();
    let out_mtime = std::fs::metadata(output)
        .and_then(|m| m.modified())
        .ok();

    match (src_mtime, out_mtime) {
        (Some(s), Some(o)) => s > o,
        _ => true,
    }
}
