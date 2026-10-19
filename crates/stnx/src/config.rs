//! Configuration module for `saturn.toml`.
//!
//! This module defines the data structures and parsing logic for the
//! Saturnite project configuration file (`saturn.toml`). It provides
//! a representation of `[package]` metadata and `[dependencies]`, the
//! minimal config needed for Phase 10.
//!
//! ## Config format
//!
//! ```toml
//! [package]
//! name = "myproject"
//! version = "0.1.0"
//! edition = "2026"
//!
//! [dependencies]
//! saturnite-stdlib = "0.1"
//! ```

use crate::backend::BackendMode;
use crate::error::CompilerError;
use crate::interop::DependencyKind;
use crate::CompilerResult;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// The root of a `saturn.toml` configuration file.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SaturnConfig {
    /// Package metadata: name, version, edition.
    #[serde(default)]
    pub package: Package,
    /// External dependencies, keyed by crate name.
    #[serde(default)]
    pub dependencies: BTreeMap<String, DependencySpec>,
    /// Build configuration: source directory, entry file, etc.
    #[serde(default)]
    pub build: BuildConfig,
}

impl SaturnConfig {
    /// Load and parse a `saturn.toml` file from the given directory path.
    /// If no file is found, a minimal default config using the directory name
    /// is synthesized.
    pub fn from_dir<P: AsRef<Path>>(dir: P) -> CompilerResult<Self> {
        let dir = dir.as_ref();
        let config_path = dir.join("saturn.toml");
        if config_path.exists() {
            let contents = std::fs::read_to_string(&config_path).map_err(|e| {
                CompilerError::config(format!("failed to read {}: {}", config_path.display(), e))
            })?;
            Self::from_toml_str(&contents)
        } else {
            // Synthesize a minimal config from the directory name.
            let name = dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("project")
                .to_string();
            Self::from_name(&name)
        }
    }

    /// Parse a `saturn.toml` from a string.
    pub fn from_toml_str(contents: &str) -> CompilerResult<Self> {
        toml::from_str::<SaturnConfig>(contents)
            .map_err(|e| CompilerError::config(format!("TOML parse error: {}", e)))
    }

    /// Create a minimal default config with just a package name and version.
    pub fn from_name(name: &str) -> CompilerResult<Self> {
        let config_str = format!(
            r#"[package]
name = "{}"
version = "0.1.0"
edition = "2026"
"#,
            name
        );
        Self::from_toml_str(&config_str)
    }
}

/// Build configuration section: `[build]`.
///
/// Controls where the compiler looks for source files, what the default
/// entry point name is, and which compilation backend to use.
///
/// All fields are optional — sensible defaults are provided so that a
/// minimal `saturn.toml` with just `[package]` works out of the box.
///
/// ## `backend`
///
/// The `backend` field controls whether the compiler uses the LLVM-only
/// backend or the Rust-aware hybrid backend:
///
/// - `"auto"` (default) — automatically selects the hybrid backend if the
///   project has reachable `use rust:` imports, otherwise LLVM-only.
/// - `"llvm"` — forces the LLVM-only backend.
/// - `"hybrid"` — forces the Rust-aware hybrid backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildConfig {
    /// The source root directory relative to the project root (default: `src`).
    #[serde(default = "default_source_dir")]
    pub source: String,
    /// The entry point file stem relative to the source root (default: `main`).
    /// The toolchain appends `.stn` (canonical) and falls back to `.stnx`.
    #[serde(default = "default_entry")]
    pub entry: String,
    /// Backend selection mode (default: `"auto"`).
    /// See [`BackendMode`] for the available options.
    #[serde(default, skip_serializing_if = "is_default_backend")]
    pub backend: BackendMode,
}

/// Returns true if the backend mode is `Auto` (the default).
/// Used with `skip_serializing_if` so the TOML doesn't include a redundant
/// `backend = "auto"` line for projects that don't override it.
fn is_default_backend(mode: &BackendMode) -> bool {
    *mode == BackendMode::Auto
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            source: default_source_dir(),
            entry: default_entry(),
            backend: BackendMode::Auto,
        }
    }
}

fn default_source_dir() -> String {
    "src".to_string()
}

fn default_entry() -> String {
    "main".to_string()
}

/// Package metadata section: `[package]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    /// The package / project name.
    pub name: String,
    /// Semantic version string (e.g. "0.1.0").
    #[serde(default = "default_version")]
    pub version: String,
    /// Saturnite edition string (e.g. "2026").
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

/// A dependency specification in `saturn.toml`.
///
/// Supports two TOML forms:
///
/// **Simple form** (backward compatible — defaults to `Saturnite` kind):
/// ```toml
/// [dependencies]
/// saturnite-stdlib = "0.1"
/// ```
///
/// **Full form** (with explicit ecosystem kind):
/// ```toml
/// [dependencies]
/// serde = { kind = "rust", version = "1.0" }
/// numpy = { kind = "python", version = "1.24" }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum DependencySpec {
    /// Simple string form: `"0.1"` → version with default `Saturnite` kind.
    Simple(String),
    /// Full struct form: `{ kind = "rust", version = "1.0" }`.
    Full {
        /// Explicit dependency kind (rust, python, native, or saturnite).
        kind: DependencyKind,
        /// Version / requirement string.
        version: String,
        /// Optional target restriction (e.g. "linux", "windows").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<String>,
    },
}

impl DependencySpec {
    /// Return the dependency kind.
    pub fn kind(&self) -> DependencyKind {
        match self {
            DependencySpec::Simple(_) => DependencyKind::Saturnite,
            DependencySpec::Full { kind, .. } => *kind,
        }
    }

    /// Return the version string.
    pub fn version(&self) -> &str {
        match self {
            DependencySpec::Simple(v) => v,
            DependencySpec::Full { version, .. } => version,
        }
    }

    /// Return the target restriction, if any.
    pub fn target(&self) -> Option<&str> {
        match self {
            DependencySpec::Simple(_) => None,
            DependencySpec::Full { target, .. } => target.as_deref(),
        }
    }

    /// Return the version string (for backward-compatible `.version` field access).
    pub fn version_string(&self) -> String {
        self.version().to_string()
    }
}

impl std::str::FromStr for DependencySpec {
    type Err = CompilerError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::Simple(s.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_full_config() {
        let toml_content = r#"[package]
name = "myproject"
version = "0.1.0"
edition = "2026"

[dependencies]
saturnite-stdlib = "0.1"
"#;
        let config = SaturnConfig::from_toml_str(toml_content).unwrap();
        assert_eq!(config.package.name, "myproject");
        assert_eq!(config.package.version, "0.1.0");
        assert_eq!(config.package.edition, "2026");
        assert_eq!(
            config
                .dependencies
                .get("saturnite-stdlib")
                .unwrap()
                .version(),
            "0.1"
        );
        assert_eq!(
            config.dependencies.get("saturnite-stdlib").unwrap().kind(),
            DependencyKind::Saturnite
        );
    }

    #[test]
    fn test_parse_minimal_config() {
        let config = SaturnConfig::from_name("mycrate").unwrap();
        assert_eq!(config.package.name, "mycrate");
        assert_eq!(config.package.version, "0.1.0");
        assert_eq!(config.package.edition, "2026");
        assert!(config.dependencies.is_empty());
    }

    #[test]
    fn test_parse_package_only() {
        let toml_content = r#"[package]
name = "test"
"#;
        let config = SaturnConfig::from_toml_str(toml_content).unwrap();
        assert_eq!(config.package.name, "test");
        assert_eq!(config.package.version, "0.1.0");
    }

    #[test]
    fn test_parse_empty_config() {
        let config = SaturnConfig::from_toml_str("").unwrap();
        assert_eq!(config.package.name, "untitled");
    }

    #[test]
    fn test_parse_invalid_toml() {
        let result = SaturnConfig::from_toml_str("this is not = valid = toml");
        assert!(result.is_err());
    }

    #[test]
    fn test_multiple_dependencies() {
        let toml_content = r#"[package]
name = "multi"

[dependencies]
dep-a = "1.0"
dep-b = "2.3"
dep-c = "0.5"
"#;
        let config = SaturnConfig::from_toml_str(toml_content).unwrap();
        assert_eq!(config.dependencies.len(), 3);
        assert_eq!(config.dependencies.get("dep-a").unwrap().version(), "1.0");
        assert_eq!(config.dependencies.get("dep-b").unwrap().version(), "2.3");
        assert_eq!(config.dependencies.get("dep-c").unwrap().version(), "0.5");
        assert_eq!(
            config.dependencies.get("dep-a").unwrap().kind(),
            DependencyKind::Saturnite
        );
    }

    #[test]
    fn test_ecosystem_dependencies() {
        let toml_content = r#"[package]
name = "ecosys"

[dependencies]
serde = { kind = "rust", version = "1.0" }
numpy = { kind = "python", version = "1.24" }
sqlite = { kind = "native", version = "3.40" }
std = "0.1"
"#;
        let config = SaturnConfig::from_toml_str(toml_content).unwrap();
        assert_eq!(config.dependencies.len(), 4);

        let serde_dep = config.dependencies.get("serde").unwrap();
        assert_eq!(serde_dep.kind(), DependencyKind::Rust);
        assert_eq!(serde_dep.version(), "1.0");

        let numpy_dep = config.dependencies.get("numpy").unwrap();
        assert_eq!(numpy_dep.kind(), DependencyKind::Python);
        assert_eq!(numpy_dep.version(), "1.24");

        let sqlite_dep = config.dependencies.get("sqlite").unwrap();
        assert_eq!(sqlite_dep.kind(), DependencyKind::Native);
        assert_eq!(sqlite_dep.version(), "3.40");

        let std_dep = config.dependencies.get("std").unwrap();
        assert_eq!(std_dep.kind(), DependencyKind::Saturnite);
        assert_eq!(std_dep.version(), "0.1");
    }

    #[test]
    fn test_dependency_target_optional() {
        let toml_content = r#"[package]
name = "tgt"

[dependencies]
foo = { kind = "rust", version = "1.0", target = "linux" }
"#;
        let config = SaturnConfig::from_toml_str(toml_content).unwrap();
        let dep = config.dependencies.get("foo").unwrap();
        assert_eq!(dep.target(), Some("linux"));
    }

    #[test]
    fn test_config_serde_roundtrip() {
        let config = SaturnConfig {
            package: Package {
                name: "roundtrip".to_string(),
                version: "0.2.0".to_string(),
                edition: "2026".to_string(),
            },
            dependencies: BTreeMap::new(),
            build: BuildConfig::default(),
        };
        let toml_str = toml::to_string(&config).unwrap();
        let config2: SaturnConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(config2.package.name, config.package.name);
        assert_eq!(config2.package.version, config.package.version);
        assert_eq!(config2.package.edition, config.package.edition);
    }
}
