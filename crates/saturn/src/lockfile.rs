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
        let canonical = toml::to_string(deps)
            .unwrap_or_default();
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
            let resolved_version = resolve_from_registry(name, kind, version_req)?;
            self.dependencies.insert(
                name.clone(),
                LockedDependency {
                    name: name.clone(),
                    version: resolved_version.clone(),
                    kind,
                    source: None,
                    checksum: None,
                },
            );
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
        let resolved_version = resolve_from_registry(name, kind, version_req)?;

        self.dependencies.insert(
            name.to_string(),
            LockedDependency {
                name: name.to_string(),
                version: resolved_version,
                kind,
                source: None,
                checksum: None,
            },
        );
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
        let resolved_version = resolve_from_registry(name, kind, version_req)?;

        self.dependencies.insert(
            name.to_string(),
            LockedDependency {
                name: name.to_string(),
                version: resolved_version,
                kind,
                source: None,
                checksum: None,
            },
        );
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

/// Resolve a dependency from its registry.
///
/// In a full implementation, this would query crates.io / PyPI / etc.
/// For now, it returns the version requirement as the resolved version
/// when no registry integration is available, and records the dependency
/// so the lockfile structure is correct.
fn resolve_from_registry(
    _name: &str,
    _kind: DependencyKind,
    version_req: &str,
) -> Result<String> {
    // Check if version_req is already a specific version (e.g. "1.0.3").
    if version_req == "*" || version_req.starts_with('^') || version_req.starts_with(">=") {
        // For caret / range requirements, we'd query the registry.
        // Without registry integration, we strip the operator and use
        // the bare version as the resolved version.
        let bare = version_req.trim_start_matches('^').trim_start_matches('=').trim_start_matches('>').trim_start_matches('=');
        let base = bare.split('.').take(2).collect::<Vec<_>>().join(".");
        Ok(format!("{}.0", base))
    } else {
        // Bare version like "1.0" → resolved to "1.0.0"
        Ok(version_req.to_string())
    }
}

/// Path to the lockfile within a project root.
pub fn lockfile_path(project_root: &Path) -> PathBuf {
    project_root.join("saturn.lock")
}
