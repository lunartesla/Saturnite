# INTEROPERABILITY.md

Status: IMPLEMENTED (full end-to-end Rust crate and Python library interoperability).

## Implemented

### Rust Crate Interoperability (Stage A-B)
- `DependencyKind` / `DependencyEntry` (explicit kind distinction)
- Rust ABI subset (`AbiPrimitive`) — I8/I16/I32/I64/U8/U16/U32/U64/F32/F64/Bool/Pointer
- `RustWrapperFunction` / `RustDependencyWrapper` descriptor
- External function declaration syntax: `external rust "crate_name" "symbol_name"(params) -> ret`
- Automatic linking of `lib<crate_name>.a` static libraries from:
  - Output directory (`target/`)
  - Current working directory
  - `<cwd>/libs/<crate_name>/`
  - `<cwd>/libs/`
  - `/usr/local/lib/`
  - `<dir>/<crate_name>/target/release/` (cargo build output)
- Rust fixtures: `fixtures/rust/libsaturnite_interop.rs` (add_i64, multiply_i64) — builds to `libsaturnite_interop.a`
- End-to-end verified: `sat_add_i64(20, 22)` returns 42 via direct LLVM call to Rust staticlib

### Python Library Interoperability (Stage C-D)
- Python conversion contract (`PythonConversion`) — Int↔Int, Float↔Float, Bool↔Bool, Str↔Str, ListI64↔PythonList, Unit↔None, Opaque
- `PythonObjectHandle` opaque boundary design
- Thin C shim: `runtime/pyrt.h` + `runtime/pyrt_impl.c` — no CPython source copied
- Single-threaded embedded CPython interpreter (initialized once, lives for process lifetime)
- Flat ABI: `sat_py_call_flat(spec, search_path, kinds[], values[], arg_count, out)`
  - Spec format: `"module::function"` (split on first `::`)
  - Parallel `int32_t kinds[]` / `int64_t values[]` arrays (f64 bit-cast to i64)
  - Returns `sat_py_result` struct with union field for primitives or opaque handle
- External function declaration syntax: `external python "module_name" "function_name"(params) -> ret`
- Automatic linking of Python libraries (`-lpython3.13 -lpthread -ldl -lutil -lm`)
- Python search path discovered via `python3-config` at link time
- Python fixtures: `fixtures/python/test_math.py`
- End-to-end verified: `test_math.add(20, 22)` returns 42 via embedded interpreter

### Shared Infrastructure
- Dependency model supports optional `target` restriction (`DependencyEntry::target`)
- MIR carries `external_libraries` vector through monomorphization
- Linker resolves and links declared external libraries automatically
- Clear diagnostics for missing libraries (lists all searched paths)
- `-no-pie` linking for compatibility with non-PIE runtime object
- PIC codegen (`RelocMode::PIC`) for position-independent objects

## Deferred
- Full dependency resolver / package manager
- Lockfile (`saturn.lock`)
- Cache layer for wrapper artifacts
- Multi-threaded Python integration
- Cross-platform target verification beyond `target` string
- Automated wrapper generation for arbitrary crates

## Unsupported (explicit)
- Arbitrary Rust ABI without wrapper
- `Vec<T>`, `String`, `HashMap`, `Option<T>`, `Result<T,E>`, trait objects, closures, async, generics as direct ABI
- Python syntax parsing / interpreter rewrite
- Universal FFI framework
- Dynamic Python object system that leaks into Saturnite static semantics

## Rust Source Policy
No rustc source copied. No rustc internals vendored.

## Python Source Policy
No CPython source copied. No Python interpreter rewritten.