//! Cross-ecosystem interoperability dependency model + external-call contract.
//!
//! This module defines:
//!
//! * the explicit dependency-kind taxonomy (`DependencyKind`) so Rust crates,
//!   Python packages, native libraries, and Saturnite packages are never
//!   collapsed into a vague "foreign library" mechanism;
//! * the runtime external-call contract (`ExternalFunctionKind`,
//!   `ExternalFunctionDecl`, `ExternalBinding`) used by the compiler to
//!   represent calls that cross the Saturnite boundary into a foreign
//!   ecosystem at runtime.
//!
//! The external-call contract is *declarative*: it records what the compiler
//! knows at compile time (symbol, parameter types, return type, ABI, target
//! restriction). It does NOT attempt to understand arbitrary foreign source.
//! Concrete runtime behaviour (building a Rust wrapper crate, importing a
//! Python module, loading a native shared library) is implemented by the
//! runtime bridge modules and is deliberately kept out of this data model.

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

// ---------------------------------------------------------------------------
// Runtime external-call contract
// ---------------------------------------------------------------------------

/// The runtime that services an external call.
///
/// This is a *compile-time* classification. It records which bridge is
/// responsible for the call at runtime; it does not decide the bridge at
/// runtime. Rust and Python remain separate bridges (per the campaign rules),
/// and `Native` is the path for plain C-ABI shared libraries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExternalFunctionKind {
    /// A Rust crate function exposed through a `#[no_mangle] extern "C"`
    /// wrapper. Resolved at link time against a static/shared library.
    Rust,
    /// A Python module function, resolved at runtime through the Python
    /// interpreter bridge.
    Python,
    /// A native C-ABI symbol, resolved at link time against a shared library.
    Native,
}

/// A declared external function.
///
/// This is the compiler's authoritative record of an external call. It is
/// produced from explicit declaration metadata (never by parsing arbitrary
/// foreign source) and is consumed by:
///
/// * the semantic/type-checking pass — to validate call-site argument and
///   return types;
/// * the MIR/codegen backend — to emit the correct LLVM declaration and call;
/// * the runtime bridge — to locate and invoke the foreign symbol.
///
/// The representation is deterministic: the same declaration metadata always
/// produces the same `ExternalFunctionDecl`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalFunctionDecl {
    /// The runtime kind (Rust / Python / Native).
    pub kind: ExternalFunctionKind,
    /// The foreign ecosystem name (crate name, module name, or library name).
    /// Used for diagnostics and (for Rust) build integration.
    pub ecosystem: String,
    /// The symbol name to bind to. For Rust/Native this is the link-time
    /// symbol; for Python this is the module-qualified function name.
    pub symbol: String,
    /// ABI-safe parameter types, in declaration order.
    pub params: Vec<HirTypeRef>,
    /// ABI-safe return type.
    pub ret: HirTypeRef,
    /// Optional target restriction (e.g. "linux", "windows"). When present,
    /// the compiler must validate the build target against it before
    /// attempting codegen/link.
    pub target: Option<String>,
}

/// A resolved external binding: the compile-time declaration paired with the
/// concrete runtime artifact it resolved to.
///
/// For Rust this is the path to the built wrapper static library; for Native
/// it is the shared-library path; for Python it is the runtime handle that
/// was acquired during project loading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalBinding {
    /// The declaration this binding was resolved from.
    pub decl: ExternalFunctionDecl,
    /// Path to the artifact that provides the symbol at link/runtime time.
    /// Empty for Python (resolved dynamically at runtime).
    pub artifact: String,
}

/// A compiler-internal type reference used in the external-call contract.
///
/// This is intentionally a restricted, ABI-safe subset. It mirrors the
/// Saturnite primitive types so the type checker can validate external
/// calls against normal Saturnite types without a parallel type system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HirTypeRef {
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
    Bool,
    /// A C-compatible NUL-terminated string pointer.
    CStr,
    /// An opaque pointer (explicitly declared; not an escape hatch).
    Pointer,
}

impl HirTypeRef {
    /// Convert a `HirTypeRef` to its Saturnite `HirType` equivalent, if the
    /// external type has a direct Saturnite representation.
    ///
    /// Returns `None` for `CStr` and `Pointer`, which have no direct
    /// Saturnite primitive and must be handled by an explicit adapter.
    pub fn to_saturnite(&self) -> Option<crate::hir::HirType> {
        use crate::hir::HirType;
        match self {
            HirTypeRef::I8 | HirTypeRef::I16 | HirTypeRef::I32 | HirTypeRef::I64 => {
                Some(HirType::I64)
            }
            HirTypeRef::U8
            | HirTypeRef::U16
            | HirTypeRef::U32
            | HirTypeRef::U64
            | HirTypeRef::F32
            | HirTypeRef::F64 => Some(HirType::F64),
            HirTypeRef::Bool => Some(HirType::Bool),
            HirTypeRef::CStr | HirTypeRef::Pointer => None,
        }
    }
}