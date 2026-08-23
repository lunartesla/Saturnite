# Saturnite 0.4 Architecture

> **Status:** Current architecture (post MIR-migration)
>
> This document describes the MIR-based compiler architecture introduced in
> Saturnite 0.4, where MIR is the sole production codegen path.

## 1. Compilation pipeline

```
Saturnite source
   │
   ▼
┌──────────────────────────────────────────────────────────┐
│  Phase 1  │  Lexer             (src/lexer/mod.rs)         │
│           │  logos-based tokenization                    │
│           │  tokens carry byte spans                     │
├──────────────────────────────────────────────────────────┤
│  Phase 2  │  Parser           (src/parser/mod.rs)         │
│           │  chumsky 0.13                              │
│           │  produces spanned AST                       │
├──────────────────────────────────────────────────────────┤
│  Phase 3  │  Semantic analysis   (src/semantic.rs)     │
│           │  AST → HIR: type checking,                  │
│           │  mutability enforcement, scope resolution   │
├──────────────────────────────────────────────────────────┤
│  Phase 4  │  MIR lowering       (src/mir/lower.rs)      │
│           │  HIR → MIR: builds typed CFG                │
│           │  with LocalId, BlockId, BasicBlock,         │
│           │  Rvalue, Terminator                         │
├──────────────────────────────────────────────────────────┤
│  Phase 5  │  MIR verification    (src/mir/verify.rs)    │
│           │  checks CFG integrity:                      │
│           │  unreachable blocks, type consistency       │
├──────────────────────────────────────────────────────────┤
│  Phase 6  │  MIR optimization    (src/mir/opt.rs)       │
│           │  constant folding on              │
│           │  arithmetic / comparison / logical ops      │
├──────────────────────────────────────────────────────────┤
│  Phase 7  │  MIR → LLVM IR    (src/mir/codegen.rs)      │
│           │  the sole production codegen path           │
│           │  translates each MIR construct to           │
│           │  LLVM IR via inkwell                        │
├──────────────────────────────────────────────────────────┤
│  Phase 8  │  Object emission  (src/codegen/emitter.rs)  │
│           │  TargetMachine writes .o / .ll              │
├──────────────────────────────────────────────────────────┤
│  Phase 9  │  Linking         (src/codegen/linker.rs)    │
│           │  system linker (cc / clang / link.exe)      │
└──────────────────────────────────────────────────────────┘
   │
   ▼
Executable
```

**Notes on phase numbering:** Phase 5 is MIR verification (not optimization), and Phase 6 is MIR optimization (not further lowering). Optimization is implemented as constant folding in `src/mir/opt.rs` and is invoked after verification.

## 2. Module layout

| Layer              | Module                         | Description                              |
|--------------------|--------------------------------|------------------------------------------|
| Lexing             | `src/lexer/mod.rs`             | logos tokenizer with byte spans          |
| Lexer tokens       | `src/lexer/token.rs`           | TokenKind enum and conversions           |
| Parsing            | `src/parser/mod.rs`            | chumsky 0.13 parser → AST                |
| AST                | `src/ast.rs`                   | spanned AST node definitions             |
| Semantic analysis  | `src/semantic.rs`              | AST → HIR lowering entry points          |
| HIR                | `src/hir/`                     | typed, span-bearing IR                   |
| HIR lowering       | `src/hir/lower.rs`             | AST → HIR transformation                |
| HIR core           | `src/hir/function.rs`          | HirProgram, HirFunction, module fields  |
| HIR symbols        | `src/hir/symbol.rs`            | SymbolId, DefId, SymbolInterner, DefTable|
| HIR types          | `src/hir/types.rs`             | HirType enum                            |
| HIR expr/stmt      | `src/hir/expr.rs`, `stmt.rs`   | HirExpr, HirStmt definitions            |
| MIR                | `src/mir/`                     | typed CFG (lower, verify, optimize)      |
| MIR types          | `src/mir/mod.rs`               | MirProgram, MirFunction, MirRvalue, etc.|
| MIR lowering       | `src/mir/lower.rs`             | HIR → MIR CFG construction               |
| MIR verification   | `src/mir/verify.rs`            | CFG structural integrity checks          |
| MIR optimization   | `src/mir/opt.rs`               | constant folding pass                    |
| Codegen (MIR→LLVM) | `src/mir/codegen.rs`           | **sole** codegen path                    |
| Object emission    | `src/codegen/emitter.rs`       | writes .o / .ll from an LLVM module      |
| Linking            | `src/codegen/linker.rs`        | invokes the system linker                |
| Codegen facade     | `src/codegen/mod.rs`           | ObjectEmitter, Linker, host_triple, check_linker |
| Target config      | `src/target.rs`                | triple validation, opt levels, debug info, Profile enum |
| Config             | `src/config.rs`                | `saturn.toml` parsing (SaturnConfig)    |
| Module system      | `src/module.rs`                | ModuleGraph, Module, Project, discovery  |
| Errors             | `src/error.rs`                 | thiserror + miette Diagnostic            |
| CLI                | `src/main.rs`                  | build / check / run / doctor / init      |
| Public API         | `src/lib.rs`                   | crate re-exports                          |
| Runtime            | `crates/stnx/runtime/println_i64.c`| C runtime compiled via `build.rs` + `cc` |
| Build script       | `build.rs`                     | compiles runtime C, links into crate     |

## 3. MIR overview

The MIR (Mid-level IR) is the compiler's single codegen seam. It owns a typed control-flow graph:

```
MirProgram
  ├─ symbols: SymbolInterner
  ├─ functions: Vec<MirFunction>
  ├─ structs: Vec<StructDef>
  └─ enums: Vec<EnumDef>

MirFunction
  ├─ def_id: DefId
  ├─ name: SymbolId
  ├─ params: Vec<(SymbolId, MirType)>
  ├─ return_type: MirType
  ├─ locals: Vec<MirLocal>
  ├─ param_locals: Vec<LocalId>
  ├─ blocks: Vec<MirBasicBlock>
  └─ start_block: BlockId

MirBasicBlock
  ├─ id: BlockId
  ├─ name: String
  ├─ stmts: Vec<MirStmt>
  └─ terminator: MirTerminator

MirStmt
  └─ kind: MirStmtKind
      ├─ LocalDecl { local: LocalId, ty: MirType, mutable: bool }
      └─ Assign { local: LocalId, rvalue: MirRvalue }

MirTerminator
  ├─ Goto { target: BlockId }
  ├─ SwitchInt { scrutinee: MirOperand, ty: MirType, branches: Vec<(u64, BlockId)>, else_target: BlockId }
  ├─ Call { func: DefId, args: Vec<MirOperand>, destination: LocalId, next: BlockId }
  ├─ Return(Option<MirOperand>)
  └─ Unreachable

MirRvalue
  ├─ Use(MirOperand)
  ├─ Binary { op: MirBinOp, lhs: MirOperand, rhs: MirOperand }
  ├─ Unary { op: MirUnOp, operand: MirOperand }
  ├─ StructLit { struct_def: SymbolId, fields: Vec<(SymbolId, MirOperand)> }
  ├─ FieldAccess { local: LocalId, field: SymbolId }
  ├─ EnumCtor { enum_def: SymbolId, variant: SymbolId }
  └─ StrLit(SymbolId)

MirOperand
  ├─ Const(MirConst)
  └─ Local(LocalId)

MirConst
  ├─ I64(i64)
  ├─ F64(f64)
  └─ Bool(bool)

MirType  (= HirType)
  ├─ I64, F64, Bool, Str, Unit
  ├─ Struct(SymbolId)
  └─ Enum(SymbolId)
```

### Key design decisions

- **Stack-alloced locals:** Every `MirLocal` is lowered to an `alloca` in the entry block, preserving mutable-variable semantics across basic-block boundaries (including loops).
- **SwitchInt type selection:** The `SwitchInt` terminator includes a `ty: MirType` field so the codegen selects the correct LLVM integer width (e.g. `i1` for `Bool`), avoiding type-mismatch segfaults.
- **Shadowing safety:** `lower_stmt` inserts `LocalDecl` before evaluating the initializer rvalue, so `let x = x + 1` reads the *previous* value correctly.
- **Builtins:** `println` is a builtin (`println_i64`) declared at module level with a sentinel `DefId` (`u32::MAX - 1`); user functions with that sentinel are skipped during declaration.
- **HashMap signature lookup:** MIR lowering builds `sigs: HashMap<DefId, (Vec<HirType>, HirType)>` (not parallel `Vec`s). `lower_call` performs `sigs.get(&def_id)` (HashMap lookup), falling back to `result_ty` if not found.
- **Module identity erased at MIR:** `MirProgram` does not carry module fields. Module identity is resolved during HIR lowering; once lowered to MIR, only `DefId` and `SymbolId` survive.

## 4. HIR overview

The HIR (High-level IR) is the compiler's single authoritative semantic representation, produced by the AST→HIR lowering pass in `src/semantic.rs` → `src/hir/lower.rs`.

```
HirProgram
  ├─ functions: Vec<HirFunction>
  ├─ structs: Vec<StructDef>
  ├─ enums: Vec<EnumDef>
  ├─ symbols: SymbolInterner
  ├─ modules: Vec<Module>               // all discovered modules
  ├─ root_module: ModuleId              // ModuleId::ROOT = 0
  ├─ module_paths: HashMap<DefId, ModuleId>
  ├─ def_table: DefTable
  ├─ module_scopes: Vec<ModuleScope>
  ├─ use_decls: Vec<HirUseDecl>
  └─ mod_decls: Vec<HirModDecl>

HirFunction
  ├─ def_id: DefId
  ├─ name: SymbolId
  ├─ params: Vec<(SymbolId, HirType)>
  ├─ return_type: HirType
  ├─ body: Vec<HirStmt>
  ├─ span: SourceSpan
  ├─ module: ModuleId
  └─ visibility: Visibility

StructDef / EnumDef
  ├─ def_id: DefId
  ├─ name: SymbolId
  ├─ (fields / variants)
  ├─ span: SourceSpan
  ├─ module: ModuleId
  └─ visibility: Visibility
```

### Identifier spaces

Three distinct `u32`-based identifier types:

| Type      | Purpose                        | Source                              |
|-----------|--------------------------------|-------------------------------------|
| `SymbolId`| Interned string                | `src/hir/symbol.rs`                 |
| `DefId`   | Globally-unique definition index| `src/hir/symbol.rs`                 |
| `ModuleId`| Module identity (separate space)| `src/module.rs`                     |

### Module-aware lowering paths

- **`lower_program`** (`hir/lower.rs`): Single-file path. All items tagged to `ModuleId::ROOT`.
- **`lower_program_with_graph`** (`hir/lower.rs`): Multi-module entry point. Iterates `graph.modules`, clones the graph's `SymbolInterner`, assigns `ModuleId`s from the graph, resolves `mod` declarations to child module IDs via `child_module_lookup`, and populates per-module `ModuleScope`s with correct parent chains.
- **`resolve_modules`** (`hir/lower.rs`): Resolves `use` declarations across modules by walking parent chains via `ModuleScope::lookup_with_parent`.
- **`analyze_and_lower`** (`semantic.rs`): Single-file entry point — calls `lower()`.
- **`analyze_and_lower_with_graph`** (`semantic.rs`): Multi-module entry point — calls `lower_with_graph()` + `resolve_modules()`.

## 5. Module system

The module system is fully integrated into the compilation pipeline. See `docs/audit_notes/module_language_design.md` for the full design audit.

### Key components

| Component       | File                              | Description                              |
|-----------------|-----------------------------------|------------------------------------------|
| `ModuleId`      | `src/module.rs`                   | `u32` identity, separate from `DefId`    |
| `ModulePath`    | `src/module.rs`                   | `Vec<SymbolId>` interned path segments   |
| `Module`        | `src/module.rs`                   | id, path, file_path, ast, parent, mod_decls |
| `ModuleScope`   | `src/module.rs`                   | items/imports HashMap + parent chain     |
| `ModuleGraph`   | `src/module.rs`                   | All modules, root, interner, imports     |
| `Project`       | `src/module.rs`                   | saturn.toml discovery, load, load_from   |
| `SaturnConfig`  | `src/config.rs`                   | TOML parsing for `[package]`/`[dependencies]` |

### Discovery

`ModuleGraph::discover_modules(root_file)` (`src/module.rs`):

1. Creates the root module with an empty `ModulePath`.
2. Reads and parses the root file.
3. **AST-based extraction** (`extract_mod_declarations_from_ast`) is the primary path — it scans `ast::ItemKind::ModDecl` items.
4. Text-based fallback (`extract_mod_declarations`) is used if AST parsing fails.
5. For each child mod name, resolves the file via `resolve_module_file` (`<dir>/<name>.stnx` first, then `<dir>/<name>/mod.stnx`), then recursively discovers.
6. `add_module` assigns `ModuleId` sequentially and indexes by path.

### CLI project workflow

- `Project::discover(start)` walks upward for `saturn.toml`, parses config via `SaturnConfig::from_dir`, sets `source_root = root.join("src")`.
- `Project::load()` discovers modules from `<source_root>/main.stnx` and returns the root module's AST.
- `Project::load_from(file)` does the same from an explicit file path.
- The CLI (`src/main.rs`) calls `Project::discover()` in all three command paths (Build, Check, Run). When no input file is given, it defaults to `src/main.stnx` from the discovered project.

### Gap: CLI uses single-file semantic path

`main.rs:255` calls `analyze_and_lower(&program)` (single-file). The multi-module entry point `analyze_and_lower_with_graph(&program, &project.graph)` is implemented in `semantic.rs` but is not yet invoked by the CLI. The graph is used for file discovery via `Project::discover`, but not passed to HIR lowering.

## 6. Codegen seam

```
main.rs (Build / Run command)
  │
  ├─ Project::discover(&cwd) → Project { config, source_root, graph }
  ├─ project.load() / project.load_from(entry) → AST (ast::Program)
  ├─ stnx::semantic::analyze_and_lower(&program) → HIR (hir::HirProgram)
  │
  ├─ stnx::mir::lower::lower_program(&hir) → MIR (mir::MirProgram)
  ├─ mir.verify() → Result<(), Vec<MirVerifyError>>
  ├─ stnx::mir::optimize(&mut mir)
  │
  ├─ match output_kind:
  │     OutputKind::Ir  → generate_ir_from_mir(&mir) → write .ll text file
  │     OutputKind::Obj → compile_from_mir_ext(&mir, path, config, save_temps)
  │     OutputKind::Exe → compile_from_mir_ext(&mir, path, config, save_temps)
  │
  └─ compile_from_mir_ext dispatches to ObjectEmitter + Linker
```

Entry points (all in `src/mir/codegen.rs`):

| Function               | Purpose                              |
|------------------------|--------------------------------------|
| `generate_ir_from_mir` | Emit LLVM IR text from a `MirProgram`|
| `compile_from_mir`     | Compile a `MirProgram` to an artifact|
| `compile_from_mir_ext` | Same, with `save_temps` flag          |

These functions delegate object emission and linking to the shared `codegen::ObjectEmitter` and `codegen::Linker` infrastructure.

## 7. Code generation infrastructure (shared)

The `codegen` module (`src/codegen/`) provides the object-emission and linking seams that the MIR backend delegates to:

- **`ObjectEmitter`** (`src/codegen/emitter.rs`): Wraps an LLVM module and a `TargetMachine` to emit `.o` object files or `.ll` IR text files.
- **`Linker`** (`src/codegen/linker.rs`): Invokes the system linker (`cc` on Linux, `clang` on macOS, `link.exe`/`gcc` on Windows) to produce a final executable from an object file.
- **`check_linker`** / **`host_triple`**: Utility functions in `src/codegen/mod.rs` used by `main.rs` for cross-compilation guards and diagnostics.

These are **not** tied to any particular IR (HIR or MIR) — they operate on generic LLVM modules.

### Target configuration

The `Profile` enum (`src/target.rs`) centralizes build profiles:

| Variant | Opt level         | Debug info | `as_str()`  |
|---------|--------------------|------------|-------------|
| `Debug` | `OptimizationLevel::None` | `DebugInfo::Yes` | `"debug"` |
| `Release`| `OptimizationLevel::Aggressive` | `DebugInfo::No` | `"release"` |

Methods: `opt_level()`, `debug_info()`, `is_release()`, `as_str()`. `Profile` is `#[derive(Default)]`, defaulting to `Debug`.

`TargetConfig` (`src/target.rs`) holds triple, architecture, OS, environment, opt level, debug info, output kind, CPU, and features. Key methods:
- `host()` — build config for the host platform
- `from_triple(triple_str)` — build config from a target triple string
- `apply_profile(profile)` — apply a `Profile` (optimization + debug info defaults)
- `set_opt_level`, `set_debug_info`, `set_output_kind` — explicit overrides

**Cross-compilation guard:** The Saturnite runtime (`println_i64.c`) is compiled via `build.rs` from host-platform C source (`cc` crate). The CLI rejects non-host target triples with a clear error message. This guard is enforced in all command paths (Build, Check, Run).

## 8. Runtime

The Saturnite runtime is a minimal C library providing `println_i64`:

```
crates/stnx/runtime/println_i64.c  →  compiled at build time via build.rs + cc crate
                                    →  linked into every Saturnite executable
```

The runtime is host-only: cross-compilation to a non-host target is rejected at the `Build` command level with a clear error message.

## 9. CLI

```
saturnite build <FILE> [OPTIONS]    # Build to executable / object / IR
saturnite check <FILE>              # Type & semantic check (no codegen)
saturnite run <FILE>                # Build then execute
saturnite doctor                    # Print environment diagnostics
saturnite init [NAME]               # Scaffold a new project
```

All `build`/`run` paths go through the full MIR pipeline: `parse → semantic → lower → verify → optimize → codegen → emit → link`.

### Build command

The Build command (`src/main.rs`):

1. **Profile determination:** `Profile::Debug` / `Profile::Release` / `Profile::default()`.
2. **Entry point resolution:** If `input` is given, use it directly. If not, call `Project::discover(&cwd)`, then use `<source_root>/main.stnx` as the entry and extract `package_name` from `project.config.package.name`.
3. **Target configuration:** `TargetConfig::host()` or `from_triple(triple)`, then `config.apply_profile(profile)` with optional `--opt-level` overrides.
4. **Project + module discovery:** `Project::discover(&entry_path)`, then `project.load_from(entry)` or `project.load()`.
5. **Semantic analysis:** `analyze_and_lower(&program)` (single-file path).
6. **MIR lowering + verify + optimize.**
7. **Codegen:** `generate_ir_from_mir` for IR, or `compile_from_mir_ext` for object/exe.

### Check and Run commands

`check_file` and `build_run_file` both call `Project::discover` and `project.load_from`, then run the full pipeline (or semantic analysis only for `check`).

### Note on multi-module CLI gap

The CLI currently calls `analyze_and_lower(&program)` (single-file). The graph-aware entry point `analyze_and_lower_with_graph(&program, &project.graph)` exists in `semantic.rs` but is not yet wired into the CLI `Build`/`Check`/`Run` paths. The module graph is discovered but not passed to HIR lowering.

## 10. Testing

**Total: 364 tests, all passing**, across 18 test binaries:

| Test binary                    | Tests | What it covers                                  |
|-------------------------------|-------|-------------------------------------------------|
| Library unit tests            | 115   | Internal module tests (lexer, parser, HIR, MIR) |
| `codegen.rs`                  | 24    | MIR codegen: IR output, exe, object, etc.       |
| `diagnostics.rs`              | 6     | Error span reporting                            |
| `lexer.rs`                    | 17    | Tokenization accuracy                           |
| `mir_lower.rs`                | 27    | HIR → MIR lowering                              |
| `native_compilation.rs`       | 63    | Full build+run of native executables            |
| `semantic.rs`                 | 35    | Type checking, mutability, scope                |
| `test_doctor.rs`              | 9     | Doctor command diagnostics                      |
| `test_end_to_end_modules.rs`  | 2     | End-to-end multi-module builds                  |
| `test_full_compile.rs`        | 1     | End-to-end build                                |
| `test_ir_only.rs`             | 1     | IR-only generation                              |
| `test_module_graph.rs`        | 41    | Project discovery, module file discovery, nested modules |
| `test_module_resolution.rs`   | 3     | `resolve_modules` cross-module use resolution   |
| `test_multi_module_codegen.rs`| 3     | Multi-module codegen (MIR → LLVM → exe)         |
| `test_native_only.rs`         | 1     | Native compile+run                              |
| `test_project_loading.rs`     | 3     | Project discovery, config parsing, load         |
| `test_target_config.rs`       | 12    | Profile mapping, TargetConfig setters          |
| `test_target_machine.rs`      | 1     | Raw inkwell TargetMachine                       |
