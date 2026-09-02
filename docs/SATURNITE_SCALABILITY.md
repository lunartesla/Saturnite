# SATURNITE SCALABILITY (0.5.2)

Labels: IMPLEMENTED / DESIGNED / PLANNED / DEFERRED

## Current architecture

Pipeline: lexer -> parser -> AST -> HIR lowering (with name resolution folded in) -> resolver/module graph -> MIR -> codegen -> runtime.
Builtins: minimal `Builtin` metadata structure added in `hir/symbol.rs` (`BuiltinRegistry`). No plugin framework.
Module: `ModuleGraph`, `ModulePath`, `ModuleScope`, `ModuleId`, `DefTable`, `DefKind` exist; `detect_cycle` defensive guard implemented; visibility enforcement deferred.
Runtime boundary: runtime primitives (`println_i64.c`) owned by runtime; compiler references ABI via inline declarations; no full ABI interface file yet (designed, not fully implemented).
String interpolation: minimal arena (`SymbolInterner`); unchanged.

## What is implemented in 0.5.2

- `Builtin` / `BuiltinRegistry` (IMPLEMENTED) — centralized metadata, not full plugin framework.
- `ErrCategory` convention (IMPLEMENTED) — categories defined; codes not exhaustively assigned.
- `detect_cycle` in module graph (IMPLEMENTED) — defensive DFS; visibility enforcement deferred.
- Runtime boundary documentation and minimal registry (DESIGNED / PARTIAL).
- Assessment document (`.tau/specs/phase0_5_scalability_assessment.md`) (IMPLEMENTED).

## What is intentionally NOT implemented

- Full builtin registry with automatic lowering (DEFERRED).
- Visibility enforcement (DEFERRED to 1.0).
- Error code assignment for all variants (DEFERRED; only category convention exists).
- Package manager / registry (DEFERRED).
- Standard library (`saturnite-std`) (DEFERRED).
- Python / Rust-lang crate interoperability (DEFERRED; architecture leaves room).
- Memory-management redesign / GC (DEFERRED; arena model preserved).

## Extension strategy

Builtins: use `Builtin` metadata; special lowering remains explicit.
Module growth: resolver is separate; module graph handles adjacency and cycle detection; visibility is the remaining gap.
Diagnostics: `ErrCategory` supports future `E0xxx` convention.
Runtime: ABI interface should become explicit file (future work); compiler should reference it rather than embedding runtime symbols directly in codegen.
