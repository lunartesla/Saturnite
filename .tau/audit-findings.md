# Compiler Architecture Audit: Findings Report

**Scope:** `crates/stnx/src/target.rs`, `crates/stnx/src/codegen/`, `crates/stnx/src/mir/`, `crates/stnx/src/main.rs`, `crates/stnx/src/config.rs`

**Date:** 2026-08-22

---

## 1. Duplicated Profile Logic (debug/release mapping)

**Finding:** The debug-vs-release → opt_level/debug_info mapping is duplicated across three call sites in `main.rs`.

- **`/home/dimitar/saturnite/Saturnite/crates/stnx/src/main.rs`**
  - **Lines 180–242** — `Commands::Build` arm: `Profile` selection (lines 181–187) followed by the `opt_level` match (lines 220–242). The `None` arm (release/debug fallback at lines 234–240) maps `Profile::Release` → `OptimizationLevel::Aggressive` + `DebugInfo::No`, and `Profile::Debug` → `OptimizationLevel::None` + `DebugInfo::Yes`.
  - **Lines 391–397** — `Commands::Run` arm: identical `Profile` selection logic from `release`/`debug` flags (lines 391–397).
  - **Lines 506–512, 514–520** — `build_run_file()` function: `Profile` → `TargetConfig` mapping repeated at lines 514–520 (`profile.is_release()` → Aggressive/No vs None/Yes).

Three distinct call sites each encode the same `Profile → (OptimizationLevel, DebugInfo)` mapping. If the mapping ever changes (e.g. debug uses `OptimizationLevel::Less`), all three sites must be updated independently — a classic maintenance hazard.

### Recommendation
Extract a single function or method, e.g. `TargetConfig::apply_profile(&mut self, profile: Profile)`, and have both `main.rs` and `build_run_file` call it.

---

## 2. Duplicated Target Configuration Logic

**Finding:** The `OptimizationLevel → InkwellOptLevel` mapping appears in **two** places, plus the `TargetConfig::host()` construction is repeated.

### OptimizationLevel mapping duplication

- **`/home/dimitar/saturnite/Saturnite/crates/stnx/src/target.rs`, lines 228–235** — `TargetConfig::to_inkwell_opt_level()` provides the canonical mapping.
- **`/home/dimitar/saturnite/Saturnite/crates/stnx/src/mir/codegen.rs`, lines 795–810** — `compile_from_mir_ext()` **re-implements** the same mapping inline instead of calling `target_config.to_inkwell_opt_level()`. Lines 795–799 duplicate the `OptimizationLevel → InkwellOptLevel` conversion; lines 805–810 separately re-map `OptimizationLevel` to pass-manager strings (`"default<O1>"`, etc.).

### TargetConfig::host() construction duplication

- **`/home/dimitar/saturnite/Saturnite/crates/stnx/src/main.rs`, lines 211–217** — `Build` command constructs `TargetConfig::host()` or `from_triple()`.
- **`/home/dimitar/saturnite/Saturnite/crates/stnx/src/main.rs`, lines 506–512** — `build_run_file()` duplicates the identical `TargetConfig::host()` / `from_triple()` construction.

### Default field initialization duplication

- **`/home/dimitar/saturnite/Saturnite/crates/stnx/src/target.rs`**
  - `host()` (lines 76–97) initializes `opt_level: OptimizationLevel::default()`, `debug_info: DebugInfo::No`, `output_kind: OutputKind::Exe`.
  - `from_triple()` (lines 99–124) initializes the **exact same** defaults (`OptimizationLevel::default()`, `DebugInfo::No`, `OutputKind::Exe`, `cpu: "generic"`, `features: String::new()`).

### Recommendation
- `compile_from_mir_ext()` should call `target_config.to_inkwell_opt_level()` instead of re-implementing the mapping.
- Extract a private `TargetConfig::with_defaults()` helper for the shared default field initialization.

---

## 3. Diagnostic Leakage (CLI/env diagnostics in codegen/target)

**Finding:** `run_diagnostics()` — a CLI/environment diagnostic function — lives in the `codegen` module, which is a backend code-generation seam.

- **`/home/dimitar/saturnite/Saturnite/crates/stnx/src/codegen/mod.rs`, lines 38–90** — `pub fn run_diagnostics() -> CompilerResult<()>`. This function:
  - Prints `"Saturnite Compiler Diagnostics"` (line 39)
  - Prints host triple, architecture, OS, environment, opt level (lines 43–70)
  - Calls `host_triple()` and `check_linker()` (lines 43, 72–73)
  - Prints `"inkwell 0.9 with LLVM 21.x (dynamic linking)"` (line 87)

This is pure CLI/environment diagnostic logic (user-facing output formatting), but it resides in `codegen/mod.rs` alongside the object-emission and linking seams. The `mod.rs` doc comment (lines 1–13) claims the module only provides "object-emission and linking stages," yet it also houses a user-facing diagnostic printer — a category violation.

Additionally:
- **`/home/dimitar/saturnite/Saturnite/crates/stnx/src/main.rs`, lines 625–652** — `run_doctor()` calls `codegen::run_diagnostics()` then re-checks the linker and runtime availability. This **double-checks** the linker: `run_diagnostics()` already calls `check_linker` (line 72), and `run_doctor()` calls `codegen::check_linker(&config)` again (line 631), producing potentially conflicting or redundant diagnostic output.

The `env!("OUT_DIR")` usage for runtime path construction in `run_doctor()` (line 641) is a build-time environment concern leaking into runtime CLI code.

### Recommendation
Move `run_diagnostics()` and its callers to a dedicated `diagnostics` module or keep it in `main.rs` (which is already CLI-specific). Remove the redundant linker check in `run_doctor()`.

---

## 4. Dead CLI/codegen APIs (unused public exports)

**Finding:** Multiple public APIs are exported but never called.

### `compile_from_mir` (not `_ext`)
- **`/home/dimitar/saturnite/Saturnite/crates/stnx/src/mir/codegen.rs`, lines 764–771** — `pub fn compile_from_mir(...)` is a thin wrapper around `compile_from_mir_ext(..., false)`.
- **Re-exported in `lib.rs` line 52.**
- **Never called anywhere.** `main.rs` imports and uses only `compile_from_mir_ext` (line 4: `use stnx::mir::codegen::{compile_from_mir_ext, generate_ir_from_mir}`), and both call sites (main.rs:336, main.rs:541) use `compile_from_mir_ext` directly.

### `ObjectEmitter::emit_ir` and `ObjectEmitter::emit_ir_to_file`
- **`/home/dimitar/saturnite/Saturnite/crates/stnx/src/codegen/emitter.rs`**
  - `emit_ir()` (line 33) — returns IR as a `String`. **Never called.** IR generation for the `Build` command goes through `mir::codegen::generate_ir_from_mir` (main.rs:329), not through this emitter method.
  - `emit_ir_to_file()` (line 37) — writes IR to a file. **Never called.** The `OutputKind::Ir` path in `compile_from_mir_ext` (codegen.rs:827–831) calls `ctx.module.print_to_file()` directly, bypassing `ObjectEmitter` entirely.

### `TargetConfig::to_inkwell_opt_level`
- **`/home/dimitar/saturnite/Saturnite/crates/stnx/src/target.rs`, line 228** — `pub fn to_inkwell_opt_level()`.
- **Called only by** `TargetConfig::create_target_machine()` (line 248). It is **not** called by `compile_from_mir_ext()` (codegen.rs:795–799), which re-implements the mapping inline instead. So while it is used internally, it represents a missed reuse opportunity rather than dead code per se.

### `TargetConfig::default_file_type`
- **`/home/dimitar/saturnite/Saturnite/crates/stnx/src/target.rs`, line 260** — `pub fn default_file_type()`.
- **Never called anywhere** in the codebase. The `compile_from_mir_ext()` function at codegen.rs:826 uses `target_config.output_kind()` to dispatch, but never calls `default_file_type()`. The `ObjectEmitter` creates a `TargetMachine` that is never actually used for emission (it uses `write_to_file` on the module directly).

### `TargetConfig::set_cpu` and `TargetConfig::set_features`
- **`/home/dimitar/saturnite/Saturnite/crates/stnx/src/target.rs`, lines 220–225** — both `pub fn`.
- **Never called from any code.** The `main.rs` Build command never sets CPU or features; defaults of `"generic"` and `""` are used silently.

### `target_triple` parameter in `check_file` and `resolve_output`
- **`/home/dimitar/saturnite/Saturnite/crates/stnx/src/main.rs`, line 547** — `fn check_file(input: &Path, _target_triple: Option<&str>)` — the second parameter is **prefixed with underscore** (unused).
- **`/home/dimitar/saturnite/Saturnite/crates/stnx/src/main.rs`, line 441** — `fn resolve_output(..., _target: &Option<String>)` — also prefixed with underscore and never used in the body.

### `MirVerifyError::to_compiler_error`
- **`/home/dimitar/saturnite/Saturnite/crates/stnx/src/mir/verify.rs`, line 35** — `pub fn to_compiler_error()`.
- **Never called.** `main.rs` (lines 278–284, 496–502) converts `MirVerifyError`s via `.to_string()` + `.join(", ")`, never via `to_compiler_error()`.

### Recommendation
Remove all dead APIs or mark them `#[deprecated]`. At minimum, remove `compile_from_mir` (the non-ext wrapper), `default_file_type`, `emit_ir`, `emit_ir_to_file`, `to_compiler_error`, `set_cpu`, `set_features`, and the unused parameters in `check_file`/`resolve_output`.

---

## 5. Stale Public Exports in lib.rs

**Finding:** `lib.rs` re-exports several items that are not consumed by `main.rs` or that belong to a CLI-only concern.

### `codegen::run_diagnostics` and `codegen::check_linker` and `codegen::host_triple` re-exported but partially redundant

- **`/home/dimitar/saturnite/Saturnite/crates/stnx/src/lib.rs`, line 49** — `pub use codegen::{check_linker, host_triple, run_diagnostics, Linker, ObjectEmitter};`
- `host_triple` IS used by `main.rs` (lines 174, 297, 356, 524).
- `check_linker` IS used by `main.rs` (line 631).
- `run_diagnostics` IS used by `main.rs` (line 626).
- `Linker` is used internally by `mir::codegen::compile_from_mir_ext` (codegen.rs:842) but not by `main.rs`.
- `ObjectEmitter` is used internally by `mir::codegen::compile_from_mir_ext` (codegen.rs:833, 839) but not by `main.rs`.

So `Linker` and `ObjectEmitter` are in the public `stnx` API but not used by the CLI consumer — they are part of the stable library API but have no external caller. This is a design smell: these are internal backend seams that should arguably be `pub(crate)`.

### `Architecture`, `Environment`, `OperatingSystem` exported but unused by consumers

- **`/home/dimitar/saturnite/Saturnite/crates/stnx/src/lib.rs`, lines 58–61** — `pub use target::{Architecture, DebugInfo, Environment, OperatingSystem, OptimizationLevel, OutputKind, TargetConfig};`
- `Architecture`, `Environment`, and `OperatingSystem` are **never referenced in `main.rs`** (which only uses `DebugInfo`, `OptimizationLevel`, `OutputKind`, `TargetConfig`). They are used internally by `target.rs` (parse_triple) and `linker.rs` (select_linker, describe_os, describe_env), but those are `pub` exports with no external consumer.
- `OptimizationLevel` and `OutputKind` and `DebugInfo` ARE used by `main.rs`.

### `DependencySpec` and `Package` exported but unused at runtime

- **`/home/dimitar/saturnite/Saturnite/crates/stnx/src/lib.rs`, line 76** — `pub use config::{DependencySpec, Package, SaturnConfig};`
- `SaturnConfig`, `Package`, and `DependencySpec` are **never imported or used anywhere** in `main.rs` or any other source file. The config module is completely unused at runtime — `main.rs` constructs its own `saturn.toml` string inline in `init_project()` (lines 599–604) rather than using `SaturnConfig`.

### `MirVerifyError` and `VerifyResult` exported

- **`/home/dimitar/saturnite/Saturnite/crates/stnx/src/lib.rs`, line 54** — `pub use mir::verify::{MirVerifyError, VerifyResult};`
- `MirVerifyError` is used internally by the verify module (verify.rs:30-38, 62, 69, etc.).
- `VerifyResult` is used internally (verify.rs:32, verify.rs:42, verify.rs:51).
- Neither is consumed by `main.rs` or any external code.

### Recommendation
- Remove `DependencySpec`, `Package`, `SaturnConfig` from public re-exports until the config module is actually wired into the compilation pipeline (or remove the config module entirely if it remains unused).
- Consider making `Linker`, `ObjectEmitter`, `Architecture`, `Environment`, `OperatingSystem` `pub(crate)` since they have no external consumers.
- Remove `MirVerifyError`/`VerifyResult` from public exports if no external consumers exist.

---

## 6. Project/Config Boundaries (where config logic lives vs. where it's used)

**Finding:** The `config` module is structurally complete but **completely disconnected** from the compilation pipeline.

### Config module exists but is never invoked

- **`/home/dimitar/saturnite/Saturnite/crates/stnx/src/config.rs`** (lines 27–132) — Defines `SaturnConfig`, `Package`, `DependencySpec` with full TOML parsing (`from_dir`, `from_toml_str`, `from_name`), serde derive, and a `FromStr` impl for `DependencySpec`.
- The module is re-exported in `lib.rs` (line 76) and has comprehensive unit tests (lines 134–222).
- **However, `main.rs` never reads or uses `SaturnConfig`.** The `Build` command (lines 156–377) constructs `TargetConfig` directly from CLI flags or the `--target` triple — no `saturn.toml` is read. The `Check` command (line 379–383) ignores config entirely. The `Run` command (lines 385–410) likewise bypasses config.
- The **only** use of `saturn.toml` is in `init_project()` (lines 566–623), which **writes** a template `saturn.toml` as a raw string literal (lines 599–604) rather than using `SaturnConfig::from_name()` or the config types at all.

### Config writes diverge from config types

- `init_project()` writes `[package]\nname = "..."\nversion = "..."\nedition = "2026"` as a hardcoded string (line 600).
- `SaturnConfig::from_name()` (config.rs:67–77) generates the same template but is **never called** by `init_project()`.

### Config boundary mismatch

- `SaturnConfig::from_dir()` (lines 41–58) accepts a directory and reads `saturn.toml`, with a fallback to synthesize a config from the directory name. This is clearly designed to be called at build time.
- There is **no call site** for `from_dir()` anywhere in `main.rs`, `codegen/`, or `mir/`. The config loading seam exists but has no caller.

### Cross-compilation guard logic duplicates config concerns

- The cross-compilation guard appears in `main.rs` lines 291–310 (Build) and lines 522–537 (build_run_file). It checks the host triple via `codegen::host_triple()` and compares against the requested target. This logic is hardcoded, not driven by any `saturn.toml` `[target]` configuration.

### Recommendation
Wire `SaturnConfig::from_dir()` into the `Build`, `Check`, and `Run` commands to load project configuration. Replace the raw string template in `init_project()` with a call to `SaturnConfig::from_name()`. Add a `[profile]` or `[target]` section to `saturn.toml` for profile/target settings instead of hardcoding defaults.

---

## 7. Module-System Prerequisites (what infrastructure exists or is missing)

**Finding:** There is **no module system infrastructure** whatsoever. The compiler is strictly single-file.

### No AST/parser support for `use`, `import`, `mod`, or `extern`

- A broad search for `mod_decl`, `use_decl`, `ImportDecl`, `extern crate`, `import_mod`, `resolve_mod`, `module_path`, `use_decl` in `/home/dimitar/saturnite/Saturnite/crates/stnx/src` returned **zero matches**.
- There are no `Mod`, `Use`, `Import`, or `ExternItem` variants in `HirStmtKind`, `HirExprKind`, or `MirStmtKind`.
- The parser (referenced in `main.rs` lines 265–271, 488–491, 557–559) has no module-related constructs.

### No path resolution or symbol table for inter-file references

- `SymbolInterner` (lib.rs line 42) manages symbols within a single file but has no concept of file scope or module hierarchy.
- `HirProgram` (referenced at main.rs:271, 490, 559) is constructed per-invocation from a single source file. `lower_program` (lower.rs:30) iterates `hir.functions` — there is no notion of imported functions from other files.
- `lower_call` (lower.rs:486–519) resolves `DefId` to function name via `sigs` table indexed by `def_id.0 as usize` (line 499) — a flat, single-file index. There is no path-based resolution (e.g. `std::io::println`).

### Builtin `println` uses a sentinel DefId, not module resolution

- `PRINTLN_DEF_ID` is `DefId(u32::MAX - 1)` — a magic sentinel constant defined identically in both `lower.rs` (line 28) and `codegen.rs` (line 27).
- There is no module system behind this — `println` is a single hardcoded builtin with no namespace, no import mechanism, no resolution table. Adding a standard library or multi-module support would require building the entire module system from scratch.

### No file-loading or dependency-resolution infrastructure connected to compilation

- `SaturnConfig.dependencies` (config.rs:34) stores `BTreeMap<String, DependencySpec>`, but there is **no code** that reads this field and loads dependency sources.
- `DependencySpec::from_str` (config.rs:127–130) is implemented but **never called**.
- There is no source-file discovery, no crate registry, no build pipeline for dependencies.

### What exists that *could* be a foundation

- `SymbolInterner` provides symbol table infrastructure that a module system would need.
- `HirProgram` already carries `symbols`, `structs`, and `enums` collections that could be extended to `modules` or `use_tree`.
- The `DefId` system has room for module-scoped IDs (it's just a `u32`), but currently uses a flat namespace with a sentinel for builtins.

### What's missing for a module system

A minimal module system would need:
1. **AST/Parser** support for `use`/`import` statements or `mod` declarations.
2. **Path resolution** — a resolver that maps module paths (e.g. `core::io::println`) to `DefId`s across multiple source files.
3. **File loading** — a mechanism to locate and parse source files by module path (e.g. `core/io.stnx`).
4. **Dependency resolution** — use of `SaturnConfig.dependencies` to fetch/load external crates.
5. **Name mangling** — the current `function_name` lookup (mod.rs:337–342) uses `SymbolId` directly; a module system would need qualified names to avoid collisions.
6. **Module-level scoping** in `MirProgram` — currently `MirProgram` is a flat collection of `MirFunction`s with no module hierarchy.

### Recommendation
The module system does not exist. Any module system work would be greenfield. The existing `SymbolInterner` and `DefId` infrastructure provide a starting point but lack any path-based resolution or multi-file loading capability.

---

## Summary Table

| # | Category | Severity | Key Files |
|---|----------|----------|-----------|
| 1 | Duplicated profile logic | Medium | `main.rs` (3 sites) |
| 2 | Duplicated target config logic | Medium | `target.rs`, `mir/codegen.rs`, `main.rs` |
| 3 | Diagnostic leakage in codegen | Medium | `codegen/mod.rs`, `main.rs` |
| 4 | Dead public APIs | Medium | `mir/codegen.rs`, `codegen/emitter.rs`, `target.rs`, `mir/verify.rs` |
| 5 | Stale lib.rs exports | Medium | `lib.rs` |
| 6 | Config module disconnected | High | `config.rs` never called from pipeline |
| 7 | No module system infrastructure | High | Entire `src/` tree — zero module support |
