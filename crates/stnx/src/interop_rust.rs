//! Rust wrapper/ABI metadata for Stage A.
//!
//! Defines an explicit wrapper descriptor without parsing arbitrary Rust AST.

use crate::interop::DependencyKind;

/// Supported ABI primitive types for the Rust interoperability boundary.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AbiPrimitive {
    I8, I16, I32, I64,
    U8, U16, U32, U64,
    F32, F64,
    Bool,
    Pointer,
}

/// A declared ABI-safe Rust function exposed through a wrapper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RustWrapperFunction {
    /// Symbol name (e.g. `sat_add_i64`).
    pub symbol: String,
    /// ABI-safe parameter types.
    pub params: Vec<AbiPrimitive>,
    /// ABI-safe return type.
    pub ret: AbiPrimitive,
    /// Whether the wrapper requires a `#[no_mangle] extern "C"` declaration.
    pub requires_wrapper: bool,
}

/// A Rust dependency wrapper descriptor. This avoids parsing Rust ASTs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RustDependencyWrapper {
    /// Underlying dependency kind (always Rust for this descriptor).
    pub kind: DependencyKind,
    /// Crate or wrapper identifier.
    pub name: String,
    /// Exposed ABI-safe functions.
    pub functions: Vec<RustWrapperFunction>,
}
