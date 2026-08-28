# SATURNITE — ACTUAL ARCHITECTURE (FORENSIC)

> Source-only map of the Saturnite 0.4 compiler at commit
> `35f6132103897be4bcf88d2bd1cdc28425d5b9ca`. Every claim is backed by
> a file path and a line number or symbol. Generated as part of the
> 2026-08-28 Saturnite×Rust forensic audit.

---

## 0. Repository metadata

| Property | Value | Source |
|---|---|---|
| Path | `/home/dimitar/Saturnite` | working tree |
| Git commit | `35f6132` | `git log` |
| Crate version | `0.1.0` (stnx) — user-facing 0.4 | `crates/stnx/Cargo.toml:5`, git tag `bea7f57` |
| Rust edition | `2021` | `crates/stnx/Cargo.toml:4` |
| License | `MIT OR Apache-2.0` (project); runtime/MIT (Dimitar.Simovski 2026) | `LICENSE`, `Cargo.toml:8-9` |
| Workspace members | `crates/stnx` only | root `Cargo.toml:2-3` |
| Author | Dimitar.Simovski | `LICENSE:2` |

---

## 1. Top-level layout

```
/home/dimitar/Saturnite/
├── Cargo.toml                  # workspace manifest
├── Cargo.lock                  # 28 KB
├── LICENSE                     # MIT, Dimitar.Simovski 2026
├── README.md                   # 6 KB, design + CLI summary
├── saturn.toml                 # the example project's config
├── .gitmodules                 # (none)
├── docs/                       # prior audits and design docs
├── crates/
│   └── stnx/                   # the compiler crate
│       ├── Cargo.toml
│       ├── build.rs            # compiles runtime/println_i64.c via cc
│       ├── runtime/
│       │   └── println_i64.c   # the one runtime function
│       ├── examples/
│       │   ├── hello.stn
│       │   └── smoke_test.stnx
│       ├── tests/              # integration tests
│       │   └── common/         # shared test helpers
│       └── src/
│           ├── lib.rs          # 84 lines, public API surface
│           ├── main.rs         # 718 lines, CLI dispatch
│           ├── ast.rs          # 238 lines, AST types
│           ├── config.rs       # 222 lines, saturn.toml types
│           ├── error.rs        # 158 lines, thiserror+miette errors
│           ├── module.rs       # 1516 lines, module graph + project loader
│           ├── semantic.rs     # 53 lines, thin façade over HIR lowering
│           ├── target.rs       # 481 lines, target / profile config
│           ├── lexer/          # 423 lines
│           │   ├── mod.rs
│           │   └── token.rs
│           ├── parser/         # 1456 lines
│           │   └── mod.rs
│           ├── hir/            # 3205 lines
│           │   ├── mod.rs
│           │   ├── expr.rs
│           │   ├── function.rs
│           │   ├── lower.rs
│           │   ├── stmt.rs
│           │   ├── symbol.rs
│           │   └── types.rs
│           ├── mir/            # 2284 lines
│           │   ├── mod.rs
│           │   ├── codegen.rs
│           │   ├── lower.rs
│           │   ├── opt.rs
│           │   └── verify.rs
│           └── codegen/        # 277 lines
│               ├── mod.rs
│               ├── emitter.rs
│               └── linker.rs
```

Total Rust LOC: **11 115** in `src/` (per `wc -l` 2026-08-28).

There are no submodules (`.gitmodules` has no entries). There is **one** runtime source file
(`runtime/println_i64.c`), compiled at build time.

---

## 2. Pipeline (call chain, top to bottom)

```
file.stn
  └─→ CLI dispatch                     src/main.rs:Commands enum
        └─→ lex(src)                   src/lexer/mod.rs (logos)
              └─→ parse(tokens)        src/parser/mod.rs (chumsky 0.13)
                    └─→ Program (AST)  src/ast.rs
                          └─→ analyze_and_lower_with_graph  src/semantic.rs:42
                                ├─→ lower_with_graph         src/hir/lower.rs
                                └─→ resolve_modules          src/hir/lower.rs
                                      └─→ HirProgram          src/hir/function.rs
                                            └─→ lower_program  src/mir/lower.rs
                                                  └─→ MirProgram src/mir/mod.rs
                                                        └─→ optimize       src/mir/opt.rs
                                                        └─→ MirProgram::verify  src/mir/verify.rs
                                                              └─→ compile_from_mir_ext  src/mir/codegen.rs
                                                                    ├─→ inkwell LLVM IR
                                                                    └─→ ObjectEmitter       src/codegen/emitter.rs
                                                                          └─→ Linker           src/codegen/linker.rs
                                                                                └─→ exec
```

Concrete handoff citations (call graph, file:line of the call site):
- `main.rs:259` — `lower_program(&hir)`
- `main.rs:497-500` — `analyze_and_lower` (check subcommand)
- `main.rs:517` — `TargetConfig::host()`
- `main.rs:321` — `compile_from_mir_ext(...)`
- `mir/codegen.rs` — `compile_from_mir_ext` calls `compile_from_mir` → inkwell LLVM IR
- `mir/codegen.rs` then constructs `ObjectEmitter` and `Linker`

---

## 3. Lexer

| Property | Value | Source |
|---|---|---|
| Implementation | `logos = "0.16"` | `lexer/mod.rs:7` |
| Token kind enum | `LexicalToken` (logos internal) + `TokenKind` (consumer-facing) | `lexer/mod.rs:11`, `lexer/token.rs:5` |
| Spans | `Range<usize>` byte offsets | `lexer/token.rs:69-72` |
| Skipped | whitespace + `//` line comments | `lexer/mod.rs:10` |
| Number of keywords | 23 (`fn`, `let`, `mut`, `if`, `elif`, `else`, `for`, `while`, `in`, `return`, `i64`, `f64`, `bool`, `str`, `unit`, `true`, `false`, `println`, `struct`, `enum`, `mod`, `use`, `pub`, `as`) | `lexer/mod.rs:12-66` |
| String literal | basic `"..."` (no escape decoding beyond strip-quotes) | `lexer/mod.rs:91-95` |
| Integer regex | `[0-9]+` → `String` (parsed in `parser`) | `lexer/mod.rs:84-85` |
| Float regex | `[0-9]+\.[0-9]+` | `lexer/mod.rs:86-88` |
| Errors | `LexError` (`thiserror + miette`) in `error.rs:8-30` | emitted by parser pipeline only — logos' own errors are not propagated as `LexError` yet |

Span type is **byte range**, not chumsky's `SimpleSpan` — the lexer
emits raw byte ranges; the parser re-spans tokens into chumsky spans
(`parser/mod.rs:16-31` converts token index span → byte span).

The lexer is **small, span-carrying, and lossy** (it does not preserve
trivia such as whitespace or comment positions). This is the most
plausible subsystem for **architectural adoption of `rustc_lexer`** if
any future refactor seeks Rust-Project source code; `rustc_lexer` is
itself an MIT/Apache-2.0 standalone crate that produces (kind, &str)
pairs without spans — see Phase 5.

---

## 4. Parser

| Property | Value | Source |
|---|---|---|
| Implementation | `chumsky = "0.13"` (with `memoization`) | `Cargo.toml:18` |
| Entry | `pub fn parse(src: &str, tokens: Vec<Token>) -> CompilerResult<Program>` | `parser/mod.rs:35` |
| Output | `ast::Program { items, functions }` | `ast.rs:23-35` |
| Span model | Chumsky `SimpleSpan<usize>`, converted to `Range<usize>` byte spans in the AST | `parser/mod.rs:60-67` |
| Error reporting | First parse error wins; aggregates "(plus N more)" into the message | `parser/mod.rs:60-82` |

Parser top-level (`parser/mod.rs:84-89`):

```rust
fn program<'a>() -> impl Parser<'a, &'a [Token], Program, ParserExtra<'a>> {
    item().repeated().collect::<Vec<_>>().map(Program::from_items)
}
```

The parser is **chumsky-recursive** with explicit
`chumsky::recursive::Direct` (import at `parser/mod.rs:5`). It supports
modules (`mod foo;`), use-decls (`use foo::bar;`), structs, enums, and
functions with bodies, all attached to the `Item` enum.

This is a **non-trivial custom parser**. It is not a hand-written
recursive-descent parser; it relies entirely on chumsky's combinator
infrastructure, and would be near-impossible to replace with a
mechanical port of any rustc parser. (Rust's `rustc_parse` is
hand-written recursive-descent; it does not map onto chumsky.)

---

## 5. AST

`ast.rs:1-238`. Key types:

- `Type` (`ast.rs:5-19`) — `I64 | F64 | Bool | Str | Unit | Struct(String) | Enum(String)`. User types are `String` (unresolved).
- `Program { items, functions }` (`ast.rs:23-35`).
- `Item { name, visibility, kind, span }` (`ast.rs:40-49`).
- `ItemKind` (`ast.rs:55-78`) — `Function | StructDef | EnumDef | ModDecl | UseDecl`.
- `Visibility { Private, Public }` (`ast.rs:51-53`).
- `Function`, `Stmt`, `Expr` (`ast.rs:80-238`) — every node carries a `Range<usize>`.

Notes:
- `Program` keeps both `items` and `functions` for backward compat with 0.2 codegen. (`ast.rs:23-35`).
- The `Type::Struct(String)` carries the unresolved name. **HIR**
  resolves this to `HirType::Struct(SymbolId)` (`hir/types.rs:14-26`).
- The AST is **discarded after lowering** (only used to construct HIR). It is not stored or serialized.

---

## 6. Semantic / HIR lowering

The 0.3 semantic-analysis layer is a thin façade (`semantic.rs:53
lines`) that delegates to `hir::lower`. There is no separate "type
checker" pass; type checking is performed as a side effect of
lowering.

### HIR subsystem

`hir/` (3 205 lines). Structure:

| File | Lines | Role |
|---|---|---|
| `mod.rs` | 39 | module root, re-exports |
| `expr.rs` | 118 | `HirExpr` + `HirExprKind` (resolved expressions) |
| `function.rs` | 221 | `HirFunction`, `HirProgram`, `HirModDecl`, `HirUseDecl`, `StructDef`, `EnumDef` |
| `lower.rs` | 2 531 | **the heart** — `HirLower` driver, scope chain, name resolution, type inference |
| `stmt.rs` | 54 | `HirStmt` + `HirStmtKind` |
| `symbol.rs` | 186 | `SymbolId`, `DefId`, `SymbolInterner`, `DefTable`, `Visibility` |
| `types.rs` | 56 | `HirType` (compiler-internal type enum) |

### Identifier system (critical)

`hir/symbol.rs`:

- `SymbolId(u32)` — stable id for an interned string. Used everywhere instead of `String` after lowering.
- `DefId(u32)` — stable id for a top-level definition (function, struct, enum, mod, use).
- `SymbolInterner` — `Vec<String>` + `HashMap<String, SymbolId>`. Methods: `intern`, `lookup`, `next_id`.
- `DefTable` — `Vec<DefEntry>`. Maps `DefId(i) → (ModuleId, local_index, DefKind)`.
- `DefKind` — `Function | Struct | Enum | Module | Use`.
- `Visibility` (HIR-side) — `Private | Public`.

Notable design choices:
- `DefId`s are assigned sequentially and never reused. This is the same approach rustc used historically before the Red-Green / "generation" of `DefId` (post-2019). Saturnite's flat `DefId` is the **simpler scheme**.
- `PRINTLN_DEF_ID = DefId(u32::MAX - 1)` is a hard-coded sentinel for the builtin. This is used in both `hir/lower.rs:50` and `mir/lower.rs:14`. This is a known-ugly coupling — see Phase 3 classification.
- `ModuleId` is a separate `u32` space, defined in `module.rs:50-69`. The two spaces are bridged by `DefEntry.module`.

### `HirLower`

`hir/lower.rs:130-...` — a struct holding the `SymbolInterner` and the
driver. Two-pass: Pass 1 collects all top-level items (functions,
structs, enums, use-decls, mod-decls) to populate the symbol/def
table; Pass 2 lowers function bodies with the type signature table in
hand.

The `LowerScope` is a parent-linked `HashMap<SymbolId, VarInfo>` stack
(`hir/lower.rs:70-95`). Name lookup walks up the parent chain.

A `LowerContext<'a>` (`hir/lower.rs:55-65`) bundles the function-signature
table, struct/enum registries, and an `enum_names` set used to
disambiguate `Type::Struct` references that are actually enums
(parser produces `Type::Struct` for all user types — `hir/lower.rs:107-121`).

### `HirProgram`

`hir/function.rs:66-...`:

```rust
pub struct HirProgram {
    pub functions: Vec<HirFunction>,
    pub structs:    Vec<StructDef>,
    pub enums:      Vec<EnumDef>,
    pub symbols:    SymbolInterner,
    pub modules:    Vec<Module>,         // multi-module
    pub root_module: ModuleId,
    pub module_paths: HashMap<DefId, ModuleId>,  // bridge
    pub def_table:  DefTable,
    pub module_scopes: HashMap<ModuleId, ModuleScope>,
    pub use_decls:  Vec<HirUseDecl>,
    pub mod_decls:  Vec<HirModDecl>,
}
```

The `HirProgram` **owns the symbol table**. Codegen does not
re-intern. This is structurally similar to rustc's `TyCtxt`
containing the `Interner`, but without lifetime parameters.

### `HirType`

`hir/types.rs:14-26`:

```rust
pub enum HirType {
    I64, F64, Bool, Str, Unit,
    Struct(SymbolId), Enum(SymbolId),
}
```

There is **no `ty::Ty<'tcx>` / interned type system**. The `SymbolId`
references a definition in `HirProgram.structs` / `.enums`. The trade
off: type-equality is `==` on the enum, which is cheap, but there is
no support for higher-kinded types, generics, or trait bounds.

### `HirExpr` / `HirExprKind`

`hir/expr.rs:13-50+`. Variants include: `Integer | Float | Bool |
StrLit | Unit | Variable { symbol } | Assign { symbol, value } |
AugAssign | Call { def_id, args } | Binary | Unary | If { cond,
then, else } | While | For | Range | StructLit { struct_def, fields }
| FieldAccess { local, field } | EnumCtor { enum_def, variant } |
Println`.

`HirExpr` carries `kind`, `ty: HirType`, and `span: SourceSpan`
(miette span). This is the structural analogue of rustc's
`rustc_hir::Expr` (minus the ` HirId` and the rustc-specific
generics/borrows).

---

## 7. MIR

`mir/` (2 284 lines). Structure:

| File | Lines | Role |
|---|---|---|
| `mod.rs` | 343 | MIR data model: `LocalId`, `BlockId`, `MirLocal`, `MirOperand`, `MirConst`, `MirRvalue`, `MirStmt`, `MirStmtKind`, `MirTerminator`, `MirBasicBlock`, `MirFunction`, `MirProgram` |
| `lower.rs` | 734 | HIR → MIR: explicit CFG, `MirLower` driver |
| `verify.rs` | 203 | structural CFG verifier, returns `Vec<MirVerifyError>` |
| `opt.rs` | 163 | constant-folding pass on `MirConst` |
| `codegen.rs` | 841 | MIR → LLVM IR via inkwell |

### MIR design

(Mostly from `mir/mod.rs` and `mir/lower.rs`.)

- **`LocalId(u32)`** + `MirLocal { id, ty, name, mutable }` — flat locals; no place projection. This is **simpler than rustc's MIR**, which has `Place` (base + projections) for field accesses.
- **`BlockId(u32)`** + `MirBasicBlock { id, name, stmts, terminator }` — each block has exactly one terminator.
- **Rvalues are flat** (`mir/mod.rs:182-218`):
  - `Use(MirOperand)`
  - `Binary { op, lhs, rhs }`
  - `Unary { op, operand }`
  - `StructLit { struct_def, fields }`
  - `FieldAccess { local, field }`
  - `EnumCtor { enum_def, variant }`
  - `StrLit(SymbolId)` — string literal as a global string pointer.
- **Terminators** (`mir/mod.rs:255-294`):
  - `Goto { target }`
  - `SwitchInt { scrutinee, ty, branches, else_target }` — for both `if` and `match`-like conditions.
  - `Call { func: DefId, args, destination, next }` — **calls are terminators**, so the call destination and continuation block are explicit.
  - `Return(Option<MirOperand>)`
  - `Unreachable`
- **`MirType = HirType`** — there is no parallel type system in MIR. This is documented explicitly at `mir/mod.rs:7`.

### HIR → MIR lowering

`mir/lower.rs:50-...`. Key choices:
- Block builder: `Vec<MirBasicBlock>` indexed by `BlockId.0`. `current: usize` tracks the block being built. `create_block` allocates fresh; `finish` sets a terminator.
- Per-function: `MirLower<'hir>` with `hir: &HirProgram`, `func: &HirFunction`, `sigs: &HashMap<DefId, (Vec<HirType>, HirType>)>`.
- `if` and `while` lower to `SwitchInt` + `Goto` (`mir/lower.rs:300+`).
- For loops over ranges: similar to `while` (start, end, step locals).
- Calls end the current block; the return value is placed in a destination local, control continues in `next`.

### MIR verification

`mir/verify.rs`. Checks: every `Goto`/`SwitchInt`/`Call` target is a valid `BlockId`; every block has exactly one terminator; types of operands match their use sites. Errors are returned as `Vec<MirVerifyError>` and turned into `CompilerError::codegen` at error-reporting time. **No panics.**

### MIR optimization

`mir/opt.rs`. **One pass**: `ConstantFolder` walks every `Assign` statement and folds `Binary`/`Unary` rvalues when both operands are `MirConst`. Limited to `i64`, `f64`, `bool`. Does **not** touch LLVM IR; it operates on MIR. This is the only opt pass. There is no dead-code elimination, copy propagation, inlining, GVN, or LICM on the MIR level — those are delegated to LLVM.

### MIR → LLVM

`mir/codegen.rs:841 lines`. The bulk of the codegen. It is a
per-function, per-block, per-statement/terminator walk. Backed by
`inkwell = "0.9"` with `llvm21-1-prefer-dynamic`. Constructs an
LLVM `Module`, `IRBuilder`, function `alloca`s for mutable locals
(`mir/codegen.rs:81-89` — `local_allocas: HashMap<LocalId, AllocaInfo>`),
and emits IR.

Notable details:
- **The `PRINTLN_DEF_ID` sentinel** (`mir/codegen.rs:30`) — when MIR's
  `Call.func == PRINTLN_DEF_ID`, codegen emits a call to the runtime
  function `saturnite_runtime_println_i64` (C linkage from
  `runtime/println_i64.c`).
- LLVM `FunctionType` is built from the function's `return_type`
  (`mir/codegen.rs:60-90`), not hardcoded to `i64` (this is a documented
  bug fix vs. an earlier version that hardcoded it).
- `OptimizationLevel` is mapped to `InkwellOptLevel` twice — once in
  `target.rs:228-235` (`TargetConfig::to_inkwell_opt_level`) and once in
  `mir/codegen.rs:795-810`. **Code duplication** noted in prior audit
  findings (`docs/audit-findings.md`, item 2).
- LLVM pass-manager configuration is hand-rolled: `compile_from_mir_ext`
  re-implements the level→string mapping instead of calling
  `target_config.to_inkwell_opt_level()`.

### `compile_from_mir_ext` entry

`mir/codegen.rs`. Two public entry points:

- `compile_from_mir_ext(...)` — produces `.o` or `.ll` and returns a
  `Vec<u8>` of the object. Wired to `main.rs:321`.
- `generate_ir_from_mir(...)` — emits LLVM IR text only.

---

## 8. Codegen (object emission + linking)

`codegen/` (277 lines). Thin layer over inkwell.

| File | Lines | Role |
|---|---|---|
| `mod.rs` | 36 | re-exports + `check_linker` / `host_triple` helpers |
| `emitter.rs` | 42 | `ObjectEmitter` — wraps an inkwell `Module` + `TargetMachine`; `emit_object`, `emit_ir`, `emit_ir_to_file` |
| `linker.rs` | 199 | `Linker` — system-linker invocation. Picks `cc` / `clang` / `link.exe` / `gcc` by `OperatingSystem × Environment`; locates the binary via `which`; builds args; runs `Command::new(...)`; parses exit code & stderr into `LinkError`. |

### Runtime

`runtime/println_i64.c` — a single C function:

```c
void saturnite_runtime_println_i64(long long v) { printf("%lld\n", v); }
```

Compiled at build time by `build.rs` (which uses the `cc` crate) into
a static library `libsaturnite_runtime.a` placed in `OUT_DIR`. Linked
into every Saturnite executable by `Linker::build_linker_args`.

There is **no other runtime** — no allocator, no panic handler, no
I/O. Saturnite programs that need anything beyond `println(i64)` and
return-from-main must add their own C runtime.

---

## 9. Target configuration

`target.rs:481 lines`. Independent hand-rolled target model. Not
derived from rustc's `rustc_target::spec::Target`.

Types:

- `Architecture` — `X86_64 | Aarch64 | X86 | Arm | Riscv64 | Mips | Powerpc64 | Wasm32 | Unknown` (`target.rs:9-19`).
- `OperatingSystem` — `Windows | Linux | Darwin | FreeBSD | Unknown` (`target.rs:21-27`).
- `Environment` — `Msvc | Gnu | Musl | Unknown` (`target.rs:29-35`).
- `OptimizationLevel` — `None | Less | Default | Aggressive` (default: `None`).
- `DebugInfo` — `Yes | No` (default: `No`).
- `OutputKind` — `Ir | Object | Exe` (default: `Exe`).
- `Profile` — `Debug | Release` (default: `Debug`).

`TargetConfig::host()` (lines 76-97) detects the host via
`target-lexicon` is **not** used; instead Saturnite detects the host
via `std::env::consts::{ARCH, OS, FAMILY}` strings. There is **no
JSON target-spec ingestion**; `TargetConfig::from_triple` accepts a
raw triple string and parses the arch / OS / env parts manually
(`target.rs:99-124`).

`to_inkwell_opt_level()` (`target.rs:228-235`) maps
`OptimizationLevel → inkwell::OptimizationLevel`.

---

## 10. Errors and diagnostics

`error.rs:158 lines`. One `CompilerError` enum (thiserror) with
variants `Lex | Parse | Semantic | Codegen | Target | Config |
Link | ...`. Every variant carries a `miette::SourceSpan` and a
`source_code: String` so `miette` can render a Rustc-style
diagnostic with underlines. Codes are `stnx::*` (`error.rs:9-29`).

Diagnostics are routed through the `Diagnostic` derive macro (miette),
not through a `DiagCtxt`/thread-local-emitter pattern as in rustc.

There is **no** concept of "lint" — Saturnite has no `rustc_lint`
analogue. The compiler is silent unless something fails.

---

## 11. CLI

`main.rs:718 lines`. Built on `clap` (derive). Subcommands:

| Subcommand | What it does |
|---|---|
| `Build <FILE>` | Full build; flags: `--debug`, `--release`, `--target`, `--opt-level`, `--emit-ir`, `--emit-object`, `--emit-exe`, `--no-link`, `--save-temps`, `--json`, `--verbose`, `--print-target` |
| `Run <FILE>` | Build + execute |
| `Check <FILE>` | Type & semantic check only (no codegen) |
| `Doctor` | Print environment diagnostics |

The CLI is small and human-friendly. There is **no support for
`--extern`, `--cfg`, or `--crate-type`** — the compiler is single-crate,
single-target (host only), single output mode per invocation.

---

## 12. Project system

### `saturn.toml`

`config.rs:222 lines`. Schema:

```toml
[package]
name = "..."
version = "..."
edition = "..."   # 2026 is the only recognized value currently

[dependencies]
name = "version"   # version string; not yet resolved
```

Loaded by `Project::load(...)` from the directory containing the root
`.stn` file (walks upward looking for `saturn.toml`, per
`module.rs:Project::discover_root`).

### Module graph

`module.rs:1 516 lines`. This is the largest file in the compiler.

- `ModuleId(u32)` — separate from `DefId`.
- `ModulePath { segments: Vec<SymbolId> }` — interned path.
- `Module { id, path, file_path, scope, items }` — what was found in a file.
- `ModuleScope` — per-module name→DefId table.
- `ModuleGraph { modules: Vec<Module>, root }` — the discovered graph.
- `Project { root_path, config, graph, symbols }` — top-level loader.

`Project::discover_root` (around line 1300) walks upward to find
`saturn.toml`. `Project::load` then lexes+parses the root file and
recursively follows `mod foo;` declarations to discover child modules.
File resolution: `<dir>/foo.stnx` or `<dir>/foo/mod.stnx`.

### Resolution

`hir/lower.rs:resolve_modules(...)` is a second pass that walks the
`mod` and `use` declarations after the file graph is built. It
populates `HirProgram.module_paths`, `module_scopes`, `def_table`.

There is **no** package manager. Dependencies in `saturn.toml` are
parsed but not resolved (`config.rs` only deserializes them).
`Cargo.lock` and a registry protocol are not present. The example
project's `saturn.toml` references `saturnite-stdlib = "0.1"` but
that crate is not part of the workspace.

---

## 13. Build system

`build.rs` — `crates/stnx/build.rs:54 lines`. Compiles
`runtime/println_i64.c` via the `cc` crate. Emits
`cargo:rerun-if-changed=...` for the runtime source. Fails loudly
if the runtime source is missing or no C compiler is available
(intentionally — no silent fallback to a checked-in object).

The compiler is built by **Cargo only**; there is no `x.py` /
bootstrap / configure script. Release / debug selection is
standard `cargo build --release`.

---

## 14. Tests

`crates/stnx/tests/` (saturnite also has a top-level `tests/`
directory, but it is empty / placeholder). The integration tests
are under `crates/stnx/tests/`. They use `tempfile = "3"` to
isolate build outputs. There is no `compiletest` / `compiletest_rs`
analogue — the test strategy is: compile a `.stn` sample, run the
resulting binary, compare stdout.

There are no UI tests, no snapshot tests, no property tests, no
fuzz harnesses visible in the repo.

---

## 15. Incremental / caching / serialization

**There is no incremental compilation.** Every invocation re-lexes,
re-parses, re-lowers, and re-emits. The only on-disk persistence is
the `.o` / executable written by the linker.

The MIR data model derives `Serialize, Deserialize` (`mir/mod.rs:1`),
and `HirType` and `SymbolId` / `DefId` derive the same. The intent
appears to be future incremental compilation, but no consumer reads
or writes these serialized forms yet. This is a hook for Phase 5
reuse but **not a current capability**.

There is no caching of dep-graphs, no hash-based fingerprinting, no
`Cargo.lock`-equivalent build cache.

---

## 16. Notable observations

1. **The compiler is small but production-quality.** 11 KLOC of Rust
   in `src/`, ~3 000-line MIR-to-LLVM backend, real-world targets
   (x86_64/aarch64/wasm32/more), real profile-driven optimization
   (Debug vs Release → O0 vs O3). Not a toy.

2. **The HIR / MIR split is genuine and similar in spirit to rustc's**:
   HIR owns name resolution and types; MIR owns the explicit CFG;
   LLVM IR is the codegen target. The decision to give MIR its own
   flat `LocalId` / `BlockId` (no `Place` projection) is a
   deliberate simplification that pays off in the size of the
   backend.

3. **There is no `TyCtxt`-style interned type system.** `HirType`
   is a plain `Copy` enum. The trade-off: cheap type-equality, no
   generics, no trait bounds, no higher-kinded types. Adequate for
   0.4.

4. **There is no `TyCtxt<'tcx>` analogue for context-passing.** The
   `HirProgram` is moved by value into codegen. There are no
   lifetime parameters threading interned data through the
   pipeline. This makes the code far easier to read than rustc but
   precludes incremental compilation without refactoring.

5. **The `DefId` scheme is the pre-2019 "flat" rustc scheme.** No
   generation numbers, no per-crate `CrateNum`. This is fine for
   single-crate 0.4; a future multi-crate setup will need to add a
   crate-id dimension or migrate to a `DefId` with a generation
   component.

6. **`PRINTLN_DEF_ID` is a hard-coded sentinel** used in three
   places (`hir/lower.rs:50`, `mir/lower.rs:14`, `mir/codegen.rs:30`).
   This is a known wart; documented in the prior audit findings
   (`docs/audit-findings.md`).

7. **There is no `Borrowck`, no ownership, no lifetimes.** The
   language currently supports `i64, f64, bool, str, unit, struct,
   enum, mod, use`. The `mut` keyword exists on `let` and is
   enforced at lowering, but the language does not currently do
   borrow checking — it is a simpler 0.4 design.

8. **There is no `rustc_arena` / typed arena.** `HirProgram` owns
   `Vec<StructDef>` / `Vec<EnumDef>` / `Vec<HirFunction>` directly.
   Allocations are ordinary. Performance is not yet a concern at
   the project scale (a single Saturnite program has at most a few
   dozen functions).

9. **No FFI surface for tools.** Unlike rustc, which exposes
   `rustc_public` and stable-MIR for rust-analyzer / IDE use,
   Saturnite has no public stable API beyond the binary CLI.

10. **The `module.rs` is unusually large (1 516 lines).** This is
    where multi-file projects, `Project::load`, and
    `ModuleGraph::discover` live. It is the largest single file in
    the compiler; likely a refactor candidate.

11. **The compiler is dual-licensed MIT/Apache-2.0.** Every Rust
    source file in the repo inherits this license; the
    `runtime/println_i64.c` is MIT-only (per the `LICENSE` file at
    the repo root). The `LICENSE` file declares
    "Copyright (c) 2026 Dimitar.Simovski" — single-copyright
    authorship; no per-file copyright headers.

12. **The compiler uses `chumsky` (parser) and `logos` (lexer) and
    `inkwell` (LLVM) and `miette` (diagnostics) and `thiserror` /
    `clap` / `serde` / `toml` / `anyhow` / `which` / `cc` /
    `tempfile` as Cargo deps.** All of these are MIT/Apache-2.0
    crates; `miette` is MIT-only, `logos` is MIT/Apache-2.0,
    `chumsky` is MIT, `inkwell` is MIT/Apache-2.0, `which` is MIT,
    `cc` is MIT/Apache-2.0, `tempfile` is MIT/Apache-2.0, `serde`
    is MIT/Apache-2.0, `clap` is MIT/Apache-2.0, `thiserror` is
    MIT/Apache-2.0, `anyhow` is MIT/Apache-2.0, `toml` is
    MIT/Apache-2.0. None impose copyleft.

---

## 17. Where Saturnite currently does NOT match rustc

| Area | Saturnite 0.4 | rustc (current) |
|---|---|---|
| Total LOC | ~11 000 Rust | ~600 000 Rust (compiler/) |
| Type system | `enum HirType` | `ty::Ty<'tcx>` with interned kinds |
| Context type | `HirProgram` (owned) | `TyCtxt<'tcx>` (lifetime-tied) |
| Query system | none | `rustc_query_system` + `rustc_query_impl` |
| Incremental | none | dep-graph + `DepNode` |
| Lints | none | `rustc_lint` |
| Type inference | none (annotate) | full unification (`rustc_infer`) |
| Trait solving | none | `rustc_traits` + `rustc_next_trait_solver` |
| Borrow checking | none | `rustc_borrowck` (Polonius) |
| Const eval | none | `rustc_const_eval` (full CTFE) |
| Codegen backends | LLVM only | LLVM (default) + Cranelift + GCC |
| Targets | 9 hand-rolled | 290+ JSON target specs |
| Procedural macros | none | `proc_macro` server + bridge |
| Edition support | n/a | 2015/2018/2021/2024 |
| Stable MIR | none | `rustc_public` / `rustc_public_bridge` |
| Public tool API | binary only | `rustc_interface` (unstable) + `rustc_public` (stable) |
| Build system | Cargo only | bootstrap (2.5 MB Rust) + Cargo |
| Test infrastructure | `tempfile` integration | `compiletest`, `compiletest_rs`, `ui`, `run-make` |

The vast majority of the gap is intentional (Saturnite 0.4 is a small
language, not a 25-year-old production compiler). The reuse question
in Phase 3-5 is: **of rustc's components, which would actually help
Saturnite close the gap it wants to close, and at what license /
provenance / maintenance cost?**
