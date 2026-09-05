//! Phase 9 — Cross-ecosystem interoperability dependency model.
//!
//! Defines explicit dependency kinds so Rust crates, Python packages,
//! native libraries, and Saturnite packages remain conceptually separate.

use serde::{Deserialize, Serialize};

/// Dependency source / ecosystem. This is the authoritative distinction
/// required by Phase 9: Rust and Python must not be collapsed into a
/// vague "foreign library" mechanism.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DependencyKind {
    /// A Saturnite package / native dependency.
    Saturnite,
    /// A Rust crate dependency (wrapper/ABI boundary required).
    Rust,
    /// A Python package dependency (runtime/API boundary required).
    Python,
    /// A native/C ABI library dependency.
    Native,
}

/// A dependency entry in `saturn.toml` with explicit kind metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyEntry {
    /// Explicit dependency kind. Not inferred from name alone.
    pub kind: DependencyKind,
    /// Version / requirement string.
    pub version: String,
    /// Optional target restriction (e.g. "linux").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}
