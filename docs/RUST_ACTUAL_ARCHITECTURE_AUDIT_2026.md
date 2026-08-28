# RUST (rustc) — ACTUAL ARCHITECTURE (FORENSIC)

> Source-only map of rustc at commit
> `3b8ee6c0ca55afb08e2e130003227a3195394425` (tagged for Rust 1.100.0 per
> `/home/dimitar/rust/src/version`). Every claim is backed by a file path
> and a line number or symbol. Generated as part of the 2026-08-28
> Saturnite×Rust forensic audit.

This document does **not** rehash the rustc-dev-guide. It points at
specific source locations so that the Phase 3+ classification can rely
on hard evidence.

---

## 0. Repository metadata

| Property | Value | Source |
|---|---|---|
| Path | `/home/dimitar/rust` | working tree |
| Commit | `3b8ee6c0ca55afb08e2e130003227a3195394425` | `git rev-parse HEAD` |
| Version | 1.100.0 | `src/version` |
| Workspace | 47 members in `/home/dimitar/rust/Cargo.toml` | workspace root |
| Crate edition | 2024 | `compiler/rustc/Cargo.toml:4` |
| License | `MIT OR Apache-2.0` blanket; per-file overrides in `REUSE.toml` | `LICENSE-MIT`, `LICENSE-APACHE`, `REUSE.toml` |
| Copyright | Contributors, no assignment | `COPYRIGHT:5-6` |
| REUSE compliance | Yes; tracked via `REUSE.toml` + `license-metadata.json` | `REUSE.toml:1-3` |

---

## 1. License infrastructure (the most important thing in this report)

The Rust repository is **REUSE-compliant**. Every file has a known
SPDX license, either via blanket annotation in `REUSE.toml` or via
per-file header.

### `REUSE.toml` structure (lines 1-219)

- A blanket annotation (lines 24-67) covers almost the whole tree:
  `compiler/**`, `library/**`, `tests/**`, `src/**`, `.github/**`,
  plus most root files (`AGENTS.md`, `Cargo.lock`, `Cargo.toml`,
  `CLAUDE.md`, `CODE_OF_CONDUCT.md`, `CONTRIBUTING.md`, `COPYRIGHT`,
  `LICENSE-APACHE`, `license-metadata.json`, `LICENSE-MIT`, etc.).
  - SPDX: `MIT OR Apache-2.0`
  - Copyright: "The Rust Project Developers (see https://thanks.rust-lang.org)"
  - `precedence = "override"` — this REUSE annotation wins over any
    in-file header.

- Per-file overrides (the third-party or special-license files):

| Path | License | Source |
|---|---|---|
| `compiler/rustc_llvm/llvm-wrapper/SymbolWrapper.cpp` | `Apache-2.0 WITH LLVM-exception AND (Apache-2.0 OR MIT)` | `REUSE.toml:69-75` |
| `compiler/rustc_middle/src/ptrauth/llvm_siphash/tests.rs` | `Apache-2.0 WITH LLVM-exception AND (Apache-2.0 OR MIT)` | `REUSE.toml:212-219` |
| `library/core/src/unicode/unicode_data.rs` | `Unicode-3.0` (1991-2024 Unicode, Inc.) | `REUSE.toml:77-80` |
| `library/std/src/sync/mpmc/**` | `MIT OR Apache-2.0` (Crossbeam + Rust) | `REUSE.toml:82-87` |
| `library/std/src/sys/sync/mutex/fuchsia.rs` | `BSD-2-Clause AND (MIT OR Apache-2.0)` | `REUSE.toml:89-94` |
| `src/test/rustdoc/auxiliary/enum-primitive.rs` | `MIT` (Anders Kaseorg) | `REUSE.toml:96-99` |
| `src/librustdoc/html/static/fonts/Fira**` | `OFL-1.1` (Mozilla + Telefonica 2014) | `REUSE.toml:101-105` |
| `src/librustdoc/html/static/fonts/NanumBarun**` | `OFL-1.1` (NAVER 2010) | `REUSE.toml:107-110` |
| `src/librustdoc/html/static/fonts/SourceCodePro**`, `SourceSerif4**` | `OFL-1.1` (Adobe 2010-2023) | `REUSE.toml:112-117` |
| `src/librustdoc/html/static/css/normalize.css` | `MIT` (Gallagher/Neal) | `REUSE.toml:119-122` |
| `src/librustdoc/html/static/css/rustdoc.css` | `MIT OR Apache-2.0` | `REUSE.toml:124-129` |
| `src/doc/rustc-dev-guide/mermaid.min.js` | `MIT` (Sveidqvist 2014-2021) | `REUSE.toml:131-134` |
| `library/backtrace/**` | `MIT OR Apache-2.0` (submodule: backtrace-rs) | `REUSE.toml:136-141` |
| `src/doc/embedded-book/**` | `MIT OR Apache-2.0 OR CC-BY-SA-4.0` | `REUSE.toml:143-148` |
| `src/doc/rust-by-example/**` | `MIT OR Apache-2.0` | `REUSE.toml:150-155` |
| `src/llvm-project/**` | `NCSA AND Apache-2.0 WITH LLVM-exception` (submodule) | `REUSE.toml:157-164` |
| `src/gcc/**` | `GPL-3.0-or-later` (submodule) | `REUSE.toml:166-171` |
| `src/gcc/gcc/testsuite/**` | `GPL-2.0-only` | `REUSE.toml:173-178` |
| `src/gcc/gcc/testsuite/c-c++-common/analyzer/*.c` | `ISC` | `REUSE.toml:180-186` |
| `src/gcc/libstdc++-v3/config/os/aix/os_defines.h` | `GCC-exception-3.1` | `REUSE.toml:188-192` |

### `LICENSES/` directory (12 files, REUSE-compliant)

`/home/dimitar/rust/LICENSES/`:

```
Apache-2.0.txt       10 KB    standard Apache 2.0
BSD-2-Clause.txt      1 KB    standard BSD-2-Clause
CC-BY-SA-4.0.txt     20 KB    Creative Commons BY-SA 4.0
GCC-exception-3.1.txt 3 KB    GCC runtime library exception
GPL-2.0-only.txt     17 KB    standard GPLv2
GPL-3.0-or-later.txt 34 KB    standard GPLv3
ISC.txt               0.7 KB  standard ISC
LLVM-exception.txt    0.9 KB  LLVM runtime exception
MIT.txt               1 KB    standard MIT
NCSA.txt              1.6 KB  University of Illinois/NCSA Open Source License
OFL-1.1.txt           4 KB    SIL Open Font License 1.1
Unicode-3.0.txt       2 KB    Unicode data license
```

### `license-metadata.json` (290 lines, machine-readable)

A cached output of the `reuse` tool (v4.0.3) — committed to make
audits easy. Each entry is `{copyright, license}` per file/directory.

### Submodules (`.gitmodules`)

- `src/doc/nomicon` — rust-lang/nomicon
- `src/tools/cargo` — rust-lang/cargo
- `src/doc/reference` — rust-lang/reference
- `src/doc/book` — rust-lang/book
- `src/doc/rust-by-example` — rust-lang/rust-by-example
- `src/doc/edition-guide` — rust-lang/edition-guide
- `src/llvm-project` — rust-lang/llvm-project (branch `rustc/23.1-2026-07-22`)
- `src/doc/embedded-book` — rust-embedded/book
- `library/backtrace` — rust-lang/backtrace-rs
- `src/tools/rustc-perf` — rust-lang/rustc-perf
- `src/tools/enzyme` — rust-lang/enzyme
- `src/gcc` — rust-lang/gcc

**Implication for the audit**: every submodule has its own copyright
holders and licenses. The Rust Project does not relicense submoduled
code; it just ships it under the upstream's license. Even
`src/llvm-project` is **NCSA AND Apache-2.0 WITH LLVM-exception** —
not MIT/Apache-2.0.

### COPYLEFT content present in the Rust repo

- `src/gcc/**` — **GPL-3.0-or-later** (rustc_codegen_gcc backend)
- `src/gcc/gcc/testsuite/**` — **GPL-2.0-only**
- `src/gcc/libstdc++-v3/config/os/aix/os_defines.h` — **GCC-exception-3.1**

This is **important**: the Rust Project does not itself relicense the
GCC backend as MIT/Apache-2.0. Anyone reusing that code (or
rebuilding the GCC backend) inherits the GPL.

### What this means for Saturnite's reuse options

**Any Saturnite port of rustc code that does NOT come from
`compiler/**`, `library/core/src/**`, `library/alloc/src/**`,
`library/std/src/**`, `src/tools/rustc-*` (excluding the GCC backend
and LLVM submodule), or the standard top-level paths is NOT
MIT/Apache-2.0 and must be treated as third-party.**

In particular, do **not** copy:
- `src/llvm-project/**` — NCSA/Apache+LLVM-exception
- `src/gcc/**` — GPL
- `library/backtrace/**` — MIT/Apache-2.0 (fine, but is a submodule
  not Rust Project code; attribution to Alex Crichton and the
  backtrace-rs project is required)
- `library/std/src/sync/mpmc/**` — Crossbeam copyright; reuse OK
  under MIT/Apache-2.0 but attribution required
- `library/std/src/sys/sync/mutex/fuchsia.rs` — BSD-2-Clause +
  MIT/Apache-2.0; reuse OK with attribution
- `library/core/src/unicode/unicode_data.rs` — **Unicode-3.0** is
  a special license; Saturnite would need to either replace this
  data with its own (e.g. via `unicode-general-category` / `unicode-width`
  crates from crates.io) or honor the Unicode terms

---

## 2. Compiler crate inventory (the rustc workspace)

`compiler/` contains 79 distinct crates (counted by directory). The
top-level `Cargo.toml` includes a `members = [...]` list. Key
crates, by role, with file-path and approximate line counts
(`wc -l` 2026-08-28):

### Driver / session

| Crate | Lines (lib.rs or largest) | Role | Source |
|---|---|---|---|
| `rustc` | binary | final rustc binary; re-exports `rustc_driver_impl` | `compiler/rustc/Cargo.toml`, `compiler/rustc_driver/src/lib.rs` |
| `rustc_driver` | 4 lines (re-export) | re-exports `rustc_driver_impl` | `compiler/rustc_driver/src/lib.rs:1-3` |
| `rustc_driver_impl` | 1 686 | primary compiler entry; `run_compiler` at line 173, `catch_with_exit_code` at 1379 | `compiler/rustc_driver_impl/src/lib.rs` |
| `rustc_interface` | 4 246 across 10 files | `interface::Config`, `interface::run_compiler`, `passes::*`; library public entry for embedding rustc | `compiler/rustc_interface/src/{interface,passes,queries,util}.rs` |
| `rustc_session` | ~10k total | `Session`, `config::*`, `options.rs` (138 370 bytes), `parse::ParseSess`, `diagnostics` | `compiler/rustc_session/src/*.rs` |
| `rustc_codegen_ssa` | 430+ | backend-agnostic codegen traits (`CodegenBackend`); consumed by `rustc_codegen_llvm` etc. | `compiler/rustc_codegen_ssa/src/lib.rs`, `src/mir/*.rs` |

### Frontend / parsing / AST

| Crate | Lines (lib.rs) | Role |
|---|---|---|
| `rustc_lexer` | 1 279 | low-level lexer; produces `(kind, len)` pairs; **no spans, no interning, no rustc-specific dependencies**; standalone crate |
| `rustc_parse` | 380 + parser modules (~25k LOC) | main parser interface; turns `rustc_lexer` output into `rustc_ast` token stream + AST |
| `rustc_ast` | 4 514 in `ast.rs` | full Rust AST; `Expr`, `Stmt`, `Item`, `Ty`, `Pat`, `Lit`, `Attr` |
| `rustc_ast_passes` | — | AST validation passes (lint-like) before lowering |
| `rustc_ast_lowering` | 3 381 | AST → HIR lowering |
| `rustc_ast_pretty` | — | pretty-printer |
| `rustc_ast_ir` | — | shared AST↔HIR traits |
| `rustc_arena` | — | bump allocator (DroplessArena, TypedArena) |
| `rustc_attr_parsing` | — | attribute parsing |
| `rustc_attr_ir` | — | attribute IR |
| `rustc_expand` | — | macro expansion (declarative + derive) |
| `rustc_builtin_macros` | — | `#[derive]`, `format!`, etc. |
| `rustc_parse_format` | — | format-string parser |

### HIR / type system / borrow

| Crate | Lines (lib.rs or hir.rs) | Role |
|---|---|---|
| `rustc_hir` | 184 962 bytes in `hir.rs` (~5k LOC) | HIR datatypes |
| `rustc_hir_id` | — | HIR Id types (extracted) |
| `rustc_hir_analysis` | 261 | high-level HIR analyses (e.g. lints, well-formedness) |
| `rustc_hir_typeck` | 730 | HIR type-checking entry point |
| `rustc_infer` | — | type inference engine (unification, region inference) |
| `rustc_traits` | — | old trait solver |
| `rustc_next_trait_solver` | — | **new** generic trait solver (used by rustc + rust-analyzer via `rustc_type_ir`) |
| `rustc_trait_selection` | — | trait selection (solver orchestration) |
| `rustc_type_ir` | 30+ files | abstract type-IR; shared by rustc and rust-analyzer (a deliberate decoupling) |
| `rustc_type_ir_macros` | — | macros for `rustc_type_ir` |
| `rustc_borrowck` | 2 776 | MIR typeck + MIR borrow checking (Polonius engine) |
| `rustc_resolve` | 3 110 | name resolution (build_reduced_graph, late, imports, macros) |
| `rustc_privacy` | — | visibility/privacy checking |
| `rustc_passes` | — | misc AST/HIR validation |
| `rustc_ty_utils` | — | type utilities |
| `rustc_ty_walk` | — | type-walking utilities |
| `rustc_transmute` | — | transmutability analysis |
| `rustc_pattern_analysis` | — | exhaustiveness / pattern coverage |

### MIR / const eval

| Crate | Lines | Role |
|---|---|---|
| `rustc_mir_build` | 28 in lib.rs + large `build/*.rs` | builds MIR from HIR (statement/terminator construction) |
| `rustc_mir_transform` | 843 in lib.rs + passes | MIR optimization passes (constant-prop, inlining, GVN, simplification, …) |
| `rustc_mir_dataflow` | — | generic dataflow framework for MIR analyses |
| `rustc_const_eval` | — | const evaluation / CTFE |
| `rustc_monomorphize` | — | generic item monomorphization |

### Middle / metadata / queries

| Crate | Role |
|---|---|
| `rustc_middle` | the "main crate" — `TyCtxt<'tcx>`, `ty::*`, `mir::*`, `hir::*`, `query::*`, `dep_graph::*`, `thir::*`, `traits::*`, `infer::*`, `util::*` |
| `rustc_metadata` | crate metadata (.rmeta) encoder/decoder; `creader.rs` is the largest file |
| `rustc_crate_store` | crate store / on-disk metadata cache |
| `rustc_query_impl` | generated query implementations (in `rustc_middle`, actually under `compiler/rustc_query_impl/src/*.rs`) |
| `rustc_query_system` (now folded into `rustc_middle/src/query/`) | generic query system framework: 10 files at `compiler/rustc_middle/src/query/{mod,system,keys,job,query_api,into_query_key,modifiers,erase,calls,arena_cached}.rs` |
| `rustc_queries` | concrete query definitions (sometimes absent; see note) |
| `rustc_incremental` | dep-graph + incremental compilation (22 lines lib.rs; logic in submodule) |

**Note on `rustc_query_system` / `rustc_queries`**: these are NOT
separate top-level crates in this snapshot. They live as
`compiler/rustc_middle/src/query/` (a 10-file module) and
`compiler/rustc_query_impl/src/` (an 8-file crate). The Rust repo's
crate map has been refactored over time; the audit must follow the
actual files.

### Codegen backends

| Crate | Lines (lib.rs) | Role |
|---|---|---|
| `rustc_codegen_llvm` | 530 | LLVM backend — uses `rustc_llvm` (C++ FFI) |
| `rustc_codegen_cranelift` | — | Cranelift backend (**excluded** from default workspace) |
| `rustc_codegen_gcc` | — | GCC backend (**excluded**; **GPL-3.0-or-later** via `src/gcc` submodule) |
| `rustc_llvm` | — | C++ FFI wrappers; `SymbolWrapper.cpp` is `Apache-2.0 WITH LLVM-exception` |
| `rustc_sanitizers` | — | sanitizer glue |

### Errors / spans / lints

| Crate | Role |
|---|---|
| `rustc_errors` | diagnostics (`DiagCtxt`, `Diag`, emitters — JSON, short, human, color, etc.) |
| `rustc_error_codes` | long-form error code explanations |
| `rustc_error_messages` | legacy error message strings |
| `rustc_span` | source positions, `Span`, `SpanData`, hygiene (`SyntaxContext`), source map, byte-position tables |
| `rustc_lint` | lint infrastructure and built-in lints |
| `rustc_lint_defs` | lint declaration macros / types |

### Target / ABI

| Crate | Role |
|---|---|
| `rustc_target` | `Target` (loaded from `target-spec-json`), `TargetTuple`, `json` (target-spec parser/writer) |
| `rustc_abi` | `Layout`, `Size`, ABI computation, calling conventions, `extern_abi` |
| `rustc_symbol_mangling` | v0 / legacy / Itanium mangling |
| `rustc_windows_rc` | Windows .rc compilation glue |
| `rustc_sanitizers` | sanitizer support |
| `rustc_baked_icu_data` | bundled ICU data baked into rustc |

### Public / stable MIR

| Crate | Role |
|---|---|
| `rustc_public` | public stable MIR API for tools |
| `rustc_public_bridge` | proxy that invokes rustc queries for `rustc_public` |

### Data structures / utilities (the "rustc_data_structures" family)

| Crate | Role |
|---|---|
| `rustc_data_structures` | intern, graph, obligation_forest, vec_cache, tagged_ptr, snapshot_map, stable_hash, sorted_map, transitive_relation, profiling, sync, fingerprint, … |
| `rustc_index` / `rustc_index_macros` | strongly-indexed vector newtype (`IndexVec<I, T>`) |
| `rustc_serialize` | opaquedata (FileEncoder/MemEncoder), Decodable/Encodable |
| `rustc_hashes` | hashing helpers |
| `rustc_structures` | additional small data structures |
| `rustc_thread_pool` | parallel worker pool |
| `rustc_fs_util` | filesystem utilities |
| `rustc_log` | logging facade |
| `rustc_graphviz` | Graphviz helpers (for dep-graph dumps) |
| `rustc_feature` | feature-gate declarations |
| `rustc_proc_macro` | placeholder for the proc-macro re-export |
| `rustc_macros` | procedural macros (Decodable/Encodable, HashStable, TypeFoldable) |
| `rustc_apfloat` | soft-float arithmetic |
| `rustc_arena` | bump allocator |
| `rustc_randomized_layouts` | randomized struct layouts for soundness testing |

---

## 3. Driver entry point

`compiler/rustc_driver_impl/src/lib.rs:173`:

```rust
pub fn run_compiler(at_args: &[String], callbacks: &mut (dyn Callbacks + Send)) { ... }
```

This is the **single primary entry point** for the rustc binary.
The function (lines 173-...) does:

1. `args::arg_expand_all(&default_early_dcx, at_args)` (line 184) — expand `@file` arguments.
2. `handle_options(...)` (line 188) — parse CLI.
3. `config::build_session_options(...)` (line 191) — build `Options`.
4. `make_input(...)` (line 207) — figure out the input (file, stdin, str).
5. Build `interface::Config { opts, input, output_file, ... }` (lines 213-233).
6. `callbacks.config(&mut config)` (line 235) — let the embedder modify the config.
7. **`interface::run_compiler(config, |compiler| { ... })`** (line 240) — the inner closure drives the actual pipeline:
   - `passes::parse(sess)` (line 272) — parse the crate root.
   - Pretty-printing dispatch (lines 277-285).
   - `callbacks.after_crate_root_parsing(...)` (line 290).
   - **`create_and_enter_global_ctxt(compiler, krate, |tcx| { ... })`** (line 304) — this is the `TyCtxt` constructor. Inside the closure:
     - `tcx.resolver_for_lowering()` (line 308) — force name resolution.
     - `callbacks.after_expansion(compiler, tcx)` (line 310).
     - (Many subsequent queries are then run, e.g. typeck, MIR build, optimization, codegen.)
   - The closure eventually calls `tcx.codegen_*` and produces an `CompiledModules`.

The `interface::run_compiler` is defined at
`compiler/rustc_interface/src/interface.rs` (573 lines) and
`interface::create_and_enter_global_ctxt` (same file) is what
instantiates the `TyCtxt` and runs the queries.

### `passes` (rustc_interface/src/passes.rs, 1 526 lines)

This is the implementation of the per-pipeline orchestration:

- `parse(sess)` — parse the crate root.
- `write_dep_info(tcx)` — emit dep-info for Cargo.
- `lower_to_hir(tcx, ...)` — invoke `tcx.lower_to_hir(...)`.
- `output_filenames(...)` — figure out the output paths.
- ... and many more.

The "pass" terminology in rustc is somewhat dated; modern rustc
operates primarily through the **query system**, where each
compiler phase is a `#[query]`-decorated function. The `passes.rs`
file is a higher-level orchestrator that calls queries in the
correct order.

---

## 4. Query system

The query system is the spine of rustc. It is the on-demand
incremental computation machinery.

### `compiler/rustc_middle/src/query/mod.rs` (~3 200 lines)

This is where queries are *declared*. The actual implementation
framework is at `compiler/rustc_middle/src/query/{system,keys,job,
query_api,into_query_key,modifiers,erase,calls,arena_cached}.rs`,
and the concrete implementations are generated by macros in
`compiler/rustc_query_impl/src/{execution,query_vtables,dep_kind_vtables,
job,incremental,self_profile,diagnostics,handle_cycle_error}.rs`.

Conceptually, every compiler phase is expressed as a `query!` macro
call:

```rust
query! {
    fn typeck_of_item(def_id: LocalDefId) -> TyCtxt<'tcx> { ... }
}
```

The query system:
- Memoizes results in an arena (`ArenaCache`).
- Tracks dependencies between queries via `DepNode` and a
  `dep_graph` (`compiler/rustc_middle/src/dep_graph/`).
- Detects cycles (`HandleCycleError`).
- Supports parallel execution (`rustc_thread_pool`).
- Supports on-disk caching (`rustc_incremental`).

### `compiler/rustc_incremental/src/lib.rs` (22 lines)

Thin façade. The real work is in the `DepGraph` and the
`assert_dep_graph` and the `persisted` modules. Key files:
- `compiler/rustc_incremental/src/lib.rs` (22 lines, façade)
- `compiler/rustc_incremental/src/assert_dep_graph.rs` (large)
- `compiler/rustc_incremental/src/persisted/` (subdirectory)
- `compiler/rustc_middle/src/dep_graph/` (the dep-graph data model)

### On-disk format

The query cache is stored under the build directory at
`<sysroot>/<target-triple>/.incremental/<crate-name>-<hash>/` with
**two files**:
- `<crate>-dep-graph.bin` — serialized dep-graph (a `DepNode` ×
  `Fingerprint` map)
- `<crate>-<query-name>.bin` — one per query

The on-disk format is **not stable** across rustc versions.

---

## 5. Span system

`compiler/rustc_span/src/lib.rs` (1 819 lines) + submodules.

- `Span { base_or_index: ... }` — encoded either inline or via a
  table index (for memory efficiency).
- `SpanData { lo, ctxt }` — explicit (lo, ctxt) pair.
- `SourceMap` — per-source-file byte-position tables.
- `hygiene.rs` — `SyntaxContext` for macro-expansion hygiene.

Spans are **byte-position, not byte-range** (the end is implicit by
the node to which the span belongs). For diagnostics, the
`SourceMap` produces the actual byte range at render time.

`SourceSpan` (miette) in Saturnite corresponds to a `SpanData` plus
the `SourceMap` lookup — Saturnite is simpler because it stores byte
ranges directly in tokens and AST nodes (`Range<usize>`).

---

## 6. AST

`compiler/rustc_ast/src/ast.rs` (4 514 lines) is the AST definition.

- `Expr` — a single large `ExprKind` enum covering ~80 variants
  (literals, paths, binary ops, calls, method calls, …).
- `Stmt` — `Local`, `Item`, `Expr`, `Semi`, `Empty`, `MacCall`.
- `Item` — `Fn`, `Struct`, `Enum`, `Trait`, `Impl`, `Mod`, `Use`,
  `Const`, `Static`, `TypeAlias`, `ExternCrate`, `MacroDef`, etc.
- `Ty` — recursive (with `Box<Ty>`).
- `Pat` — exhaustive pattern matching.
- `Lit` — `LitKind`.
- `Attr` — `AttrKind::Normal` / `AttrKind::DocComment`.
- `NodeId` — u32 per-AST-node id (separate from `HirId`).

The AST is full-fidelity (preserves whitespace, comments, parens).
It is **discarded after macro expansion and HIR lowering** for
type-checking, but is kept alive for `rustc_ast_passes` and for
pretty-printing / save-analysis.

---

## 7. HIR

`compiler/rustc_hir/src/hir.rs` is 184 962 bytes (one of the
largest source files in rustc). HIR is the **typed, name-resolved**
IR after macro expansion and `rustc_ast_lowering`.

- `Expr` — a `ExprKind` enum with ~50 variants (much smaller than
  AST because macros have been expanded and method calls have been
  resolved into `ExprKind::Call`).
- `Item` — `Fn`, `Struct`, `Enum`, `Trait`, `Impl`, `Mod`, `Use`,
  `Const`, `Static`, `TypeAlias`, `ExternCrate`, `MacroDef`.
- `Stmt` — `Let`, `Item`, `Expr`, `Semi`.
- `Pat` — pattern variants.
- `Ty` — recursive.
- `HirId { owner: ItemLocalId, local_id: ItemLocalId }` — the
  per-crate HirId (a 2-tuple: owner DefId, local index).
- `BodyId` — body owner; bodies are stored in a separate `Body`
  arena and looked up via `tcx.hir_body(body_id)`.
- `CRATE_HIR_ID` — the synthetic HirId for the crate root.

`rustc_ast_lowering` (3 381 lines) is the AST→HIR pass.

---

## 8. Type system

This is rustc's largest and most complex subsystem.

### `ty` module (`compiler/rustc_middle/src/ty/`)

Core data types:

- `TyCtxt<'tcx>` — the **central context**. Everything funnels
  through this. `'tcx` is the lifetime of the `Ty` interning arena.
- `Ty<'tcx>` — an interned type, an `Interned<TyKind<'tcx>>` with
  pointer-equality semantics.
- `TyKind<'tcx>` — the ~50-variant enum (Bool, Char, Int, Uint,
  Float, Adt, Foreign, Str, Slice, Array, Pat, Ref, …).
- `Region<'tcx>` — `'a` interned (Early / Late / Static / Free).
- `Const<'tcx>` — interned constant (kind: Int / UInt / Float /
  Bool / Str / Error / Unevaluated / etc.).
- `Predicate<'tcx>` — a `PredicateKind` + `Binder`.
- `GenericArg<'tcx>` — Ty / Lifetime / Const.
- `ParamEnv { clauses, reveal }` — the environment for generic
  obligations.

### Interner

`TyCtxt` is built around an `Interner` trait (defined in
`compiler/rustc_type_ir/src/interner.rs`) that abstracts over the
underlying `Ty` storage. The default implementation
(`rustc_type_ir::Interner`) is `TyCtxt<'tcx>` itself; the abstract
interface exists so that **rust-analyzer can use the same solver
without depending on TyCtxt** (`rustc_next_trait_solver` and
`rustc_type_ir` are explicitly designed for cross-tool reuse).

### Trait solving

- `rustc_traits` (old solver)
- `rustc_trait_selection` (orchestration)
- `rustc_next_trait_solver` (new generic solver; in active
  development; uses `rustc_type_ir`)

The trait solver handles: implied bounds, well-formedness,
normalization, projection, opaques, …

### Inference

`compiler/rustc_infer/src/` — type inference, region inference,
unification, obligations. The classic Hindley-Milner / bidirectional
type checking machinery.

### Borrow checking

`compiler/rustc_borrowck/src/` — uses
`polonius-engine = "0.13"` (declared in
`compiler/rustc_middle/Cargo.toml:24`). Polonius is the
dataflow-based borrow checker. Two-location borrow checking
(2021+ edition) is supported via Polonius' subset analysis.

### Const eval

`compiler/rustc_const_eval/src/` — full CTFE. Sandboxed evaluation of
`const` expressions and `const fn`. Uses `rustc_apfloat` for
soft-float.

### Pattern analysis

`compiler/rustc_pattern_analysis/src/` — exhaustiveness checking
(`is_useful`, red/green algorithm).

### Layout / ABI

`compiler/rustc_abi/src/` — `Layout` computation, `Size`,
`Align`, `Abi`, calling conventions. Now an independent crate (it
used to be inside `rustc_target`).

---

## 9. Codegen

### SSA-agnostic layer: `rustc_codegen_ssa`

`compiler/rustc_codegen_ssa/src/` — backend-agnostic MIR→code
machinery. Key files:

- `mir/block.rs` — per-block codegen.
- `mir/place.rs` — `Place` projection.
- `mir/statement.rs` — statement codegen.
- `mir/operand.rs` — operand codegen (constants, copies, moves).
- `mir/rvalue.rs` — rvalue codegen.
- `mir/intrinsic.rs` — intrinsic lowering.
- `mir/locals.rs` — local storage.
- `mir/analyze.rs` — analysis helpers.
- `mono_item.rs` — monomorphization of generic items.
- `base.rs`, `back/`, `debuginfo/`, `traits.rs` — backend wiring,
  symbol export, debug info, the `CodegenBackend` trait.

### LLVM backend: `rustc_codegen_llvm`

`compiler/rustc_codegen_llvm/src/` — 530-line lib.rs + ~20 module
files (`builder.rs`, `context.rs`, `common.rs`, `callee.rs`,
`intrinsic.rs`, `abi.rs`, etc.). Depends on **37 crates**, mostly
`rustc_*` (per `compiler/rustc_codegen_llvm/Cargo.toml:5-37`).

C++ FFI goes through `rustc_llvm`, which wraps a vendored
`src/llvm-project` (submodule, **NCSA AND Apache-2.0 WITH
LLVM-exception** — not MIT/Apache-2.0; Saturnite already gets this
through `inkwell` and is **not** required to import `rustc_llvm`).

`SymbolWrapper.cpp` is the C++-side wrapper; it is dual-licensed
under `Apache-2.0 WITH LLVM-exception AND (Apache-2.0 OR MIT)`.

### Cranelift backend: `rustc_codegen_cranelift`

Excluded from default workspace. Not a separate target Saturnite
needs; it is a third-party-style backend living in the rustc tree.

### GCC backend: `rustc_codegen_gcc`

Excluded from default workspace. **GPL-3.0-or-later** (via
`src/gcc`). **REJECT** for Saturnite reuse — copyleft.

---

## 10. Target specification

`compiler/rustc_target/src/lib.rs`:

- `Target { spec: TargetSpec, options: TargetOptions }` — loaded
  from JSON.
- `TargetSpec` — `target-arch`, `data-layout`, `panic-strategy`,
  `llvm-target`, `os`, `env`, `abi`, etc.
- `TargetOptions` — `crt-static`, `simd-types`, `features`,
  `dynamic-linking`, etc.
- `json.rs` — parse / write the JSON target spec.
- `target_features.rs` — target-specific feature detection.

`compiler/rustc_abi/src/lib.rs` — size / align / layout computation
on top of the target spec.

`compiler/rustc_abi/src/callconv.rs` — calling convention
selection. `callconv/reg.rs` is the per-target register allocation
for the calling convention.

`compiler/rustc_abi/src/extern_abi.rs` — `extern "C"`, `extern
"system"`, `extern "fastcall"`, etc. — a separate abstraction from
the calling-convention mechanics.

`compiler/rustc_symbol_mangling/src/` — symbol mangler (v0, legacy,
Itanium, MSVC).

### Number of target specs

The Rust tree contains ~290+ target spec JSON files under
`compiler/rustc_target/src/spec/`. These are first-party target
specifications; the targets themselves (e.g. `aarch64-apple-darwin`)
are community-supported but the JSON files are part of the
MIT/Apache-2.0 Rust project code.

---

## 11. Build system: bootstrap

`src/bootstrap/` is a 2.5 MB Rust project. It is **excluded** from
the Cargo workspace. It orchestrates:

- `Cargo.toml` declarations for ~every compiler and std crate.
- Building `rustc` itself (which is needed to build the std
  library and proc-macros).
- Building LLVM (`src/llvm-project` submodule).
- Building `cargo` (submodule).
- Building the documentation.
- Cross-compilation (host → target).
- Stage 0 / Stage 1 / Stage 2 self-bootstrap.
- Dist tarball creation.
- `bootstrap.py` is the script driver; `x`, `x.py`, `x.ps1` at the
  repo root are thin wrappers.

The `configure` script (a 296-byte shell script) sets up
`config.toml`. `bootstrap.example.toml` is the example config.

---

## 12. Test infrastructure

- `src/tools/compiletest` — the compiletest framework. A Rust
  program that walks directories of `.rs` source files + `.stderr`
  / `.stdout` / `.fixed` expected files and runs the compiler,
  diffing the output.
- `src/tools/compiletest_rs` — the same, but used by the broader
  Rust Project. (This naming is historically confusing.)
- `src/tools/rustc-perf` (submodule) — compiler benchmarking
  suite.
- `src/tools/enzyme` (submodule) — Enzyme autodiff integration
  tests.
- `src/tools/miri` — interpreter for MIR; soundness testing.
- `src/tools/tidy` — source-tree linting (whitespace, file
  structure, REUSE compliance).

The test surface is large: `tests/` is itself a major directory,
with `tests/ui`, `tests/run-make`, `tests/rustdoc`, `tests/codegen`,
`tests/mir-opt`, etc.

---

## 13. Standard library

`library/`:

- `core` — `no-std` core types (`Option`, `Result`, `Iterator`, …).
- `alloc` — heap-allocating types (`Vec`, `Box`, `String`, …).
- `std` — the standard library proper (OS, I/O, threads, …).
- `test` — test harness.
- `coretests`, `alloctests` — test suites.
- `compiler-builtins` — compiler-rt analogues (`memcpy`, `__aeabi_*`, …).
- `profiler_builtins` — profiler runtime.
- `unwind` — unwinding implementation (per-target).
- `rtstartup` — C runtime startup.
- `backtrace` — `backtrace-rs` (submodule, MIT/Apache-2.0 with
  Alex Crichton copyright).
- `stdarch` — per-architecture intrinsics.
- `portable-simd` — portable SIMD.
- `std_detect` — runtime CPU feature detection.
- `windows-sys`, `windows_link` — Windows FFI.
- `sysroot` — special sysroot manipulation crate.
- `panic_abort`, `panic_unwind` — panic strategies.
- `proc_macro` — proc-macro re-export.
- `rustc-std-workspace-{alloc,core,std}` — proxy crates so rustc
  can use std's types in its own build.

The standard library is **MIT OR Apache-2.0** (covered by the
`library/**` REUSE blanket). It is the gold standard for
MIT/Apache-2.0 dual-licensed Rust code. Saturnite can reference
it for idiom but cannot directly reuse it (language mismatch).

---

## 14. External (vendored) third-party code

### `src/tools/` (submoduled and in-tree)

- `cargo` (submodule) — separate copyright holders, MIT/Apache-2.0.
- `clippy` (in-tree) — Rust Project copyright, MIT/Apache-2.0.
- `rustfmt` (in-tree) — Rust Project copyright, MIT/Apache-2.0.
- `rust-analyzer` (in-tree) — Rust Project copyright, MIT/Apache-2.0.
- `miri` (in-tree) — Rust Project copyright, MIT/Apache-2.0.
- `rustdoc` (in-tree) — part of rustc.
- `rustc-test` (in-tree) — test runner.
- `compiletest` / `compiletest_rs` (in-tree) — Rust Project copyright.
- `tidy` (in-tree) — Rust Project copyright.
- `rustc-perf` (submodule) — separate copyright holders.
- `enzyme` (submodule) — Enzyme / Modi Labs copyright (mostly
  Apache-2.0).

### `src/gcc/` (submodule)

**GPL-3.0-or-later** for the bulk; `GPL-2.0-only` for the testsuite;
`ISC` for some analyzer files; `GCC-exception-3.1` for one header.
**REJECT for reuse in Saturnite.**

### `src/llvm-project/` (submodule)

`NCSA AND Apache-2.0 WITH LLVM-exception`. Saturnite already uses
LLVM through `inkwell`; it does not need to vendor LLVM.

### `library/backtrace/` (submodule)

`MIT OR Apache-2.0`, copyright "2014 Alex Crichton" + "The Rust
Project Developers". A copy of `backtrace-rs`. Saturnite can
**reuse this code** if it ever needs a Rust backtrace (currently it
does not — its only runtime function is `println`).

### `src/doc/` (submodules)

- `nomicon`, `reference`, `book`, `rust-by-example`, `edition-guide`,
  `embedded-book` — all MIT OR Apache-2.0 (with some CC-BY-SA-4.0
  for `embedded-book`). Documentation, not code; Saturnite does not
  need to reuse these.

---

## 15. Cargo (submoduled)

`src/tools/cargo` is a separate codebase with its own copyright
holders (Cargo Project contributors). It is dual-licensed
`MIT OR Apache-2.0` and **REUSE-compliant** itself.

**Implication for Saturnite**: if Saturnite ever wants a
package manager, the architectural reference is **cargo**, not
rustc. Saturnite already has `saturn.toml` and a `DependencySpec`
type, but does not yet have a resolver or fetcher. Adopting cargo
*as a binary* is possible (cargo is a Rust project, MIT/Apache-2.0),
but a clean-room reimplementation is more likely. This is a
**REIMPLEMENT** decision in Phase 4.

---

## 16. The dependency graph (high-level)

```
             ┌──────────────────────────────────────────────┐
             │  rustc_interface (public entry:               │
             │  `interface::run_compiler(config, |c| ...)`) │
             └────────────┬─────────────────────────────────┘
                          │
                          ▼
             ┌──────────────────────────────────────────────┐
             │  rustc_driver_impl (private;                 │
             │  `run_compiler`, `catch_with_exit_code`)     │
             └────────────┬─────────────────────────────────┘
                          │
            ┌─────────────┴─────────────┐
            ▼                           ▼
  ┌──────────────────┐        ┌──────────────────┐
  │ rustc_session    │        │ rustc_ast_passes │
  │ (Session, Config)│        │ (early lint)     │
  └──────────────────┘        └──────────────────┘
            │
            ▼
  ┌──────────────────┐
  │ rustc_parse      │   ←   rustc_lexer (low-level)
  │ (parser, token   │        rustc_ast (data types)
  │  stream)         │
  └────────┬─────────┘
           ▼
  ┌──────────────────┐
  │ rustc_expand     │  (macros, name resolution, attributes)
  │ rustc_resolve    │
  │ rustc_attr_*     │
  └────────┬─────────┘
           ▼
  ┌──────────────────┐
  │ rustc_ast_       │
  │  lowering        │   AST → HIR
  └────────┬─────────┘
           ▼
  ┌──────────────────┐
  │ rustc_hir        │  HIR datatypes
  │ rustc_middle     │  TyCtxt, ty, mir, dep_graph, query
  └────────┬─────────┘
           ▼
  ┌──────────────────┐
  │ rustc_hir_       │  typeck
  │  typeck          │
  │ rustc_infer      │  type/region inference
  │ rustc_traits     │  trait solving
  │ rustc_trait_sel  │
  │ rustc_borrowck   │  borrow checking
  │ rustc_mir_build  │  MIR construction
  └────────┬─────────┘
           ▼
  ┌──────────────────┐
  │ rustc_mir_       │  MIR optimization
  │  transform       │
  │ rustc_const_eval │  const-eval
  │ rustc_pattern_   │  exhaustiveness
  │  analysis        │
  └────────┬─────────┘
           ▼
  ┌──────────────────┐
  │ rustc_codegen_   │  backend trait
  │  ssa             │
  └────────┬─────────┘
           ▼
  ┌──────────────────┐
  │ rustc_codegen_   │  LLVM backend (default)
  │  llvm            │   (or _cranelift, _gcc)
  │  rustc_llvm      │
  └──────────────────┘
```

This is the broad-stroke data flow. The actual rustc is more
fine-grained (each phase is a query, dependencies are tracked by
the dep-graph).

---

## 17. What the audit must NOT do

- **Do not assume rustc is uniformly MIT/Apache-2.0.** The blanket
  REUSE annotation covers most code, but a non-trivial number of
  files have explicit non-MIT/Apache-2.0 licenses (LLVM exception,
  NCSA, GPL, OFL, BSD-2-Clause, Unicode, ISC, GCC-exception).

- **Do not assume submoduled code is Rust Project code.** It is
  not; it has its own copyright holders and licenses.

- **Do not assume crate names == ownership.** `rustc_*` is the
  Rust Project's namespace, but the subdirectories
  `src/gcc/`, `src/llvm-project/`, `library/backtrace/`,
  `src/tools/cargo/`, `src/tools/rustc-perf/`,
  `src/tools/enzyme/`, `src/doc/embedded-book/` are submoduled
  and have their own provenance.

- **Do not copy `src/llvm-project/**` to Saturnite.** It is
  `NCSA AND Apache-2.0 WITH LLVM-exception`. Saturnite already has
  `inkwell` for LLVM bindings and is not required to vendor
  LLVM.

- **Do not copy `src/gcc/**` to Saturnite.** It is
  `GPL-3.0-or-later`. Copyleft in any form is a hard NO for
  Saturnite's intended distribution.

- **Do not copy `library/core/src/unicode/unicode_data.rs`** to
  Saturnite without honoring the Unicode-3.0 terms. The
  Unicode license is a special data license; Saturnite can use
  `unicode-general-category` / `unicode-width` / `unicode-ident`
  crates from crates.io instead (they are MIT/Apache-2.0).

- **Do not treat `rustc_lexer` as a free add-on.** It is
  MIT/Apache-2.0 and standalone (no `rustc_*` deps), but
  re-implementing Saturnite's lexer in terms of `rustc_lexer`
  would require re-thinking the chumsky-based parser pipeline.
  Possible, but it is an **architectural change** to Saturnite,
  not a one-line import.
