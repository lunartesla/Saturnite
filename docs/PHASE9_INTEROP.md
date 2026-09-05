# Phase 9 — Cross-Ecosystem Interoperability Foundation

Status: ARCHITECTURAL FOUNDATION IMPLEMENTED; full runtime bridges DESIGNED / DEFERRED.

## Dependency Model (Implemented in interop.rs)

`DependencyKind` distinguishes:
- Saturnite
- Rust
- Python
- Native

`DependencyEntry` carries kind, version, optional target restriction.
No universal foreign-object framework created.

## ABI Boundary (Designed)

Rust interoperability requires explicit ABI declarations. Supported types for first boundary:
- i8, i16, i32, i64, u8, u16, u32, u64, f32, f64
- bool where ABI-safe
- raw pointers, C-compatible strings, explicitly defined C-compatible structs

Unsupported Rust types (explicitly rejected / deferred):
Vec<T>, String, HashMap<K,V>, Result<T,E>, Option<T>, trait objects, closures, async, generics.

Python interoperability uses runtime/API boundary, not ABI translation.

## Rust Fixture

fixtures/rust/libsaturnite_interop.rs exposes `sat_add_i64` with `#[no_mangle]` / `extern "C"`. This is the wrapper pattern.

## Python Fixture

fixtures/python/test_math.py exposes `add(a,b)`. Actual CPython runtime integration deferred; bridge interfaces established.

## Python Object Lifetimes (Documented Boundary)

- Ownership/reference counting deferred to future phase.
- Interpreter lifetime must outlive all Python calls.
- Exceptions must become controlled Saturnite errors (not silent aborts).
- Opaque PythonObject handle designed but not fully implemented.

## Security / Trust Boundary

Rust crates and Python packages execute arbitrary native/runtime code. Saturnite's type checker does NOT make them safe. Boundary is explicit.

## saturn.toml Extension (Designed)

Conceptual future form (not forced in this phase):

```toml
[rust.dependencies]
mycrate = { version = "1.0", kind = "rust" }

[python.dependencies]
numpy = { version = "*", kind = "python" }
```

Current DependencySpec remains minimal; DependencyKind is available for future extension.

## Supported / Unsupported / Deferred

Implemented:
- DependencyKind enum
- DependencyEntry structure
- Rust fixture wrapper (ABI-safe)
- Python fixture module
- Design docs for ABI, lifetimes, exceptions, targets, linking model

Designed / Deferred:
- Full CPython bridge execution
- Python object opaque handles
- Automated wrapper generation
- Deeper crate/package resolution
- Full dependency download / package manager

Unsupported (explicit):
- Arbitrary Rust ABI without wrapper
- Python syntax parsing
- Universal FFI framework
- Dynamic Python object system

## Target Sensitivity

Dependency metadata supports optional `target` restriction. Actual platform verification remains future work.

## Existing Pipeline Integrity

No changes to lexing, parsing, AST, HIR lowering, MIR, LLVM codegen, list semantics, interpolation, module cycle detection, or runtime behavior unrelated to interop.
