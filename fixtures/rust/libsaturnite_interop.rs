use std::os::raw::c_longlong;

/// ABI-safe fixtures for Saturnite interop campaign.
/// Only `#[no_mangle] pub extern "C"` functions are exposed.

#[no_mangle]
pub extern "C" fn sat_add_i64(a: c_longlong, b: c_longlong) -> c_longlong {
    a + b
}

#[no_mangle]
pub extern "C" fn sat_multiply_i64(a: c_longlong, b: c_longlong) -> c_longlong {
    a * b
}
