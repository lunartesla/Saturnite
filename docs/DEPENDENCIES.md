# Saturnite Dependency Model

## Status: IMPLEMENTED (dependency model); RESOLVER DEFERRED (no package manager yet)

This document describes Saturnite's dependency model — how `saturn.toml`
declares dependencies, how the four dependency kinds map to the compiler's
import system, and what is implemented vs. deferred.

---

## 1. Dependency kinds

Saturnite distinguishes four kinds of dependency, selected by the import
prefix in source code:

| Import form | `DependencyKind` | Meaning |
|---|---|---|
| `use math` | `Saturnite` | Import a Saturnite-native module (source file or `saturn.toml` dependency). |
| `use rust:serde` | `Rust` | Import a Rust crate (wrapper-backed C ABI boundary). |
| `use python:numpy` | `Python` | Import a Python library (runtime bridge via embedded interpreter). |
| `use native:sqlite` | `Native` | Import a native C-ABI library. |

### Ecosystem imports in the AST

Ecosystem imports (`use rust:serde`, `use python:numpy`, `use native:sqlite`)
are parsed as `ItemKind::UseDecl { ecosystem: Some(ExternalKind::...) }`.
The resolver **skips** ecosystem imports — they are not resolved through
the module graph. Instead, they are handled by the interop bridge at link
and runtime time (see [`INTEROPERABILITY.md`](./INTEROPERABILITY.md)).

Saturnite-native imports (`use math`) are resolved through the module graph
by the dedicated resolver pass (see `crates/stnx/src/resolver.rs`).

---

## 2. `saturn.toml` configuration

The project manifest (`saturn.toml`) declares metadata and dependencies:

```toml
[package]
name = "myproject"
version = "0.1.0"
edition = "2026"

[dependencies]
saturnite-stdlib = "0.1"
```

### Fields

| Section | Field | Type | Default | Description |
|---|---|---|---|---|
| `[package]` | `name` | `String` | — | Project name. |
| `[package]` | `version` | `String` | `"0.1.0"` | Semantic version. |
| `[package]` | `edition` | `String` | `"2026"` | Saturnite edition (informational for current release). |
| `[dependencies]` | `<name>` | `String` | — | Version requirement for a Saturnite-native dependency. |

### Version requirement semantics

| Format | Meaning |
|---|---|
| `"1.0"` | Compatible with 1.0 (`>=1.0.0, <2.0.0`). |
| `"1.0.*"` | Any 1.0.x patch. |
| `">=0.1, <0.3"` | Explicit range. |
| `"*"` | Any version (discouraged). |

> **Note:** Full version-range parsing and dependency resolution are
> **deferred** (see §4). The `DependencySpec` currently records a version
> string; the resolver that resolves these against a local or remote index
> is not yet implemented.

---

## 3. Rust interoperability boundary

Rust interop uses explicit ABI declarations. Only ABI-safe primitive types
are supported at the boundary:

- `i8`, `i16`, `i32`, `i64`, `u8`, `u16`, `u32`, `u64`
- `f32`, `f64`
- `bool` (where ABI-safe)
- Raw pointers
- C-compatible strings and explicitly-defined C-compatible structs

**Unsupported Rust types** (must pass through explicit ABI wrappers):

- `Vec<T>`, `String`, `HashMap<K,V>`
- `Result<T,E>`, `Option<T>`
- Trait objects, closures, async, generics
- `repr(Rust)` structures

See [`INTEROPERABILITY.md`](./INTEROPERABILITY.md) for the full ABI contract.

---

## 4. Deferred (not yet implemented)

- Full dependency resolver / package manager (`stnx add`, `stnx remove`, `stnx install`)
- Lockfile (`saturn.lock`)
- Cache layer for wrapper artifacts
- Automated wrapper generation for arbitrary Rust crates
- Multi-threaded Python integration
- Cross-platform target verification for dependencies
- Remote registry / package download

---

## 5. Source policy

- **No rustc source** is copied or vendored. Saturnite may use Rust crates
  and interoperate with them via the C ABI boundary; it does not reuse
  rustc's source code.
- **No CPython source** is copied or vendored. Python interoperability uses
  the CPython C API as the runtime boundary.
- See the top-level [`README.md`](../README.md) → "Source policy" for the
  full policy statement.

---

## 6. References

- [`SATURNITE_CRATE_DEPENDENCY_AUDIT.md`](./SATURNITE_CRATE_DEPENDENCY_AUDIT.md)
  — audit of all crate dependencies in `Cargo.toml`.
- [`INTEROPERABILITY.md`](./INTEROPERABILITY.md) — Rust and Python interop
  model, ABI rules, fixtures.
- `crates/stnx/src/interop.rs` — `DependencyKind` enum and `DependencyEntry`
  struct (source).
- `crates/stnx/src/config.rs` — `SaturnConfig`, `Package`, `DependencySpec`
  (source).
