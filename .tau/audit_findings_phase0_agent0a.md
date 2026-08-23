# Phase 0 Compiler Architecture Audit — Agent 0A Findings

**Scope:** Read-only inspection of `crates/stnx/src/` (target, config, codegen, mir, main.rs, lib.rs, module.rs, error.rs) and `crates/stnx/tests/`.
**Date:** 2026-08-23
**Note:** Findings are based on the **current source state** (working tree). Some references in `.tau/audit-findings.md` are stale and do not match the actual source (e.g., `run_diagnostics` no longer exists in `codegen/mod.rs`).

---

## 1. Duplicated Profile Logic (Profile → OptLevel/DebugInfo mapping)

The `Profile` → `(OptimizationLevel, DebugInfo)` mapping appears in **four** independent locations:

### 1a. `crates/stnx/src/target.rs`, lines 57–93
The canonical `Profile` enum with `opt_level()` and `debug_info()` methods. This is the centralized definition that `target.rs` documentation says callers "should use instead."

```rust
// target.rs:79-92
pub fn opt_level(&self) -> OptimizationLevel {
    match self { Profile::Debug => OptimizationLevel::None, Profile::Release => OptimizationLevel::Aggressive }
}
pub fn debug_info(&self) -> DebugInfo {
    match self { Profile::Debug => DebugInfo::Yes, Profile::Release => DebugInfo::No }
}
```
**Also provides:** `TargetConfig::with_profile()` (line 190) and `TargetConfig::apply_profile()` (line 202) as centralized entry points — **neither of which is called by main.rs or any test.**

### 1b. `crates/stnx/src/main.rs`, lines 21–40
A **duplicate** `Profile` enum defined inside the binary crate, with `as_str()` and `is_release()` methods. This shadows the library `Profile`. The `Build` command (lines 181–187) and `Run` command (lines 391–397) both construct it. The actual opt-level/debug-info application is then inlined in the `Build` command (lines 220–242):

```rust
// main.rs:220-242 — inlined profile mapping
match opt_level {
    Some(0) => { config.set_opt_level(OptimizationLevel::None); config.set_debug_info(DebugInfo::Yes); }
    Some(1) => config.set_opt_level(OptimizationLevel::Less),
    Some(2) => config.set_opt_level(OptimizationLevel::Default),
    Some(3) => config.set_opt_level(OptimizationLevel::Aggressive),
    None => {
        if profile.is_release() {
            config.set_opt_level(OptimizationLevel::Aggressive);
            config.set_debug_info(DebugInfo::No);
        } else {
            config.set_opt_level(OptimizationLevel::None);
            config.set_debug_info(DebugInfo::Yes);
        }
    }
}
```

### 1c. `crates/stnx/src/main.rs`, lines 514–520 (in `build_run_file`)
A **second** inlined instance of the same mapping:

```rust
// main.rs:514-520 — duplicate of the Build command's profile mapping
if profile.is_release() {
    config.set_opt_level(OptimizationLevel::Aggressive);
    config.set_debug_info(DebugInfo::No);
} else {
    config.set_opt_level(OptimizationLevel::None);
    config.set_debug_info(DebugInfo::Yes);
}
```

### 1d. `crates/stnx/tests/common/mod.rs`, lines 32–51
A **third** duplicate: a test-only `Profile` enum with `apply_to()` method that hardcodes the same mapping:

```rust
// tests/common/mod.rs:39-50
pub fn apply_to(self, config: &mut TargetConfig) {
    match self {
        Profile::Debug => { config.set_opt_level(OptimizationLevel::None); config.set_debug_info(DebugInfo::Yes); }
        Profile::Release => { config.set_opt_level(OptimizationLevel::Aggressive); config.set_debug_info(DebugInfo::No); }
    }
}
```
The comment on line 29 explicitly acknowledges the duplication: *"main.rs does not expose `Profile` publicly, so the tests reproduce the same mapping here."* This is incorrect — `stnx::target::Profile` IS public via `lib.rs:60`, but the tests define their own local copy instead.

**Finding:** The `Profile` enum and its opt-level/debug-info mapping are duplicated in 4 places (`target.rs`, `main.rs` Build command, `main.rs` build_run_file, `tests/common/mod.rs`). The centralized `target::Profile::apply_profile()` (target.rs:202) is never used.

---

## 2. Duplicated Target Configuration Logic

### 2a. `OptimizationLevel → InkwellOptLevel` mapping (two locations)
- `target.rs:309-316` — `TargetConfig::to_inkwell_opt_level()` (canonical)
- `mir/codegen.rs:795-800` — `compile_from_mir_ext()` **re-implements** the same match:

```rust
// mir/codegen.rs:795-800 — duplicate of target.rs:309-316
let opt_level = match target_config.opt_level() {
    OptimizationLevel::None => InkwellOptLevel::None,
    OptimizationLevel::Less => InkwellOptLevel::Less,
    OptimizationLevel::Default => InkwellOptLevel::Default,
    OptimizationLevel::Aggressive => InkwellOptLevel::Aggressive,
};
```

### 2b. `OptimizationLevel → pass pipeline name` (two locations)
- `target.rs:325-332` — `TargetConfig::opt_pass_name()` (canonical, returns `"default<O0>".."default<O3>"`)
- `mir/codegen.rs:805-810` — `compile_from_mir_ext()` **re-implements** the same match:

```rust
// mir/codegen.rs:805-810 — duplicate of target.rs:325-332
let opt_passes = match target_config.opt_level() {
    OptimizationLevel::Less => "default<O1>",
    OptimizationLevel::Default => "default<O2>",
    OptimizationLevel::Aggressive => "default<O3>",
    OptimizationLevel::None => "default<O0>",
};
```

**Finding:** `compile_from_mir_ext()` in `mir/codegen.rs` duplicates both the `OptimizationLevel → InkwellOptLevel` mapping and the `OptimizationLevel → pass pipeline name` mapping that are already centralized in `TargetConfig::to_inkwell_opt_level()` and `TargetConfig::opt_pass_name()`. The `opt_pass_name()` method (target.rs:332) is tested (target.rs:423-462) but never called in production code.

### 2c. `with_defaults` helper (target.rs:126-145)
`TargetConfig::host()` (line 147) and `TargetConfig::from_triple()` (line 161) both delegate to `with_defaults()`. This is correctly centralized — no duplication here.

---

## 3. Diagnostic Leakage

**Status:** The `run_diagnostics` function that was previously documented as leaking into `codegen/mod.rs` has been **removed** in the current source. The current `codegen/mod.rs` (lines 1–36) contains only `check_linker()`, `host_triple()`, and re-exports. No diagnostic/environment-check functions exist in the codegen module.

However, there is a **related diagnostic concern**: `run_doctor()` in `main.rs` (lines 625–683) mixes environment diagnostics (host triple, target config, linker, runtime) with the `check_linker` call. The test `test_doctor.rs` (lines 162–176) verifies that "Linker:" appears exactly once, confirming the redundant double-check was already fixed. The current code does NOT double-check the linker.

**Finding:** No diagnostic leakage from codegen module into backend modules. The previously documented `run_diagnostics` leak has been resolved. The `run_doctor()` function in `main.rs:625-683` is appropriately located in the CLI binary (not in the library codegen module).

**Note:** The task list includes "Phase 2: Remove run_diagnostics codegen leak" — this appears to be already completed in the current codebase.

---

## 4. Dead CLI / Codegen APIs

### 4a. `compile_from_mir` (non-ext wrapper) — `mir/codegen.rs:765-771`
A thin wrapper that calls `compile_from_mir_ext(mir, output_path, target_config, false)`. **Never called** anywhere in the codebase (grep confirms zero call sites). Only re-exported in `lib.rs:53`. Should be removed or deprecated.

### 4b. `TargetConfig::default_file_type()` — `target.rs:357-363`
Returns a `FileType` based on `output_kind`. **Never called** anywhere in the codebase (grep confirms zero references). Should be removed.

### 4c. `MirVerifyError::to_compiler_error()` — `mir/verify.rs:35-37`
A conversion method that is **never called**. Callers of `MirProgram::verify()` in both `main.rs:496-502` and `tests/common/mod.rs:134-137` format the errors via `to_string()` instead. Should be removed.

### 4d. `set_cpu()` and `set_features()` — `target.rs:301-308`
Setters for CPU and feature strings that are **never called** anywhere in the codebase (grep confirms zero call sites including tests). The `cpu` and `features` fields are initialized in `with_defaults()` to `"generic"` and `""` and never changed. Should be removed or documented as future-use.

### 4e. `check_file()` unused parameter — `main.rs:547`
`fn check_file(input: &std::path::Path, _target_triple: Option<&str>)` — the `_target_triple` parameter is unused (prefixed with underscore). The `Commands::Check` handler at line 380 calls `check_file(&input, target.as_deref())` but the parameter is ignored. Should be removed along with the caller.

### 4f. `resolve_output()` unused parameter — `main.rs:433-441`
`fn resolve_output(..., _target: &Option<String>)` — the `_target` parameter is unused. Called at line 245 with `&target` that is discarded. Should be removed along with the caller.

### 4g. `ObjectEmitter::emit_ir()` and `emit_ir_to_file()` — `codegen/emitter.rs:33-42`
- `emit_ir()` (line 33) returns IR as a `String`. **Never called** — IR generation for the `Build` command goes through `mir::codegen::generate_ir_from_mir` (main.rs:329), not through this method.
- `emit_ir_to_file()` (line 37) **Never called** — the only IR file writing happens in `mir/codegen.rs:828-829` via `module.print_to_file()` directly.

### 4h. `Profile` methods not used by main binary — `target.rs:66-92`
`Profile::as_str()` (line 66), `Profile::is_release()` (line 74), `opt_level()` (line 79), `debug_info()` (line 87) — these are **public** on `target::Profile`. The `main.rs` `Profile` enum has its own `as_str()` and `is_release()` (lines 30, 37), and main.rs inlines the opt-level/debug-info mapping instead of calling `target::Profile::opt_level()`/`debug_info()`. The library `Profile` methods are only exercised internally in `target.rs` tests (lines 466-488) and the test helper in `tests/common/mod.rs` (which defines its own enum, not using the library one).

---

## 5. Stale Public Exports (lib.rs)

### 5a. `compile_from_mir` — `lib.rs:53`
Re-exported but **never called** (see dead API 4a). No external consumer exists.

### 5b. `MirVerifyError` — `lib.rs:55`
Re-exported from `mir::verify`. Used **nowhere** outside its own module except the re-export itself. Callers of `mir.verify()` receive `Vec<MirVerifyError>` and use `to_string()`, never importing the type by name.

### 5c. `HirLower` — `lib.rs:41`
Re-exported from `hir::lower`. Used **only** in the HIR crate itself (`hir/lower.rs:130`). No external consumer (tests or main.rs) references `stnx::HirLower`. The `hir::lower::lower()` function is used by `tests/mir_lower.rs:7` via path `stnx::hir::lower::lower`, not via re-export.

### 5d. `DefEntry, DefKind, DefTable` — `lib.rs:41`
Re-exported from `hir::symbol`. A search across the entire codebase shows these are **never referenced** by any external code (main.rs, tests, examples). Only used internally within the `hir` module.

### 5e. `HirModDecl` — `lib.rs:41`
Re-exported from `hir::function`. Used **nowhere** outside the HIR module itself.

### 5f. `Linker` — `lib.rs:50`
Re-exported from `codegen::linker`. The struct is used internally by `mir/codegen.rs:780,842` via `crate::codegen::Linker`, but the `stnx::Linker` re-export at `lib.rs:50` is **never imported by name** in any external consumer (tests use `stnx::codegen::{check_linker, host_triple}` only — see `test_doctor.rs:24`).

### 5g. `ObjectEmitter` — `lib.rs:50`
Similarly, the `stnx::ObjectEmitter` re-export is **never imported by name** externally. Only used internally via `crate::codegen::{Linker, ObjectEmitter}` in `mir/codegen.rs:780`.

### 5h. `DependencySpec, Package, SaturnConfig` — `lib.rs:77`
Re-exported from `config`. `SaturnConfig` is used by `module.rs:24` via `crate::config::SaturnConfig`. The types are **not imported by name** from `stnx::` in any test or the binary. They are accessible via the `config` module path.

### 5i. `Program` (AST re-export) — `lib.rs:32`
Re-exported as `stnx::Program`. Used by `tests/mir_lower.rs:6` as `stnx::ast::Program` (not via the re-export). The direct `stnx::Program` re-export is **never used**.

---

## 6. Project / Config Boundaries

### 6a. `config.rs` responsibilities (lines 1–223)
Defines `SaturnConfig`, `Package`, `DependencySpec` — the parsed representation of `saturn.toml`. Provides:
- `SaturnConfig::from_dir()` (line 41) — loads `saturn.toml` from a directory, synthesizing a default if none exists.
- `SaturnConfig::from_toml_str()` (line 61) — parses TOML string.
- `SaturnConfig::from_name()` (line 67) — creates a minimal config from a directory name.

### 6b. `target.rs` responsibilities (lines 1–489)
Defines `TargetConfig`, `Profile`, `Architecture`, `OperatingSystem`, `Environment`, `OptimizationLevel`, `DebugInfo`, `OutputKind` — target compilation configuration (triple, CPU, features, opt level, debug info, output format).

### 6c. `main.rs` relationship
`main.rs` uses:
- `stnx::target::{DebugInfo, OptimizationLevel, OutputKind, TargetConfig}` (line 7) — but **does not** import `stnx::target::Profile`. Instead defines its own local `Profile` enum (line 23).
- `stnx::codegen` (line 3) — for `host_triple()` and `check_linker()`.
- `stnx::mir::codegen::{compile_from_mir_ext, generate_ir_from_mir}` (line 4).
- `stnx::CompilerError` (line 9).

**Key finding:** `main.rs` does NOT use the library `Profile` type. It defines its own. The `TargetConfig::with_profile()` and `apply_profile()` methods (target.rs:190, 202) exist as the intended centralized API but are never called by the binary.

### 6d. `module.rs` relationship
`module.rs` (untracked, 5584 bytes) imports `crate::config::SaturnConfig` (line 24) and `crate::error::{CompilerError, CompilerResult}` (line 25). It defines `Project` which bundles `SaturnConfig`, project root, source root, and `ModuleGraph`. `Project::load()` and `Project::load_from()` parse source files via `parse_source()` (line 683) which uses `crate::lexer::Lexer` and `crate::parser::parse`.

### 6e. Config boundary assessment
The config module (`config.rs`) is cleanly separated: it only deals with `saturn.toml` parsing. The target module (`target.rs`) deals with compilation target configuration. The `Project` struct (`module.rs`) is the integration point that combines `SaturnConfig` + `ModuleGraph` + filesystem paths. However, `main.rs` does NOT currently use the `Project` or `Module` infrastructure at all — it still does direct single-file lex/parse/analyze/lower/compile. The module system infrastructure exists but is **not yet wired into the CLI**.

---

## 7. Module-System Prerequisites

### 7a. `crates/stnx/src/module.rs` (untracked, exists)
A substantial module (1498 lines including tests). Exposes (via `lib.rs:83`):
```rust
pub use module::{Module, ModuleGraph, ModuleId, ModulePath, ModuleScope, Project};
```

**Key types:**
- `ModuleId(pub u32)` — stable module identity space, separate from `DefId`. Has `ROOT` constant (line 45).
- `ModulePath` — interned `Vec<SymbolId>` path segments.
- `Module` — id, path, file_path, optional AST, parent, mod_declarations (line 217-231).
- `ModuleScope` — per-module namespace: items (HashMap<SymbolId, DefId>), imports, parent (line 290-298).
- `ModuleGraph` — modules vec, root id, symbol_interner, module_index (HashMap), imports edges (line 364-376).
- `Project` — config + root + source_root + graph (line 703-712).

**Key methods:**
- `ModuleGraph::discover_modules(root_file)` (line 497) — text-based `mod` declaration scanning + recursive file discovery.
- `ModuleGraph::add_module()` (line 406), `find_module()` (line 417), `get_module()` (line 422).
- `Project::discover(start)` (line 728) — walks upward for `saturn.toml`, like Cargo.
- `Project::load()` (line 787) — discovers module graph from default entry.
- `Project::load_from(file)` (line 811) — discovers from explicit file.

**Text-based `mod` scanning:** `extract_mod_declarations()` (line 621) is a lightweight regex-free scanner that looks for lines starting with `mod <ident>` or `pub mod <ident>`. The doc comment at line 493-496 notes: "The `mod` keyword is not yet in the lexer (Phase 5 adds it), so this method currently scans the source text for `mod <ident>` declarations using a lightweight text-based approach."

### 7b. `tests/test_module_graph.rs` (untracked, 907 lines)
Comprehensive test suite (25+ tests) covering:
- Project discovery (walk-up, file-path, no-toml synthesis, src/ root)
- Module file discovery (single-file, directory form, precedence, no-mod-case)
- Nested modules (chains, deeply nested)
- Missing modules (error detection)
- Duplicate modules (documented as not-yet-implemented — see comment at line 435-440)
- `saturn.toml` loading (package section, dependencies, defaults)
- `Project::load` end-to-end (graph loading, explicit file, no-entry-point error)
- `ModuleGraph`, `ModulePath`, `ModuleId` public API tests

### 7c. `tests/test_doctor.rs` (untracked, 229 lines)
Tests for the `doctor` command:
- `host_triple()` returns valid triple
- `check_linker()` does not panic
- `TargetConfig::host()` exposes expected fields
- End-to-end `stnx doctor` binary test (exit code, output sections, single linker check)

### 7d. `tests/test_target_config.rs` (untracked, 262 lines)
Tests for profile mapping and target config:
- Debug profile → (None, Yes)
- Release profile → (Aggressive, No)
- Profile consistency (opposite extremes)
- `to_inkwell_opt_level()` mapping (all 4 variants)
- `OutputKind` preservation (Exe, Object, Ir, setter roundtrips)
- Target triple preservation (host config + through compilation)
- Profile mapping end-to-end consistency

**Critical observation (test_target_config.rs:23-31):** The test file's own documentation states: *"The profile mapping itself (Debug → None/Yes, Release → Aggressive/No) mirrors what `main.rs` encodes inline (see findings #1 and #2 in the Phase 0 audit: duplicated profile logic in three call sites)."* However, the tests use a **local `Profile` enum** from `tests/common/mod.rs` (imported at line 17: `use common::{..., Profile}`), NOT the library `stnx::target::Profile`. This is a missed opportunity — the centralized `target::Profile` could be used directly.

### 7e. `tests/common/mod.rs` (386 lines, tracked)
Shared test helpers. Defines its own `Profile` enum (lines 32-51) duplicating the library's. Provides:
- `compile_src(src)` — full pipeline to executable
- `compile_to_object(src)` — compile to .o
- `ir_only(src)` — generate IR text
- `to_mir(src)` — lex→parse→HIR→MIR→verify→optimize
- `lower_to_mir(hir)` — HIR→MIR→verify→optimize
- `analyze_src(src)` — lex→parse→analyze
- `read_file`, `assert_file_exists`

### 7f. `error.rs` — compiler error types (lines 1-158)
Defines `LexError`, `ParseError`, `TargetError`, `LinkError`, `CompilerError` (enum with variants: Lexer, Parse, Semantic, Type, Codegen, Target, Link, Io, Process, Config, IrEmissionError). Provides `CompilerResult<T>` and `TargetResult<T>` type aliases.

**Dead variant:** `CompilerError::IrEmissionError { message: String }` (error.rs:128) — **never constructed** anywhere in the codebase (grep confirms zero references outside the definition).

---

## Summary: Key Findings

| # | Category | Severity | Location | Summary |
|---|----------|----------|----------|---------|
| 1 | Profile duplication | High | target.rs:57-93, main.rs:21-40, main.rs:220-242, main.rs:514-520, tests/common/mod.rs:32-51 | `Profile` enum + mapping duplicated in 4+ locations; centralized `apply_profile()` never used |
| 2 | Target config duplication | Medium | target.rs:309-332, mir/codegen.rs:795-810 | OptLevel→InkwellOptLevel and OptLevel→pass-name mappings duplicated in `compile_from_mir_ext` |
| 3 | Diagnostic leakage | None (resolved) | — | `run_diagnostics` no longer exists; `run_doctor()` is correctly in binary only |
| 4a | Dead API: `compile_from_mir` | Medium | mir/codegen.rs:765 | Non-ext wrapper, never called |
| 4b | Dead API: `default_file_type` | Low | target.rs:357 | Never called |
| 4c | Dead API: `MirVerifyError::to_compiler_error` | Low | mir/verify.rs:35 | Never called |
| 4d | Dead APIs: `set_cpu`, `set_features` | Low | target.rs:301-308 | Never called |
| 4e | Dead param: `check_file` | Low | main.rs:547 | `_target_triple` unused |
| 4f | Dead param: `resolve_output` | Low | main.rs:433 | `_target` unused |
| 4g | Dead APIs: `emit_ir`, `emit_ir_to_file` | Low | codegen/emitter.rs:33,37 | Never called |
| 4h | Unused Profile methods | Low | target.rs:66-92 | `opt_level()`, `debug_info()`, `with_profile()` never called by binary |
| 5a-g | Stale lib.rs exports | Medium/Low | lib.rs:41-83 | `compile_from_mir`, `MirVerifyError`, `HirLower`, `DefEntry/DefKind/DefTable`, `HirModDecl`, `Linker`, `ObjectEmitter`, `Program`, `DependencySpec/Package/SaturnConfig` — various stale or unused re-exports |
| 5h | Dead error variant | Low | error.rs:128 | `IrEmissionError` never constructed |
| 7e | Config boundary gap | High | main.rs | `main.rs` does not use `Project`, `Module`, or `ModuleGraph` — single-file pipeline only; module infrastructure is fully built but unwired to CLI |
