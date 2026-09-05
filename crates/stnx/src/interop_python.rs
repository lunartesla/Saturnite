//! Python bridge contract for Stage B.
//!
//! Defines conversion rules and opaque object boundary. Full CPython
//! runtime integration remains deferred; this module documents the
//! exact contract to prevent unsafe partial implementation.

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
