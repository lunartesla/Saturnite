# Saturnite Project/Build Architecture — Current State (Phase 3–9 Audit)

**Status:** Audit document — describes the ACTUAL state of the codebase as of Phase 9.
**Scope:** How `saturn.toml` and the CLI discover, scope, and compile Saturnite projects; module discovery and file mapping; package identity; the compilation pipeline; and where the module system is integrated.
**Constraint note:** This is a documentation-only audit. No compiler source code was modified.

---

## 1. Project model and configuration

### What exists on disk

The repository root (`/home/dimitar/saturnite/Saturnite/saturn.toml`) currently contains:

```toml
[package]
name = "myproject"
version = "0.1.0"
edition = "2026"

[dependencies]
saturnite-stdlib = "0.1"
```

This matches the schema in `config.rs`.

### The config data model (`crates/stnx/src/config.rs`)

`SaturnConfig` is the deserialized root of `saturn.toml`:

```rust
pub struct SaturnConfig {
    #[serde(default)]
    pub package: Package,
    #[serde(default)]
    pub dependencies: BTreeMap<String, DependencySpec>,
}
```

`SaturnConfig` provides three constructors:

| Method | Location | Behavior |
|---|---|---|
| `from_dir(dir)` | `config.rs:41-58` | Reads `saturn.toml` from `dir`. If absent, synthesizes a minimal config using the directory name as the package name (delegates to `from_name`). |
| `from_toml_str(contents)` | `config.rs:61-64` | Parses a TOML string via `toml::from_str::<SaturnConfig>`. |
| `from_name(name)` | `config.rs:67-77` | Produces a TOML string with `[package]` `name`, `version = "0.1.0"`, `edition = "2026"` and parses it. |

`Package` (`config.rs:81-92`):

| Field | Type | Default / Notes |
|---|---|---|
| `name` | `String` | Required by the TOML schema. `Default` impl uses `"untitled"`. |
| `version` | `String` | `"0.1.0"` (via `#[serde(default = "default_version")]`) |
| `edition` | `String` | `"2026"` (via `#[serde(default = "default_edition")]`) |

`Package` uses `#[serde(deny_unknown_fields)]` — unknown keys produce a parse error.

`DependencySpec` (`config.rs:119-132`): `#[serde(transparent)]` over a single `version: String` field. `FromStr` just clones the string.

### What is NOT in the schema

There is no `[lib]`, `[[bin]]`, `[source]`, or `[profile]` section in `SaturnConfig`. The config struct has only `package` and `dependencies`.

---

## 2. Project root discovery mechanism

### What exists (`crates/stnx/src/module.rs`)

`Project::discover(start: &Path)` (`module.rs:728-798`) implements root discovery by walking upward for `saturn.toml`:

1. If `start` is a file, begin from its parent directory.
2. Check each directory (and its ancestors) for `saturn.toml`.
3. The first directory containing `saturn.toml` is the **project root**.
4. The source root is `<root>/src/`.
5. If no `saturn.toml` is found, synthesize a config from the starting directory name (`SaturnConfig::from_name`) and use the starting directory as the project root.
6. The `ModuleGraph` is initialized empty (discovery happens in `load()` / `load_from()`).

This mirrors Cargo's `Cargo.toml` root-finding.

### `Project` struct (`module.rs:718-731`)

```rust
pub struct Project {
    pub config: SaturnConfig,
    pub root: PathBuf,       // directory containing saturn.toml
    pub source_root: PathBuf, // typically <root>/src/
    pub graph: ModuleGraph,   // starts empty; populated by load()
}
```

### `Project::load()` and `Project::load_from()` (`module.rs:805-838`)

- `load()`: entry point is `<source_root>/main.stnx`. Calls `ModuleGraph::discover_modules(entry)` and returns the root module's AST (`Program`). The full graph is stored in `self.graph`.
- `load_from(file)`: same as `load()` but from an explicit file path.

Both return the root module's AST and populate `Project::graph` with the full module graph (all discovered modules with their ASTs).

### CLI usage: `Project::discover()` IS called from `main.rs`

This is the key correction from the Phase 3 audit. `main.rs` imports `Project` (`main.rs:4`) and calls it in all three commands:

```rust
use stnx::module::Project;
// main.rs:4
```

- **Build command** (`main.rs:176, 249`): When no input file is given, calls `Project::discover(&cwd)` to find the project root and default entry point. Then calls `Project::discover(&entry_path)` again and `project.load_from()` or `project.load()`.
- **Check command** (`main.rs:368`): Calls `Project::discover(&cwd)` when no input given.
- **Run command** (`main.rs:494`): `build_run_file` calls `Project::discover(input)`.

The `Init` command writes `saturn.toml` and `src/main.stnx` (`main.rs:557-613`).

`Project`, `ModuleGraph`, `ModulePath`, `ModuleId`, `ModuleScope` are re-exported from `lib.rs` (`lib.rs:83`) and used by the CLI.

---

## 3. Module discovery and graph construction

### Types (`crates/stnx/src/module.rs`)

| Type | Purpose |
|---|---|
| `ModuleId(u32)` | Stable module identity, separate from `DefId`. `ROOT = ModuleId(0)`. |
| `ModulePath` | A `Vec<SymbolId>` — interned path segments. |
| `Module` | `{ id, path, file_path, ast: Option<Program>, parent, mod_declarations }`. |
| `ModuleScope` | Per-module namespace: `{ items: HashMap<SymbolId, DefId>, imports: HashMap<SymbolId, DefId>, parent: Option<ModuleId> }`. |
| `ModuleGraph` | `{ modules: Vec<Module>, root: ModuleId, symbol_interner: SymbolInterner, module_index, imports }`. |

### Discovery algorithm (`ModuleGraph::discover_modules`, `module.rs:497-575`)

1. Create the root module from `root_file` with an empty `ModulePath`.
2. Read the root file's source text.
3. Call `parse_source(&source)` to parse to AST (best-effort, failures non-fatal).
4. Call `extract_mod_declarations_from_ast(root_ast.as_ref(), &source)` to get `Vec<String>` of module names — **AST-based primary path**.
5. `add_module()` assigns `ModuleId` sequentially and indexes by path.
6. Iteratively process a worklist: for each `mod_name` in the module's `mod_declarations`, resolve file via `resolve_module_file`, recursively discover.

### File resolution rules (`resolve_module_file`, `module.rs:590-611`)

Given a directory and module name:
1. `<dir>/<name>.stnx` — single file form.
2. `<dir>/<name>/mod.stnx` — directory module form.

### AST-based module extraction (`extract_mod_declarations_from_ast`, `module.rs:613-632`)

This is the **primary** discovery path. It walks `Program::items`, filtering for `ItemKind::ModDecl` items, and collects each item's `name`. If the AST is `None` (parse failure), it falls back to the text-based `extract_mod_declarations` scanner on the raw source.

### Text-based fallback scanner (`extract_mod_declarations`, `module.rs:641-662`)

Used only when AST parsing fails. Line-by-line scan for `mod <ident>` or `pub mod <ident>` patterns.

### ModuleGraph::discover_modules returns the full graph

The graph contains all discovered modules with their ASTs. `Project::load()` returns only the root module's AST (`root.ast.clone()`), but the full graph with all child module ASTs is stored in `Project::graph`.

---

## 4. Compilation pipeline (current path)

The actual end-to-end compilation pipeline as it exists today, exercised by all three CLI commands (`Build`, `Run`, `Check`):

```
Source file (.stnx)
    -> Project::discover (walks up for saturn.toml)
    -> Project::load() / load_from(file) (runs ModuleGraph::discover_modules)
    -> Lexer  (stnx::lexer::Lexer)
    -> Parser (stnx::parser::parse -> ast::Program)
    -> HIR    (stnx::semantic::analyze_and_lower -> hir::lower::lower -> HirProgram)
    -> MIR    (stnx::mir::lower::lower_program -> MirProgram)
    -> MIR verify (MirProgram::verify)
    -> MIR optimize (stnx::mir::opt::optimize)
    -> LLVM IR + object emission + linking (stnx::mir::codegen::compile_from_mir_ext)
    -> Executable (.o + system linker + libsaturnite_runtime.a)
```

### Detailed code path (Build command, `main.rs:136-360`)

1. **Profile determination** (lines 161-167): `Profile::Debug` → `OptimizationLevel::None` + `DebugInfo::Yes`; `Profile::Release` → `OptimizationLevel::Aggressive` + `DebugInfo::No`. Applied via `config.apply_profile(profile)`.
2. **Entry point resolution** (lines 169-187): If `input` is `Some`, use it directly. If `None`, call `Project::discover(&cwd)`, use `source_root.join("main.stnx")`, and extract `package_name` from `project.config.package.name`.
3. **Target configuration** (lines 206-229): `TargetConfig::host()` or `TargetConfig::from_triple(triple)`. Apply profile, then apply `--opt-level` overrides.
4. **Project + module discovery** (lines 249-254): `Project::discover(&entry_path)` → `project.load_from(&entry_path)` (if input given) or `project.load()` (if discovered from project root).
5. **Source processing**: `Project::load_from()` / `load()` returns the root module's AST (already parsed internally by `discover_modules`).
6. **Semantic analysis**: `stnx::semantic::analyze_and_lower(&program)` → `hir::lower::lower` → `HirProgram`.
7. **MIR lowering**: `lower_program(&hir)` iterates `hir.functions`, builds `sigs: HashMap<DefId, (Vec<HirType>, HirType)>`.
8. **MIR verification**: `mir.verify()` — checks CFG integrity.
9. **MIR optimization**: `optimize(&mut mir)` — constant folding (`mir/opt.rs:16-20`).
10. **Code generation**: Either `generate_ir_from_mir(&mir)` (writes `.ll` text) or `compile_from_mir_ext(&mir, path, config, save_temps)` (`mir/codegen.rs:774-841`).

### `compile_from_mir_ext` flow (`mir/codegen.rs:774-841`)

1. Create LLVM context, `MirCodeGenContext`, declare builtin functions (`println_i64`), declare all MIR functions, generate LLVM IR for each function.
2. Set module triple from `target_config.triple()`.
3. If optimization level is non-None: create target machine, run LLVM pass pipeline via `opt_pass_name()`.
4. Based on `target_config.output_kind()`:
   - `OutputKind::Ir`: write LLVM IR text via `module.print_to_file()`.
   - `OutputKind::Object`: emit object file via `ObjectEmitter::new(&module, &target_config).emit_object()`.
   - `OutputKind::Exe`: emit object to `<stem>.o`, then `Linker::new(&target_config).link(&obj_path, output_path)`, then optionally delete the `.o` (unless `save_temps`).
5. The linker (`crates/stnx/src/codegen/linker.rs`) invokes the system `cc`/`clang`/`gcc`/`link.exe` and links against `libsaturnite_runtime.a` (the C runtime compiled by `build.rs` from `runtime/println_i64.c`).

### Check command (`main.rs:363-381`)

`check_file(&entry)` (`main.rs:547-553`): calls `Project::discover(input)`, `project.load_from(input)`, then `stnx::semantic::analyze(&program)`. No codegen.

### Run command (`main.rs:383-436`)

Calls `build_run_file(&input, &tmp_output, target, profile)` (`main.rs:488-545`), which calls `Project::discover(input)`, `project.load_from(input)`, `analyze_and_lower`, `lower_program`, verify, optimize, and `compile_from_mir_ext` to a temp directory, then executes the result.

### Build script (`crates/stnx/build.rs`)

Compiles `runtime/println_i64.c` into `libsaturnite_runtime.a` via the `cc` crate, targeting the host platform only. The archive is linked by `Linker::link()` during the `OutputKind::Exe` path.

### Runtime (`crates/stnx/runtime/println_i64.c`)

A single C function `println_i64(long long)` — the only runtime builtin. Called from MIR codegen when the `PRINTLN_DEF_ID` sentinel (`DefId(u32::MAX - 1)`) is encountered. The `DefId` sentinel is declared in both `hir/lower.rs:43` and `mir/lower.rs:30` and `mir/codegen.rs:27`, and the name `"println_i64"` is declared as an LLVM function in `mir/codegen.rs:93-97`.

---

## 5. CLI integration

### What `main.rs` uses

| Concern | Used from `main.rs`? | Source |
|---|---|---|
| `TargetConfig` | Yes — `TargetConfig::host()`, `TargetConfig::from_triple()`, `apply_profile()`, `set_*()` setters, `triple()`, `opt_pass_name()` | `target.rs`, `main.rs:206-229` |
| `Profile` | Yes — `Profile::Debug`/`Release`/`default()`, `as_str()`, passed to `config.apply_profile()` | `target.rs:57-93`, `main.rs:161-167` |
| `OutputKind` | Yes — `resolve_output()` returns it; `config.set_output_kind()` | `target.rs:95-101`, `main.rs:232-241` |
| `DebugInfo` | Yes — set via `set_debug_info()` for `--opt-level 0` | `target.rs`, `main.rs:218` |
| `OptimizationLevel` | Yes — set via `set_opt_level()` for `--opt-level` overrides | `target.rs`, `main.rs:216-222` |
| `SaturnConfig` / `Package` / `DependencySpec` | No — not directly, but `Project::discover` reads `saturn.toml` via `SaturnConfig::from_dir`. `package_name` is extracted from `project.config.package.name`. | `config.rs`, `main.rs:184` |
| `Project` / `ModuleGraph` / `ModulePath` / `ModuleId` / `ModuleScope` | Yes — `Project::discover()`, `project.load()`, `project.load_from()`, `project.source_root`, `project.config.package.name` | `module.rs`, `main.rs:4, 176, 249, 368, 494, 548` |
| `compile_from_mir_ext` | Yes — `main.rs:320` (Build and Run paths) | `mir/codegen.rs:774` |
| `generate_ir_from_mir` | Yes — `main.rs:313` (IR emit path) | `mir/codegen.rs:752` |
| `lower_program` | Yes — `main.rs:259, 500` | `mir/lower.rs:33` |
| `optimize` | Yes — `main.rs:271, 510` | `mir/opt.rs:16` |
| `codegen::host_triple` | Yes — `main.rs:154, 281, 340, 524` | `codegen` module |
| `CompilerError` | Yes — `render_diagnostic` pattern-matches on it | `error.rs`, `main.rs:674` |

### What `main.rs` does

- **Does call `Project::discover()`** — in `Build` (line 176 for default entry, line 249 for explicit), `Check` (lines 368, 548), and `Run` (line 494).
- **Does read `saturn.toml`** — via `Project::discover()` → `SaturnConfig::from_dir()`.
- **Does construct a `ModuleGraph`** — via `Project::load()` / `load_from()` → `ModuleGraph::discover_modules()`.
- **Does default to `src/main.stnx`** — when no input file is given, uses `project.source_root.join("main.stnx")`.
- **Uses `Profile`** — from `stnx::target::Profile`, applies via `config.apply_profile(profile)`.

### `resolve_output` (`main.rs:446-482`)

Uses the package name (from `Project::config.package.name`) or input file stem for the output name. Default: `target/<debug|release>/<name>` for executables, `target/<debug|release>/<name>.o` for objects with `--no-link`.

### Input file is now optional

The `input` field on `Build`, `Check`, and `Run` is `Option<PathBuf>` — when `None`, the CLI discovers the project and uses `src/main.stnx` as the default entry point.

---

## 6. Module system integration points

### The Build command pipeline (`main.rs:136-360`)

```
Project::discover(entry_path) -> load_from/load -> analyze_and_lower -> lower_program -> verify -> optimize -> compile_from_mir_ext
```

The module graph IS discovered during `Project::load()` / `load_from()` (which calls `ModuleGraph::discover_modules`). The root module's AST is returned and passed to `analyze_and_lower`. **However**, `analyze_and_lower` calls `hir::lower::lower()` (single-file path), not `analyze_and_lower_with_graph` (multi-module path). The graph is available on the `Project` but is not passed to HIR lowering.

### What `HirProgram` carries

`HirProgram` has module-aware fields: `modules`, `root_module`, `module_paths`, `def_table`, `module_scopes`, `use_decls`, `mod_decls`. For single-file programs (using `lower_program`), all are populated with ROOT-only placeholders. For multi-module programs (using `lower_program_with_graph`), the graph's modules and scopes are used.

### HIR lowering does NOT use the graph in the current CLI path

`main.rs:255` calls `analyze_and_lower(&program)` which calls `hir::lower::lower()` → `HirLower::lower_program()`. This takes a single `&Program` and does NOT accept a `ModuleGraph`. The `lower_program_with_graph` method exists at `hir/lower.rs:518` for multi-module, and `semantic::analyze_and_lower_with_graph` at `semantic.rs:42` exposes it, but neither is called by the CLI.

### MIR lowering discards module metadata

`mir/lower.rs:33-52` reads only `hir.functions`, `hir.structs`, `hir.enums`, and `hir.symbols`. It builds `sigs: HashMap<DefId, (Vec<HirType>, HirType)>` (not `Vec`). `MirProgram` (`mir/mod.rs:315-323`) stores `functions`, `symbols`, `structs`, `enums` — no module fields.

### DefId lookup in MIR — uses HashMap, not array indexing

`mir/lower.rs:499-503` — `lower_call` uses `self.sigs.get(&def_id)` (HashMap lookup by `DefId` equality), then falls back to `result_ty` if not found. This is robust to non-sequential DefIds.

### Two SymbolInterner instances

`HirLower::new()` creates a fresh `SymbolInterner` (`hir/lower.rs:141-145`). `ModuleGraph` has its own (`module.rs:370`). In `lower_program_with_graph`, the graph's interner is cloned into `self.symbols` (`hir/lower.rs:528`), unifying them. But in the CLI's current path (`analyze_and_lower` → `lower_program`), the graph's interner is never connected to HIR.

### MIR codegen function resolution

`mir/codegen.rs:643-652` resolves calls via `prog.function_name(*def_id)` (find-based, not indexing) or the `PRINTLN_DEF_ID` sentinel. `declare_functions` (`mir/codegen.rs:100-113`) iterates `prog.functions`, skipping `PRINTLN_DEF_ID`.

---

## 7. Where single-file and multi-module paths diverge

### Single-file compilation (actual, working)

```
main.rs Build command
    -> Project::discover(entry_path)
    -> project.load_from() or project.load()
        -> ModuleGraph::discover_modules(entry)
            -> returns full graph (root + children with ASTs)
            -> returns root.ast as Program
    -> Lexer::new(src)
    -> parser::parse(src, tokens)     // -> ast::Program
    -> semantic::analyze_and_lower(program)
        -> hir::lower::lower(program)
            -> HirLower::new()        // fresh SymbolInterner
            -> lower_program(program)
                -> all DefIds in ROOT module
                -> module fields = single-element vecs
    -> lower_program(&hir)            // -> MirProgram
    -> optimize(&mut mir)
    -> compile_from_mir_ext(&mir, path, config, save_temps)
```

### What the module system provides but the CLI does NOT yet use for lowering

```
Project::discover(entry_path)        // -> Project{config, root, source_root, graph}
Project::load()                       // -> root.ast, populates self.graph with all modules
    -> ModuleGraph::discover_modules(entry)
        -> AST-based: extract_mod_declarations_from_ast(ast, source)
        -> fallback: extract_mod_declarations(source) (text scan)
        -> resolve_module_file: <dir>/foo.stnx or <dir>/foo/mod.stnx
        -> recursive BFS over all child modules
        -> each Module.ast = parse_source(child)  (Option<Program>)
```

The module system discovers all modules and their ASTs, and the graph is stored in `Project::graph`. But `analyze_and_lower` (single-file) is used instead of `analyze_and_lower_with_graph` (multi-module), so child module ASTs in the graph are not yet lowered into HIR.

---

## 8. Gaps and next steps

### Gap 1: CLI does not pass ModuleGraph to HIR lowering

`main.rs:255` calls `analyze_and_lower(&program)` (single-file path). The multi-module entry point `analyze_and_lower_with_graph(&program, &project.graph)` is implemented in `semantic.rs:42-49` and `hir/lower.rs:518-935` but is not invoked by the CLI.

**Next step:** Wire `analyze_and_lower_with_graph` into the CLI `Build`/`Check`/`Run` paths when a multi-module graph is available.

### Gap 2: Project::load returns only root module's AST

`Project::load()` returns `root.ast.clone()` — only the root module's `Program`. Child module ASTs are stored in `Module::ast` within the graph but are not merged into the returned `Program`.

**Note:** `lower_program_with_graph` already iterates `graph.modules` and accesses each `module.ast` (`hir/lower.rs:589-591`), so the HIR side is ready — only the CLI call site needs to switch to `analyze_and_lower_with_graph` and pass `&project.graph`.

### Gap 3: No `[lib]`/`[[bin]]`/`[source]` config schema

`SaturnConfig` has no fields for library or binary targets, no source root override, and no profile configuration. `resolve_output()` in `main.rs` derives the output name from the package name (when available from `saturn.toml`) or the input file stem.

### Gap 4: Two-stage module loading (discovery then lowering)

Currently discovery (`discover_modules`) happens in `Project::load()` / `load_from()`, returning only the root AST. Lowering happens separately via `analyze_and_lower`. For full multi-module support, the discovered `ModuleGraph` (with all child ASTs) must be passed to `analyze_and_lower_with_graph`.