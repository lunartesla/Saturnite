//! Python bridge contract — Stage C (real runtime design, deferred execution).
//!
//! This module defines the conversion rules and lifetime requirements.
//! Full CPython execution remains deferred until interpreter/GIL
//! architecture is reviewed; no fake execution is provided.

/// Supported first-pass Python value conversions.
/// Unsupported objects must become opaque PythonObject handles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PythonConversion {
    IntToInt,
    FloatToFloat,
    BoolToBool,
    StrToStr,
    ListI64ToPythonList,
    UnitToNone,
    /// Unsupported / dynamic value — must not become arbitrary Saturnite value.
    Opaque,
}

/// Opaque handle boundary for unsupported Python objects.
/// This is only a design representation; real reference counting /
/// interpreter lifetime is deferred.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PythonObjectHandle {
    /// Opaque identifier (not a raw pointer exposed to Saturnite code).
    pub id: u64,
}

/// Python runtime requirements (documented, not yet executed).
/// First implementation is intentionally single-threaded.
pub const PYTHON_SINGLE_THREADED: bool = true;
