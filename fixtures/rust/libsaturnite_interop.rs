use std::os::raw::{c_int, c_longlong};

/// Minimal ABI-safe Rust fixture for Phase 9 interop testing.
/// Only C-compatible primitive signatures are exposed.
#[no_mangle]
#[allow(improper_ctypes_definitions)]
pub extern "C" fn sat_add_i64(a: c_longlong, b: c_longlong) -> c_longlong {
    a + b
}
