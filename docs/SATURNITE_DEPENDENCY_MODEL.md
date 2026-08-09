# Saturnite Dependency Model — Phase 13

## Status: Design Complete

## Overview

This document describes the dependency model for Saturnite 0.3, covering both
Rust-side crate dependencies (via `saturn.toml`) and Python-side interoperation
(via `cpython`/`pyo3` bindings).

---

## 1. Rust-side Dependencies (`saturn.toml`)

### Current State

The `saturn.toml` config (Phase 10) defines a `[dependencies]` table where each
entry maps a crate name to a version requirement string:

```toml
[package]
name = "myproject"
version = "0.1.0"
edition = "2026"

[dependencies]
saturnite-stdlib = "0.1"
```

### Version Requirement Semantics

| Format | Meaning | Example |
|--------|---------|---------|
| `"1.0"` | Compatible with 1.0 (`>=1.0.0, <2.0.0`) | `saturnite-stdlib = "0.1"` |
| `"1.0.*"` | Any 1.0.x patch | `saturnite-stdlib = "0.1.*"` |
| `">=0.1, <0.3"` | Explicit range | `dep = ">=0.1, <0.3"` |
| `"*"` | Any version (discouraged) | `dep = "*"` |

### Dependency Resolution Flow

```
saturn.toml → config.rs (TOML parse) → DependencySpec → Resolver → vendored/ fetched crates
```

1. **Config parsing:** `SaturnConfig::from_toml_str()` parses `[dependencies]`
   into `BTreeMap<String, DependencySpec>` using `serde` + `toml`.
2. **Version parsing:** `DependencySpec::from_str()` parses version requirement
   strings (Phase 13 implementation will extend this to support semver ranges).
3. **Resolution:** The dependency resolver (not yet implemented — Phase 14+)
   resolves each `(name, version_req)` pair against a local or remote index.
4. **Acquisition:** Vendored dependencies are used directly; remote dependencies
   are fetched and cached in a local registry.

### Dependency Storage

- **Vendored:** Dependencies committed in `vendor/` directory (deterministic
  builds).
- **Remote:** Fetched from crates.io (or a configured registry) and cached in
  `~/.cache/saturn/`.

---

## 2. Python Interoperability

### Design Goals

- Allow Saturnite code to call Python functions (FFI-style).
- Allow Python to call Saturnite-compiled functions (as a Python extension).
- No runtime dependency on CPython at compile time; linking is deferred to
  runtime when Python interop is enabled.

### Architecture

```
┌──────────────┐     ┌──────────────────┐     ┌──────────────┐
│  Saturnite   │ ←→│  FFI boundary   │←→│   Python VM    │
│   source     │  (libffi/cpython)   │     │  (CPython 3.x) │
└──────────────┘     └──────────────────┘     └──────────────┘
```

### Rust-side: Calling Python

1. Saturnite functions can be annotated with `@python` to mark them as Python
   callable.
2. The codegen emits a `pyo3`-compatible module definition alongside the native
   object.
3. At link time, the Saturnite runtime links against `libpython` (discovered via
   `python3-config`).

### Python-side: Calling Saturnite

1. Saturnite-compiled `.so`/`.pyd` files expose a C ABI entry point
   `saturnite_init_module()`.
2. Python's `ctypes` or `cffi` can call these entry points.
3. Alternatively, a thin `pyo3` wrapper can be generated.

### Dependency Graph

```
saturnite-stdlib  (pure Saturnite)
     ↑
saturnite-python-ffi  (Rust shim, pyo3)
     ↑
cpython / pyo3  (Rust crate)
     ↑
CPython 3.x  (system library, dynamic)
```

### Feature-Gating

Python interop is gated behind a Cargo feature `python-interop`:

```toml
# crates/stnx/Cargo.toml (future)
[features]
python-interop = ["dep:pyo3"]
```

This ensures the core compiler has no Python dependency by default.

---

## 3. Cross-Language Type Mapping

| Saturnite | Rust | Python |
|-----------|------|--------|
| `i64` | `i64` | `int` |
| `f64` | `f64` | `float` |
| `bool` | `bool` | `bool` |
| `str` | `String` | `str` |
| `unit` | `()` | `None` |

Future phases will extend this mapping to structs and enums.

---

## 4. Open Questions (for later phases)

- Should Python dependencies be specified in `saturn.toml` under
  `[python.dependencies]`?
- Should the resolver support both Rust crates and Python packages?
- How to handle version conflicts between Rust and Python dependency graphs?
