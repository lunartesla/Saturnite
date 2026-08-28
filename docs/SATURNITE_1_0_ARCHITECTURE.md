# SATURNITE 1.0 — TARGET ARCHITECTURE (Phase 10)

> Where Saturnite should be at the 1.0 release. Built from the
> forensic audit. Every component is explicitly labeled with its
> intended origin.

## Origin labels

| Label | Meaning |
|---|---|
| **SATURNITE NATIVE** | Code Saturnite writes from scratch in its own idiom. |
| **RUST-INSPIRED** | Architecturally follows rustc patterns; implementation is clean-room. |
| **RUST-ADAPTED** | Specific rustc source ported into Saturnite, with provenance. |
| **THIRD-PARTY** | Crates.io or other external dependency. |
| **LLVM** | inkwell + system LLVM. |
| **OTHER** | Custom / unclassified. |

---

## 1. Frontend

### 1.1 Lexer — **SATURNITE NATIVE**

- `crates/stnx/src/lexer/mod.rs` (logos 0.16)
- `crates/stnx/src/lexer/token.rs`
- **Architecture**: Logos `Logos` derive; `Range<usize>` byte spans; minimal state.
- **Reuse**: none.
- **What to add for 1.0**:
  - Unicode identifiers (use `unicode-ident` and
    `unicode-properties` crates from crates.io — both
    MIT/Apache-2.0, **THIRD-PARTY**).
  - Doc comments (`///` and `//!`).
  - Raw string literals (`r#"..."#`).
- **Why not `rustc_lexer`**: see Phase 4 — the logos-based
  pipeline is the right shape; `rustc_lexer`'s `(kind, len)`-only
  model is the wrong shape.

### 1.2 Parser — **SATURNITE NATIVE**

- `crates/stnx/src/parser/mod.rs` (chumsky 0.13)
- **Architecture**: chumsky 0.13 with `recursive::Direct`; per-
  production sub-parser functions; `SimpleSpan<usize>` →
  `Range<usize>` conversion; first-error-wins with extras
  reported in message.
- **Reuse**: none.
- **What to add for 1.0**:
  - Async / await syntax (if the language adopts it).
  - Pattern matching (if the language adopts it).
  - Generic params syntax (post-0.5).

### 1.3 AST — **SATURNITE NATIVE**

- `crates/stnx/src/ast.rs`
- **Architecture**: `Item` + `ItemKind` (5 variants);
  `Function`, `Stmt`, `Expr`, `Type` enums; every node carries
  `Range<usize>` span.
- **Reuse**: none.

### 1.4 Resolver — **SATURNITE NATIVE** (now) / **RUST-INSPIRED** (1.0)

- 0.4: `crates/stnx/src/hir/lower.rs` does single-pass name
  resolution as a side effect of lowering. Adequate for
  single-file 0.4.
- 1.0: separate name-resolution pass, **RUST-INSPIRED** by
  `rustc_resolve`'s two-phase approach (build reduced graph,
  then late resolution). Saturnite's version is much smaller
  (single crate, no macros, no privacy, no import cycles).
- **Reuse**: none (architectural reference only).

### 1.5 HIR — **SATURNITE NATIVE**

- `crates/stnx/src/hir/`
- **Architecture**: `HirProgram { functions, structs, enums,
  symbols, modules, root_module, module_paths, def_table,
  module_scopes, use_decls, mod_decls }`. `HirType` is a flat
  `Copy` enum (7 variants in 0.4). `HirExpr` and `HirStmt`
  carry `kind + ty + span`. `HirFunction` has `def_id + name +
  params + return_type + body + span + module + visibility`.
- **Reuse**: none.
- **What to add for 1.0**:
  - Generics: add `HirType::Generic(SymbolId)` and
    `HirGenericParam`. Consider interned types at this point
    (A1 in the reuse plan).
  - Lifetimes: Saturnite does not need them; do not add.
  - Traits: add `HirTraitDef`, `HirTraitImpl`. Trait solving
    is a separate problem (F. DEFER for 1.0).

---

## 2. Middle-end

### 2.1 Type system — **SATURNITE NATIVE** (now) / **RUST-INSPIRED** (1.0)

- 0.4: `HirType` is a flat enum. Type checking is structural.
- 1.0: same, possibly with interned types (per A1).
- **Reuse**: the `Interned<'a, T>` newtype (A1, **RUST-ADAPTED**)
  is the one port.
- **Why not full `TyCtxt<'tcx>`**: rustc's type system is
  200k+ LOC of interned types, predicates, inference, and trait
  solving. Saturnite 1.0 does not need that scale.

### 2.2 Symbol system — **SATURNITE NATIVE**

- `crates/stnx/src/hir/symbol.rs`
- `SymbolId(u32)` + `DefId(u32)` + `SymbolInterner` +
  `DefTable`.
- **Reuse**: none.

### 2.3 MIR — **SATURNITE NATIVE**

- `crates/stnx/src/mir/`
- `LocalId`, `BlockId`, `MirLocal`, `MirOperand`, `MirConst`,
  `MirRvalue`, `MirStmt`, `MirStmtKind`, `MirTerminator`,
  `MirBasicBlock`, `MirFunction`, `MirProgram`.
- **Reuse**: none.
- **What to add for 1.0**:
  - More MIR optimization passes (DCE, copy propagation,
    inlining). Each is ~50-200 lines.
  - A real dataflow analysis framework, possibly adapted from
    `rustc_mir_dataflow::framework` (A4, **RUST-ADAPTED**, when
    5+ analyses exist).
  - SSA form for value semantics (Saturnite 0.4 uses
    allocas for mutables; SSA would let LLVM do better).

### 2.4 MIR verification — **SATURNITE NATIVE**

- `crates/stnx/src/mir/verify.rs`
- Verifies: every block has exactly one terminator; every
  terminator target is a valid `BlockId`; operand types match
  their use sites; `Call.func` is a valid `DefId`.
- **Reuse**: none.

### 2.5 Query / incremental architecture — **F. DEFER**

- 0.4: none.
- 1.0: still none. The roadmap (Phase 11) does not require
  incremental compilation at 1.0.
- 1.5+: a query system modeled on `rustc_middle::query` is
  appropriate, but only if/when incremental compilation is
  prioritized. **RUST-INSPIRED** at that point.

---

## 3. Backend

### 3.1 MIR → LLVM — **SATURNITE NATIVE**

- `crates/stnx/src/mir/codegen.rs` (841 lines)
- Per-function, per-block, per-statement/terminator walk.
- inkwell 0.9 + LLVM 21 (dynamic).
- **Reuse**: none.
- **What to add for 1.0**:
  - SSA construction (to remove unnecessary allocas for
    immutable locals).
  - Debug info emission (DWARF).
  - Exception handling (if Saturnite adopts unwinding).

### 3.2 Object emission — **SATURNITE NATIVE**

- `crates/stnx/src/codegen/emitter.rs` (42 lines)
- inkwell `TargetMachine::write_to_file`.
- **Reuse**: none.

### 3.3 Linker — **SATURNITE NATIVE**

- `crates/stnx/src/codegen/linker.rs` (199 lines)
- System-linker (cc / clang / link.exe) invocation via
  `which::which`.
- **Reuse**: none.
- **What to add for 1.0**:
  - LTO / thin-LTO flag passthrough.
  - PIE / non-PIE flag passthrough.

### 3.4 Target configuration — **SATURNITE NATIVE** (now) / **RUST-INSPIRED** (1.0+)

- 0.4: 9 hand-rolled targets in `target.rs`. JSON target-spec
  ingestion is **RUST-INSPIRED** but not implemented.
- 1.0+: optional JSON target spec adoption (A2,
  **RUST-ADAPTED**). The schema is documented; the 290+ JSON
  files are data, not code; Saturnite writes its own parser.

### 3.5 Targets (LLVM)

- **LLVM** (via `inkwell`): x86_64, aarch64, x86, arm,
  riscv64, mips, powerpc64, wasm32. Saturnite's
  `Architecture` enum maps to these.
- **Reuse**: inkwell is **THIRD-PARTY** (MIT/Apache-2.0).

---

## 4. Project system

### 4.1 `saturn.toml` — **SATURNITE NATIVE**

- `crates/stnx/src/config.rs` (222 lines)
- `Package`, `DependencySpec`, `BTreeMap<String, DependencySpec>`.
- **What to add for 1.0**:
  - Dependency resolution (fetches from a registry).
  - Edition support (currently hard-coded to "2026").
  - Feature flags (`[features]` section).
  - Workspace support (`[workspace]` section).

### 4.2 Module system — **SATURNITE NATIVE**

- `crates/stnx/src/module.rs` (1 516 lines — the largest file
  in the compiler)
- `ModuleId`, `ModulePath`, `Module`, `ModuleGraph`, `Project`.
- **What to add for 1.0**:
  - Visibility checks (currently parsed but not enforced).
  - Cycle detection (currently may loop on cyclic `mod`
    declarations).
  - Better error messages for missing files.

### 4.3 Package manager — **SATURNITE NATIVE** (1.0) / **RUST-INSPIRED** by cargo (1.0+)

- 0.4: no fetcher; no registry.
- 1.0: minimal `stnx` command: `stnx add`, `stnx remove`,
  `stnx install`, `stnx update`, `stnx publish`. **RUST-
  INSPIRED** by Cargo's CLI (Cargo itself is not vendored).
- **Reuse**: none (architectural reference only).

### 4.4 Registry — **SATURNITE NATIVE** (deferred)

- A registry is a separate service; out of scope for the
  compiler proper.
- **Reuse**: none.

---

## 5. Infrastructure

### 5.1 Diagnostics — **SATURNITE NATIVE**

- `crates/stnx/src/error.rs` (158 lines)
- `thiserror + miette Diagnostic`. One `CompilerError` enum.
- **Reuse**: none.
- **What to add for 1.0**:
  - Lint infrastructure (a la `rustc_lint`).
  - Suggestion engine (a la `rustc_errors::diagnostic::Applicability`).
  - JSON output (already partially supported via `--json`).

### 5.2 Caching — **F. DEFER**

- 0.4: no caching. Every invocation re-does everything.
- 1.0: optional on-disk cache of `HirProgram` (for IDE-style
  use cases).
- **Reuse**: none.

### 5.3 Serialization — **SATURNITE NATIVE** (now) / **RUST-INSPIRED** (1.0)

- 0.4: `HirType`, `SymbolId`, `DefId` derive
  `Serialize, Deserialize`. `mir::*` types derive the same.
  No consumer.
- 1.0: an actual on-disk format (likely a `.stncache` file
  per module).
- **Reuse**: **RUST-INSPIRED** by `rustc_serialize` and
  `rustc_metadata`.

### 5.4 Testing — **SATURNITE NATIVE** (now) / **RUST-INSPIRED** (1.0+)

- 0.4: integration tests via `tempfile`.
- 1.0: compiletest-style UI/snapshot tests (A3, **RUST-
  ADAPTED**, when the test count justifies it).
- **Reuse**: compiletest runner scaffolding (A3).

### 5.5 Compiler driver — **SATURNITE NATIVE**

- `crates/stnx/src/main.rs` (718 lines)
- clap-derive; 4 subcommands.
- **Reuse**: none.

### 5.6 Build system — **SATURNITE NATIVE**

- `crates/stnx/build.rs` (54 lines) compiles the C runtime.
- `cc` crate as a build dep.
- **Reuse**: none.

### 5.7 Runtime — **SATURNITE NATIVE**

- `runtime/println_i64.c` (single C function).
- Compiled at build time via `build.rs`.
- **Reuse**: none.
- **What to add for 1.0**:
  - `print_str(const char*)` — string printing.
  - `i64_to_str` — number-to-string conversion.
  - Memory allocator (if the language adopts heap allocation).
  - Panic handler (if the language adopts unwinding).

### 5.8 Standard library — **SATURNITE NATIVE** (1.0+)

- 0.4: none.
- 1.0: a small `saturnite-std` crate with the bare minimum
  (currently: just `println` and arithmetic intrinsics).
- **Reuse**: none (Rust's `core`/`alloc`/`std` are language-
  incompatible).

### 5.9 Procedural macros — **F. DEFER**

- 0.4: none.
- 1.0: none. Procedural macros require a wire protocol and a
  host process. Out of scope.
- **Reuse**: none (architectural reference only).

### 5.10 Cross compilation — **SATURNITE NATIVE** (now) / **RUST-INSPIRED** (1.0+)

- 0.4: `--target <TRIPLE>` flag is accepted but only host
  is fully working.
- 1.0: any target that LLVM supports.
- **Reuse**: inkwell (THIRD-PARTY) handles the LLVM-level
  work.

---

## 6. External / third-party

| Component | Origin | License | Notes |
|---|---|---|---|
| `logos = "0.16"` | crates.io | MIT/Apache-2.0 | lexer |
| `chumsky = "0.13"` | crates.io | MIT | parser |
| `inkwell = "0.9"` | crates.io | MIT/Apache-2.0 | LLVM bindings (links to system LLVM 21) |
| `miette = "7"` | crates.io | MIT | diagnostics |
| `thiserror = "2"` | crates.io | MIT/Apache-2.0 | error derive |
| `clap = "4"` | crates.io | MIT/Apache-2.0 | CLI |
| `serde = "1"` | crates.io | MIT/Apache-2.0 | serialization |
| `serde_json = "1"` | crates.io | MIT/Apache-2.0 | JSON build report |
| `toml = "0.8"` | crates.io | MIT/Apache-2.0 | saturn.toml parser |
| `anyhow = "1"` | crates.io | MIT/Apache-2.0 | CLI error handling |
| `which = "5"` | crates.io | MIT | linker discovery |
| `cc = "1"` | crates.io | MIT/Apache-2.0 | runtime C compilation |
| `tempfile = "3"` | crates.io | MIT/Apache-2.0 | test isolation |
| system LLVM 21 | `llvm.org` | NCSA / Apache-2.0 + LLVM exception | linked dynamically |
| system C compiler | system | various (cc / gcc / clang / MSVC) | for `runtime/println_i64.c` |
| system linker | system | various (cc / gcc / clang / link.exe) | for final link |

Total **third-party** items: 14 Cargo deps + system LLVM +
system C compiler + system linker. All are MIT/Apache-2.0 or
compatible; none are copyleft.

---

## 7. The 1.0 pipeline (final shape)

```
file.stn
  └─→ Lexer (logos, Range<usize> spans)
        └─→ Parser (chumsky 0.13, recursive combinators)
              └─→ AST (typed tokens, every node has span)
                    └─→ Resolver (single-pass name resolution)
                          └─→ HIR (SymbolId / DefId, typed)
                                └─→ Type checker (structural; full for 0.4-1.0)
                                      └─→ HIR (with module + visibility info)
                                            └─→ MIR (HIR → typed CFG)
                                                  ├─→ MIR verify
                                                  ├─→ MIR optimize (const fold + 2-3 more)
                                                  └─→ MIR → LLVM IR (inkwell)
                                                        └─→ Object (.o via TargetMachine)
                                                              └─→ Linker (system cc / clang)
                                                                    └─→ Executable
```

Compare to the 0.4 pipeline (from `README.md:43-58`):

```
file.stn → Lex → Parse → AST → HIR → MIR → LLVM → Object → Linker → Executable
```

The 1.0 pipeline **adds an explicit Resolver step** (currently
folded into HIR lowering) and **expands MIR optimize** from
const-fold-only to 3-5 passes.

---

## 8. The big picture: where Saturnite 1.0 differs from rustc

| Aspect | Saturnite 1.0 | rustc |
|---|---|---|
| Total LOC | ~25 000 (estimate) | ~600 000 |
| Type system | flat enum `HirType` | interned `Ty<'tcx>` |
| Context | owned `HirProgram` | borrowed `TyCtxt<'tcx>` |
| Query system | none | `rustc_query_system` |
| Incremental | none | dep-graph + on-disk cache |
| Lints | none | `rustc_lint` |
| Borrow check | none | `rustc_borrowck` + Polonius |
| Trait solving | none | `rustc_next_trait_solver` |
| Generics | none | full |
| Const eval | none | `rustc_const_eval` |
| Codegen backends | LLVM only | LLVM + Cranelift + GCC |
| Target specs | 9 hand-rolled | 290+ JSON |
| Proc macros | none | full |
| Editions | "2026" only | 2015 / 2018 / 2021 / 2024 |
| Public tool API | binary only | `rustc_interface` (unstable) + `rustc_public` (stable) |
| Build system | Cargo only | bootstrap + Cargo |

**Saturnite 1.0 is a small, focused language. The architectural
shape is rustc's; the scale is not.**
