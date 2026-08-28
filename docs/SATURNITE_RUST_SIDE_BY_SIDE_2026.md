# SATURNITE × RUST — SIDE-BY-SIDE COMPARISON + CLASSIFICATION

> Subsystem-by-subsystem comparison. Saturnite 0.4 (commit 35f6132) vs
> rustc (commit 3b8ee6c0ca5). Each subsystem receives one
> classification:
>
> | Code | Meaning |
> |---|---|
> | **A. KEEP** | Saturnite's implementation is already appropriate. |
> | **B. REIMPLEMENT** | Use Rust as architectural reference; write Saturnite-native code. |
> | **C. ADAPT/PORT** | Bring actual Rust source into Saturnite (with provenance). |
> | **D. FUSE** | Combine Saturnite's existing implementation with selected Rust architecture/code. |
> | **E. REJECT** | Rust's solution is too Rust-specific, too expensive, or unsuitable. |
> | **F. DEFER** | Valuable eventually; not worth implementing yet. |

---

## 1. Lexer

| Subsystem | Saturnite | Rust | Similarity | Rust useful? | Recommendation |
|---|---|---|---|---|---|
| Lexer | `crates/stnx/src/lexer/mod.rs` (352 lines), logos 0.16, `Range<usize>` spans, no escape decoding, no trivia preservation | `rustc_lexer` (1 279 lines, `compiler/rustc_lexer/src/lib.rs`), standalone, no `rustc_*` deps, produces `(kind, len)` pairs with no spans, full Unicode identifiers via `unicode-ident` and `unicode-properties` | **Low–medium.** Both produce a stream of typed tokens. Different span models. Different Unicode handling. Different keyword sets. | **Architecturally yes**; the **implementation** is too rustc-specific. | **B. REIMPLEMENT.** Saturnite's lexer is fine for 0.4. If/when Unicode identifiers, doc-comments, or raw-string-literals are needed, `rustc_lexer` is a strong reference but should NOT be directly ported — the logos/chumsky pipeline Saturnite already uses is the right shape. |

**Why not ADAPT?**

`rustc_lexer` is dual-licensed MIT/Apache-2.0 (per
`compiler/rustc_lexer/Cargo.toml:4-5`, falls under REUSE blanket).
So licensing is *not* the blocker. The blocker is **architectural
coupling**: `rustc_lexer` produces `(kind, len)` pairs with no
spans; Saturnite's parser relies on a logos-derived
`LexicalToken → TokenKind + Range<usize> span` model. Bringing in
`rustc_lexer` would mean:

1. Replacing logos (and losing the very clean trait-derived
   lexer);
2. Re-spanning in a separate pass (`rustc_parse::lexer` does
   this);
3. Adapting the parser to consume raw byte slices instead of
   pre-tokenized `Token` values.

That is a 2-week refactor for no immediate benefit. Saturnite
0.4's 23 keywords and 50 punctuation tokens are far simpler than
Rust's; the existing lexer is appropriate.

**Recommendation: KEEP Saturnite's logos-based lexer; reference
`rustc_lexer`'s Unicode handling (separately, via crates.io deps
like `unicode-ident` and `unicode-properties`) when Unicode
identifiers are needed.**

---

## 2. Parser

| Subsystem | Saturnite | Rust | Similarity | Rust useful? | Recommendation |
|---|---|---|---|---|---|
| Parser | `crates/stnx/src/parser/mod.rs` (1 456 lines), chumsky 0.13, recursive combinators, `SimpleSpan<usize>` → `Range<usize>`, returns `Vec<ParseError>` aggregated as "(plus N more)" | `rustc_parse` (~25 000 lines, hand-written recursive descent, with `rustc_parse::parser::{expr,pat,function,item,ty,path,...}.rs`); produces `TokenStream` + `rustc_ast::Crate`; error recovery is elaborate; embedded `lexer/` for token-stream-wide concerns; `parser_format.rs` for `format!` strings | **Very low.** Completely different approaches (chumsky combinator vs hand-written RD). | **Architecturally, NO.** | **A. KEEP.** Saturnite's chumsky parser is the right tool for the size of the language. There is nothing to port from `rustc_parse`; the patterns do not transfer. |

**Why not port?**

`rustc_parse` is **intimately tied to `rustc_ast`** (every
`PResult<'a>` returns a `rustc_ast::ast::*` value) and to
`rustc_session::ParseSess` (which owns the `DiagCtxt`).
`rustc_ast` is in turn tied to `rustc_span`, `rustc_data_structures`,
and the `Ast`/`TyCtxt` query system. Porting any meaningful
fragment of `rustc_parse` is a multi-crate refactor.

The `rustc_parse_format` crate is a **good architectural
reference** for the `format_args!()` macro, but Saturnite does
not yet have a `format!` macro.

**Decision: KEEP chumsky; reference `rustc_parse` only as a
structural example of how a 30k-LOC hand-written parser is
organized (separation of `expr.rs`, `pat.rs`, etc.)**.

---

## 3. AST

| Subsystem | Saturnite | Rust | Similarity | Recommendation |
|---|---|---|---|---|
| AST | 238 lines; `ast.rs`; spans are `Range<usize>` | `rustc_ast/src/ast.rs` (4 514 lines); full-fidelity, `NodeId`, `Attr`, `Lit`, `TokenStream`; `TokenStream` is preserved in `ExprKind::MacCall` for round-tripping | **Conceptually aligned; implementation wildly different.** | **A. KEEP.** Saturnite's AST is small and appropriate. There is no benefit to importing `rustc_ast`. |

---

## 4. Symbol interning + identifier system

| Subsystem | Saturnite | Rust | Similarity | Recommendation |
|---|---|---|---|---|
| Symbol interner | `hir/symbol.rs:24-55`, `SymbolInterner { strings: Vec<String>, indices: HashMap<String, SymbolId> }`, `intern / lookup / next_id`, **no arena** | `rustc_span::Symbol`, backed by `Interner` trait; `rustc_data_structures::Interned<T>` (with `private::PrivateZst` for the `Interned` newtype); the actual `str` storage is in a `Lock`ed global `Interner`; `rustc_index::IndexVec` for the index | **Conceptually identical; implementation radically different.** | **B. REIMPLEMENT for now, with `D. FUSE` later.** Saturnite's hand-rolled `SymbolInterner` is adequate for 0.4. If/when incremental compilation is added, **D. FUSE** with `rustc_data_structures::Interned` (MIT/Apache-2.0, no `rustc_*` deps) to get a real interned type. |

**Why not ADAPT?**

`rustc_data_structures::intern::Interned` is dual-licensed
MIT/Apache-2.0 and is **cleanly extracted** (it depends only on
`stable_hash`, which is in the same crate). Bringing it in is
**architecturally possible**. The blocker is that it expects to
be used with the `Interner` trait, and using it without the
trait is awkward. The current Saturnite approach
(`Vec<String> + HashMap<String, SymbolId>`) is fine for the
project's current scale; the upgrade can be deferred.

**Why not KEEP for now?**

The `SymbolInterner` is **already adequate** for the compiler's
needs. It is the right level of complexity. KEEP is appropriate
here; the REIMPLEMENT classification is for the *future* once
incremental compilation arrives.

---

## 5. DefId / LocalDefId

| Subsystem | Saturnite | Rust | Similarity | Recommendation |
|---|---|---|---|---|
| `DefId` | `hir/symbol.rs:24-55`, flat `u32` + `DefTable`; `PRINTLN_DEF_ID = DefId(u32::MAX - 1)` sentinel | `DefId { krate: CrateNum, index: DefIndex }`; a generation component for incremental compilation (DefId is stable across crates); `DefId` + `LocalDefId` distinction; `CrateNum`; per-crate `DefIndex`; `DefKind` enum; full per-crate `DefPathTable`; **no flat global** since ~2019 | **Same name, different shape.** | **A. KEEP Saturnite's flat scheme**, with **F. DEFER** on the migration to `CrateNum + DefIndex`. Single-crate 0.4 is fine with flat. |

The **hard-coded `PRINTLN_DEF_ID = u32::MAX - 1`** is a known
wart, replicated in three files. It is **A. KEEP** for 0.4
(works) and **B. REIMPLEMENT** for any future builtin (use a
real `DefKind::Builtin` or a registry).

---

## 6. Module graph / crate graph

| Subsystem | Saturnite | Rust | Similarity | Recommendation |
|---|---|---|---|---|
| Module graph | `crates/stnx/src/module.rs` (1 516 lines); `ModuleId`, `ModulePath`, `Module`, `ModuleGraph`, `Project`; `Project::load` walks upward for `saturn.toml`; `mod foo;` → `<dir>/foo.stnx`; second-pass `resolve_modules` after the graph is built | `rustc_resolve` (3 110 lines `lib.rs` + `build_reduced_graph.rs`, `imports.rs`, `late.rs`, `macros.rs`, `effective_visibilities.rs`); also `rustc_crate_store` (separate crate) for the multi-crate `ExternCrate` graph; `rustc_metadata::creader` for loading `.rmeta` files | **Same problem; wildly different scale.** | **B. REIMPLEMENT for the single-crate Saturnite use case.** `rustc_resolve` is overengineered for a single-crate 0.4; Saturnite's simpler `ModuleGraph` is the right model. **D. FUSE** later when multi-crate / cargo-style dependencies arrive (use `rustc_crate_store` as architectural reference). |

---

## 7. Resolver / name resolution

| Subsystem | Saturnite | Rust | Similarity | Recommendation |
|---|---|---|---|---|
| Name resolution | `hir/lower.rs` does single-pass name resolution as a side effect of lowering; `LowerScope` parent-linked `HashMap` (`hir/lower.rs:70-95`); `LowerContext<'a>` bundles function sigs + struct/enum defs (`hir/lower.rs:55-65`); two-pass: collect defs, then lower bodies | `rustc_resolve` is a separate, multi-pass resolver (build reduced graph → late resolution → macro resolution → import-checking → privacy); `rustc_resolve::build_reduced_graph` builds the per-item name-binding graph; `rustc_resolve::late.rs` does the late-bound binding resolution | **Conceptually similar; vastly different complexity.** | **A. KEEP** for 0.4. The single-pass `LowerScope` is fine. **B. REIMPLEMENT** when generics / imports / privacy checks become a goal. |

---

## 8. HIR

| Subsystem | Saturnite | Rust | Similarity | Recommendation |
|---|---|---|---|---|
| HIR | `crates/stnx/src/hir/` (3 205 lines); `HirExpr` carries `kind + ty + span`; `HirType = enum { I64, F64, Bool, Str, Unit, Struct(SymbolId), Enum(SymbolId) }`; `HirProgram` owns the symbol table | `compiler/rustc_hir/src/hir.rs` (~5 000 lines) is the giant HIR definition; `HirId` is `(ItemId, ItemLocalId)`; bodies are split off into a `Body` arena; `rustc_ast_lowering` is a 3 381-line pass | **Conceptually very similar.** | **A. KEEP** for now. **B. REIMPLEMENT** with the architectural insight that bodies should be split out of the function. Saturnite's `HirFunction { body: Vec<HirStmt> }` is fine for 0.4; rustc's `Body` + `BodyId` indirection exists to make query-result caching possible. |

---

## 9. Type system

| Subsystem | Saturnite | Rust | Similarity | Recommendation |
|---|---|---|---|---|
| `HirType` | `hir/types.rs:14-26`; `Copy` enum; equality is `==` | `Ty<'tcx>` is `Interned<TyKind<'tcx>>`; equality is pointer-equality; 50+ variants | **Different by design.** | **A. KEEP** for 0.4. **B. REIMPLEMENT** if/when generics are added — the rustc `TyCtxt<'tcx>` is a 200k-LOC system. Saturnite can introduce interned types later, but doing so is a major refactor and not justified at 0.4. |

---

## 10. MIR

| Subsystem | Saturnite | Rust | Similarity | Recommendation |
|---|---|---|---|---|
| MIR data model | `mir/mod.rs` (343 lines); `LocalId` flat, no `Place` projection, `BlockId`, `MirRvalue`, `MirStmt`, `MirTerminator` (Goto / SwitchInt / Call / Return / Unreachable), `MirProgram` | `compiler/rustc_middle/src/mir/{mod,syntax,statement,basic_blocks,visit,pretty,coverage,generic_graph,generic_graphviz}.rs` (10+ files, ~6 000+ lines); `Place` + `Projection` for field access; ~30 terminator variants; many statement kinds; `Body<'tcx>` with `basic_blocks: IndexVec<BasicBlock, BasicBlockData>` | **Conceptually aligned; the design simplification is real.** | **A. KEEP** the simplified design. The lack of `Place` projection is a Saturnite *feature* — the backend is 5x smaller than rustc's because locals are flat. Do **not** upgrade to `Place` unless field-access patterns demand it. |
| MIR verification | `mir/verify.rs` (203 lines) | `compiler/rustc_mir_dataflow/` (~3 000 lines) and the MIR typeck in `rustc_borrowck` | Saturnite's verifier is minimal. | **A. KEEP** for 0.4. **D. FUSE** later: the `rustc_mir_dataflow` framework (MIT/Apache-2.0, no `rustc_*` deps) is a clean, generic dataflow crate that could be used by Saturnite for any future analysis pass. |
| MIR optimization | `mir/opt.rs` (163 lines); **one** pass: constant folding | `compiler/rustc_mir_transform/` (843-line lib.rs + ~25 pass files): inlining, constant propagation, GVN, LICM, simplification, GVN, match lowering, … | Saturnite's constant folder is a tiny fraction of rustc. | **A. KEEP** constant folding. **B. REIMPLEMENT** additional passes incrementally. Do not port rustc MIR passes; they are tightly coupled to the `TyCtxt` and `Place` model. |
| MIR→LLVM | `mir/codegen.rs` (841 lines) | `compiler/rustc_codegen_llvm/src/{builder,context,common,callee,intrinsic,abi,consts,...}.rs` (20+ files) + `rustc_codegen_ssa/src/mir/*.rs` (15+ files) | Same goal, totally different shape. | **A. KEEP** Saturnite's mir/codegen. The `inkwell` integration is clean. |

---

## 11. Codegen (object emission + linking)

| Subsystem | Saturnite | Rust | Similarity | Recommendation |
|---|---|---|---|---|
| Object emission | `codegen/emitter.rs` (42 lines) — inkwell `TargetMachine::write_to_file` | `rustc_codegen_ssa::back::write::write_compressed_file`; produces `.rmeta` (metadata), `.rlib`, `.o`, etc.; complex multi-backend pipeline | **Different abstraction levels.** | **A. KEEP.** |
| Linking | `codegen/linker.rs` (199 lines) — system `cc` / `clang` invocation | `rustc_codegen_ssa::back::link::link_binary`; full custom linker logic in Rust (no system linker dependency by default); `lld` is preferred on most platforms | Different. | **A. KEEP** Saturnite's system-linker approach. It is simpler and adequate for 0.4. If Saturnite needs a more reliable linker later, **B. REIMPLEMENT** based on `rustc_codegen_ssa::back::link` as architectural reference. |

---

## 12. Target configuration

| Subsystem | Saturnite | Rust | Similarity | Recommendation |
|---|---|---|---|---|
| Target spec | `target.rs` (481 lines); hand-rolled `Architecture / OS / Environment` enums; `TargetConfig::host()` detects via `std::env::consts`; no JSON target-spec ingestion | `compiler/rustc_target/` + 290+ JSON target spec files; `Target::from_json`; `TargetOptions`; `TargetTuple`; ABI / calling-convention computation in `rustc_abi` | **Different model.** | **A. KEEP** Saturnite's minimal target model for 0.4. **D. FUSE** later: the JSON target spec format is a public, de-facto standard. Saturnite could adopt it (no licensing issue — it's just JSON) and inherit rustc's target specs as a starting point, then trim to Saturnite's needs. |

---

## 13. Diagnostics / spans

| Subsystem | Saturnite | Rust | Similarity | Recommendation |
|---|---|---|---|---|
| Errors | `error.rs` (158 lines); `thiserror + miette Diagnostic`; one `CompilerError` enum with per-stage variants; `miette::SourceSpan` for underlines | `compiler/rustc_errors` (~10 000 lines); `DiagCtxt` is the per-thread emitter; `Diag` carries a structured `Diagnostic` with optional `Subdiagnostic`; renderers: short, long, JSON, with color/styling; `rustc_error_codes` for long-form explanations | **Different approach.** Saturnite uses miette; rustc uses its own emitter. | **A. KEEP** miette — it is the right tool for a small compiler. **B. REIMPLEMENT** if/when `cargo`-style structured JSON output is needed (Saturnite already has `--json` flag — check what it actually emits). |
| Spans | `Range<usize>` byte ranges stored in tokens and AST nodes; `miette::SourceSpan` for diagnostics | `Span` is a 4-byte `Span` (with `SpanData` for explicit form), backed by a `SourceMap` with byte-position tables and `BytePos`; hygiene via `SyntaxContext` | **Different.** | **A. KEEP** Saturnite's byte-range model — it is simpler and adequate. **B. REIMPLEMENT** if/when macro-expansion hygiene is added. |

---

## 14. Query / incremental compilation

| Subsystem | Saturnite | Rust | Similarity | Recommendation |
|---|---|---|---|---|
| Query system | **none** | `compiler/rustc_middle/src/query/{mod,system,keys,job,query_api,into_query_key,modifiers,erase,calls,arena_cached}.rs` (10 files) + `compiler/rustc_query_impl/src/{lib,execution,query_vtables,dep_kind_vtables,job,incremental,self_profile,diagnostics,handle_cycle_error}.rs` (9 files) — generated query implementations, dep-graph tracking, parallel execution, on-disk caching | **Massive difference.** | **F. DEFER.** Saturnite 0.4 has no query system, no incremental compilation, no dep-graph. Adding one is a multi-month project. **D. FUSE** later: `rustc_middle::query` is the architectural reference. **C. ADAPT** would be infeasible (everything is generic over `'tcx`). |
| Incremental compilation | **none** | `compiler/rustc_incremental/` (22-line lib.rs + `assert_dep_graph.rs` + `persisted/`); `DepNode` and `DepGraph`; on-disk cache in `<sysroot>/<triple>/.incremental/` | Same as above. | **F. DEFER.** |

---

## 15. Borrow checking / ownership / lifetimes

| Subsystem | Saturnite | Rust | Similarity | Recommendation |
|---|---|---|---|---|
| Borrow checker | **none** (language has `mut` but no ownership/lifetimes) | `compiler/rustc_borrowck/` (~2 800 lines) with `polonius-engine`; two-phase borrows; NLL; subset analysis; `Rvalue::Ref` borrow expressions | None. | **F. DEFER.** Saturnite 0.4 does not need a borrow checker. If/when ownership is added, **B. REIMPLEMENT** from scratch (NLL / Polonius are 10+ years of research; clean-rooming them is the right move). |

---

## 16. Trait solving / generics

| Subsystem | Saturnite | Rust | Similarity | Recommendation |
|---|---|---|---|---|
| Generics | **none** | `compiler/rustc_middle/src/ty/{generics,fold}.rs`, `compiler/rustc_hir_analysis/src/collect.rs`; `ParamEnv`, `GenericArg`, `Predicate` | None. | **F. DEFER.** Generics are a 1.0 feature, not 0.4. |
| Trait solving | **none** | `compiler/rustc_traits/` (old solver) + `compiler/rustc_trait_selection/` + `compiler/rustc_next_trait_solver/` (new solver in active development) | None. | **F. DEFER.** |

---

## 17. Const evaluation

| Subsystem | Saturnite | Rust | Similarity | Recommendation |
|---|---|---|---|---|
| Const eval | **none** (no `const fn` support) | `compiler/rustc_const_eval/` + `compiler/rustc_monomorphize/`; full CTFE interpreter | None. | **F. DEFER.** |

---

## 18. Procedural macros

| Subsystem | Saturnite | Rust | Similarity | Recommendation |
|---|---|---|---|---|
| Proc macros | **none** | `compiler/rustc_builtin_macros/`, `compiler/rustc_expand/`, `compiler/rustc_proc_macro/`, `compiler/rustc_builtin_macros/`; `proc_macro` server protocol; submoduled `proc_macro` (the runtime) | None. | **F. DEFER.** |

---

## 19. Runtime / standard library

| Subsystem | Saturnite | Rust | Similarity | Recommendation |
|---|---|---|---|---|
| Runtime | `runtime/println_i64.c` (one function, 5 lines of C) | `library/core` (no-std core), `library/alloc`, `library/std` (full standard library) | None. | **A. KEEP** the minimal C runtime. **B. REIMPLEMENT** any future `saturnite-std` from scratch (do not import rust's `core` — the language semantics differ). |
| Standard library | **none** | `library/core + alloc + std` (~150 000 LOC of Rust) | n/a | **F. DEFER** for the full std. **B. REIMPLEMENT** a small `saturnite-std` with the parts Saturnite programs need. |

---

## 20. Build system

| Subsystem | Saturnite | Rust | Similarity | Recommendation |
|---|---|---|---|---|
| Build system | Cargo only | `src/bootstrap/` (2.5 MB Rust project, 14 files at `src/bootstrap/src/` + `mk/` + `defaults/`); `x` / `x.py` / `x.ps1` thin wrappers; `configure` shell script | **Very different.** | **A. KEEP** Cargo only. Bootstrap is overkill for 0.4. **D. FUSE** later: if a Saturnite distribution ever needs to bundle a pre-built toolchain (e.g. for Windows installer), some of bootstrap's patterns may be useful. |

---

## 21. Test infrastructure

| Subsystem | Saturnite | Rust | Similarity | Recommendation |
|---|---|---|---|---|
| Test framework | `crates/stnx/tests/` integration tests using `tempfile`; no UI/snapshot tests | `src/tools/compiletest` (the `compiletest` framework), `src/tools/miri`, `src/tools/rustc-perf`, `tests/{ui,run-make,rustdoc,codegen,mir-opt}` | Different. | **B. REIMPLEMENT** a small compiletest analogue. **D. FUSE** later: `compiletest` itself is in the Rust tree under `src/tools/compiletest` (MIT/Apache-2.0, Rust Project copyright). It could be extracted as a crates.io dep. |

---

## 22. CLI

| Subsystem | Saturnite | Rust | Similarity | Recommendation |
|---|---|---|---|---|
| CLI | `main.rs` (718 lines), clap-derive; 4 subcommands | `rustc_driver_impl::run_compiler` (1 686 lines); `rustc_session::config::getopts` for flag parsing | Different. | **A. KEEP** clap-based CLI. It is the right level of complexity. |

---

## 23. Package manager / dependency resolver

| Subsystem | Saturnite | Rust | Similarity | Recommendation |
|---|---|---|---|---|
| `saturn.toml` | `config.rs` (222 lines); `Package`, `DependencySpec`; **no resolver, no fetcher, no registry** | `src/tools/cargo/` (submodule, full Cargo codebase) | Different. | **B. REIMPLEMENT** the package manager from scratch, using Cargo as architectural reference. **E. REJECT** the option of vendoring Cargo as a binary dep — too much surface, the language semantics differ. |

---

## 24. Public tool API

| Subsystem | Saturnite | Rust | Similarity | Recommendation |
|---|---|---|---|---|
| Stable MIR / public tool API | **none** (binary CLI only) | `compiler/rustc_public/` + `compiler/rustc_public_bridge/` (the **public stable MIR** API for external tools) | None. | **F. DEFER.** Saturnite 0.4 has no public tool API. |

---

## 25. Subtree ownership / external repositories (per `CONTRIBUTING.md`)

> Per `rust/CONTRIBUTING.md#making-changes-to-subtrees-and-submodules` and the rustc-dev-guide, several subtrees are externally maintained. Editing them in the rust checkout is **banned** by Rust's own policy.

The Rust repo **does not own**:
- Cargo (rust-lang/cargo) — different maintainers
- LLVM (rust-lang/llvm-project) — different maintainers
- GCC (rust-lang/gcc) — different maintainers
- backtrace-rs (rust-lang/backtrace-rs) — different maintainers
- rustc-perf, enzyme — different maintainers
- nomicon, reference, book, rust-by-example, edition-guide,
  embedded-book — doc teams
- clippy, rustfmt, rust-analyzer, miri, compiletest — separate teams

**Implication for Saturnite**: if Saturnite ever wants to **reuse
code from one of these subtrees**, Saturnite must:

1. Not route through the rustc tree as a transitive vendor.
2. Attribute the original project explicitly.
3. Honor the original project's license.
4. Not bundle code from the GPL-licensed `src/gcc/`.
5. For `src/llvm-project/`, the licensing is `NCSA AND Apache-2.0
   WITH LLVM-exception` — Saturnite already has LLVM through
   `inkwell`; it does not need to vendor `src/llvm-project/`.

---

## 26. Summary of classifications

| Subsystem | Class | Risk | License/provenance status |
|---|---|---|---|
| 1. Lexer | A. KEEP | none | n/a |
| 2. Parser | A. KEEP | none | n/a |
| 3. AST | A. KEEP | none | n/a |
| 4. Symbol interner | A. KEEP (now) / D. FUSE (later) | low | `rustc_data_structures::intern` is MIT/Apache-2.0 |
| 5. DefId | A. KEEP (now) / B. REIMPLEMENT (later) | low | n/a |
| 6. Module graph | A. KEEP (now) / B. REIMPLEMENT (later) | low | n/a |
| 7. Name resolution | A. KEEP (now) / B. REIMPLEMENT (later) | low | n/a |
| 8. HIR | A. KEEP | none | n/a |
| 9. Type system | A. KEEP (now) / B. REIMPLEMENT (later) | low | n/a |
| 10. MIR (data + verify + opt + codegen) | A. KEEP | none | n/a |
| 11. Object emission + linking | A. KEEP | none | n/a |
| 12. Target spec | A. KEEP (now) / D. FUSE (later) | low | JSON spec format is unencumbered |
| 13. Diagnostics + spans | A. KEEP | none | n/a |
| 14. Query / incremental | F. DEFER | n/a (deferred) | n/a |
| 15. Borrow checker | F. DEFER | n/a | n/a |
| 16. Generics / trait solving | F. DEFER | n/a | n/a |
| 17. Const eval | F. DEFER | n/a | n/a |
| 18. Proc macros | F. DEFER | n/a | n/a |
| 19. Runtime / std | A. KEEP (C runtime) / B. REIMPLEMENT (Saturnite std) | low | n/a |
| 20. Build system | A. KEEP (now) / D. FUSE (later) | low | n/a |
| 21. Test framework | B. REIMPLEMENT (now or later) / D. FUSE (later) | low | `compiletest` is MIT/Apache-2.0 in Rust tree |
| 22. CLI | A. KEEP | none | n/a |
| 23. Package manager | B. REIMPLEMENT | low | Cargo is a separate MIT/Apache-2.0 codebase, **but** Saturnite should not vendor Cargo |
| 24. Public tool API | F. DEFER | n/a | n/a |
| 25. Subtree ownership | n/a (governance) | n/a | n/a |
| 26. Unicode data | E. REJECT (port); use crates.io | n/a | `library/core/src/unicode/unicode_data.rs` is `Unicode-3.0`; use `unicode-general-category` / `unicode-width` / `unicode-ident` crates from crates.io (MIT/Apache-2.0) |
| 27. Backtrace | F. DEFER | n/a | `library/backtrace` is MIT/Apache-2.0, submodule |
| 28. GCC backend | E. REJECT | n/a | **GPL-3.0-or-later** — hard NO |
| 29. LLVM submodule | E. REJECT (port) | n/a | Saturnite already has LLVM through `inkwell`; do not vendor `src/llvm-project/` |
| 30. Bootstrap build system | A. KEEP Cargo only / D. FUSE later | low | n/a |

---

## 27. The C/D classification rationale (which items could be ADAPTED or FUSED)

Looking at the **only** two items in the matrix with a C or D
classification for code (not just architecture):

### D. FUSE candidates (where actual Rust source might be reused)

1. **`rustc_data_structures::intern::Interned`** (D. FUSE, later)
   - License: MIT/Apache-2.0 (under REUSE blanket)
   - Dependencies: `stable_hash` only (same crate)
   - What it provides: a real interned-type newtype with
     `Interner` trait; can be used to intern `HirType` once generics
     arrive.
   - Adaptation difficulty: medium (requires implementing the
     `Interner` trait for Saturnite's context type).

2. **`rustc_mir_dataflow` framework** (D. FUSE, later)
   - License: MIT/Apache-2.0
   - Dependencies: `rustc_index`, `rustc_data_structures`, `tracing`
   - What it provides: a generic dataflow-analysis framework
     (forward/backward, kill/gen, lattice-based) usable for any
     lattice, **independent of `rustc_middle::mir`**.
   - Adaptation difficulty: high (it is currently tightly coupled
     to `rustc_middle::mir::BasicBlock`; would require either
     porting `BasicBlock` or accepting a type-parameter).

3. **`src/tools/compiletest`** (D. FUSE, later)
   - License: MIT/Apache-2.0
   - What it provides: the compiletest framework
   - Adaptation difficulty: high (it is integrated with rustc's
     invocation; would need a thin driver).

4. **JSON target spec format** (D. FUSE, later)
   - License: not code, just a format
   - What it provides: ~290+ JSON target specs
   - Adaptation difficulty: low; the format is documented in
     `compiler/rustc_target/src/json.rs`.

### C. ADAPT/PORT candidates

There are **no** items in the table that warrant a C. ADAPT/PORT
classification. The reason: every rustc subsystem that is
MIT/Apache-2.0 is either (a) too tied to `TyCtxt<'tcx>` /
`Span` / `Session` to extract, or (b) duplicating what Saturnite
already has at a smaller scale.

### E. REJECT (license / architecture reasons)

1. **`src/gcc/**`** — **GPL-3.0-or-later**. Hard NO.
2. **`src/llvm-project/**`** — NCSA + Apache-2.0+LLVM-exception.
   Saturnite already has LLVM through `inkwell`; do not duplicate.
3. **`library/core/src/unicode/unicode_data.rs`** — Unicode-3.0.
   Use crates.io alternatives.

---

## 28. The "F. DEFER" pile

The single largest category is "valuable eventually, not worth
implementing yet." This includes:

- Borrow checking
- Trait solving
- Generics
- Const evaluation
- Proc macros
- Query system + incremental compilation
- Public/stable MIR

Saturnite 0.4 does not need any of these. Each is a multi-month
to multi-year project. None of them are blocking.

---

## 29. Architectural insight (the most important thing in this section)

Saturnite's current architecture is **already structurally aligned**
with rustc's at a coarse level:

- Source → Lex → Parse → AST → HIR (lowered) → MIR → LLVM IR → Object → Linker → Executable.

The match is **clean and intentional**: the Saturnite README
(`README.md:43-58`) describes exactly this pipeline. The 0.4
release is a working demonstration that a 11 000-LOC compiler can
implement this pipeline for a small, useful language.

The differences from rustc are **size and scale, not shape**:

- **Flat types** vs. interned types — trade off ergonomics for
  simplicity.
- **No `Place` projection** — trade off flexibility for a 5x
  smaller backend.
- **No `TyCtxt<'tcx>`** — trade off query-system sophistication
  for code-readability.
- **No `DepGraph`** — trade off incremental compilation for
  simplicity.
- **No lifetime parameters** — the `HirProgram` is moved by
  value.

These are **good engineering choices for 0.4**. They are **not
the same choices rustc made** because rustc is a different
language at a different scale.

The audit's verdict: **Saturnite's architecture is sound. Do not
"modernize" it by adopting rustc's patterns wholesale. Adopt
specific patterns only when a specific feature requires them.**
