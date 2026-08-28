# Saturnite Actual Architecture — Phase 1 Forensic Report

**Date:** 2026-08-27
**Author:** Phase 1 forensics agent (a30b038d7c088b1a5)
**Scope:** `crates/stnx/src/**` — every source file inspected line-by-line
**Method:** Direct source code reading (no external documentation relied upon)

---

## 1. Workspace & Build System

### 1.1 Workspace configuration

**Files inspected:**
- `Cargo.toml` (workspace root, 12 lines)
- `crates/stnx/Cargo.toml` (24 lines)
- `crates/stnx/build.rs` (55 lines)
- `crates/stnx/runtime/println_i64.c` (7 lines)
- `cargo/Cargo.lock` (license metadata for 122 packages)

**Workspace root (`Cargo.toml`):**

```toml
[workspace]
members = ["crates/stnx"]
resolver = "3"

[workspace.dependencies]
# 11 dependencies listed, but `toml` is MISSING from this list
```

**Critical finding (M1):** `toml 0.8` is declared only in the crate-level `Cargo.toml`, NOT in `[workspace.dependencies]`. This is a real inconsistency — `toml` should be centralized alongside the other 10 workspace deps.

**Crate (`crates/stnx/Cargo.toml`):**

| Dependency | Version | Feature flags | Purpose |
|---|---|---|---|
| `logos` | 0.16 | — | Lexer (LexicalToken derive) |
| `chumsky` | 0.13 | `memoization` | Parser |
| `inkwell` | 0.9 | `llvm21-1-prefer-dynamic` | LLVM IR generation, LLVM 21 |
| `miette` | 7 | `fancy` | Diagnostic rendering |
| `thiserror` | 2 | — | Error derive macros |
| `clap` | 4 | `derive` | CLI |
| `serde` | 1 | `derive` | Serialization |
| `serde_json` | 1 | — | JSON output |
| `toml` | 0.8 | — | Saturn.toml parsing (NOT in workspace deps) |
| `anyhow` | 1 | — | Error handling in main.rs |
| `which` | 5 | — | Linker PATH lookup |
| `cc` | 1 | (build) | Compile C runtime |
| `tempfile` | 3 | (dev) | Test temp files |

**Build script (`build.rs`):** Compiles `runtime/println_i64.c` into `libsaturnite_runtime.a` using the `cc` crate, targeting the **host** platform only. The archive is linked during the `Exe` output path via `Linker::link()`.

**Runtime (`runtime/println_i64.c`, 7 lines):**

```c
long long println_i64(long long value) {
    printf("%lld\n", value);
    return value;
}
```

This is the single C runtime builtin. It is called from MIR codegen when `PRINTLN_DEF_ID = DefId(u32::MAX - 1)` is encountered.

### 1.2 License provenance gap

**Finding (Showstopper #4):** `Cargo.toml` declares `license = "MIT OR Apache-2.0"`. The repository contains only `LICENSE` (MIT, Copyright 2026 Dimitar.Simovski). The Apache-2.0 license text file is **missing entirely**. This is a compliance risk for downstream users who rely on the Apache-2.0 dual-license.

Compare: Rust compiler uses `COPYRIGHT` + `REUSE.toml` declaring blanket `MIT OR Apache-2.0` for all source under `compiler/**`, `library/**`, `tests/**`, `src/**`, with properly documented exceptions (NCSA, GPL-3.0, Unicode, OFL, BSD, ISC, GCC-exception-3.1).

---

## 2. Actual Compilation Pipeline (8 stages)

Traced through `main.rs:262-281` and confirmed by source inspection:

```
Stage 1:  Source(.stnx)              → Raw text
Stage 2:  Lexer (logos 0.16)         → Vec<Token>  [lexer/mod.rs]
Stage 3:  Parser (chumski 0.13)      → ast::Program  [parser/mod.rs]
Stage 4:  HIR Lowering               → HirProgram  [hir/lower.rs]
Stage 5:  MIR Lowering               → MirProgram  [mir/lower.rs]
Stage 6:  MIR Verify                 → Result<(), Vec<MirVerifyError>>  [mir/verify.rs]
Stage 7:  MIR Optimize (const fold)  → MirProgram (mutated)  [mir/opt.rs]
Stage 8:  LLVM IR + Object + Link    → Executable  [mir/codegen.rs + codegen/linker.rs]
```

### Entry point: `main.rs` (718 lines)

The CLI (`stnx`, clap-display name `"saturnite"`) has 5 subcommands:

| Command | Handler | Path |
|---|---|---|
| `Build` | `main.rs:136-360` | Project::discover → load → lex → parse → HIR lower → MIR lower → verify → optimize → codegen → link |
| `Check` | `main.rs:363-381` | Stops at `semantic::analyze` (no codegen) |
| `Run` | `main.rs:383-436` | Build to temp dir, then execute |
| `Doctor` | `main.rs:398` | `run_doctor` — host triple, linker, runtime checks |
| `Init` | `main.rs:403-417` | `init_project` — writes `saturn.toml` + `src/main.stnx` |

**Build pipeline (main.rs:262-281):**
```rust
let tokens: Vec<_> = stnx::lexer::Lexer::new(&src).by_ref()
    .collect::<Result<Vec<_>, _>>()?;
let program = stnx::parser::parse(&src, tokens)?;
let hir = stnx::semantic::analyze_and_lower(&program)?;
```

**Critical gap:** `analyze_and_lower` is the **single-file** path. The multi-module path `analyze_and_lower_with_graph(&program, &project.graph)` exists at `semantic.rs:42-49` but is **never called** by the CLI. See Showstopper #2.

### Stage 2: Lexer (`lexer/mod.rs`, 353 lines)

Two-stage design:

1. **`LexicalToken`** — a `logos::Lexer` derive struct with `#[logos(skip(r"[ \t\n\f]+|//[^\n]*", allow_greedy = true))]`. Maps keyword/literal patterns to `TokenKind`.

2. **`Lexer`** — wraps `logos::Lexer`, implements `Iterator<Item = Result<Token, LexError>>`. Each `Token` carries `{ kind: TokenKind, span: Range<usize> }`.

**Token kinds:** 58 total in `lexer/token.rs:71 lines`. Categories: punctuation (30+), keywords (24 including `mod`, `use`, `pub`, `as`), literals (int, float, str, bool), and EOF/error.

### Stage 3: Parser (`parser/mod.rs`, 1457 lines)

chumsky 0.13 Pratt-style parser with 365 lines of keyword table and 57 inline `#[test]` tests. Uses `.memoized()` for recursion safety. 25 reserved keywords.

**Entry point:** `program()` = `item().repeated().collect()` — parses items until EOF.

**AST item kinds** (ast.rs:180 lines): `Function`, `StructDef`, `EnumDef`, `ModDecl`, `UseDecl`. Each carries `Visibility { Private, Public }`.

**Expr variants:** 18 total. Key: `Var(String)`, `Assign { target: String, value: Box<Expr> }`, `Call { func: String, args: Vec<Expr> }`, `If { ... }`, `For { ... }`, `While { ... }`, `StructLiteral { name: String, fields: Vec<(String, Expr)> }`, `FieldAccess { expr, field: String }`, `EnumConstructor { name: String, variant: String }`.

**Stmt variants:** 6 total. Key: `Let { name, mutable, ty, value }`, `Return(Option<Expr>)`, `Println(Expr)`, `StructDef`, `EnumDef`.

**Type variants:** 7 total: `I64`, `F64`, `Bool`, `Str`, `Unit`, `Struct(String)`, `Enum(String)`. Note: type names in AST are unresolved `String`s.

**Operators:** `BinOp` (13: Add/Sub/Mul/Div/Mod/Eq/Ne/Lt/Gt/Le/Ge/And/Or), `UnOp` (2: Neg/Not), `AugOp` (4: Add/Sub/Mul/Div).

### Stage 4: HIR Lowering (`hir/lower.rs`, 734-875 lines)

Two-pass design:

- **Pass 1** (lines 192-371): Register all signatures — iterate `program.items`, assign `DefId`s, populate `function_sigs: HashMap<SymbolId, FunctionSig>`, register struct/enum/use/mod declarations.
- **Pass 2** (lines 390-434): Lower function bodies — iterate items, lower `Expr`/`Stmt` to HIR.
- **Post-pass** (lines 436-505): Construct `DefTable` and `module_paths`.

**`HirProgram`** (hir/function.rs:10 fields):
```rust
pub struct HirProgram {
    pub functions: Vec<HirFunction>,
    pub structs: Vec<StructDef>,
    pub enums: Vec<EnumDef>,
    pub symbols: SymbolInterner,
    // Module-aware fields:
    pub modules: Vec<Module>,      // for single-file: [ROOT_ONLY]
    pub root_module: ModuleId,
    pub module_paths: HashMap<DefId, ModuleId>,
    pub def_table: DefTable,
    pub module_scopes: Vec<ModuleScope>,
    pub use_decls: Vec<HirUseDecl>,
    pub mod_decls: Vec<HirModDecl>,
}
```
Derives: **Debug only** (NOT Serialize) — Showstopper #3.

**`PRINTLN_DEF_ID = DefId(u32::MAX - 1)`** — declared at `hir/lower.rs:43`, shared with `mir/lower.rs:30` and `mir/codegen.rs:27`.

### Stage 5: MIR Lowering (`mir/lower.rs`, 734 lines)

`lower_program(hir: &HirProgram) -> MirProgram`.

Builds `sigs: HashMap<DefId, (Vec<HirType>, HirType)>` (NOT a Vec — uses HashMap lookup by equality). `lower_call` uses `sigs.get(&def_id)` with fallback to `result_ty`.

**MIR type inventory** (mir/mod.rs, 343 lines):

| Type | Definition | Derives |
|---|---|---|
| `MirProgram` | `{ functions, symbols, structs, enums }` | Debug ✗ |
| `MirFunction` | `{ name, params, return_ty, blocks, start_block, locals }` | Clone, Debug, Serialize, Deserialize ✓ |
| `MirBasicBlock` | `{ id, stmts, terminator }` | Clone, Debug, Serialize, Deserialize ✓ |
| `MirTerminator` | 5 variants: Goto, SwitchInt, Call, Return, Unreachable | Clone, Debug, Serialize, Deserialize ✓ |
| `MirStmtKind` | 2 variants: LocalDecl, Assign | Clone, Debug, Serialize, Deserialize ✓ |
| `MirRvalue` | 7 variants: Use, Binary, Unary, StructLit, FieldAccess, EnumCtor, StrLit | Clone, Debug, Serialize, Deserialize ✓ |
| `MirOperand` | 2 variants: Const(MirConst), Local(LocalId) | Clone, Debug, Serialize, Deserialize ✓ |
| `MirConst` | 3 variants: I64, F64, Bool | Clone, Debug, Serialize, Deserialize ✓ |
| `MirBinOp` | 13 variants | Clone, Debug, Serialize, Deserialize ✓ |
| `MirType` | Type alias = `HirType` | (inherits HirType's derives) ✓ |
| `MirLocal` | `{ id, ty, mutable, name, kind }` | Serialize, Deserialize ✓ |
| `BlockId` / `LocalId` | u32 newtypes | Serialize, Deserialize ✓ |

**Type alias:** `MirType = HirType` — MIR reuses HIR's 7-variant flat type system.

### Stage 6: MIR Verification (`mir/verify.rs`, 204 lines)

`MirProgram::verify()` → iterates functions, calls `verify_function()`.

Returns `Result<(), Vec<MirVerifyError>>` — structured errors, not panics.

**5 structural checks:**

1. **Terminator presence:** Every block's last statement must be a real terminator (not `Unreachable` placeholder).
2. **Valid target blocks:** All `Goto`/`SwitchInt`/`Call` target `BlockId`s exist in the function's block list.
3. **Valid LocalId refs:** All `LocalId`s in operands and locals are within `0..num_locals`.
4. **Valid param locals:** Parameters are the first N locals (index 0..n-1).
5. **Valid start block:** `start_block` refers to an existing block.

Uses `HashSet<BlockId>` for O(1) block existence checks.

### Stage 7: MIR Optimization (`mir/opt.rs`, 163 lines)

`optimize(program: &mut MirProgram)` — entry point that iterates over all functions and runs `ConstantFolder::run(func)` on each.

**ConstantFolder** — single pass, constant folding only:

- `fold_rvalue()`: Attempts to fold `Binary` and `Unary` rvalues if all operands are `MirConst`.
- `fold_binop()`: Matches on `MirBinOp`, calls `fold_i64`/`fold_f64`/`fold_bool`.
- `fold_i64()`: Uses **wrapping** arithmetic (`wrapping_add`, `wrapping_sub`, etc.) for all operations. Division by zero returns `None` (deferred to runtime).
- `fold_f64()`: IEEE 754 semantics via standard operators.
- `fold_bool()`: Logical operations.

**No other optimization passes** — no CFG simplification, no copy propagation, no dead store elimination, no GVN, no jump threading.

### Stage 8: LLVM Code Generation (`mir/codegen.rs`, 841 lines)

`compile_from_mir_ext(mir: &MirProgram, output_path: &Path, config: &TargetConfig, save_temps: bool)` — the main entry point.

**Flow:**

1. Create LLVM `Context`, `MirCodeGenContext`.
2. `declare_builtin_functions()` — declares `println_i64` as LLVM function.
3. `declare_functions()` — declares all MIR functions in the LLVM module (skips `PRINTLN_DEF_ID`).
4. `generate_function()` for each function — walks MIR blocks/stmts/terminators, emits LLVM IR.
5. Set module triple from `target_config.triple()`.
6. If opt level is non-None: create `TargetMachine`, run pass pipeline via `opt_pass_name()`.
7. Emit based on `target_config.output_kind()`:
   - `Ir`: `module.print_to_file()` → `.ll` text
   - `Object`: `ObjectEmitter::new(&module, &target_config).emit_object()` → `.o`
   - `Exe`: emit `.o` → `Linker::new(&target_config).link(&obj_path, output_path)` → executable

**`MirCodeGenContext`** (5 fields):
```rust
struct MirCodeGenContext<'ctx> {
    context: &'ctx LLVMContext,
    module: inkwell::module::Module<'ctx>,
    builder: IRBuilder<'ctx>,
    local_allocas: HashMap<LocalId, AllocaInfo<'ctx>>,  // per-function
}
```
Note: `local_allocas` is described in audit notes as per-function, but the struct holds the module-level context.

**`generate_function`** — codegens one `MirFunction`:
- Looks up/creates LLVM function via `module.get_function` / `add_function`.
- Creates one LLVM basic block per MIR `MirBasicBlock` (eager creation, not lazy).
- Allocates an LLVM **alloca** for every local (always alloca, never direct operand).
- Stores parameters into their allocas.
- Iterates blocks in **vector order** (not reverse postorder — a missed optimization opportunity).
- Calls `gen_stmt()` for each statement, `gen_terminator()` for the terminator.

**`gen_rvalue`** — matches on 7 `MirRvalue` variants:
- `Use(operand)` → `materialize_operand` (Const → LLVM const; Local → load from alloca)
- `Binary { op, lhs, rhs }` → `gen_binop`
- `Unary { op, operand }` → `gen_unop`
- `StructLit` → build LLVM struct, insert field values, alloca + store
- `FieldAccess` → load struct from local, `const_to_int` field index, `build_struct_get`
- `EnumCtor` → look up variant index, emit as `i64` constant
- `StrLit` → emit LLVM string constant via `const_string` + `const_to_ptr`

**`gen_terminator`** — matches on 5 `MirTerminator` variants:
- `Goto { target }` → `build_unconditional_branch`
- `SwitchInt { scrutinee, branches, else_target }` → `build_switch`
- `Call { func, args, destination, next }` → resolve function name, call, store return value
- `Return(Some(operand))` → `materialize_operand` + `build_return`
- `Return(None)` → `build_return(None)` (void return)
- `Unreachable` → `build_unconditional_branch` to an `unreachable` block

**`function_name(DefId)`** — O(n) `find()` scan by `DefId` equality, not array indexing.

### Object Emission (`codegen/emitter.rs`, 42-43 lines)

```rust
pub struct ObjectEmitter<'ctx, 'a> {
    module: &'a Module<'ctx>,
    target_config: &'a TargetConfig,
}
impl ObjectEmitter {
    pub fn emit_object(&self, path: &Path) -> Result<()>
    pub fn emit_ir(&self) -> Result<String>
    pub fn emit_ir_to_file(&self, path: &Path) -> Result<()>
}
```

Uses `TargetMachine::write_to_file` with `FileType::Object` for object emission.

### Linking (`codegen/linker.rs`, 199-200 lines)

```rust
pub struct Linker<'cfg> {
    target_config: &'cfg TargetConfig,
}
```

**`select_linker()`** — matches `(os, env)`:
| OS | Environment | Linker |
|---|---|---|
| Linux | — | `cc` |
| Darwin | — | `clang` |
| Windows | Msvc | `link.exe` |
| Windows | GNU | `gcc` |
| Other | — | `cc` |

Uses `which::which(linker_name)` to locate linker on PATH before spawning. `check_linker_available()` runs `--version` or `/?` to verify.

**Link args construction** (`build_linker_args`):
- Linux/Darwin/GNU: `[obj_file, "-o", output, runtime_archive]`
- Windows MSVC: `[obj_file, "/OUT:output", "/DEFAULTLIB:runtime_archive"]`

---

## 3. Identifier System (3 flat u32 spaces)

### 3.1 The three identifier types (`hir/symbol.rs:187 lines`)

```rust
pub struct SymbolId(pub u32);     // Interned string index — derives serde ✓
pub struct DefId(pub u32);         // Definition index — derives serde ✓
pub struct ModuleId(pub u32);     // Module identity — separate space from DefId
```

These are deliberately distinct types, but they share the same underlying `u32` representation.

### 3.2 SymbolInterner (`hir/symbol.rs:46-50`)

```rust
pub struct SymbolInterner {
    strings: Vec<String>,                              // heap-allocated String storage
    indices: HashMap<String, SymbolId>,                // RandomState — NON-DETERMINISTIC
}
```

Derives: **Debug, Default only** — NOT Serialize.

**Critical issues:**
1. Uses `std::collections::HashMap` with `RandomState` (Rust's default hasher) — produces **non-deterministic iteration order** across runs. This breaks reproducible builds and caching.
2. Allocates **two heap `String`s** per intern call (one for `indices` key, one for `strings` push).
3. No `StableHash`/`StableCompare` — no support for incremental compilation fingerprints.

### 3.3 DefTable and DefEntry (`hir/symbol.rs:91-168`)

```rust
pub enum DefKind {
    Function, Struct, Enum, Module, Use,
}  // Debug only ✗

pub struct DefEntry {
    pub module: ModuleId,
    pub local_index: u32,
    pub kind: DefKind,
}  // Debug only ✗

pub struct DefTable {
    entries: Vec<DefEntry>,  // indexed by DefId.0
}  // Debug, Default — NOT Serialize ✗

impl DefTable {
    pub fn register(&mut self, entry: DefEntry) -> DefId {
        let id = DefId(self.entries.len() as u32);
        self.entries.push(entry);
        id
    }
    pub fn lookup(&self, id: DefId) -> Option<&DefEntry> { ... }  // array indexing
    pub fn iter(&self) -> impl Iterator<Item = (DefId, &DefEntry)> { ... }
}
```

### 3.4 Visibility (`hir/symbol.rs`)

```rust
pub struct Visibility {
    pub is_public: bool,
}
```

Derives: **Debug, Clone, Copy, PartialEq, Eq, Hash, Default** — NOT Serialize.

### 3.5 DefId namespace collapse (Showstopper #1)

**The single most dangerous defect in the codebase.**

In `hir/lower.rs`, `DefId`s are assigned from separate per-kind counters:

- **Functions:** `func_def_id` counter (sequential: `DefId(0)`, `DefId(1)`, ...)
- **Structs:** `structs.len()` at push time (`DefId(0)`, `DefId(1)`, ...)
- **Enums:** `enums.len()` at push time (`DefId(0)`, `DefId(1)`, ...)

This means `DefId(0)` is simultaneously:
- A valid function definition (the first function)
- A valid struct definition (the first struct)
- A valid enum definition (the first enum)

The `DefTable` uses `entries: Vec<DefEntry>` indexed by `def_id.0` — this is **unsound** because the same `DefId(0)` maps to three different definition kinds.

**Why it doesn't crash in the current pipeline:** The MIR lowering uses `sigs: HashMap<DefId, (Vec<HirType>, HirType)>` (lookup by equality, not indexing), and `function_name()` uses `find()` (linear scan, not indexing). So the pipeline works for single-file programs. But any `DefId`-keyed cache, incremental fingerprint, or array-indexed lookup would be catastrophically unsound.

**Fix required:** Assign `DefId`s from a **single global counter** that encompasses all definition kinds, OR introduce a separate indexing scheme.

---

## 4. Module System (`module.rs`, 1516 lines) — Phase 3–14

### 4.1 Module types

| Type | Fields | Derives |
|---|---|---|
| `ModuleId(u32)` | ROOT = `ModuleId(0)` | — |
| `ModulePath` | `{ segments: Vec<SymbolId> }` | — |
| `Module` | `{ id, path, file_path, ast: Option<Program>, parent, mod_declarations }` | — |
| `ModuleScope` | `{ items: HashMap<SymbolId, DefId>, imports: HashMap<SymbolId, DefId>, parent: Option<ModuleId> }` | NOT Serialize ✗ |
| `ModuleGraph` | `{ modules, root, symbol_interner, module_index, imports }` | — |
| `Project` | `{ config, root, source_root, graph }` | — |

### 4.2 Discovery algorithm (`discover_modules`, module.rs:497-575)

1. Create root module from `root_file` with empty `ModulePath`.
2. Read and parse the root file's source text.
3. `extract_mod_declarations_from_ast(ast, source)` — **AST-based primary path** (walks `Program::items` for `ItemKind::ModDecl`).
4. Text-based fallback: `extract_mod_declarations(source)` — line-by-line scan for `mod <ident>`.
5. For each child mod name, resolve file via `resolve_module_file`, recursively discover.
6. `add_module()` assigns `ModuleId` sequentially and indexes by path.

**No cycle detection** in `discover_modules` — circular `mod` imports would cause infinite recursion.

### 4.3 File resolution (`resolve_module_file`, module.rs:590-611)

Given a directory and module name:
1. `<dir>/<name>.stnx` — single file form
2. `<dir>/<name>/mod.stnx` — directory module form

### 4.4 Project root discovery (`Project::discover`, module.rs:728-798)

Walks upward from start path looking for `saturn.toml`. The first directory containing it is the project root. `source_root = <root>/src/`. If no `saturn.toml` found, synthesizes a config from the directory name.

### 4.5 CLI integration

`main.rs:4` imports `Project`. `Build`/`Check`/`Run` all call `Project::discover()`:
- **Build** (line 249): `Project::discover(&entry_path)` → `project.load_from()` or `project.load()`.
- **Check** (line 368): `Project::discover(&cwd)`.
- **Run** (line 494): `build_run_file` calls `Project::discover(input)`.

**Showstopper #2 — CLI bypass:** Although `Project::discover()` is called, the CLI passes only the root module's `Program` to `analyze_and_lower(&program)` (single-file path), NOT `analyze_and_lower_with_graph(&program, &project.graph)` (multi-module path). The `ModuleGraph` is built but child module ASTs are never lowered to HIR.

The `analyze_and_lower_with_graph` entry point exists at `semantic.rs:42-49`, backed by `hir/lower.rs:518-935` (`lower_program_with_graph`), but is never invoked.

---

## 5. Serialization Dependency Chain

The chain of dependencies for HIR/MIR serialization (required for incremental compilation):

```
HirProgram (Debug only ✗)
  ├─ SymbolInterner (Debug only ✗)           ← BLOCKER M2
  ├─ HirFunction (Debug only ✗)              ← BLOCKER M5
  │   ├─ HirExpr (Debug, Clone — no serde)
  │   │   └─ HirExprKind (Debug, Clone — no serde)
  │   └─ HirStmt (Debug, Clone — no serde)
  │       └─ HirStmtKind (Debug, Clone — no serde)
  ├─ StructDef (Debug, Clone — no serde)     ← BLOCKER M4
  ├─ EnumDef (Debug, Clone — no serde)       ← BLOCKER M4
  ├─ Visibility (Debug, Clone, ... — no serde)  ← BLOCKER
  ├─ DefTable (Debug — no serde)             ← BLOCKER
  ├─ DefEntry (Debug — no serde)             ← BLOCKER
  ├─ DefKind (Debug — no serde)              ← BLOCKER
  ├─ ModuleScope (Debug — no serde)          ← BLOCKER M3
  ├─ Module (Debug — no serde)               ← BLOCKER M3
  ├─ HirModDecl (Debug — no serde)           ← BLOCKER
  ├─ HirUseDecl (Debug — no serde)           ← BLOCKER
  └─ SourceSpan (miette)                     ← BLOCKER M1
      └─ miette serde feature NOT enabled in Cargo.toml

MirProgram (Debug only ✗)
  ├─ SymbolInterner (Debug only ✗)           ← BLOCKER M2 (same)
  ├─ StructDef (Debug, Clone — no serde)     ← BLOCKER M4
  └─ EnumDef (Debug, Clone — no serde)       ← BLOCKER M4
```

**15 types** that must gain `Serialize/Deserialize` derives (and/or miette serde feature) before any incremental compilation is possible.

The `mir/` sub-types (MirFunction, MirBasicBlock, MirTerminator, MirStmtKind, MirRvalue, MirOperand, MirConst, MirBinOp, MirLocal, BlockId, LocalId) ALL derive `Serialize/Deserialize` — but the program-level container `MirProgram` and its dependencies (`SymbolInterner`, `StructDef`, `EnumDef`) do not, making them useless for serialization.

---

## 6. Target Configuration (`target.rs`, 482 lines)

| Type | Description | Derives |
|---|---|---|
| `Architecture` | 10 variants (x86, x86_64, ARM, AArch64, etc.) | Debug |
| `OS` | 5 variants (Windows, Linux, Darwin, FreeBSD, DragonFly) | Debug |
| `Environment` | 4 variants (Msvc, GNU, Android, None) | Debug |
| `OptimizationLevel` | 4 variants (None, Less, Default, Aggressive) | Debug |
| `DebugInfo` | 2 variants (Yes, No) | Debug |
| `Profile` | 3 variants (Debug, Release, Custom) | Debug |
| `OutputKind` | 3 variants (Ir, Object, Exe) | Debug |
| `TargetConfig` | 10 fields (triple, arch, os, env, opt_level, etc.) | **Debug only ✗** |

**Missing derives on TargetConfig:** NOT `Hash`, NOT `PartialEq`, NOT `Serialize`. This prevents target config caching and fingerprinting.

---

## 7. Testing (364 tests, 0 failures)

Confirmed by running `cargo test --workspace`:

| Test binary | File | Tests | Description |
|---|---|---|---|
| config (unit) | `config.rs:222` | 7 | TOML parsing roundtrips |
| library unit | `lib.rs` | 115 | Lexer/parser/MIR/HIR internal |
| codegen | `tests/codegen.rs` | 24 | IR string assertions |
| diagnostics | `tests/diagnostics.rs` | 6 | Error message checks |
| lexer | `tests/lexer.rs` | 17 | Token kind validation |
| mir_lower | `tests/mir_lower.rs` | 27 | HIR→MIR lowering checks |
| native_compilation | `tests/native_compilation.rs` | 63 | End-to-end compile+execute |
| semantic | `tests/semantic.rs` | 35 | Type/semantic checks |
| test_doctor | `tests/test_doctor.rs` | 9 | Doctor subcommand tests |
| test_end_to_end_modules | `tests/test_end_to_end_modules.rs` | 2 | Multi-module compilation |
| test_full_compile | `tests/test_full_compile.rs` | 1 | Full pipeline test |
| test_ir_only | `tests/test_ir_only.rs` | 1 | IR-only output |
| test_module_graph | `tests/test_module_graph.rs` | 41 | Module discovery + Project::discover |
| test_module_resolution | `tests/test_module_resolution.rs` | 3 | Module name resolution |
| test_multi_module_codegen | `tests/test_multi_module_codegen.rs` | 3 | MIR lowering across modules |
| test_native_only | `tests/test_native_only.rs` | 1 | Native execution |
| test_project_loading | `tests/test_project_loading.rs` | 3 | Project config loading |
| test_target_config | `tests/test_target_config.rs` | 12 | Target triple parsing |
| test_target_machine | `tests/test_target_machine.rs` | 1 | TargetMachine creation |
| **Total** | | **364** | **0 failures** |

Notable coverage gaps:
- **Parser tests:** 0 direct tests (only via integration tests)
- **HIR tests:** 0 direct tests (only via integration tests)
- **CLI tests:** 0 direct tests
- **MIR optimization tests:** 0 direct tests (only via native_compilation end-to-end)
- **Serialization tests:** 0 (not implemented yet)

---

## 7. Source File Inventory

| File | Lines | Purpose | Derive gap |
|---|---|---|---|
| `lib.rs` | 85 | 11 module declarations + re-exports | — |
| `main.rs` | 718 | CLI with 5 subcommands | — |
| `parser/mod.rs` | 1457 | chumsky parser + 57 inline tests | — |
| `ast.rs` | 180 | AST types (Debug, Clone only) | No serde |
| `lexer/mod.rs` | 353 | Two-stage logos lexer | — |
| `lexer/token.rs` | 71 | TokenKind enum (58 variants) | — |
| `hir/mod.rs` | 40 | HIR re-exports | — |
| `hir/symbol.rs` | 187 | SymbolId, DefId, SymbolInterner, DefTable | No serde on key types |
| `hir/function.rs` | 221 | HirProgram (Debug only), HirFunction, StructDef, EnumDef | No serde |
| `hir/expr.rs` | 118 | HirExpr (Debug, Clone), HirExprKind (Debug, Clone) | No serde |
| `hir/stmt.rs` | 54-55 | HirStmt (Debug, Clone), HirStmtKind (Debug, Clone) | No serde |
| `hir/types.rs` | 57 | HirType (7 variants), serde ✓ | — |
| `hir/lower.rs` | 734-875 | HIR lowering, lower_program, lower_program_with_graph | — |
| `mir/mod.rs` | 343 | MIR types (most serde ✓, MirProgram Debug only ✗) | MirProgram no serde |
| `mir/lower.rs` | 734 | HIR→MIR lowering | — |
| `mir/verify.rs` | 204 | MIR verification (5 checks) | — |
| `mir/opt.rs` | 163 | ConstantFolder (constant folding only) | — |
| `mir/codegen.rs` | 841 | MERLower→LLVM IR, ObjectEmitter, Linker | — |
| `module.rs` | 1516 | Module graph, Project, discovery | Most types no serde |
| `target.rs` | 482 | TargetConfig, Architecture, OS, etc. | No serde on TargetConfig |
| `config.rs` | 222 | SaturnConfig, Package, DependencySpec (serde ✓) | — |
| `error.rs` | 159 | CompilerError, LexError, ParseError (miette) | — |
| `semantic.rs` | 53 | analyze/analyze_and_lower/analyze_and_lower_with_graph | — |
| `codegen/mod.rs` | 37 | ObjectEmitter, Linker, TargetConfig re-exports | — |
| `codegen/emitter.rs` | 42-43 | ObjectEmitter struct + emit_object | — |
| `codegen/linker.rs` | 199-200 | Linker, select_linker, build_linker_args | — |
| `build.rs` | 55 | Compile println_i64.c → libsaturnite_runtime.a | — |

---

## 8. Documentation Accuracy Audit

### 8.1 Documents that are ACCURATE

| Document | Status | Notes |
|---|---|---|
| `SATURNITE_0_4_ARCHITECTURE.md` | Accurate (2 errors) | Cross-compilation guard claim is wrong (actual: stern warning, not hard error in all cases); shadowing description slightly off |
| `SATURNITE_POST_MODULE_ARCHITECTURE_AUDIT.md` | Accurate | Correctly identifies 4 showstoppers, 14 MUST FIX items |
| `docs/audit_notes/pipeline.md` | Accurate | Correctly describes actual MIR implementation |
| `docs/audit_notes/module_language_design.md` | Accurate | Correctly describes implemented module system |

### 8.2 Documents that are STALE or WRONG

| Document | Status | Specific errors |
|---|---|---|
| `SATURNITE_0_3_ARCHITECTURE_REVIEW.md` | Stale (0.2-era) | Pre-refactoring; many structures changed |
| `SATURNITE_0_3_HIR_DESIGN.md` | Stale (0.3-era) | Proposed types differ from actual 0.4 implementation |
| `SATURNITE_0_4_ARCHITECTURE_AUDIT.md` | **Multiple contradictions** | Claims "MIR is NOT implemented" (WRONG — it IS); claims "No module system exists" (WRONG — module.rs:1516 lines exists); claims "saturn.toml parsed but NEVER read" (WRONG — Project::discover calls SaturnConfig::from_dir) |
| `SATURNITE_MIR_DESIGN.md` | Completely stale | Describes MIR types that don't match implementation |
| `SATURNITE_FINAL_VERIFICATION.md` | Stale (0.3-era) | Claims "123 tests" — actual is 364; also internally inconsistent (table sums to 126) |
| `docs/audit_notes/infra.md` | **Critical errors** | Claims "NO MIR references in source" (WRONG — 5 MIR files exist); claims "MISSING (Design Only)" for MIR (WRONG); lists `codegen/context.rs` which doesn't exist (actual: `mir/codegen.rs`) |
| `docs/audit_notes/module_language_design.md` | Stale (Phase 3-9) | Claims MIR is "Design Only" (WRONG — implemented); lists 0 parser/HIR tests (actual: 57+ inline parser tests) |
| `SATURNITE_DEPENDENCY_MODEL.md` | Design-only | Python interop is DESIGN-ONLY, not implemented |
| `SATURNITE_INCREMENTAL_COMPILATION.md` | Partially stale | No mention of ModuleId/DefId instability issue |

**Root cause of staleness:** The audit_notes files appear to be from a Phase 0/1 audit that was written against an earlier codebase snapshot. The pipeline.md and module_language_design.md files were updated to reflect the actual state (including MIR implementation and module system), but infra.md and module_language_design.md were NOT updated and still reflect the pre-MIR, pre-module-system state.

---

## 9. Showstoppers Summary

### Showstopper #1: DefId Namespace Collapse
- **Location:** `hir/lower.rs` — functions/structs/enums all assign from `DefId(0)` independently
- **Impact:** Any `DefId`-keyed array index or cache is unsound. Current codegen works only because it uses HashMap lookups and `find()`.
- **Fix:** Single global `DefId` counter, or separate index spaces with kind-qualified lookups.

### Showstopper #2: CLI Bypass of Module System
- **Location:** `main.rs:255` calls `analyze_and_lower(&program)` (single-file path)
- **Impact:** Multi-module programs compile only the root module; child modules are discovered but never lowered to HIR or MIR.
- **Fix:** Wire `analyze_and_lower_with_graph(&program, &project.graph)` into CLI Build/Check/Run paths.

### Showstopper #3: No Serialization
- **Location:** 15 types across `hir/symbol.rs`, `hir/function.rs`, `hir/expr.rs`, `hir/stmt.rs`, `mir/mod.rs`, `module.rs`, `target.rs` lack `Serialize/Deserialize`
- **Impact:** HIR/MIR cannot be cached; incremental compilation impossible; no build artifacts.
- **Fix:** Add serde derives to all 15 types; enable miette `serde` feature (M1); replace `RandomState` with `FxBuildHasher` for determinism (M2).

### Showstopper #4: Missing Apache-2.0 License File
- **Location:** Repository root — only `LICENSE` (MIT) exists, `LICENSE-APACHE` missing
- **Impact:** `Cargo.toml` claims `MIT OR Apache-2.0` but Apache-2.0 text is absent. Compliance risk.
- **Fix:** Add `LICENSE-APACHE` file matching the Rust compiler's Apache-2.0 text.

---

## 10. 14 MUST FIX Items (from SATURNITE_POST_MODULE_ARCHITECTURE_AUDIT.md)

Dependency chain for incremental compilation readiness:

| ID | Item | Gate | Depends on |
|---|---|---|---|
| M1 | Enable miette `serde` feature for `SourceSpan` | Provenance | — |
| M2 | Replace `RandomState` in `SymbolInterner` with `FxBuildHasher`; add `Serialize` to `SymbolInterner` | Soundness | M1 |
| M3 | Add `Serialize/Deserialize` to `ModuleGraph`, `Module`, `ModuleScope`, `ModulePath` | Soundness | M2 |
| M4 | Add `Serialize/Deserialize` to `StructDef`, `EnumDef`, `Visibility` | Soundness | M3 |
| M5 | Add `Serialize/Deserialize` to `HirProgram`, `HirFunction`, `HirExpr`, `HirStmt` | Soundness | M4 |
| M6 | Fix `DefId` namespace collapse — single global counter | Soundness | M5 |
| M7 | Add `Serialize/Deserialize` to `DefTable`, `DefEntry`, `DefKind` | Soundness | M6 |
| M8 | Add `Serialize/Deserialize` to `MirProgram` (and its new deps) | Soundness | M7 |
| M9 | Add `Serialize/Deserialize` to `TargetConfig`; add `Hash + PartialEq` | Soundness | M8 |
| M10 | Implement `StableHash`/`StableCompare` on `SymbolId`/`DefId` | Soundness | M9 |
| M11 | Implement `ModuleId` stability (separate from `DefId` space) | Soundness | M10 |
| M12 | Add cycle detection to `discover_modules` | Soundness | M11 |
| M13 | Wire `analyze_and_lower_with_graph` into CLI | Integration | M12 |
| M14 | Add serialization tests + incremental compilation smoke tests | Testing | M13 |

---

## 11. Comparison with Rust Compiler Architecture

### Pipeline comparison

| Stage | Saturnite | Rust compiler |
|---|---|---|
| Parse | logos + chumsky (single crate) | Hand-written recursive descent (`rustc_parse`) |
| Expand | N/A | `rustc_expand` (macro expansion) |
| Resolve | HIR-level `resolve_modules` post-pass | Dedicated `rustc_resolve` resolver |
| Analysis | Combined with HIR lowering | `rustc_hir_analysis` (type checking, trait solving) |
| Borrowck | N/A | N/A (no borrow checking in Saturnite) |
| MIR | `mir/lower.rs:734`, `mir/verify.rs:204` | `rustc_mir_build`, `rustc_mir_transform` |
| CodeGen | `mir/codegen.rs:841` (inkwell 0.9, LLVM 21) | `rustc_codegen_ssa`, `rustc_codegen_llvm` |
| Link | `codegen/linker.rs:199` (system linker) | `rustc_codegen_ssa/src/back/link.rs:4213` |

### MIR scope comparison

| Concept | Saturnite | Rust |
|---|---|---|
| MIR phases | None (Built → Optimized → Codegen) | 4 phases: Analysis(Initial/PostCleanup) → Runtime(Initial/PostCleanup/Optimized) |
| MIR passes | 1 (constant folding) | ~40 passes via `MirPass` trait + `declare_passes!` |
| MIR validation | 5 structural checks | Validator pass (~20 checks + type system integration) |
| MIR types | 7 Rvalue, 5 Terminator, 2 Stmt variants | ~25 Rvalue, ~15 Terminator, ~15 Stmt variants |
| Optimization location | MIR-level constant folding + LLVM IR passes | MIR-level (many) + LLVM IR passes |

### Codegen comparison

| Concept | Saturnite | Rust |
|---|---|---|
| Codegen context | `MirCodeGenContext` (5 fields, flat) | `FunctionCx` (16 fields, generic over `BuilderMethods`) |
| Local storage | Always `alloca` for every local | `LocalRef` enum: Immediate (direct SSA) vs. Place (alloca) |
| Block order | Vector order | Reverse postorder (RPO) |
| Debug info | None | Full `FunctionDebugContext` + per-local debug info |
| ABI handling | Hardcoded `mir_type_to_llvm` | `FnAbi` via `rustc_target` |

### Identification system comparison

| Concept | Saturnite | Rust |
|---|---|---|
| Strings | `SymbolId(u32)` → `Vec<String>` + `HashMap<String, SymbolId>` (RandomState) | `Symbol(u32)` → `DroplessArena` + `HashTable` (FxBuildHasher) |
| Definitions | `DefId(u32)` (flat, collapsed) | `DefId` (globally unique, includes crate disambiguator) |
| Modules | `ModuleId(u32)` (separate space) | `DefId` + `LocalDefId` (interned, hierarchical) |
| Thread model | Explicit passing | Thread-local singleton (`SessionGlobals`) |

---

## 12. Key Findings for Phases 3–13

### High-value KEEP items (aligned with Rust design):
1. MIR as explicit CFG IR between HIR and LLVM (matches Rust's architecture)
2. Structured error returns from verifier (`Result<(), Vec<MirVerifyError>>`) — better than Rust's panicking Validator
3. Platform-aware linker selection via `(OS, Environment)` matching
4. `which::which` pre-flight linker check — superior to Rust's implicit PATH assumption
5. Explicit interner passing (vs. Rust's thread-local singleton) — better for testability
6. chumsky parser with `.memoized()` for recursion safety

### High-value ADAPT items:
1. Replace `RandomState` with `FxBuildHasher` in `SymbolInterner` (trivial, high-impact)
2. Introduce `MirPass` trait + pass manager (for extensible optimization pipeline)
3. Per-local storage optimization (immediate operands for non-address-taken locals)
4. RPO block iteration (instead of vector order)
5. `rustc_index::IndexVec` or typed index wrapper (prevent `LocalId`/`BlockId`/`DefId` mixing)

### REIMPLEMENT items:
1. Fix `DefId` namespace collapse (single global counter)
2. Add cycle detection to `discover_modules`
3. Port simplified `SimplifyCfg` (block merging, dead block elimination)
4. Port simplified `SimplifyLocals` (dead local elimination)
5. Add type-level MIR verification (Assign target type consistency)

### REJECT items:
1. Full `TyCtxt`-based type system (Saturnite's `MirType = HirType` is correct for scope)
2. Exception handling / unwinding in codegen (Saturnite's 5 terminators are correct)
3. Macro resolution system (Saturnite has no macros)
4. `rustc_errors::Emitter` (Saturnite's `miette` choice is superior for standalone)
5. LTO support (overkill for current scope)
6. External `extern` FFI block handling (Saturnite has no `extern` grammar)

---

*End of Phase 1 deliverable. This document was compiled from direct source code inspection of all files under `C:\Users\atimo\Saturnite\crates\stnx\src\`.**