# Phase 0.5.2 Scalability Assessment

Status: ASSESSMENT / ARCHITECTURAL HARDENING (not full 1.0)
Version line: 0.5.x

## 1. Current architecture (HEAD 79e09e0)

Pipeline (real):
lexer/prepare -> parser (chumsky) -> AST -> HIR lowering (name resolution folded in) -> resolver/module graph (separate) -> MIR lower -> MIR verify -> MIR opt (const-fold only) -> codegen (inkwell/LLVM) -> runtime C (println_i64.c) -> link

Crates:
- stnx (compiler only; workspace root)

Modules (actual):
- lexer, parser, ast, hir (lower, types, symbol, stmt, expr), mir (lower, opt, verify, codegen), module, error, runtime (C)

Builtins: no registry file exists; sentinels are hard-coded in `hir/lower.rs`, `mir/lower.rs`, `mir/codegen.rs`:
- PRINTLN_DEF_ID = DefId(u32::MAX - 1)
- PRINTLN_STR_DEF_ID = DefId(u32::MAX - 2)
- CONCAT_STR_DEF_ID = DefId(u32::MAX - 3)
- STR_I64_DEF_ID = DefId(u32::MAX - 4)

String interpolation: minimal arena; `StrLit` uses `SymbolId`; interpolation lowers to nested `Call` to `concat_str` / `str_i64`.

Module system: `ModuleGraph`, `ModulePath`, `ModuleScope`, `ModuleId`, `DefTable`, `DefKind` already exist (`module.rs`). Resolver is partially separate but cycle detection and visibility enforcement are deferred.

Diagnostics: `miette::Diagnostic`, `thiserror`, `CompilerError`, `LinkError`. No structured numeric codes yet; no `E0xxx` convention.

## 2. Extension hotspots

A. Builtin coupling — HIGH
Evidence: `hir/lower.rs:55-60` defines `CONCAT_STR_DEF_ID` / `STR_I64_DEF_ID` as `DefId(u32::MAX - 3/4)`. `mir/codegen.rs:27-39` duplicates them. `mir/lower.rs` repeats. Every new builtin requires inventing another sentinel, duplicating it in 3+ files, duplicating runtime symbol name, duplicating LLVM declaration. No registry structure exists.

B. Runtime boundary — MEDIUM
Evidence: runtime owns `println_i64.c`, compiled by `build.rs`. Compiler knows runtime ABI (i64, ptr, etc.) directly in `mir/codegen.rs:104-121`. There is no documented ABI interface file; runtime primitives are declared inline in codegen. This leaks runtime-specific knowledge into compiler layers.

C. HIR lowering feature fan-out — MEDIUM
Evidence: `hir/lower.rs` handles expressions, statements, interpolation, builtin lowering all in one file. Adding a new expression kind requires touching parser, AST, HIR lowering, MIR lowering, codegen. This is appropriate for a compiler pipeline, but no abstraction exists for builtin lowering specifically.

D. Module/resolver — LOW-MEDIUM
Evidence: `module.rs` is 1514 lines but architecture is sound. `ModuleGraph`, `ModulePath`, `ModuleScope` are clean. Missing: visibility enforcement, cycle detection (documented as deferred), import resolution full integration.

E. Diagnostics — MEDIUM
Evidence: `error.rs` defines `CompilerError` enum and `LinkError`. No code convention (`E0xxx` etc.). No structured `Related` / `Suggestion` infrastructure beyond basic `miette` usage. Adding 1000 diagnostics would become unmanageable without numbering convention and categories.

F. String interpolation / runtime ABI — LOW (deliberately minimal)
Evidence: minimal arena (`SymbolInterner`). No heap management redesign needed. Changing the memory model is out of scope.

## 3. Severity

A — HIGH
B — MEDIUM
C — MEDIUM
D — LOW-MEDIUM
E — MEDIUM
F — LOW (deliberately left minimal)

## 4. Recommended changes (for 0.5.2)

A — Introduce a minimal `Builtin` metadata structure (not full plugin framework). It should hold: source name, runtime symbol string, `DefKind` / `DefId` source, expected ABI (input/output types), and whether it requires special lowering. Keep special lowering explicit (e.g., interpolation needs nested Call construction) rather than pretending all builtins are identical.

B — Document runtime ABI interface explicitly (not redesign runtime). A small document or interface module defining runtime primitives and their signatures. Compiler should reference this rather than embedding strings directly in codegen.

C — Do NOT eliminate pipeline stages. Keep pipeline; only improve boundary clarity (HIR owns its representation; MIR owns CFG; codegen owns LLVM mapping).

D — Add minimal cycle detection and visibility boundary documentation; do not fully implement visibility enforcement (deferred to 1.0).

E — Introduce error-code convention (`E0xxx` lex/parse, `E1xxx` semantic/HIR, `E2xxx` MIR, `E3xxx` codegen/link). Only define categories and a small convention file; do not assign all codes.

F — Leave arena model untouched.

## 5. Non-goals (explicitly NOT implemented in 0.5.2)

- List runtime / Vec<T>
- First-class closures / lambda lifting
- Real Result / error propagation
- Titan
- AI-assisted programming
- Package manager / registry
- Python interoperability (PyO3 / CPython)
- Rust crate interoperability (FFI linking to crates, cargo dependency resolution, rustc source reuse)
- Standard library (`saturnite-std`)
- Full 1.0 feature set
- Memory-management rewrite / GC / reference counting
- Incremental compilation / query system

## 6. Rust reuse declaration

Rust lang (compiler implementation language): YES (standard Rust crates, cargo, rustc 1.98.0)
Rust source / rustc source reuse: NO
No rustc internals, no `rustc_lexer`, `rustc_resolve`, `rustc_data_structures`, `rustc_mir_dataflow`, `compiletest` runner source, or JSON target specs copied.
