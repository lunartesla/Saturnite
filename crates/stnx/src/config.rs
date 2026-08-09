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

use crate::error::CompilerError;
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

/// A dependency specification.
///
/// In `saturn.toml`, dependencies are written as key-value pairs:
/// ```toml
/// [dependencies]
/// saturnite-stdlib = "0.1"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct DependencySpec {
    pub version: String,
}

impl std::str::FromStr for DependencySpec {
    type Err = CompilerError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self {
            version: s.to_string(),
        })
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
            config.dependencies.get("saturnite-stdlib").unwrap().version,
            "0.1"
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
        assert_eq!(config.dependencies.get("dep-a").unwrap().version, "1.0");
        assert_eq!(config.dependencies.get("dep-b").unwrap().version, "2.3");
        assert_eq!(config.dependencies.get("dep-c").unwrap().version, "0.5");
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
        };
        let toml_str = toml::to_string(&config).unwrap();
        let config2: SaturnConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(config2.package.name, config.package.name);
        assert_eq!(config2.package.version, config.package.version);
        assert_eq!(config2.package.edition, config.package.edition);
    }
}
