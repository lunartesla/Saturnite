# Phase 9 — Cross-Ecosystem Interoperability Design

Status: Foundation implemented; deeper execution deferred.

## Dependency Kinds (Implemented)

- `DependencyKind::Saturnite`
- `DependencyKind::Rust`
- `DependencyKind::Python`
- `DependencyKind::Native`

These are kept separate. No universal foreign-language framework.

## ABI Boundary (Explicit)

Rust ABI is not stable for arbitrary types. First supported boundary:
- i8/i16/i32/i64, u8/u16/u32/u64, f32/f64
- bool (ABI-safe)
- raw pointers
- C-compatible strings / structs (explicit)

Not automatically supported:
- Vec<T>, String, HashMap<K,V>
- Result<T,E>, Option<T>
- trait objects, closures, async, generics

## Python Interop Model

Conceptual path:
```
Saturnite → Python bridge → Python runtime → Python package
```

No CPython source embedded. No Python syntax parser. Opaque `PythonObject` handle designed; full integration deferred.

## Python Value Boundary (First Pass)

Potential supported primitive conversions (future):
- Saturnite integer ↔ Python int
- float ↔ float
- bool ↔ bool
- string ↔ str
- list ↔ list (element-wise rules required)
- null/unit ↔ None

Unsupported Python objects must map to an opaque handle, not arbitrary Saturnite values.

## Python Lifetimes / Exceptions (Documented)

- Ownership/reference counting deferred.
- Interpreter lifetime must outlive Python calls.
- Exceptions must become controlled Saturnite errors; no silent swallow, no arbitrary process abort.
- Full lifetime management not implemented in Phase 9.

## Dependency Metadata (Implemented / Extended)

`DependencyEntry` carries:
- `kind: DependencyKind`
- `version: String`
- `target: Option<String>`

`SaturnConfig` remains minimal; future extension can support:
- `[rust.dependencies]`
- `[python.dependencies]`
- `[native.dependencies]`

No Cargo or pip replacement.

## Security / Trust

Rust crates, Python packages, and native libraries execute arbitrary native/runtime code. Saturnite's type checker does not make them safe. The boundary is explicit.

No sandbox added.
