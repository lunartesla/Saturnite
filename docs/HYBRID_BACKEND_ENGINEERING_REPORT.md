# Hybrid Backend Engineering Report

> **Status:** Complete — implemented with adapter-based architecture
> **Date:** 2026-10-19
> **Scope:** Saturnite compiler's Rust-aware hybrid backend (LLVM + Rust wrapper adapters)

---

## 1. Overview

The hybrid backend enables Saturnite programs to use external Rust crates
(`use rust: serde`) alongside native Saturnite code. When Rust imports are
detected, the compiler:

1. Emits an LLVM object file from the MIR program (**LLVM backend**).
2. Locates **pre-built** Rust wrapper static libraries that were compiled
   during `saturn install` (via `rust_wrapper::build_rust_wrapper`).
3. Links the LLVM object, Rust staticlibs, and Saturnite runtime support
   into a single executable.

**Key architectural constraint:** Saturnite source never leaves the LLVM
path. HIR → MIR → LLVM is always used for Saturnite code. Only Rust adapter
wrappers (thin `#[no_mangle] extern "C"` shims that forward to real Rust
crates) are compiled with cargo/rustc, and only during `saturn install`.

---

## 2. Architecture

```
Saturnite source
   │
   ▼
Lexer → Parser → Semantic analysis → HIR
                                    │
                                    ▼
                               MIR lowering
                                    │
                                    ▼
                       MIR typed CFG
                                    │
                                    ▼
                        ┌──────────┴──────────┐
                        │  Backend selection  │
                        │  (auto/hybrid/llvm) │
                        └──────────┬──────────┘
                                   │
              ┌───────────────────┴────────────────────┐
              │ LLVM-only                          │ Hybrid (use rust:) │
              ▼                                    ▼
       emit_object_from_mir                emit_object_from_mir
       → LLVM object                       → LLVM object
       → LLVM link → exe                   │
                                           ▼
                              locate pre-built Rust staticlibs
                              (target/saturnite/wrappers/<crate>_wrapper/
                               target/release/)
                                           │
                                           ▼
                              link_hybrid() — LLVM obj + Rust staticlibs
                              + Saturnite runtime → exe
```

### 2.1 Backend Selection

**File:** `crates/stnx/src/backend.rs`

The `select_backend` function determines which backend to use based on:

- `BackendMode::Auto` — selects `Llvm` when no Rust imports exist, `Hybrid` when
  Rust imports (`use rust:`) are present **and reachable** from `main`.
- `BackendMode::Hybrid` — forces hybrid regardless of imports.
- `BackendMode::Llvm` — forces LLVM-only.

The decision is made after semantic analysis produces the HIR program, which
contains `use_decls` with ecosystem classification (`ExternalKind::Rust`).
Reachability analysis in `find_reachable_rust_imports` walks the call graph
from `main` to determine which Rust imports are actually used.

### 2.2 Rust Wrapper Discovery

**File:** `crates/stnx/src/rust_wrapper.rs`

When the hybrid backend is selected, `build_hybrid` calls `find_rust_staticlibs`
which scans the HIR program's `use_decls` for `ExternalKind::Rust` entries and
locates the corresponding pre-built static libraries at:

```
<project_root>/target/saturnite/wrappers/<crate>_wrapper/target/release/
```

These static libraries were built during `saturn install` by
`cmd.rs:install_dependencies`, which calls
`rust_wrapper::build_rust_wrapper()` with real dependency versions from the
lockfile.

The wrapper staticlib naming convention:
- Unix: `lib<crate>-saturnite-wrapper.a`
- MSVC: `<crate>-saturnite-wrapper.lib`

Each wrapper crate exposes `#[no_mangle] extern "C"` shim functions with
stable ABI boundaries (i64, f64, bool, `*const c_char`, unit).

### 2.3 Hybrid Build Orchestration

**File:** `crates/stnx/src/hybrid_build.rs`

`build_hybrid` is the primary entry point for the hybrid backend. It
orchestrates:

1. `emit_object_from_mir` — emits a `.o` file from MIR using the LLVM backend
2. `find_rust_staticlibs` — locates pre-built wrapper staticlibs for reachable
   Rust imports (errors clearly if they're missing — user must run
   `saturn install` first)
3. `link_hybrid` — links the LLVM object + Rust staticlibs + runtime into the
   final executable

`link_hybrid` reuses the `Linker` infrastructure from
`crates/stnx/src/codegen/linker.rs`, inserting each Rust staticlib on the
link line before the `-o` / `/OUT:` flag so symbols resolve correctly.

### 2.4 MIR Object Emission

**File:** `crates/stnx/src/mir/codegen.rs`

`emit_object_from_mir` emits an LLVM object file (`.o`) without linking.
This is used by the hybrid backend. The MIR codegen at `gen_external_call()`
(line ~958) handles `MirExternalKind::Rust` as direct symbol calls via
`add_function(symbol, ...)`, where `symbol` comes from the `HirExternalFunction`
populated during HIR lowering from wrapper metadata JSON.

### 2.5 HIR Lowering: Ecosystem Import Resolution

**File:** `crates/stnx/src/hir/lower.rs`

During HIR lowering, `process_ecosystem_imports` reads wrapper metadata JSON
files from `target/saturnite/wrappers/<crate>_saturnite.json` and creates
`HirExternalFunction` entries with:
- `symbol` — the link-time symbol (e.g., `sat_serde_deserialize_i64`)
- `ecosystem` — the crate name (e.g., `serde`)

These symbols are what the LLVM backend references at link time.

---

## 3. Key Design Decisions

### 3.1 Saturnite stays on LLVM — adapters are Rust-only

MIR uses a flat CFG representation that does not map to Rust's structured
control flow (if/else, for loops, etc.). Instead of translating the entire
Saturnite program to Rust (which would be lossy and lose LLVM optimization),
Saturnite source always compiles through HIR → MIR → LLVM. Only thin adapter
shims (`#[no_mangle] extern "C"` wrappers) are written in Rust and compiled
with cargo.

### 3.2 Pre-built staticlibs during `saturn install`

Rust wrapper crates are built during `saturn install` (not during `saturn
build`). This separates dependency preparation from compilation, so the build
step only needs to locate and link the pre-built artifacts. This also means
dependency versions come from the lockfile, not derived from crate names.

### 3.3 staticlib crate-type for linking

The generated Rust wrapper crate uses `crate-type = ["staticlib"]` so it
produces a `.a` (or `.lib` on Windows) file that can be linked alongside the
LLVM object file.

### 3.4 Process API for Cargo invocation

All subprocess invocations use `std::process::Command` (never shell strings).
This is enforced throughout `build_rust_wrapper` and `link_hybrid`.

### 3.5 Clear error when wrappers are missing

If a Rust wrapper staticlib is not found on disk, `build_hybrid` returns a
clear error message instructing the user to run `saturn install` first.

---

## 4. Test Coverage

### 4.1 Unit Tests (in-module `#[cfg(test)]`)

| Module | Tests | Coverage |
|--------|-------|----------|
| `hybrid_build.rs` | 3 | `rust_crates_to_build`, `is_rust_ecosystem_crate`, `find_rust_staticlibs` error path |
| `rust_wrapper.rs` | 11 | `generate_cargo_toml`, `generate_wrapper_src`, `abi_type_to_rust_param`, `generate_wrapper_src_param_count`, `find_staticlib_logic`, `cargo_toml_sanitized_name`, `unit_param_filtered`, `str_param_maps_to_c_char_ptr`, `sanitize_crate_name`, `abi_type_to_rust_ret`, `param_name` |
| `rust_discovery.rs` | 8 | `is_abi_safe_type`, `classify_external_calls`, `discover_rust_dependencies`, `is_valid_rust_crate_name`, `sanitize_crate_name` |
| `backend.rs` | 4 | `backend_mode_from_str`, `backend_mode_default`, `backend_mode_ser_de`, `selected_backend_values` |
| **Total** | **26** | |

### 4.2 Integration Tests

**File:** `crates/stnx/tests/test_dual_backend.rs` (9 tests)

| Test | Description |
|------|-------------|
| `test_backend_auto_selects_llvm_for_no_rust_imports` | Auto mode → LLVM when no Rust imports |
| `test_backend_hybrid_forced` | Hybrid mode → Hybrid regardless |
| `test_sat_type_to_rust_all_mappings` | Type mapping for primitives |
| `test_sat_type_to_rust_complex_types_fallback` | Struct/Enum/List → i64 fallback |
| `test_escape_rust_string_edge_cases` | Empty, unicode, quotes, backslashes |
| `test_emit_object_from_mir_produces_object_file` | LLVM object file is produced |
| `test_build_hybrid_errors_when_wrappers_missing` | LLVM-only path succeeds without wrappers |
| `test_find_rust_staticlibs_missing_wrapper_error` | Clear error when wrapper staticlib missing |
| `test_hybrid_build_links_prebuilt_rust_staticlib` | End-to-end hybrid build (LLVM path) |

**File:** `crates/stnx/tests/test_rust_wrapper.rs` (5 tests)

| Test | Description |
|------|-------------|
| `test_rust_wrapper_generation_generates_correct_source` | Generated wrapper has `#[no_mangle]` shims with correct symbols |
| `test_rust_wrapper_generation_handles_multiple_functions` | Multiple shim functions generated |
| `test_rust_wrapper_generation_produces_valid_staticlib` | Full cargo build of wrapper produces `.a` |
| `test_rust_wrapper_cargo_toml_uses_path_dependency` | Cargo.toml has `crate-type = ["staticlib"]` |
| `test_rust_wrapper_abi_version_embedded` | ABI version is embedded in generated source |

### 4.3 Total Test Results

```
248 library unit tests — all passing
  9 dual-backend integration tests — all passing
  5 rust-wrapper integration tests — all passing
262 total tests — zero failures
```

## 5. Files Modified/Created

### Created

| File | Lines | Description |
|------|-------|-------------|
| `crates/stnx/src/rust_wrapper.rs` | ~446 | Rust adapter wrapper crate generator (`#[no_mangle] extern "C"` shims) |
| `crates/stnx/src/hybrid_build.rs` | ~340 | Hybrid build orchestration (LLVM object + pre-built staticlib linking) |
| `crates/stnx/tests/test_rust_wrapper.rs` | ~120 | Integration tests for rust_wrapper |

### Modified

| File | Change |
|------|--------|
| `crates/stnx/src/hybrid_build.rs` | Rewrote from whole-program Rust translation to adapter-based linking — finds pre-built staticlibs instead of generating Rust source from HIR |
| `crates/stnx/src/lib.rs` | Added `rust_wrapper` module declaration and public re-exports; updated hybrid build re-exports |
| `crates/stnx/src/mir/codegen.rs` | Added `emit_object_from_mir()` |
| `crates/stnx/src/codegen/linker.rs` | Made `select_linker`, `build_linker_args`, `resolve_linker_path`, `describe_os`, `describe_env` public |
| `crates/saturn/src/build.rs` | Branch on backend selection: hybrid vs LLVM-only |
| `crates/saturn/src/cmd.rs` | `install_dependencies` builds Rust wrappers via `stnx::rust_wrapper::build_rust_wrapper()` with real versions from lockfile |

### Deprecated (removed from production path)

| File | Status |
|------|--------|
| `crates/stnx/src/rust_codegen.rs` | No longer used in production hybrid build path. Module retained for its standalone tests (`sat_type_to_rust`, `escape_rust_string`). |

## 6. Prerequisites

The hybrid backend requires:

- **Rust toolchain** (`cargo`, `rustc`) on PATH
- **C linker** for the target platform (for `link_hybrid`)
- **LLVM** at `C:\LLVM-Saturnite` on Windows (set
  `LLVM_SYS_211_PREFIX=C:/LLVM-Saturnite` with forward slashes)

`verify_hybrid_prerequisites` checks for all three at startup and produces
a clear error message if any are missing.

**Important:** `saturn install` must be run before `saturn build` when using
`use rust:` imports. The install step builds wrapper static libraries that
the build step locates and links.

## 7. Clippy Status

All modules pass clippy with zero warnings.
