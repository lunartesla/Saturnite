// A minimal Rust crate with ABI-safe functions for testing the
// Saturnite wrapper generation pipeline.
//
// These functions are deliberately simple: they take and return only
// ABI-safe types (i64, f64, bool, str as *const c_char).

use std::os::raw::c_char;
use std::ffi::CStr;
use std::str;

/// Add two i64 values and return the result.
pub fn add(a: i64, b: i64) -> i64 {
    a + b
}

/// Greet by name — returns the length of the input string.
/// In a real implementation this would format a string; here we just
/// echo back the length to keep the test deterministic.
pub fn greet(name: &str) -> i64 {
    name.len() as i64
}

/// String length via raw C string pointer.
pub fn cstring_len(s: *const c_char) -> i64 {
    if s.is_null() {
        return 0;
    }
    let cstr = unsafe { CStr::from_ptr(s) };
    let bytes = cstr.to_bytes();
    bytes.len() as i64
}
