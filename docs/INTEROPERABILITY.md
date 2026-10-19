# Saturnite Interoperability Guide

This document describes the actual supported Rust and Python interoperability
capabilities. The implementation has been verified end-to-end.

## Status

**Implemented:** Rust crate calls via auto-generated wrappers with Cargo-built
static libraries, and Python module calls via the embedded CPython runtime.

## Rust Crate Interoperability

### Import syntax

```saturnite
use rust:crate_name
```

Imports a Rust crate whose wrapper metadata is available under
`target/saturnite/wrappers/<crate_name>_saturnite.json`.

### Calling functions

Qualified call syntax — `crate.function(args)`:

```saturnite
use rust:example_crate as rust_lib

fn main() -> i64:
    let result = rust_lib.add(20, 22)
    say result
    result
```

### Wrapper metadata format

```json
{
  "crate_name": "example_crate",
  "functions": [
    {
      "name": "add",
      "symbol": "example_add",
      "params": ["i64", "i64"],
      "ret": "i64",
      "qualified_name": "example_crate.add"
    }
  ]
}
```

### Supported Rust types (ABI-safe)

`i64`, `i32`, `i16`, `i8`, `u64`, `u32`, `u16`, `u8`, `f64`, `f32`, `bool`,
`unit`. Integers narrower than `i64`/`u64` are widened. Floating-point supports
both `f32`→`f64` widening and truncation.

### Linking

When the hybrid backend is selected, the compiler searches for pre-built
Rust wrapper static libraries at:

- `<project_root>/target/saturnite/wrappers/<crate>_wrapper/target/release/`

The naming convention is `lib<crate>-saturnite-wrapper.a` (Linux/macOS) or
`<crate>-saturnite-wrapper.lib` (Windows MSVC). These wrappers are built
during `saturn install` by `rust_wrapper::build_rust_wrapper()`.

The LLVM object (from Saturnite source) and Rust staticlibs are linked
together with the Saturnite runtime via `link_hybrid`.

### End-to-end test

Located in `fixtures/rust/example_crate/`. Build with Cargo:

```bash
cargo build --release --manifest-path fixtures/rust/example_crate/Cargo.toml
```

The Saturnite example at `examples/interop_rust.stn` calls `example_crate.add(20, 22)`
and returns `42`.

## Python Library Interoperability

### Import syntax

```saturnite
use python:module_name
```

### Calling functions

```saturnite
use python:math as py_math

fn main() -> f64:
    let root = py_math.sqrt(81.0)
    say root
    root
```

### Runtime bridge

The CPython C API bridge lives in `crates/stnx/runtime/`. Key functions:

- `sat_py_set_search_path` — initializes the embedded interpreter and sets `sys.path`
- `sat_py_import_and_cache` — imports and caches a module (linear-scan cache)
- `sat_py_get_attr` — retrieves an attribute from a module or object handle
- `sat_py_call_method` — invokes a method on a handle via `PyObject_CallMethod`

All calls go through `sat_py_call_flat` which takes a `"module::function"` spec
string, a `search_path`, parallel `kinds[]`/`values[]` arrays, and an out parameter
for the result. The result struct distinguishes success, `None`, wrong-type, and
Python exceptions.

### Supported Python types

`i64` (int within range), `f64` (float), `bool` (boolean), `unit` (None),
`str` (UTF-8 text). Other types return an opaque handle that must be released
via `sat_py_release_handle`.

### Venv setup and interpreter discovery

When `saturn install` processes a Python dependency (`use python:module`), it
creates a virtual environment at `target/saturnite/venvs/<project_name>/` and
pip-installs the resolved package version into it. The venv's site-packages
directory becomes the `search_path` passed to `sat_py_call_flat` at runtime,
so `PyImport_Import` can locate the module.

Python interpreter discovery is platform-aware:

- **Unix (Linux/macOS)**: candidates `["python3", "python"]`
- **Windows**: candidates `["python", "python3"]`

The first candidate that responds to `python --version` (or `--version`)
successfully is used. This follows the convention that Unix systems typically
provide `python3` (while `python` may not exist), whereas Windows CPython
installs are registered as `python` (while `python3` may not exist).

Both `PyEnv::create()` in `crates/stnx/src/pyenv.rs` and
`find_python_with_pip()` in `crates/saturn/src/lockfile.rs` use this
platform-aware discovery. The Python headers and library used to compile the
`pyrt_impl.c` shim are discovered via `python3-config --includes` / `--ldflags`
on Unix, or via `python` + `sysconfig` on Windows (see `crates/stnx/build.rs`).

### End-to-end test

Wrapper metadata for `math` at `target/saturnite/wrappers/math_saturnite.json`
enables `math.sqrt(81.0)` to execute through the embedded CPython interpreter.

## Shared External-Call Infrastructure

Both ecosystems share the `MirRvalue::ExternalCall { kind, symbol, args, ret_ty }`
representation in MIR. Code generation differs:

- **Rust/Native:** Direct LLVM `call` to the resolved symbol
- **Python:** Call to `sat_py_call_flat` with the `"module::func"` spec string

## Linker Support

On Windows MSVC, when `link.exe` is unavailable, the compiler falls back to
`rust-lld` (shipped with the Rust toolchain) in `lld-link` mode. This provides
full MSVC-link compatibility without requiring Visual Studio Build Tools.

## Limitations

- Complex types (`Vec<T>`, `String`, `HashMap`) require handwritten adapters or
  are represented as opaque handles.
- Generic Rust APIs and trait objects need user-provided adapters.
- Python's GIL is held for the duration of each call; true multithreading across
  the boundary requires explicit handle management.
- Cross-compilation to targets without matching runtime artifacts is not supported.
