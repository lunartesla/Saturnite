//! Saturnite Rust runtime shim.
//!
//! This is a *thin* C ABI surface that the Saturnite runtime calls to
//! interact with externally-built Rust wrapper crates. It is NOT a copy of
//! rustc source and does not reimplement any part of the Rust compiler. It
//! only provides:
//!
//! * a deterministic entry point for Rust wrapper crates to register their
//!   exported symbols (so the linker can find them without Saturnite having
//!   to parse Rust source);
//! * a small set of helpers used by the Saturnite runtime to validate that a
//!   Rust wrapper artifact is present and ABI-compatible before a call.
//!
//! Wrapper crates are compiled by `rustc --crate-type=staticlib` (external
//! tooling) and linked into the Saturnite executable. The exported wrapper
//! functions are `#[no_mangle] pub extern "C"` symbols that the Saturnite
//! linker resolves directly; this shim is only a coordination surface.

#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>

/// Opaque handle to a loaded Rust wrapper artifact.
///
/// The actual symbol resolution happens at link time against the static
/// library; this handle exists so the runtime can record that a given
/// wrapper crate was linked and report a clear diagnostic if it was not.
typedef struct sat_rust_artifact {
    /// The crate name the artifact was built from (NUL-terminated, owned by
    /// the caller; the runtime does not read it after registration).
    const char *crate_name;
    /// Number of exported ABI-safe symbols registered with this artifact.
    uint32_t symbol_count;
    /// ABI version the wrapper was built against. Must match the runtime's
    /// expected ABI version or the artifact is rejected.
    uint32_t abi_version;
} sat_rust_artifact;

/// ABI version expected by the Saturnite runtime for Rust wrapper crates.
/// Wrapper crates built against a different ABI version must be rebuilt.
#define SAT_RUST_ABI_VERSION 1u

/// Register a Rust wrapper artifact with the Saturnite runtime.
///
/// Returns `true` if the artifact's ABI version is compatible with the
/// runtime; `false` otherwise. The runtime records the registration so that
/// a missing artifact can be reported as a clear diagnostic rather than a
/// link failure with an opaque symbol.
bool sat_rust_register_artifact(const sat_rust_artifact *artifact);

/// Look up a registered Rust wrapper artifact by crate name.
///
/// Returns `NULL` if no artifact with that crate name was registered.
const sat_rust_artifact *sat_rust_find_artifact(const char *crate_name);

/// Validate that the given symbol is exported by the named crate's artifact.
///
/// This is a *compile-time/link-time* check: it verifies the runtime's
/// internal registry, not the symbol table. The actual symbol resolution
/// is performed by the linker when the executable is built.
bool sat_rust_has_symbol(const char *crate_name, const char *symbol);