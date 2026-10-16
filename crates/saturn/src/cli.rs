//! Command-line interface definition for the `saturn` toolchain.
//!
//! Uses `clap` derive macros. The CLI is project-oriented: when no source
//! file is specified, the toolchain discovers the project via `saturn.toml`
//! upward from the current directory.

use crate::build::BuildContext;
use crate::profile::BuildProfile;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// Saturnite toolchain — project-oriented build system.
#[derive(Parser, Debug)]
#[command(name = "saturn")]
#[command(about = "Saturnite toolchain: build, run, and manage Saturnite projects", long_about = None)]
#[command(version, propagate_version = true)]
pub struct SaturnCli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Build the project (default: debug profile).
    ///
    /// Discovers `saturn.toml` from the current directory (or a parent),
    /// loads the module graph from the entry point (`src/main.stn` by default),
    /// and compiles to `target/<profile>/<package>.exe`.
    Build {
        /// Optional source file (advanced: bypasses project entry resolution).
        #[arg(value_name = "FILE", required = false)]
        input: Option<PathBuf>,

        /// Release build (optimized, no debug info).
        #[arg(long, conflicts_with = "debug")]
        release: bool,

        /// Debug build (default, low optimization, debug info).
        #[arg(long, conflicts_with = "release")]
        debug: bool,

        /// Output path override.
        #[arg(short, long, value_name = "FILE")]
        #[allow(unused)]
        output: Option<PathBuf>,

        /// Emit object file only; skip linking.
        #[arg(long)]
        no_link: bool,

        /// Keep intermediate object files.
        #[arg(long)]
        save_temps: bool,

        /// Verbose output.
        #[arg(short, long)]
        verbose: bool,
    },

    /// Build and run the project.
    Run {
        /// Optional source file (advanced: bypasses project entry resolution).
        #[arg(value_name = "FILE", required = false)]
        input: Option<PathBuf>,

        /// Release build (optimized).
        #[arg(long, conflicts_with = "debug")]
        release: bool,

        /// Debug build (default).
        #[arg(long, conflicts_with = "release")]
        debug: bool,

        /// Verbose output.
        #[arg(short, long)]
        verbose: bool,
    },

    /// Type-check and run semantic analysis without producing an executable.
    Check {
        /// Optional source file (advanced).
        #[arg(value_name = "FILE", required = false)]
        input: Option<PathBuf>,

        /// Verbose output.
        #[arg(short, long)]
        verbose: bool,
    },

    /// Run project tests.
    Test {
        /// Verbose output.
        #[arg(short, long)]
        verbose: bool,
    },

    /// Remove build artifacts.
    Clean {
        /// Remove the entire target/ directory (all profiles).
        #[arg(long)]
        all: bool,
    },

    /// Scaffold a new Saturnite project.
    Init {
        /// Project name (creates `name/` directory).
        #[arg(value_name = "NAME", required = false)]
        name: Option<String>,

        /// Create the project in the current directory instead of a subdirectory.
        #[arg(short, long, default_value_t = false)]
        in_place: bool,

        /// Package version string (default: 0.1.0).
        #[arg(short, long, value_name = "VERS")]
        pkg_version: Option<String>,
    },

    /// Add a dependency to the project.
    ///
    /// Resolves the dependency spec against the appropriate registry:
    /// - `rust` → crates.io
    /// - `python` → PyPI
    /// - `native` → system package manager
    /// - `saturnite` → (future) Saturnite registry
    ///
    /// The resolved version is recorded in `saturn.lock` and the dependency
    /// entry is appended to `saturn.toml`.
    Add {
        /// Dependency spec, e.g. `serde`, `numpy>=1.24`, `rust:serde@1.0`.
        /// Format: `[kind:]name[version_spec]`.
        dep: String,
    },

    /// Resolve and install all dependencies declared in `saturn.toml`.
    ///
    /// For Rust deps, this generates wrapper crates and builds static libraries
    /// into `target/saturnite/wrappers/`. For Python deps, this creates/ensures
    /// virtual environments and pip-installs packages into them.
    Install {
        /// Re-resolve all dependencies even if the lockfile is up to date.
        #[arg(long)]
        force: bool,
    },

    /// Update dependencies to their latest compatible versions.
    ///
    /// Without `--all`, updates only the named dependency (if provided).
    /// With `--all`, updates every dependency in the lockfile.
    Update {
        /// Update a specific dependency by name. If omitted and --all is set,
        /// all dependencies are updated.
        dep: Option<String>,
        /// Update all dependencies to latest compatible versions.
        #[arg(long)]
        all: bool,
    },

    /// Print diagnostics about the compiler environment.
    Doctor,
}

impl SaturnCli {
    /// Resolve the build profile from CLI flags.
    pub fn resolve_profile(release: bool, debug: bool) -> BuildProfile {
        if release {
            BuildProfile::Release
        } else if debug {
            BuildProfile::Debug
        } else {
            BuildProfile::Debug // default
        }
    }

    /// Create a `BuildContext` from CLI flags + optional input file.
    pub fn make_build_context(
        input: Option<&PathBuf>,
        release: bool,
        debug: bool,
        no_link: bool,
        save_temps: bool,
    ) -> anyhow::Result<BuildContext> {
        let profile = Self::resolve_profile(release, debug);
        let cwd = std::env::current_dir()?;

        match input {
            Some(entry) => {
                // Explicit file mode: discover the project from the file's
                // parent (which may be a subdirectory of the project root),
                // then build using the explicit entry.
                let project_root = crate::discover::ProjectDiscovery::discover(entry)?.root;
                let mut ctx = BuildContext::with_entry(project_root, entry.clone(), profile)?;
                ctx.no_link = no_link;
                ctx.save_temps = save_temps;
                Ok(ctx)
            }
            None => {
                // Project mode: discover from CWD.
                let project = crate::discover::ProjectDiscovery::discover(&cwd)?;
                let mut ctx = BuildContext::new(project.root, profile)?;
                ctx.no_link = no_link;
                ctx.save_temps = save_temps;
                Ok(ctx)
            }
        }
    }
}
