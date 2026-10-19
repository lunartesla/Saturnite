# Hybrid Backend Engineering Report

> **Status:** Complete — implemented across Milestones 1–6
> **Date:** 2026-10-19
> **Scope:** Saturnite compiler's Rust-aware hybrid backend (LLVM + Rust codegen)

---

## 1. Overview

The hybrid backend enables Saturnite programs to use external Rust crates
(`use rust: serde`) alongside native Saturnite code. When Rust imports are
detected, the compiler:

1. Emits an LLVM object file from the MIR program (LLVM backend).
2. Generates Rust source from the HIR program (Rust backend).
3. Compiles the generated Rust source into a `staticlib` via `cargo build`.
4. Links the LLVM object, Rust staticlib, and Saturnite runtime support
   into a single executable.

This document describes the architecture, components, and test coverage of
the hybrid backend.

## 2. Architecture

```
Saturnite source
   │
   ▼
Lexer → Parser → Semantic analysis → HIR
                                    │
                    ┌───────────────┴───────────────┐
                    ▼                               ▼
               MIR lowering                     Backend
                  │                           selection
                  ▼                           (auto/hybrid/llvm)
         MIR typed CFG          ┌────────────────────┴────────────────────┐
              │                 │ LLVM-only            │ Hybrid (Rust use:) │
              ▼                 │                      │                    │
   emit_object_from_mir()       ▼                      ▼                    ▼
            │            compile_from_mir            emit_object_from_mir  generate_rust_source
            │            → LLVM object               → LLVM object         → Rust source
            │            → LLVM link → exe           │                     │
            │                                        ▼                     ▼
            │                                  build_rust_crate           Cargo.toml
            │                                  → staticlib              → cargo build
            │                                        │                     │
            │                                        ▼                     ▼
            │                                   link_hybrid() ←───────────────┘
            │                                   (LLVM obj + Rust staticlib + runtime)
            │                                         │
            │                                         ▼
            └────────────────────────────────→  final executable
```

### 2.1 Backend Selection

**File:** `crates/stnx/src/backend.rs`

The `select_backend` function determines which backend to use based on:

- `BackendMode::Auto` — selects `Llvm` when no Rust imports exist, `Hybrid` when
  Rust imports (`use rust:`) are present.
- `BackendMode::Hybrid` — forces hybrid regardless of imports.
- `BackendMode::Llvm` — forces LLVM-only.

The decision is made after semantic analysis produces the HIR program, which
contains `use_decls` with ecosystem classification (`ExternalKind::Rust`).

### 2.2 Rust Crate Discovery

**File:** `crates/stnx/src/rust_discovery.rs`

When the hybrid backend is selected, `discover_rust_dependencies` scans the
HIR program's `use_decls` for `ExternalKind::Rust` entries and validates them
against the project's `saturn.toml` manifest. It produces
`RustDependencyInfo` structs containing:

- Crate name and alias
- ABI-safe external functions (types are all ABI-safe: I64, F64, Bool, Str)
- ABI-incompatible functions (types that can't cross the FFI boundary)

Type safety is enforced by `is_abi_safe_type`, which checks that all
parameters and return types are primitives that have direct Rust equivalents.

### 2.3 Rust Source Code Generation

**File:** `crates/stnx/src/rust_codegen.rs`

`generate_rust_source` translates HIR directly into Rust source code. This
operates on HIR (not MIR) because MIR's flat control-flow graph (CFG) does
not map cleanly to Rust's structured control flow (if/else, for loops, etc.).

Key functions:

- `sat_type_to_rust(&HirType) -> &str` — maps Saturnite types to Rust types
  (`I64 → i64`, `F64 → f64`, `Bool → bool`, `Str → &'static str`, `Unit → ()`)
- `escape_rust_string(&str) -> String` — escapes backslashes and double quotes
  for safe embedding in generated Rust string literals
- `generate_cargo_toml(...)` — produces a `Cargo.toml` with `staticlib`
  crate-type (or `bin` when `as_binary` is set)
- `build_rust_crate(...)` — invokes `cargo build --release` via
  `std::process::Command` (never shell strings)

The generated Rust source uses `extern "C"` function signatures so that
Saturnite's LLVM codegen can call into the Rust staticlib at link time.

### 2.4 Hybrid Build Orchestration

**File:** `crates/stnx/src/hybrid_build.rs`

`build_hybrid` is the primary entry point for the hybrid backend. It
orchestrates all four steps:

1. `emit_object_from_mir` — emits a `.o` file from MIR using the LLVM backend
2. `generate_rust_source` — produces Rust source from HIR
3. `build_rust_crate` — runs `cargo build --release` on the generated source
4. `link_hybrid` — links the LLVM object + Rust staticlib + runtime into the
   final executable

`link_hybrid` reuses the `Linker` infrastructure from
`crates/stnx/src/codegen/linker.rs`, inserting the Rust staticlib on the
link line before the `-o` flag so symbols resolve correctly.

### 2.5 MIR Object Emission

**File:** `crates/stnx/src/mir/codegen.rs`

`emit_object_from_mir` was added to emit an LLVM object file (`.o`) without
linking. This is used by both the LLVM-only backend (when `OutputKind::Object`
is selected) and the hybrid backend (step 1). The function takes a
`&TargetConfig` (not an owned `TargetConfig` since it doesn't implement
`Clone`).

## 3. Key Design Decisions

### 3.1 HIR → Rust (not MIR → Rust)

MIR uses a flat CFG representation with `LocalId`, `BlockId`, basic blocks,
`Rvalue`, and `Terminator`. This does not map to Rust's structured control
flow (if/else, match, for loops). Instead, the hybrid backend translates HIR
directly to Rust, which preserves the structured control flow that Rust
requires.

### 3.2 staticlib crate-type for linking

The generated Rust crate uses `crate-type = ["staticlib"]` so it produces a
`.a` (or `.lib` on Windows) file that can be linked alongside the LLVM object
file. When `as_binary = true`, it generates a `bin` crate with a
`fn main()` entry point that calls `std::process::exit()`.

### 3.3 Process API for Cargo invocation

All subprocess invocations (cargo build, linker) use
`std::process::Command`, never shell strings. This is enforced throughout
`build_rust_crate` and `link_hybrid`.

## 4. Test Coverage

### 4.1 Unit Tests (in-module `#[cfg(test)]`)

| Module | Tests | Coverage |
|--------|-------|----------|
| `rust_codegen.rs` | 6 | `sat_type_to_rust`, `escape_rust_string`, `generate_rust_source`, `generate_cargo_toml` |
| `rust_discovery.rs` | 7 | `is_abi_safe_type`, `discover_rust_dependencies`, `classify_external_calls`, `is_valid_rust_crate_name`, `sanitize_crate_name` |
| `hybrid_build.rs` | 3 | `rust_crates_to_build`, `is_rust_ecosystem_crate` |
| **Total** | **16** | |

### 4.2 Integration Tests

**File:** `crates/stnx/tests/test_dual_backend.rs` (10 tests)

| Test | Description |
|------|-------------|
| `test_backend_auto_selects_llvm_for_no_rust_imports` | Auto mode → LLVM when no Rust imports |
| `test_backend_hybrid_forced` | Hybrid mode → Hybrid regardless |
| `test_sat_type_to_rust_all_mappings` | Type mapping for primitives |
| `test_sat_type_to_rust_complex_types_fallback` | Struct/Enum/List → i64 fallback |
| `test_escape_rust_string_edge_cases` | Empty, unicode, quotes, backslashes |
| `test_generate_rust_source_produces_valid_structure` | Generated source has `pub fn main`, returns 42 |
| `test_generate_rust_source_as_binary` | Binary mode has `fn main()` and `std::process::exit` |
| `test_generate_cargo_toml_with_dependencies` | Cargo.toml with deps, staticlib crate-type |
| `test_generate_cargo_toml_as_binary` | Binary mode has no crate-type |
| `test_emit_object_from_mir_produces_object_file` | LLVM object file is produced |

### 4.3 Total Test Results

```
248 library unit tests — all passing
 10 integration tests — all passing
258 total tests — zero failures
```

## 5. Files Modified/Created

### Created

| File | Lines | Description |
|------|-------|-------------|
| `crates/stnx/src/rust_codegen.rs` | ~600 | Rust source code generator |
| `crates/stnx/src/rust_discovery.rs` | ~310 | Rust crate discovery and ABI validation |
| `crates/stnx/src/hybrid_build.rs` | ~370 | Hybrid build orchestration |
| `crates/stnx/tests/test_dual_backend.rs` | ~210 | Integration tests |

### Modified

| File | Change |
|------|--------|
| `crates/stnx/src/lib.rs` | Added module declarations and re-exports |
| `crates/stnx/src/mir/codegen.rs` | Added `emit_object_from_mir()`, changed signature to `&TargetConfig` |
| `crates/stnx/src/codegen/linker.rs` | Made `select_linker`, `build_linker_args`, `resolve_linker_path`, `describe_os`, `describe_env` public |
| `crates/saturn/src/build.rs` | Branch on backend selection: hybrid vs LLVM-only |

## 6. Prerequisites

The hybrid backend requires:

- **Rust toolchain** (`cargo`, `rustc`) on PATH
- **C linker** for the target platform (for `link_hybrid`)
- **LLVM** at `C:\LLVM-Saturnite` on Windows (set
  `LLVM_SYS_211_PREFIX=C:/LLVM-Saturnite` with forward slashes)

`verify_hybrid_prerequisites` checks for all three at startup and produces
a clear error message if any are missing.

## 7. Clippy Status

All new modules pass clippy with zero warnings:

- `rust_codegen.rs` — clean (converted `impl Default` to `#[derive(Default)]`,
  replaced `push_str("\n")` with `push('\n')`, replaced `.iter().next()` with
  `.first()`, removed unnecessary `format!` in `push_str`)
- `rust_discovery.rs` — clean
- `hybrid_build.rs` — clean (fixed doc link for `Linker`)
