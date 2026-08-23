# Phase 9A: End-to-End Module Test Design

## File to Create

```
crates/stnx/tests/test_end_to_end_modules.rs
```

A new integration test file. Add `mod common;` at the top (matching the pattern used by `test_project_loading.rs` and `test_multi_module_codegen.rs`).

---

## Overview

This test file drives the full production compilation seam — `Project::discover` + `Project::load_from` + `analyze_and_lower_with_graph` + `lower_program` + `mir.verify` + `optimize` + `compile_from_mir_ext` — end-to-end for multi-module projects on disk, then executes the resulting binary to verify cross-module function calls work at runtime.

It reuses helpers from `tests/common/mod.rs` (notably `Artifact` for running executables, though the helpers there use `analyze_and_lower` without a graph — so the test defines its own helpers that accept a `ModuleGraph`).

---

## Test File Structure

### Imports

```rust
mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use stnx::ast::Program;
use stnx::hir::{HirFunction, HirProgram};
use stnx::mir::lower::lower_program;
use stnx::mir::opt::optimize;
use stnx::mir::{MirFunction, MirProgram, MirTerminator};
use stnx::module::{Module, ModuleGraph, ModuleId, Project};
use stnx::semantic::analyze_and_lower_with_graph;
use stnx::target::{OutputKind, TargetConfig};
use stnx::DefId;
use tempfile::TempDir;
```

### Helper Functions

#### `write_file(dir: &Path, rel: &str, contents: &str) -> PathBuf`

Write a file at `dir/<rel>` with the given contents, creating parent directories as needed. Mirrors the helper in `test_project_loading.rs`.

#### `write_saturn_toml(dir: &Path, name: &str)`

Write a minimal `saturn.toml` into `dir` with package name, version `"0.1.0"`, edition `"2026"`, and an empty `[dependencies]` section. Format:

```toml
[package]
name = "<name>"
version = "0.1.0"
edition = "2026"

[dependencies]
```

#### `fn compile_program_to_exe_with_graph(program: &Program, graph: &ModuleGraph, exe_path: &Path)`

Drive the full production pipeline starting from a `Program` AST and a `ModuleGraph`:

1. `analyze_and_lower_with_graph(program, graph)` — AST → HIR (semantic analysis + `use`/`mod` resolution via `resolve_modules`)
2. `lower_program(&hir)` — HIR → MIR
3. `mir.verify()` — MIR CFG sanity checks
4. `optimize(&mut mir)` — MIR-level optimization
5. `compile_from_mir_ext(&mir, exe_path.to_str(), config, false)` — MIR → LLVM → object → link

```rust
fn compile_program_to_exe_with_graph(
    program: &Program,
    graph: &ModuleGraph,
    exe_path: &Path,
)
```

Internally constructs `TargetConfig::host()`, sets `OutputKind::Exe`, and panics on any stage failure (using `.expect(...)`).

#### `fn assert_cross_module_call(
    mir: &MirProgram,
    caller_name: &str,
    callee_def_id: DefId,
    expected_ret_type: HirType,
)`

Walk the MIR for `caller_name`, collect all `MirTerminator::Call` terminators (using the `collect_calls` pattern from `test_multi_module_codegen.rs`), and assert that at least one `Call` references `callee_def_id`, and that the destination local is typed `expected_ret_type`.

```rust
fn assert_cross_module_call(
    mir: &MirProgram,
    caller_name: &str,
    callee_def_id: DefId,
    expected_ret_type: HirType,
)
```

#### `fn collect_calls(func: &MirFunction) -> Vec<(DefId, stnx::mir::LocalId)>`

Collect all `Call` terminators across all basic blocks of a MIR function. Returns `(func: DefId, destination: LocalId)` for each call. (Copied from `test_multi_module_codegen.rs` — the same shape.)

```rust
fn collect_calls(func: &MirFunction) -> Vec<(DefId, stnx::mir::LocalId)>
```

#### `fn mir_function<'a>(prog: &'a MirProgram, name: &str) -> &'a MirFunction`

Find a MIR function by symbol name. (Copied from `test_multi_module_codegen.rs`.)

```rust
fn mir_function<'a>(prog: &'a MirProgram, name: &str) -> &'a MirFunction
```

#### `fn hir_function<'a>(hir: &'a HirProgram, name: &str) -> &'a HirFunction`

Find a HIR function by interned name string. (Copied from `test_multi_module_codegen.rs`.)

```rust
fn hir_function<'a>(hir: &'a HirProgram, name: &str) -> &'a HirFunction
```

#### `fn find_module_by_name(hir: &HirProgram, name: &str) -> ModuleId`

Find a module in the HIR by its path's last segment name. (Copied from `test_multi_module_codegen.rs`.)

```rust
fn find_module_by_name(hir: &HirProgram, name: &str) -> ModuleId
```

#### `fn run_exe(exe_path: &Path) -> (i32, String)`

Execute the compiled binary, return `(exit_code, stdout)`.

```rust
fn run_exe(exe_path: &Path) -> (i32, String)
```

---

## Test Case 1: Cross-Module Call via `use` (root → child)

### Scenario

The root module (`src/main.stnx`) declares `mod math`, imports `use math::compute`, and calls `compute()` from `main`. The child module (`src/math.stnx`) defines `fn compute() -> i64` that calls `println(77)` and returns `77`.

### Source Layout

```
<tmp>/saturn.toml
<tmp>/src/main.stnx      — mod math, use math::compute, fn main() -> i64 { compute() return 0 }
<tmp>/src/math.stnx      — fn compute() -> i64 { println(77) return 77 }
```

### Source Contents

**main.stnx:**
```
mod math
use math::compute
fn main() -> i64 {
    compute()
    println(0)
    return 0
}
```

**math.stnx:**
```
fn compute() -> i64 {
    println(77)
    return 77
}
```

### Steps

1. Create `TempDir`, write `saturn.toml` + `src/main.stnx` + `src/math.stnx`.
2. `Project::discover(&main_file)` — locate project root via `saturn.toml`.
3. `project.load_from(&main_file)` — get root `Program` AST + populate `project.graph`.
4. Assert: `project.graph.len() == 2` (root + `math`).
5. Assert: `project.graph.find_module(path for "math")` returns `Some`.
6. Assert: root AST `items` contains a `ModDecl` for `math`.
7. Call `compile_program_to_exe_with_graph(&program, &project.graph, &exe_path)`.
8. Run the exe: `run_exe(&exe_path)`.
9. Assert: exit code == 0.
10. Assert: stdout is `"77\n"` (the child module's `println`).
11. (Optional static check) Lower to MIR, find `main`'s MIR, collect calls, assert a `Call` with `helper`'s `DefId` exists and destination local is `i64`.

### Key Invariants to Assert

- `project.graph.len() == 2`
- The graph's `modules[0]` is the root (`is_root()` == true); `modules[1]` has `path.name == Some("math")`.
- `hir.modules.len() == 2` and the child module's `id` matches `ModuleId(1)`.
- `hir.module_paths` maps `compute`'s `DefId` to the child `ModuleId`.
- The `use math::compute` import is registered in the root module scope's `imports`.
- MIR `main` contains a `Call` terminator whose `func` field is the `DefId` of `compute` and whose destination local is typed `I64`.
- The executable prints `77` at runtime and exits 0.

---

## Test Case 2: Mutual Cross-Module Calls (root ↔ child)

### Scenario

The root module declares `mod helper`, imports `use helper::double`, and calls `double(21)`. The child module defines `fn double(x: i64) -> i64` that calls `println(x)` then calls back into a root function via `use crate::main` (or a root function `add_one`). 

For the callback, the child module uses `crate::add_one` to call back into the root. The root's `add_one` function returns `x + 1`. So `double(21)` prints `21`, calls `add_one(21)` → 22, and the root's `main` calls `double(21)`, gets 22, prints `22`, returns 0.

### Source Layout

```
<tmp>/saturn.toml
<tmp>/src/main.stnx      — mod helper, use helper::double, fn main, fn add_one
<tmp>/src/helper.stnx    — use crate::add_one, fn double
```

**main.stnx:**
```
mod helper
use helper::double
fn add_one(x: i64) -> i64 {
    return x + 1
}
fn main() -> i64 {
    let result = double(21)
    println(result)
    return 0
}
```

**helper.stnx:**
```
use crate::add_one
fn double(x: i64) -> i64 {
    println(x)
    return add_one(x)
}
```

### Steps

1. Create `TempDir`, write `saturn.toml` + `src/main.stnx` + `src/helper.stnx`.
2. `Project::discover(&main_file)` → `project.load_from(&main_file)`.
3. Assert: `project.graph.len() == 2`.
4. Assert: child module path name is `"helper"`.
5. Call `compile_program_to_exe_with_graph(&program, &project.graph, &exe_path)`.
6. Run the exe: `run_exe(&exe_path)`.
7. Assert: exit code == 0.
8. Assert: stdout is `"21\n22\n"` (first `println(21)` from `double`, then `println(22)` from `main` after `add_one(21) = 22`).

### Key Invariants to Assert

- `project.graph.len() == 2`, modules are root + `helper`.
- `hir.modules` contains both modules with correct `ModuleId`s.
- `hir.def_table` has entries for both `double` (in `helper` module) and `add_one` (in root module).
- The root module scope has `add_one` as an item and `double` as an import.
- The child module scope has `double` as an item and `add_one` as an import (via `use crate::add_one`).
- MIR `main` contains a `Call` to `double`'s `DefId`; MIR `double` contains a `Call` to `add_one`'s `DefId`.
- Runtime: stdout matches `"21\n22\n"`, exit code 0.

### Note on `use crate::add_one`

The child module uses the `crate::` prefix to reference a root-level function. Based on the `resolve_modules` implementation (lines 1629–1786 of `hir/lower.rs`), the first path segment `crate` is handled as a special case: the root module has an empty `ModulePath` with `name()` returning `None`, but `resolve_modules` checks `m.path.name(&hir.symbols) == Some(first_name) && !m.is_root()`. The `crate::` prefix may need to be handled specially by the path resolution logic. If `crate::add_one` does not resolve (because `crate` is not a module name in the graph), the test should use the simpler form `use add_one` (single-segment, parent-chain lookup via `lookup_with_parent`). The Implementer should verify which form actually works by checking how `resolve_modules` handles `crate` as the first segment.

**Fallback for Test Case 2** (if `crate::` prefix is not supported): The child module declares `use add_one` (single-segment import). This triggers the single-segment path in `resolve_modules` (lines 1754–1774), which looks up `add_one` via `lookup_with_parent` walking the parent chain from the child module scope to the root module scope. This is the minimal viable form for mutual calls.

---

## Pipeline Diagram

```
temp dir (saturn.toml + src/main.stnx + src/child.stnx)
  │
  │  Project::discover(&main_file)
  ▼
Project { config, root, source_root, graph: ModuleGraph::new() }
  │
  │  project.load_from(&main_file)
  │    → ModuleGraph::discover_modules(entry)
  │    → recursively lex/parse mod declarations
  │    → returns root Program AST (root.ast)
  ▼
Program (root AST, contains mod_decls in items)
  │
  │  analyze_and_lower_with_graph(&program, &graph)
  │    → lower_with_graph: clones graph.symbol_interner, iterates
  │      every module in graph, builds function_sigs, lowers function
  │      bodies with ModuleId tagging, builds module_scopes,
  │      use_decls, mod_decls
  │    → resolve_modules: walks use_decls, resolves cross-module
  │      paths through module scopes, populates imports
  ▼
HirProgram { functions, modules, module_scopes, use_decls,
              mod_decls, def_table, module_paths, ... }
  │
  │  lower_program(&hir)
  │    → builds sigs: HashMap<DefId, (Vec<HirType>, HirType)>
  │    → for each HirFunction: MirLower::lower_function
  │      → lower_expr for Call: MIR Terminator::Call { func: DefId,
  │        args, destination: LocalId, next: BlockId }
  ▼
MirProgram { functions: Vec<MirFunction>, symbols, structs, enums }
  │
  │  mir.verify()  → VerifyResult
  │  optimize(&mut mir)
  │  compile_from_mir_ext(&mir, exe_path, config, false)
  ▼
Executable on disk
  │
  │  Command::new(&exe_path).output()
  ▼
(stdout, exit_code) — assert against expected output
```

---

## Key Type Reference

| Type | Location | Used In |
|------|----------|---------|
| `Project` | `stnx::module::Project` | Discovery + loading |
| `ModuleGraph` | `stnx::module::ModuleGraph` | `graph.modules`, `graph.len()`, `graph.find_module()` |
| `ModuleId` | `stnx::module::ModuleId` | `ModuleId::ROOT`, `ModuleId(u32)` |
| `ModulePath` | `stnx::module::ModulePath` | `from_segments()`, `name()` |
| `Module` | `stnx::module::Module` | `module.id`, `module.path`, `module.is_root()` |
| `Program` | `stnx::ast::Program` | Root AST from `load_from` |
| `HirProgram` | `stnx::hir::HirProgram` | `hir.modules`, `hir.functions`, `hir.module_scopes`, `hir.def_table`, `hir.module_paths`, `hir.use_decls`, `hir.mod_decls` |
| `HirFunction` | `stnx::hir::HirFunction` | `func.def_id`, `func.name`, `func.module` |
| `MirProgram` | `stnx::mir::MirProgram` | `mir.functions`, `mir.symbols` |
| `MirFunction` | `stnx::mir::MirFunction` | `func.blocks`, `func.locals`, `func.def_id` |
| `MirTerminator` | `stnx::mir::MirTerminator` | `MirTerminator::Call { func, args, destination, next }` |
| `TargetConfig` | `stnx::target::TargetConfig` | `TargetConfig::host()`, `set_output_kind()` |
| `OutputKind` | `stnx::target::OutputKind` | `OutputKind::Exe` |
| `DefId` | `stnx::DefId` (re-export) | `DefId(u32)` |
| `HirType` | `stnx::hir::HirType` | `HirType::I64`, `HirType::F64`, `HirType::Bool` |
| `LocalId` | `stnx::mir::LocalId` | `LocalId(u32)` |

---

## Invariants Summary (what every test must assert)

1. **Module discovery**: `graph.len() == 2` for two-module projects; root is `modules[0]`, child is `modules[1]` with correct path name.
2. **HIR module metadata**: `hir.modules.len() >= 2`; `hir.module_paths` maps child function `DefId` → child `ModuleId`; `hir.module_scopes[child_id].items` contains the child's functions; `hir.module_scopes[ROOT].imports` contains the `use`-imported names.
3. **DefId uniqueness**: `main`'s `DefId` != `compute`'s/`double`'s `DefId` != `add_one`'s `DefId`.
4. **MIR call integrity**: `MirFunction` for caller contains a `MirTerminator::Call { func: <callee_def_id>, ... }`; destination `LocalId` maps to a `MirLocal` with `ty` == callee's return type.
5. **IR presence**: `generate_ir_from_mir` output contains `call i64 @<callee_name>` (or appropriate return type prefix).
6. **Runtime executable**: compiled binary exits 0; stdout matches expected `println` output.

---

## Source Syntax Notes (from existing tests)

- Statement separator: **newline or space** (no semicolons required). Statements like `let x = 42` and `return 0` are separated by newlines.
- `println(expr)` is a statement/expression that prints an `i64` (or `bool`, `enum`).
- `return expr` exits the function.
- Function syntax: `fn name(params) -> return_type { body }` where params are `(name: type)`.
- `mod child` declares a child module (resolved via `src/child.stnx`).
- `use child::func` imports a name from a child module.
- `use crate::func` may or may not be supported (see fallback note above).
