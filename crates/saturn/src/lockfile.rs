//! `saturn.lock` — lockfile for reproducible dependency resolution.
//!
//! Mirrors Cargo's `Cargo.lock` approach: the lockfile records the exact
//! resolved version (and source) for each dependency declared in
//! `saturn.toml`. On subsequent `install` runs, if the lockfile is present
//! and consistent with `saturn.toml`, dependencies are resolved from the
//! lockfile directly (no registry lookup needed).
//!
//! The lockfile is always regenerated when the manifest changes (detected
//! via a content hash of the `[dependencies]` section).

use crate::manifest::{DependencyKind, DependencySpec};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A single resolved dependency entry in the lockfile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LockedDependency {
    /// The name as declared in `saturn.toml`.
    pub name: String,
    /// The resolved version string (e.g. "1.0.3").
    pub version: String,
    /// The ecosystem kind of this dependency.
    pub kind: DependencyKind,
    /// Optional source URL (e.g. a git URL or registry URL).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Hash of the dependency's manifest in its registry, used to detect
    /// tampering or source changes. Optional until we integrate with a real
    /// registry resolver.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
}

/// The root lockfile structure.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Lockfile {
    /// Version of the lockfile format (for future migrations).
    #[serde(default = "default_version")]
    pub version: u32,
    /// A hash of the manifest's `[dependencies]` section, to detect when the
    /// lockfile needs to be regenerated.
    #[serde(default)]
    pub manifest_hash: String,
    /// Resolved dependencies, keyed by name.
    #[serde(default)]
    pub dependencies: BTreeMap<String, LockedDependency>,
}

fn default_version() -> u32 {
    1
}

impl Lockfile {
    /// Load a lockfile from a project directory. Returns a default (empty)
    /// lockfile if none exists.
    pub fn load(project_root: &Path) -> Result<Self> {
        let lock_path = project_root.join("saturn.lock");
        if lock_path.exists() {
            let contents = std::fs::read_to_string(&lock_path)
                .map_err(|e| anyhow::anyhow!("failed to read saturn.lock: {}", e))?;
            toml::from_str(&contents)
                .map_err(|e| anyhow::anyhow!("failed to parse saturn.lock: {}", e))
        } else {
            Ok(Self::default())
        }
    }

    /// Write the lockfile to the project directory.
    pub fn save(&self, project_root: &Path) -> Result<()> {
        let lock_path = project_root.join("saturn.lock");
        let contents = toml::to_string_pretty(self)
            .map_err(|e| anyhow::anyhow!("failed to serialize lockfile: {}", e))?;
        std::fs::write(&lock_path, contents)
            .map_err(|e| anyhow::anyhow!("failed to write saturn.lock: {}", e))?;
        Ok(())
    }

    /// Compute a hash of the manifest's dependencies section.
    /// Used to detect when the lockfile is stale.
    pub fn compute_manifest_hash(deps: &BTreeMap<String, DependencySpec>) -> String {
        let canonical = toml::to_string(deps).unwrap_or_default();
        // Simple FNV-1a hash (deterministic, no external deps).
        let mut hash: u64 = 0xcbf29ce484222325;
        for byte in canonical.as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        format!("{:x}", hash)
    }

    /// Returns `true` if the lockfile is stale (manifest hash mismatch or
    /// missing entries).
    pub fn is_stale(&self, deps: &BTreeMap<String, DependencySpec>) -> bool {
        let current_hash = Self::compute_manifest_hash(deps);
        if self.manifest_hash != current_hash {
            return true;
        }
        // Check that every manifest dep has a lockfile entry.
        for name in deps.keys() {
            if !self.dependencies.contains_key(name) {
                return true;
            }
        }
        false
    }

    /// Resolve all dependencies from the manifest into the lockfile.
    ///
    /// For each dependency in the manifest, if an existing lockfile entry
    /// satisfies the version spec, it is kept. Otherwise, the resolver
    /// queries the appropriate registry to find a compatible version.
    pub fn resolve(&mut self, deps: &BTreeMap<String, DependencySpec>) -> Result<()> {
        self.manifest_hash = Self::compute_manifest_hash(deps);

        for (name, spec) in deps {
            let kind = spec.kind();
            let version_req = spec.version();

            // If the lockfile already has a compatible entry, keep it.
            if let Some(existing) = self.dependencies.get(name) {
                if existing.kind == kind && Self::satisfies(&existing.version, version_req) {
                    continue;
                }
            }

            // Otherwise, resolve from the registry.
            let resolved = resolve_from_registry(name, kind, version_req)?;
            self.dependencies.insert(name.clone(), resolved);
        }

        // Remove lockfile entries for deps no longer in the manifest.
        let to_remove: Vec<String> = self
            .dependencies
            .keys()
            .filter(|k| !deps.contains_key(*k))
            .cloned()
            .collect();
        for name in to_remove {
            self.dependencies.remove(&name);
        }

        Ok(())
    }

    /// Add or update a single dependency in the lockfile.
    pub fn add(
        &mut self,
        deps: &BTreeMap<String, DependencySpec>,
        name: &str,
        spec: &DependencySpec,
    ) -> Result<()> {
        let kind = spec.kind();
        let version_req = spec.version();
        let resolved = resolve_from_registry(name, kind, version_req)?;

        self.dependencies.insert(name.to_string(), resolved);
        self.manifest_hash = Self::compute_manifest_hash(deps);
        Ok(())
    }

    /// Update a single dependency to its latest compatible version.
    pub fn update(
        &mut self,
        _deps: &BTreeMap<String, DependencySpec>,
        name: &str,
        spec: &DependencySpec,
    ) -> Result<()> {
        let kind = spec.kind();
        let version_req = spec.version();
        let resolved = resolve_from_registry(name, kind, version_req)?;

        self.dependencies.insert(name.to_string(), resolved);
        Ok(())
    }

    /// Check whether a resolved version satisfies a version requirement.
    ///
    /// Supports simple formats:
    /// - `"1.0"` → exact major.minor match (e.g. "1.0.3" matches)
    /// - `"1.0.3"` → exact version match
    /// - `">=1.0"` → minimum version
    /// - `"^1.0"` → semver caret (compatible with 1.0.0)
    /// - `"*"` → any version
    fn satisfies(resolved: &str, req: &str) -> bool {
        if req == "*" {
            return true;
        }

        // Parse the resolved version.
        let (resolved_major, resolved_minor, resolved_patch) = parse_version(resolved);
        let rm = resolved_major.unwrap_or(0);
        let rmin = resolved_minor.unwrap_or(0);
        let rp = resolved_patch.unwrap_or(0);

        if req.starts_with('^') {
            let inner = &req[1..];
            let (major, minor, patch) = parse_version(inner);
            let m = major.unwrap_or(0);
            let mn = minor.unwrap_or(0);
            let p = patch.unwrap_or(0);
            // Caret: compatible with major.minor.patch
            // For 0.x.y, only patch changes are allowed.
            if m == 0 {
                rm == 0 && rmin == mn && rp >= p
            } else {
                rm == m && (rmin, rp) >= (mn, p)
            }
        } else if let Some(inner) = req.strip_prefix("=") {
            let (major, minor, patch) = parse_version(inner);
            let m = major.unwrap_or(0);
            let mn = minor.unwrap_or(0);
            let p = patch.unwrap_or(0);
            rm == m && rmin == mn && rp == p
        } else if let Some(rest) = req.strip_prefix(">=") {
            let (major, minor, patch) = parse_version(rest);
            let m = major.unwrap_or(0);
            let mn = minor.unwrap_or(0);
            let p = patch.unwrap_or(0);
            (rm, rmin, rp) >= (m, mn, p)
        } else {
            // Bare version string: match major.minor (and patch if present in req).
            let (major, minor, patch) = parse_version(req);
            let m = major.unwrap_or(0);
            let mn = minor.unwrap_or(0);
            match patch {
                Some(p) => rm == m && rmin == mn && rp == p,
                None => rm == m && rmin == mn,
            }
        }
    }
}

/// Parse a version string like "1.0.3" into (major, minor, patch) as Option<u64>.
fn parse_version(s: &str) -> (Option<u64>, Option<u64>, Option<u64>) {
    let parts: Vec<&str> = s.split('.').collect();
    let major = parts.get(0).and_then(|p| p.parse::<u64>().ok());
    let minor = parts.get(1).and_then(|p| p.parse::<u64>().ok());
    let patch = parts.get(2).and_then(|p| p.parse::<u64>().ok());
    (major, minor, patch)
}

/// Resolve a dependency from its registry, querying the actual package source.
///
/// - Rust deps → queries crates.io index via `cargo`
/// - Python deps → queries PyPI via `pip index` or `pip install --dry-run`
/// - Saturnite deps → returns the version as-is (no remote registry yet)
/// - Native deps → returns the version as-is (delegating to system resolution at link time)
fn resolve_from_registry(name: &str, kind: DependencyKind, version_req: &str) -> Result<LockedDependency> {
    match kind {
        DependencyKind::Rust => resolve_rust_crate(name, version_req),
        DependencyKind::Python => resolve_python_package(name, version_req),
        DependencyKind::Saturnite => {
            // Saturnite packages have no remote registry yet — return the req as-is.
            Ok(LockedDependency {
                name: name.to_string(),
                version: normalize_saturnite_version(version_req),
                kind,
                source: None,
                checksum: None,
            })
        }
        DependencyKind::Native => {
            // Native deps are resolved at link time from system search paths.
            // The lockfile records the declared version.
            Ok(LockedDependency {
                name: name.to_string(),
                version: normalize_saturnite_version(version_req),
                kind,
                source: None,
                checksum: None,
            })
        }
    }
}

/// Normalize a Saturnite/native version requirement to a concrete version.
fn normalize_saturnite_version(req: &str) -> String {
    if req == "*" {
        return "*".to_string();
    }
    // Strip leading operators and normalize to a concrete version string.
    let bare = req
        .trim_start_matches('^')
        .trim_start_matches('=')
        .trim_start_matches('>');
    // If it already has 3 components, return as-is.
    let parts: Vec<&str> = bare.split('.').collect();
    match parts.len() {
        1 => format!("{}.0.0", parts[0]),
        2 => format!("{}.{}.0", parts[0], parts[1]),
        _ => bare.to_string(),
    }
}

/// Resolve a Rust crate from crates.io using `cargo`.
///
/// Uses `cargo add --dry-run` to verify the crate exists and a compatible
/// version is available. Falls back to querying the crates.io API for the
/// resolved version and checksum.
fn resolve_rust_crate(name: &str, version_req: &str) -> Result<LockedDependency> {
    // First, try to resolve using cargo's own dependency resolution.
    // We create a temporary Cargo project, add the dependency, and read
    // the resolved version from Cargo.lock.
    let temp = tempfile::tempdir().map_err(|e| {
        anyhow::anyhow!("failed to create temp dir for cargo resolution: {}", e)
    })?;

    let cargo_toml = format!(
        r#"[package]
name = "saturnite_resolve"
version = "0.1.0"
edition = "2021"

[dependencies]
{} = "{}"
"#,
        name, version_req
    );

    std::fs::write(temp.path().join("Cargo.toml"), cargo_toml)
        .map_err(|e| anyhow::anyhow!("failed to write temp Cargo.toml: {}", e))?;

    // Create a minimal source file so cargo can parse the manifest.
    std::fs::create_dir_all(temp.path().join("src"))
        .map_err(|e| anyhow::anyhow!("failed to create temp src dir: {}", e))?;
    std::fs::write(temp.path().join("src/main.rs"), "fn main() {}\n")
        .map_err(|e| anyhow::anyhow!("failed to write temp main.rs: {}", e))?;

    // Run `cargo update` to generate Cargo.lock with resolved versions.
    let offline_output = std::process::Command::new("cargo")
        .arg("update")
        .arg("--offline")
        .current_dir(temp.path())
        .output();

    // If offline fails, try online.
    let _output = match offline_output {
        Ok(o) if o.status.success() => o,
        _ => {
            let output = std::process::Command::new("cargo")
                .arg("update")
                .current_dir(temp.path())
                .env("CARGO_HTTP_TIMEOUT", "30")
                .output()
                .map_err(|e| anyhow::anyhow!("failed to run cargo update: {}", e))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(anyhow::anyhow!(
                    "failed to resolve Rust crate '{}@{}': {}",
                    name,
                    version_req,
                    stderr.trim()
                ));
            }
            output
        }
    };

    // Read the Cargo.lock to find the resolved version.
    let lock_path = temp.path().join("Cargo.lock");
    if !lock_path.exists() {
        return Err(anyhow::anyhow!(
            "cargo update did not produce a Cargo.lock for '{}'",
            name
        ));
    }

    let lock_contents = std::fs::read_to_string(&lock_path)
        .map_err(|e| anyhow::anyhow!("failed to read Cargo.lock: {}", e))?;

    // Parse Cargo.lock to find the resolved version.
    let lock: toml::Value = toml::from_str(&lock_contents)
        .map_err(|e| anyhow::anyhow!("failed to parse Cargo.lock: {}", e))?;

    let resolved_version = lock["package"]
        .as_array()
        .and_then(|pkgs| {
            pkgs.iter().find(|p| p["name"].as_str() == Some(name)).and_then(|p| {
                p["version"].as_str().map(|v| v.to_string())
            })
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "crate '{}' not found in Cargo.lock after resolution — \
                 the crate may not exist or no compatible version satisfies '{}'",
                name,
                version_req
            )
        })?;

    // Get checksum if available.
    let checksum = lock["package"]
        .as_array()
        .and_then(|pkgs| {
            pkgs.iter().find(|p| p["name"].as_str() == Some(name)).and_then(|p| {
                p["checksum"].as_str().map(|s| s.to_string())
            })
        });

    Ok(LockedDependency {
        name: name.to_string(),
        version: resolved_version,
        kind: DependencyKind::Rust,
        source: Some("crates.io".to_string()),
        checksum,
    })
}

/// Resolve a Python package from PyPI using pip.
///
/// Uses `pip download --no-deps --dry-run` to verify the package exists and
/// a compatible version is available, then extracts the resolved version.
fn resolve_python_package(name: &str, version_req: &str) -> Result<LockedDependency> {
    // Find a Python interpreter with pip.
    let python = find_python_with_pip()?;

    // Use pip to resolve the version. We construct a pip requirement specifier.
    let pip_req = if version_req == "*" {
        name.to_string()
    } else {
        format!("{}{}", name, build_pip_version_spec(version_req))
    };

    // Use pip install --dry-run to resolve (pip >= 22.2 supports --dry-run).
    let output = std::process::Command::new(&python)
        .arg("-m")
        .arg("pip")
        .arg("install")
        .arg("--dry-run")
        .arg("--no-deps")
        .arg("--quiet")
        .arg(&pip_req)
        .env("PIP_TIMEOUT", "30")
        .output()
        .map_err(|e| anyhow::anyhow!("failed to run pip for resolution: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Check if it's a "no matching distribution" error.
        if stderr.contains("Could not find a version") || stderr.contains("No matching distribution") {
            return Err(anyhow::anyhow!(
                "Python package '{}' not found on PyPI or no version satisfies '{}': {}",
                name,
                version_req,
                stderr.trim()
            ));
        }
        return Err(anyhow::anyhow!(
            "failed to resolve Python package '{}@{}': {}",
            name,
            version_req,
            stderr.trim()
        ));
    }

    // Parse the resolved version from pip's output.
    // pip outputs lines like: "  Using cached numpy-1.26.4-cp313...whl"
    // or with --report we'd get JSON, but we use stdout from --quiet mode.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let resolved_version = parse_pip_version_from_output(&stdout, name)
        .or_else(|| parse_pip_version_from_stderr(&String::from_utf8_lossy(&output.stderr), name));

    let resolved_version = match resolved_version {
        Some(v) => v,
        None => {
            // Fallback: query PyPI JSON API for the latest compatible version.
            match resolve_pypi_version_api(name, version_req) {
                Ok(v) => v,
                Err(e) => {
                    return Err(anyhow::anyhow!(
                        "could not determine resolved version for Python package '{}@{}': {}",
                        name,
                        version_req,
                        e
                    ));
                }
            }
        }
    };

    Ok(LockedDependency {
        name: name.to_string(),
        version: resolved_version,
        kind: DependencyKind::Python,
        source: Some("pypi.org".to_string()),
        checksum: None,
    })
}

/// Find a Python interpreter that has pip available.
fn find_python_with_pip() -> Result<String> {
    for candidate in &["python3", "python"] {
        let output = std::process::Command::new(candidate)
            .arg("-m")
            .arg("pip")
            .arg("--version")
            .output();

        if let Ok(o) = output {
            if o.status.success() {
                return Ok(candidate.to_string());
            }
        }
    }
    Err(anyhow::anyhow!(
        "no Python interpreter with pip found; \
         install Python 3 and pip to use Python dependencies"
    ))
}

/// Convert a Saturnite version requirement to a pip version specifier.
fn build_pip_version_spec(req: &str) -> String {
    // Saturnite version requirements use semver-style: "1.0" → ">=1.0"
    // Cargo-style: "^1.0" → ">=1.0,<2.0"
    // ">=1.0" → ">=1.0"
    // "1.0.3" → "==1.0.3"
    if req == "*" {
        return String::new();
    }

    if let Some(rest) = req.strip_prefix('^') {
        // Caret: ^1.0 → >=1.0,<2.0; ^0.1 → >=0.1,<0.2
        let parts: Vec<&str> = rest.split('.').collect();
        if parts.is_empty() {
            return String::new();
        }
        let major: u64 = parts[0].parse().unwrap_or(0);
        if major == 0 {
            let next_minor = parts.get(1).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0) + 1;
            format!(">={},<0.{}", rest, next_minor)
        } else {
            let next_major = major + 1;
            format!(">={},<{}", rest, next_major)
        }
    } else if let Some(rest) = req.strip_prefix("=") {
        format!("=={}", rest)
    } else if let Some(rest) = req.strip_prefix(">=") {
        format!(">={}", rest)
    } else {
        // Bare version like "1.0" → treat as ">=1.0"
        format!(">={}", req)
    }
}

/// Extract version from pip dry-run output.
fn parse_pip_version_from_output(output: &str, crate_name: &str) -> Option<String> {
    // Look for lines like: "Would install numpy-1.26.4" or "Using cached numpy-1.26.4"
    for line in output.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        for part in parts {
            // Match patterns like "numpy-1.26.4" or "numpy-1.26.4-cp313"
            if let Some(rest) = part.strip_prefix(&format!("{}-", crate_name)) {
                // Extract the version: first segment that starts with a digit
                let version: String = rest
                    .split('-')
                    .find(|s| s.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false))
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                if !version.is_empty() {
                    // Strip any wheel suffix like ".whl" or ".tar.gz"
                    let clean = version.split('.').take(3).collect::<Vec<_>>().join(".");
                    if clean.parse::<u64>().is_ok() || clean.split('.').count() >= 2 {
                        return Some(clean);
                    }
                }
            }
        }
    }
    None
}

/// Extract version from pip stderr output (pip sometimes prints to stderr).
fn parse_pip_version_from_stderr(stderr: &str, crate_name: &str) -> Option<String> {
    parse_pip_version_from_output(stderr, crate_name)
}

/// Fallback: query PyPI's JSON API to find a compatible version.
fn resolve_pypi_version_api(name: &str, version_req: &str) -> Result<String> {
    // Use the PyPI JSON API to get all available versions.
    let url = format!("https://pypi.org/pypi/{}/json", name);
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| anyhow::anyhow!("failed to create HTTP client: {}", e))?;

    let response = client
        .get(&url)
        .send()
        .map_err(|e| anyhow::anyhow!("failed to query PyPI for '{}': {}", name, e))?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!(
            "PyPI returned {} for package '{}'",
            response.status(),
            name
        ));
    }

    let json: serde_json::Value = response
        .json()
        .map_err(|e| anyhow::anyhow!("failed to parse PyPI JSON response: {}", e))?;

    let releases = &json["releases"];
    let mut versions: Vec<String> = releases
        .as_object()
        .map(|m| m.keys().map(|k| k.clone()).collect())
        .unwrap_or_default();

    // Sort versions (descending) and find the first that satisfies the requirement.
    versions.sort_by(|a, b| {
        let av: Vec<u64> = a.split('.').filter_map(|s| s.parse().ok()).collect();
        let bv: Vec<u64> = b.split('.').filter_map(|s| s.parse().ok()).collect();
        bv.cmp(&av)
    });

    for v in &versions {
        if Lockfile::satisfies(v, version_req) {
            return Ok(v.clone());
        }
    }

    Err(anyhow::anyhow!(
        "no version of '{}' satisfies requirement '{}'",
        name,
        version_req
    ))
}

/// Path to the lockfile within a project root.
pub fn lockfile_path(project_root: &Path) -> PathBuf {
    project_root.join("saturn.lock")
}
