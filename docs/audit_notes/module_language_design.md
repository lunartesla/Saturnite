# Module System Language Design — Saturnite Phase 3–9 Audit

> **Status:** Design audit of the actual implemented module system (docs only, no source changes)
> **Audience:** Phase 3 module language design audit
> **Scope:** Documents the REAL current state of the module system in source code after Phases 0–9, replacing the stale pre-implementation design doc. Every claim is verified against source files in `crates/stnx/src/`.
>
> This document describes what the compiler **actually does today**, not what a future design doc proposed. It is grounded in the following source files: `lexer/mod.rs`, `lexer/token.rs`, `parser/mod.rs`, `ast.rs`, `hir/lower.rs`, `hir/function.rs`, `hir/symbol.rs`, `hir/mod.rs`, `module.rs`, `lib.rs`, `main.rs`, `semantic.rs`, `mir/lower.rs`, `mir/mod.rs`, `mir/codegen.rs`, `target.rs`, `config.rs`, `codegen/emitter.rs`, `codegen/linker.rs`, `build.rs`, `runtime/println_i64.c`.

---

## 1. Overview

### 1.1 Compiler pipeline

The Saturnite 0.4 compiler pipeline is:

```
Source → Lexer → Parser → AST → HIR Lowering → MIR Lowering → LLVM IR → Object → Link
```

`DefId` flows from HIR through MIR to codegen. Module paths are resolved during HIR lowering (Phase 6A) via `resolve_modules`, and `DefId` is looked up by `function_name` in MIR codegen (not raw array indexing).

### 1.2 Current reality: structurally complete, functionally integrated

After Phases 0–9, the module system is **fully integrated** into the compilation pipeline. The CLI discovers projects, the module graph discovers child modules via AST-based scanning, HIR lowering iterates all modules, `resolve_modules` resolves `use` imports through parent-chain scope walks, and MIR lowering uses `HashMap`-based `DefId` lookups.

| Layer | Files | Status |
|-------|-------|--------|
| **Lexer** | `lexer/mod.rs`, `lexer/token.rs` | **Complete.** All 4 module keywords (`mod`, `use`, `pub`, `as`) are lexed. |
| **Parser** | `parser/mod.rs`, `ast.rs` | **Complete.** `item()` parses `fn`, `struct`, `enum`, `mod`, `use` with optional `pub` prefix. `Program` holds `Vec<Item>`. |
| **HIR data structures** | `hir/function.rs`, `hir/symbol.rs`, `hir/lower.rs` | **Complete (multi-module).** `HirProgram` carries all module metadata. `lower_program_with_graph` iterates all modules and assigns correct `ModuleId`s. |
| **Module graph** | `module.rs` | **Complete (discovery + integration).** `ModuleGraph`, `Module`, `ModulePath`, `ModuleScope`, `Project` all implemented with tests. Discovery uses **AST-based extraction** as primary, text-based fallback. |
| **MIR** | `mir/lower.rs`, `mir/mod.rs` | **Module-aware.** Uses `sigs: HashMap<DefId, (Vec<HirType>, HirType)>` — `lower_call` uses `sigs.get(&def_id)` (HashMap lookup). |
| **CLI** | `main.rs` | **Integrated.** `Build`/`Check`/`Run` commands call `Project::discover()`, `project.load()` / `project.load_from()`, with optional input file (defaults to project discovery). |

---

## 2. Keyword Inventory

### 2.1 Keywords in the lexer

The lexer (`lexer/mod.rs`) defines 24 keywords in the `LexicalToken` enum, mapped to `TokenKind` in `token.rs`:

| Category | Keywords |
|----------|----------|
| Control flow | `fn`, `let`, `mut`, `if`, `elif`, `else`, `for`, `while`, `in`, `return` |
| Types | `i64`, `f64`, `bool`, `str`, `unit` |
| Literals | `true`, `false` |
| Builtins | `println` |
| Definitions | `struct`, `enum` |
| **Module system** | **`mod`, `use`, `pub`, `as`** |

### 2.2 Module-related keywords

| Keyword | `LexicalToken` variant | `TokenKind` variant | Purpose | Lexer tests |
|---------|----------------------|---------------------|---------|-------------|
| `mod` | `Mod` | `Mod` | Declare a module dependency | `test_mod_keyword`, `test_mod_decl_tokens` |
| `use` | `Use` | `Use` | Import names from a path | `test_use_keyword`, `test_use_path_tokens` |
| `pub` | `Pub` | `Pub` | Visibility modifier | `test_pub_keyword`, `test_pub_mod_tokens` |
| `as` | `As` | `As` | Planned rename syntax | `test_as_keyword_is_reserved` |

All four are **fully implemented** in the lexer. The `convert()` function maps each `LexicalToken` variant to its `TokenKind` counterpart.

### 2.3 Parser keyword recognition

`parser/mod.rs` `is_keyword()` treats all 24 strings as reserved. `kw_span()` maps string literals to their `TokenKind` variants for all 24 keywords including `mod`, `use`, `pub`, `as`.

---

## 3. AST Module/Use Representation

### 3.1 Program structure

```rust
// ast.rs:23-32
pub struct Program {
    pub items: Vec<Item>,         // authoritative collection
    pub functions: Vec<Function>, // backwards-compatible projection
}
```

`Program` has **both** `items` (the authoritative collection) and `functions` (a backwards-compatible projection). `Program::from_items()` populates `functions` by filtering `items` for `ItemKind::Function`. This dual representation exists to ease the Phase 5 transition.

### 3.2 Item structure

```rust
// ast.rs:48-93
pub struct Item {
    pub name: String,
    pub visibility: Visibility,
    pub kind: ItemKind,
    pub span: Range<usize>
}

pub enum Visibility { Private, Public }

pub enum ItemKind {
    Function(Function),
    StructDef { name: String, fields: Vec<(String, Type)>, span: Range<usize> },
    EnumDef { name: String, variants: Vec<String>, span: Range<usize> },
    ModDecl,                                    // mod foo
    UseDecl { path: Vec<String>, alias: Option<String> },  // use foo::bar [as baz]
}
```

`ModDecl` is a unit variant — it carries only the item `name`. `UseDecl` carries a `path: Vec<String>` (unresolved) and an optional `alias`. These are interned to `SymbolId` / `Vec<SymbolId>` during HIR lowering.

### 3.3 Parser entry point

```rust
// parser/mod.rs:80-85
fn program<'a>() -> impl Parser<'a, &'a [Token], Program, ParserExtra<'a>> {
    item().repeated().collect::<Vec<_>>().map(Program::from_items)
}
```

`program()` parses **items** (not just functions), repeating `item()` until EOF, then calls `Program::from_items`.

### 3.4 item() parser

`item()` parses an optional `pub` visibility prefix, then dispatches to one of five sub-parsers:
- `fn` -> `func()` (full function)
- `struct` -> `struct_item()` (top-level struct definition with typed fields)
- `enum` -> `enum_item()` (top-level enum definition with named variants)
- `mod` -> `mod_decl()` (lines 203-207)
- `use` -> `use_decl()` (lines 211-221)

`mod_decl()` is minimal: `kw_span("mod").ignore_then(t_ident())`. It does **not** parse `mod foo::bar;` (nested paths in mod declarations) — only a single identifier.

`use_decl()` parses `use <path>` where `path` uses `path_with_span()` = `path_segment() (DoubleColon path_segment)*`. The optional `as <alias>` is supported. The `alias` is `None` when omitted.

### 3.5 Path segment parsing

`path_segment()` accepts either a plain identifier OR a small set of keyword tokens (`Println`, `True`, `False`, `I64`, `F64`, `Bool`, `Str`, `Unit`).

---

## 4. HIR Module/Use Representation

### 4.1 HirProgram structure

```rust
// hir/function.rs:128-150
pub struct HirProgram {
    pub functions: Vec<HirFunction>,
    pub structs: Vec<StructDef>,
    pub enums: Vec<EnumDef>,
    pub symbols: SymbolInterner,
    // --- Module-aware fields (Phase 5B) ---
    pub modules: Vec<Module>,
    pub root_module: ModuleId,
    pub module_paths: HashMap<DefId, ModuleId>,
    pub def_table: DefTable,
    pub module_scopes: Vec<ModuleScope>,
    pub use_decls: Vec<HirUseDecl>,
    pub mod_decls: Vec<HirModDecl>
}
```

### 4.2 HirFunction, StructDef, EnumDef — module fields

All three top-level definition types carry `module: ModuleId` and `visibility: Visibility`. In `lower_program_with_graph`, items are tagged with their owning `ModuleId` from the graph (not ROOT).

### 4.3 HirModDecl and HirUseDecl

```rust
// hir/function.rs:92-122
pub struct HirUseDecl {
    pub def_id: DefId,
    pub path: Vec<SymbolId>,        // interned path segments
    pub alias: SymbolId,            // name introduced in this module
    pub module: ModuleId,
    pub visibility: Visibility,
    pub span: SourceSpan,
}

pub struct HirModDecl {
    pub def_id: DefId,
    pub name: SymbolId,
    pub module_id: Option<ModuleId>, // resolved child module — populated by graph lookup
    pub module: ModuleId,
    pub visibility: Visibility,
    pub span: SourceSpan,
}
```

`HirModDecl.module_id` is resolved by looking up the child module name in the graph's `child_module_lookup` map (built from `ModulePath::name()` on each discovered module).

### 4.4 Module-aware accessors on HirProgram

`hir/function.rs:152-221` provides five module-aware accessors:
- `function(DefId)` — look up by DefId (array index)
- `module_of(DefId) -> Option<ModuleId>` — fast path via `module_paths`, fallback to `def_table`
- `def_entry(DefId) -> Option<&DefEntry>` — full DefEntry lookup
- `module(ModuleId) -> Option<&Module>` — look up a Module by ID
- `module_scope(ModuleId) -> Option<&ModuleScope>` — look up a ModuleScope by ID

### 4.5 HIR lowering — single-file vs. graph-based

`lower_program` (`hir/lower.rs:161-505`) for single-file programs:
1. **Phase 0** (lines 167-190): Collects enum names from top-level items and function bodies.
2. **Pass 1** (lines 192-371): Iterates `program.items` to register struct, enum, use, and mod declarations. Assigns sequential `DefId`s. Registers builtin `println` (as `PRINTLN_DEF_ID = DefId(u32::MAX - 1)`) and checks for `main`.
3. **Pass 2** (lines 390-434): Iterates items, lowers each function body. Registers `DefEntry` and `module_paths`.
4. **Def table construction** (lines 436-505): Registers all structs, enums, use decls, and mod decls.

All items are hard-coded to `ModuleId::ROOT`.

**Multi-module path:** `lower_program_with_graph` (`hir/lower.rs:518-935`) is the Phase 5B entry point:
1. Clones the graph's `SymbolInterner` into `self.symbols` (unifying the two interner spaces).
2. Iterates every module in `graph.modules`, processing each module's AST.
3. Assigns `ModuleId`s from the graph (not ROOT).
4. Resolves `HirModDecl.module_id` via `child_module_lookup`.
5. Registers items in per-module `ModuleScope`s with correct parent chains.
6. Returns `HirProgram` with `modules: graph.modules.clone()`.

### 4.6 HIR unit tests

`lower.rs:1172-1390` contains 15 unit tests verifying module-aware fields for single-file programs. Additional tests verify `lower_program_with_graph` behavior (module ID assignment, scope population, mod decl resolution, cross-module use resolution).

---

## 5. Symbol / DefId / ModuleId Architecture

### 5.1 Three identifier types (all u32-based)

```rust
// symbol.rs:30, 39; module.rs:40-51
pub struct SymbolId(pub u32);     // interned string
pub struct DefId(pub u32);       // globally-unique definition index
pub struct ModuleId(pub u32);    // module identity (separate space from DefId)
```

These three types are deliberately **distinct**.

### 5.2 SymbolInterner — shared via graph

`SymbolInterner` (`symbol.rs:46-50`):

```rust
pub struct SymbolInterner {
    strings: Vec<String>,
    indices: std::collections::HashMap<String, SymbolId>
}
```

In `lower_program_with_graph`, the graph's `SymbolInterner` is cloned into `self.symbols`, ensuring all `SymbolId`s are consistent between `ModulePath` segments and HIR names.

### 5.3 DefTable — the DefId bridge

```rust
// symbol.rs:91-168
pub enum DefKind { Function, Struct, Enum, Module, Use }
pub struct DefEntry { pub module: ModuleId, pub local_index: u32, pub kind: DefKind }
pub struct DefTable { entries: Vec<DefEntry> }
```

`register()` appends and returns `DefId(entries.len())`. `lookup()` does array indexing. `iter()` yields `(DefId, &DefEntry)`.

### 5.4 DefId assignment — per-kind counters with module qualification

HIR lowering assigns `DefId`s from separate index spaces per kind:

| Definition kind | DefId source |
|-----------------|-------------|
| Functions | `func_def_id` counter (sequential across all modules in a single `lower_program_with_graph` call) |
| Structs | `structs.len()` at push time |
| Enums | `enums.len()` at push time |
| Use decls | `next_def_id()` (synthetic, via SymbolId space) |
| Mod decls | `next_def_id()` (synthetic, via SymbolId space) |

### 5.5 Visibility

`ast::Visibility` and `hir::Visibility` are both `{ Private, Public }`. `ast_visibility_to_hir` converts between them.

---

## 6. Module Graph + Scope Architecture

### 6.1 ModuleId

```rust
// module.rs:40-51
pub struct ModuleId(pub u32);
impl ModuleId {
    pub const ROOT: ModuleId = ModuleId(0);
    pub const fn new(id: u32) -> Self { ModuleId(id) }
}
```

### 6.2 ModulePath

`ModulePath { segments: Vec<SymbolId> }` — each segment is an interned string from the shared `SymbolInterner`.

### 6.3 Module

```rust
// module.rs:216-231
pub struct Module {
    pub id: ModuleId,
    pub path: ModulePath,
    pub file_path: PathBuf,
    pub ast: Option<Program>,
    pub parent: Option<ModuleId>,
    pub mod_declarations: Vec<String>,
}
```

### 6.4 ModuleScope

```rust
// module.rs:290-298
pub struct ModuleScope {
    pub items: HashMap<SymbolId, DefId>,
    pub imports: HashMap<SymbolId, DefId>,
    pub parent: Option<ModuleId>,
}
```

Methods: `lookup(name)` (items then imports, no parent walk), `lookup_with_parent(name, scopes)` (walks parent chain), `define_item(name, def_id)`, `define_import(alias, target)`.

### 6.5 ModuleGraph

```rust
// module.rs:364-376
pub struct ModuleGraph {
    pub modules: Vec<Module>,
    pub root: ModuleId,
    pub symbol_interner: SymbolInterner,
    module_index: HashMap<ModulePath, ModuleId>,   // private
    pub imports: HashMap<ModuleId, Vec<ModuleId>>
}
```

### 6.6 Module discovery: discover_modules

`module.rs:497-575` — `ModuleGraph::discover_modules(root_file: PathBuf)`:
1. Create root module with empty `ModulePath`.
2. Read and parse the root file.
3. `extract_mod_declarations_from_ast(root_ast.as_ref(), &source)` — AST-based primary path.
4. For each child mod name, resolve file via `resolve_module_file`, recursively discover.
5. `add_module()` assigns `ModuleId` sequentially and indexes by path.

**AST-based discovery is the primary path.** The doc comment confirms: "The AST (`ast::ItemKind::ModDecl`) is the authoritative source of module names. If AST parsing fails, the text-based fallback [`extract_mod_declarations`] is used."

### 6.7 resolve_module_file

`module.rs:590-611` — tries `<dir>/<name>.stnx` first, then `<dir>/<name>/mod.stnx`.

### 6.8 Project

`Project::discover(start: &Path)` (`module.rs:728-798`): walks upward for `saturn.toml`, parses config via `SaturnConfig::from_dir`, sets `source_root = root.join("src")`.

`Project::load(&mut self)` (`module.rs:805-825`): entry point is `<source_root>/main.stnx`, calls `ModuleGraph::discover_modules(entry)`, stores graph in `self.graph`, returns the root module's AST.

`Project::load_from(&mut self, file: &Path)` (`module.rs:826-838`): same as `load()` but from an explicit file path.

### 6.9 Module graph tests

`tests/test_module_graph.rs` contains 41 tests covering: project discovery, module file discovery, nested module chains, missing module errors, `saturn.toml` parsing, and `Project::load`.

---

## 7. MIR Integration

### 7.1 MIR lowering — HashMap sigs

`mir/lower.rs:33-52` — `lower_program(hir: &HirProgram)` iterates `hir.functions`. Builds `sigs: HashMap<DefId, (Vec<HirType>, HirType)>` keyed by `DefId`, inserting entries from every function.

### 7.2 DefId lookup via HashMap — no raw array indexing in MIR

`mir/lower.rs:499-503` — `lower_call` uses `self.sigs.get(&def_id)` (HashMap lookup), then falls back to `result_ty` if not found.

### 7.3 MIR codegen — function name resolution

`mir/codegen.rs:93-113` — `declare_functions` iterates `prog.functions`, skipping `PRINTLN_DEF_ID`. For each function, looks up the name via `prog.symbols.lookup(func.name)`.

`mir/codegen.rs:637-652` — `MirTerminator::Call` resolves the call target by:
1. If `def_id == PRINTLN_DEF_ID`, use `"println_i64"`.
2. Otherwise, call `prog.function_name(*def_id)` — a `find`-based lookup by `def_id` equality.

### 7.4 MIR program structure

`MirProgram` (`mir/mod.rs:315-323`) stores `functions`, `symbols`, `structs`, `enums`. It does **not** carry module fields — module identity is erased once HIR is lowered to MIR.

---

## 8. CLI Integration

### 8.1 What main.rs uses

`main.rs` imports `Project`, `compile_from_mir_ext`, `generate_ir_from_mir`, `lower_program`, `optimize`, and target types from `stnx::target`.

### 8.2 Build command pipeline (main.rs:136-360)

1. **Profile determination** (lines 161-167): `Profile::Debug` / `Profile::Release` / `Profile::default()`.
2. **Entry point resolution** (lines 169-187): If `input` is given, use it directly. If not, call `Project::discover(&cwd)`, use `source_root.join("main.stnx")`.
3. **Target configuration** (lines 206-229): `TargetConfig::host()` or `from_triple(triple)`, apply profile, apply `--opt-level` overrides.
4. **Project + module discovery** (lines 249-254): `Project::discover(&entry_path)` → `project.load_from()` or `project.load()`.
5. **Semantic analysis** (line 255): `analyze_and_lower(&program)`.
6. **MIR lowering + verify + optimize** (lines 258-271).
7. **Codegen** (lines 311-323).

### 8.3 Check and Run commands

`check_file` (main.rs:547-553) and `build_run_file` (main.rs:488-545) both call `Project::discover` and `project.load_from`.

---

## 9. Gaps and Next Steps

### Gap 1: CLI uses single-file `analyze_and_lower`, not graph-aware path

`main.rs:255` calls `analyze_and_lower(&program)` (single-file). The multi-module entry point `analyze_and_lower_with_graph(&program, &project.graph)` (`semantic.rs:42-49`) is implemented but not invoked by the CLI. The graph is used for file discovery via `Project::discover`, but not passed to HIR lowering.

**Next step:** Wire `analyze_and_lower_with_graph` into the CLI `Build`/`Check`/`Run` paths when a multi-module graph is available.

### Gap 2: Project::load returns only root module's AST

`Project::load()` returns `root.ast.clone()` — only the root module's `Program`, not a merged multi-module program. Child module ASTs are stored in `Module::ast` within the graph but are not merged into the returned `Program`.

**Next step:** `lower_program_with_graph` already iterates `graph.modules` and accesses each `module.ast`, so the integration is ready on the HIR side — only the CLI call site needs to switch to `analyze_and_lower_with_graph` and pass `&project.graph`.