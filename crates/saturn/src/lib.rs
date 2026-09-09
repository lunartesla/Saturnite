//! Saturnite toolchain — project-oriented build system.
//!
//! The `saturn` binary is the user-facing entry point for the Saturnite
//! toolchain, analogous to Cargo for Rust. It owns:
//!
//! - project discovery (walk-up for `saturn.toml`)
//! - configuration & build profiles (debug / release)
//! - build directory management (`target/<profile>/`)
//! - compiler invocation through the `stnx` library API
//! - linking orchestration (delegating to `stnx::codegen::Linker`)
//! - `run`, `check`, `test`, `clean`, `init`
//!
//! The compiler itself (lexer → parser → HIR → MIR → LLVM → object → link)
//! lives in the `stnx` crate and is consumed as a library.

pub mod build;
pub mod cli;
pub mod cmd;
pub mod discover;
pub mod manifest;
pub mod profile;

pub use build::BuildContext;
pub use cli::SaturnCli;
pub use discover::ProjectDiscovery;
pub use manifest::Manifest;
pub use profile::{BuildProfile, PlatformInfo, executable_name, platform_triple};

pub mod prelude {
    pub use crate::build::BuildContext;
    pub use crate::cli::SaturnCli;
    pub use crate::discover::ProjectDiscovery;
    pub use crate::manifest::Manifest;
    pub use crate::profile::{BuildProfile, PlatformInfo, executable_name, platform_triple};
    pub use clap::{Parser, Subcommand};
}
