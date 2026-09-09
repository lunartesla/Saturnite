//! `saturn.toml` manifest representation for the toolchain.
//!
//! The toolchain owns its own `Manifest` type (mirroring the compiler's
//! `SaturnConfig`) so that the CLI is self-contained for project discovery,
//! profile management, and build orchestration. When interfacing with the
//! compiler, the `Manifest` is converted to/from `stnx::config::SaturnConfig`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The root of a `saturn.toml` manifest.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Manifest {
    /// Package metadata.
    #[serde(default)]
    pub package: Package,
    /// External dependencies.
    #[serde(default)]
    pub dependencies: BTreeMap<String, DependencySpec>,
    /// Build configuration.
    #[serde(default)]
    pub build: BuildSection,
}

/// `[package]` section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Package {
    /// The package / project name.
    pub name: String,
    /// Semantic version string.
    #[serde(default = "default_version")]
    pub version: String,
    /// Saturnite edition.
    #[serde(default = "default_edition")]
    pub edition: String,
}

fn default_version() -> String {
    "0.1.0".to_string()
}

fn default_edition() -> String {
    "2026".to_string()
}

impl Default for Package {
    fn default() -> Self {
        Self {
            name: "untitled".to_string(),
            version: default_version(),
            edition: default_edition(),
        }
    }
}

/// `[build]` section — controls source layout.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildSection {
    /// Source root directory relative to the project root (default: `src`).
    #[serde(default = "default_source_dir")]
    pub source: String,
    /// Entry point file stem relative to the source root (default: `main`).
    /// The toolchain appends `.stn` (canonical) with `.stnx` as fallback.
    #[serde(default = "default_entry")]
    pub entry: String,
}

fn default_source_dir() -> String {
    "src".to_string()
}

fn default_entry() -> String {
    "main".to_string()
}

impl Default for BuildSection {
    fn default() -> Self {
        Self {
            source: default_source_dir(),
            entry: default_entry(),
        }
    }
}

/// A dependency specification (version requirement).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct DependencySpec {
    pub version: String,
}

impl Manifest {
    /// Parse a `saturn.toml` from a string.
    pub fn from_str(contents: &str) -> anyhow::Result<Self> {
        toml::from_str(contents).map_err(|e| anyhow::anyhow!("TOML parse error: {}", e))
    }

    /// Load a manifest from a directory (looks for `saturn.toml`).
    pub fn from_dir(dir: &Path) -> anyhow::Result<Self> {
        let path = dir.join("saturn.toml");
        let contents = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("failed to read {}: {}", path.display(), e))?;
        Self::from_str(&contents)
    }

    /// Synthesize a minimal manifest from a package name.
    pub fn from_name(name: &str) -> anyhow::Result<Self> {
        let toml = format!(
            r#"[package]
name = "{}"
version = "0.1.0"
edition = "2026"

[build]
source = "src"
entry = "main"
"#,
            name
        );
        Self::from_str(&toml)
    }

    /// The source root directory (absolute or relative to `project_root`).
    pub fn source_root(&self, project_root: &Path) -> PathBuf {
        project_root.join(&self.build.source)
    }

    /// The default entry file name as specified in the manifest (e.g. `main`).
    pub fn entry_stem(&self) -> &str {
        &self.build.entry
    }

    /// Resolve the entry point file path, trying `.stn` first then `.stnx`.
    pub fn resolve_entry(&self, project_root: &Path) -> PathBuf {
        let src_root = self.source_root(project_root);
        let entry = self.entry_stem();

        // If the entry already has an extension, try it as-is.
        let exact = src_root.join(entry);
        if exact.is_file() {
            return exact;
        }

        // Try .stn (canonical) then .stnx (legacy).
        let stem = entry;
        if !stem.ends_with(".stn") && !stem.ends_with(".stnx") {
            let stn = src_root.join(format!("{}.stn", stem));
            if stn.is_file() {
                return stn;
            }
            let stnx = src_root.join(format!("{}.stnx", stem));
            if stnx.is_file() {
                return stnx;
            }
        }

        // Return the canonical .stn path (caller checks existence).
        src_root.join(format!("{}.stn", entry))
    }

    /// Generate the canonical `saturn.toml` text for a new project.
    pub fn generate_toml(&self) -> String {
        let deps_str = if self.dependencies.is_empty() {
            "# saturnite-stdlib = \"0.1\"".to_string()
        } else {
            self.dependencies
                .iter()
                .map(|(k, v)| format!("{} = \"{}\"", k, v.version))
                .collect::<Vec<_>>()
                .join("\n")
        };

        format!(
            r#"[package]
name = "{}"
version = "{}"
edition = "{}"

[dependencies]
{}

[build]
source = "src"
entry = "main"
"#,
            self.package.name, self.package.version, self.package.edition, deps_str
        )
    }

    /// Write `saturn.toml` to the given project directory.
    pub fn write_to(&self, dir: &Path) -> anyhow::Result<()> {
        let path = dir.join("saturn.toml");
        std::fs::write(&path, self.generate_toml())
            .map_err(|e| anyhow::anyhow!("Failed to write saturn.toml: {}", e))?;
        Ok(())
    }
}
