# Saturnite Dependency Model

## Status: IMPLEMENTED

This document describes Saturnite's dependency model — how `saturn.toml`
declares dependencies, how the four dependency kinds map to the compiler's
import system, and the `saturn` CLI commands for adding, installing, and
updating dependencies.

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
# Saturnite-native dependency (simple version string)
saturnite-stdlib = "0.1"

# Ecosystem dependencies (full form with kind)
serde = { kind = "rust", version = "1.0" }
numpy = { kind = "python", version = "1.24" }
sqlite = { kind = "native", version = "3.40" }

# The simple form defaults to kind = "saturnite"
myutil = "0.1"
```

### Dependency kinds

| `kind` value | Import syntax | Meaning |
|---|---|---|
| `saturnite` | `use module` | Import a Saturnite-native module. |
| `rust` | `use rust:crate` | Import a Rust crate (wrapper-backed C ABI boundary). |
| `python` | `use python:module` | Import a Python library (runtime bridge via embedded interpreter). |
| `native` | `use native:lib` | Import a native C-ABI library. |

### Fields

| Section | Field | Type | Default | Description |
|---|---|---|---|---|
| `[package]` | `name` | `String` | — | Project name. |
| `[package]` | `version` | `String` | `"0.1.0"` | Semantic version. |
| `[package]` | `edition` | `String` | `"2026"` | Saturnite edition (informational for current release). |
| `[dependencies]` | `<name>` | `String` or `Table` | — | Version requirement. Simple string defaults to `kind = "saturnite"`. Table form allows `{ kind = "...", version = "..." }`. |

### Version requirement semantics

| Format | Meaning |
|---|---|
| `"1.0"` | Compatible with 1.0 (`>=1.0.0, <2.0.0`). |
| `"1.0.*"` | Any 1.0.x patch. |
| `">=0.1, <0.3"` | Explicit range. |
| `"*"` | Any version (discouraged). |

**Rust and Saturnite/Native** deps use Cargo-style semver semantics via the
`semver` crate (`VersionReq::parse`). Python deps use PEP 440 semantics,
which are delegated to `pip` at install time. The `satisfies()` helper
in `lockfile.rs` is used as a fallback for the PyPI JSON API path and does
not fully support PEP 440 operators like `~=` or pre-release versions.

---

## 3. CLI commands

Dependencies are managed via the `saturn` CLI:

```sh
saturn init myproject      # create a new project
saturn add serde           # add a Saturnite dep (defaults to kind = saturnite)
saturn add rust:serde@1.0  # add a Rust dep with explicit version
saturn add python:numpy@1.24
saturn install             # resolve + install all deps from saturn.toml
saturn install --force     # force re-resolution, then install
saturn update              # update a single dep (saturn update <name>)
saturn update --all        # update all deps to latest compatible versions
```

### `saturn add <dep_spec>`

Adds a dependency to `saturn.toml` and resolves it immediately against the
appropriate registry:

- `rust:` → crates.io (via `cargo update` in a temp project)
- `python:` → PyPI (via `pip install --dry-run`)
- `native:` → records the version (system library resolved at link time)
- `saturnite:` → records the version as-is (no remote registry yet)

If resolution succeeds, both `saturn.toml` and `saturn.lock` are updated
transactionally. If resolution fails, the project files are left untouched.

### `saturn install [--force]`

Installs all dependencies declared in `saturn.toml`. The install process has
two phases:

**Phase 1 — Resolution** (only when stale or `--force`):
- If the lockfile is current (manifest hash matches and all deps are locked),
  re-resolution is skipped.
- If `--force` is given or the lockfile is stale, all dependencies are
  re-resolved from their registries.
- The lockfile (`saturn.lock`) is written to disk.

**Phase 2 — Preparation** (always runs, idempotent):
- **Rust deps**: If wrapper metadata JSON exists at
  `target/saturnite/wrappers/<name>_saturnite.json`, a Rust wrapper crate
  is built into a static library. If no metadata is found, the dependency
  is recorded as a failure (the crate was resolved but can't be used from
  Saturnite without a wrapper).
- **Python deps**: A virtual environment is created (if it doesn't already
  exist) at `target/saturnite/venvs/<project_name>/`, and the package is
  `pip install`ed into it.
- **Native deps**: No installation needed; the system library is linked at
  compile time.
- **Saturnite deps**: No preparation needed.

**Failure reporting**: Failures are accumulated across all dependencies.
After all eligible dependencies have been processed, a formatted summary
is printed listing each failure with its kind and reason. If any failures
occurred, `install` exits with a nonzero status code.

### `saturn.lock` format

The lockfile (`saturn.lock`) records the exact resolved version and source
for each dependency:

```toml
version = 1
manifest_hash = "ab12cd34..."

[dependencies.serde]
name = "serde"
version = "1.0.210"
kind = "rust"
source = "crates.io"
checksum = "abcd1234..."

[dependencies.numpy]
name = "numpy"
version = "1.26.4"
kind = "python"
source = "pypi.org"
```

The `manifest_hash` is an FNV-1a hash of the serialized `[dependencies]`
section, used to detect when the lockfile needs re-resolution.

---

## 4. Rust interoperability boundary

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
- `crates/saturn/src/lockfile.rs` — `Lockfile`, `LockedDependency`, resolver
  for crates.io, PyPI, Saturnite, and native deps.
- `crates/saturn/src/cmd.rs` — `add_dependency`, `install_dependencies`,
  `update_dependency` CLI orchestration.
- `crates/stnx/src/config.rs` — `SaturnConfig`, `Package`, `DependencySpec`
  (source).
