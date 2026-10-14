//! Example Rust crate for Saturnite ecosystem interoperability testing.
//! Exposes ABI-safe functions via `#[no_mangle] extern "C"`.

use std::os::raw::c_char;

/// Add two i64 values. ABI-safe.
#[no_mangle]
pub extern "C" fn sat_example_crate_add_i64(a: i64, b: i64) -> i64 {
    a + b
}

/// Multiply two i64 values. ABI-safe.
#[no_mangle]
pub extern "C" fn sat_example_crate_multiply_i64(a: i64, b: i64) -> i64 {
    a * b
}

/// Greet a user by name. Returns a C string.
/// The returned pointer is valid for the lifetime of the program.
#[no_mangle]
pub extern "C" fn sat_example_crate_greet(name: *const c_char) -> *const c_char {
    use std::ffi::{CStr, CString};
    use std::sync::OnceLock;

    static GREETING: OnceLock<CString> = OnceLock::new();

    let name_str = unsafe { CStr::from_ptr(name).to_str().unwrap_or("stranger") };
    let greeting = format!("Hello, {}!", name_str);
    let cstring = GREETING.get_or_init(|| CString::new(greeting).unwrap());
    cstring.as_ptr()
}
