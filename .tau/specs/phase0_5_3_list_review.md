# Phase 0.5.3 — Runtime List<T> Architecture Review
Status: ARCHITECTURAL REVIEW (not implementation)
Labels: IMPLEMENTED / DESIGNED / PLANNED / DEFERRED / BLOCKED

## 1. Review scope
This document audits `.tau/specs/phase0_5_3_list.md` against the actual repository at `/home/kali/saturn/Saturnite` (HEAD 5a01c7a) and determines whether the proposed minimal `List<i64>` ABI is compatible with the existing HIR → MIR → LLVM → C runtime pipeline.

No List runtime is implemented in this phase. The review produces a sound design and identifies prerequisites/blockers.

## 2. Evidence from source

### 2.1 AST / Parser
`ast.rs:18-22`: `Type::List(Box<Type>)` exists. `parser/mod.rs:597-606`: `[...]` literal lowers to `Expr::StrLit(format!("[list:{}]", ...)` — a placeholder. `parser/mod.rs:379`: `Type::List(Box<Type>)` parses `List<T>` syntax.
Evidence: `parser/mod.rs`, `ast.rs`
Status: PLACEHOLDER IMPLEMENTED / REAL LIST BLOCKED

### 2.2 HIR types
`hir/types.rs:92-96`: `ast::Type::List(_) => HirType::Struct(SymbolId(0))`. `hir/lower.rs:159-176`: same placeholder (names symbol after inner type but maps to `Struct`).
Evidence: `hir/types.rs`, `hir/lower.rs`
Status: PLACEHOLDER IMPLEMENTED / REAL `HirType::List` BLOCKED

### 2.3 HIR expressions
`hir/expr.rs`: `HirExprKind` has `Integer`, `Float`, `StrLit`, `Bool`, `Unit`, `Variable`, `Assign`, `AugAssign`, `Binary`, `Unary`, `Call`, `If`, `For`, `While`, `Range`, `StructLiteral`, `FieldAccess`, `EnumConstructor`. No `ListLiteral`, `Index`, or `Length`.
Evidence: `hir/expr.rs`
Status: BLOCKED (no list expression variants)

### 2.4 HIR statements
`hir/stmt.rs`: `HirStmtKind` has `Let`, `Expr`, `Return`, `Println`, `PrintlnStr`, `Raise`, `StructDef`, `EnumDef`. No list mutation or index assignment statements.
Evidence: `hir/stmt.rs`
Status: BLOCKED

### 2.5 MIR types / representations
`mir/mod.rs:51`: `MirType = HirType`. `mir/verify.rs`: verifies operand types match use sites. `mir/codegen.rs:768-822`: `mir_type_to_llvm` matches `HirType` variants; `Str` → `ptr_type`, `Struct` → `ptr_type`, `I64` → `i64_type`, etc. `List` is not a `HirType` variant, so `mir_type_to_llvm` has no case for it.
Evidence: `mir/mod.rs`, `mir/codegen.rs`
Status: BLOCKED

### 2.6 MIR lowering (for loops)
`mir/lower.rs:670-774`: `lower_for` requires `HirExprKind::Range`; it creates `loop_var` (`i64`), compares with `end_local`, increments by `MirConst::I64(1)`. It does not handle list iteration.
Evidence: `mir/lower.rs`
Status: BLOCKED (requires new `lower_for_list` or list iterator representation)

### 2.7 Builtin registry
`hir/symbol.rs` (post-0.5.2): `Builtin` struct and `BuiltinRegistry` exist (`BuiltIn` with `runtime_symbol`, `def_id`, `requires_special_lowering`). No list runtime functions (`list_new`, `list_get`, etc.) are registered.
Evidence: `hir/symbol.rs`
Status: DESIGNED (registry exists) / BLOCKED (no list entries)

### 2.8 Runtime C functions
`runtime/println_i64.c`: provides `println_i64`, `println_str`, `concat_str`, `str_i64`, arena (`sat_arena_own`, `sat_arena_free_all`). No list functions. Arena model is string-specific; it frees all heap strings at process exit via `atexit`. It does not support structured collections.
Evidence: `runtime/println_i64.c`
Status: IMPLEMENTED (arena) / BLOCKED (no list runtime)

### 2.9 Runtime boundary documentation
`docs/SATURNITE_SCALABILITY.md` (post-0.5.2): documents that the runtime boundary should become an explicit ABI interface file; it is currently partial (runtime primitives declared inline in codegen). `docs/SATURNITE_1_0_ARCHITECTURE.md`: runtime owns `println_i64.c`; compiler knows ABI directly.
Status: DESIGNED / PARTIAL

### 2.10 LLVM / codegen ABI
`mir/codegen.rs:104-121`: `declare_builtin_functions` declares `println_i64` (i64→i64), `println_str` (ptr→i64), `concat_str` (ptr, ptr → ptr), `str_i64` (i64 → ptr). Function names are hard-coded strings. List functions (`list_new`, etc.) are not declared.
Evidence: `mir/codegen.rs`
Status: BLOCKED

### 2.11 Module / resolver
`module.rs`: `ModuleGraph`, `ModulePath`, `ModuleScope` implemented. `detect_cycle` defensive guard implemented. Visibility enforcement deferred (`docs/SATURNITE_SCALABILITY.md`: deferred).
Status: IMPLEMENTED (graph, cycle) / DEFERRED (visibility)

## 3. Compatibility audit of proposed minimal ABI

The proposed model from `.tau/specs/phase0_5_3_list.md` (before removal):
- Runtime representation: pointer to elements + length + capacity.
- Allocation: `malloc` / `realloc`.
- Type representation: `HirType::List(SymbolId)` referencing element type.
- First pass: `i64` elements only.

### 3.1 Compatibility with current pipeline
- `ast.rs`: `Type::List` exists — COMPATIBLE.
- `parser/mod.rs`: `[...]` parses but lowers to placeholder — COMPATIBLE (requires changing `parser/mod.rs:605` to emit a real list expression).
- `hir/types.rs`: needs new `HirType::List(SymbolId)` — COMPATIBLE (additive).
- `hir/lower.rs`: needs real lowering instead of placeholder (`Struct(SymbolId(0))`) — COMPATIBLE (refactor existing branch).
- `hir/expr.rs`: needs `ListLiteral`, `Index`, `Length` variants — COMPATIBLE (additive).
- `hir/stmt.rs`: needs mutation assignment if mutation supported — COMPATIBLE (additive).
- `mir/mod.rs`: `MirType = HirType`; `MirRvalue` needs `ListLiteral`, `Index`, `Length` — COMPATIBLE (additive).
- `mir/lower.rs`: needs `lower_expr` branches for new `HirExprKind` variants; `lower_for` needs list iteration — COMPATIBLE but requires significant new lowering logic.
- `mir/codegen.rs`: `mir_type_to_llvm` needs `List` case; runtime functions need declaration; `declare_builtin_functions` needs list entries — COMPATIBLE (additive).
- `runtime/println_i64.c`: needs new C file (`runtime/list.c`) with `list_new`, `list_get`, `list_set`, `list_len` — COMPATIBLE (new file).
- `build.rs`: needs to compile new `runtime/list.c` — COMPATIBLE (additive).

### 3.2 Runtime ABI requirements
A C-compatible list structure for `i64` elements:
```c
typedef struct {
    int64_t *data;
    size_t len;
    size_t cap;
} sat_list;
```
LLVM ABI: passed by pointer (`ptr_type`). Runtime functions:
- `sat_list* list_new(size_t cap)` — returns pointer to list struct.
- `int64_t list_get(sat_list* list, size_t index)` — reads element; returns `i64`.
- `void list_set(sat_list* list, size_t index, int64_t value)` — writes element.
- `size_t list_len(sat_list* list)` — returns length.
- `int64_t list_index(sat_list* list, size_t index, int64_t default)` — safe read with bounds check (optional for first pass).

This ABI fits the current `mir/codegen.rs` pattern: runtime functions declared in module, called via `MirTerminator::Call`, arguments materialized by `materialize_operand`, results stored in destination locals.

### 3.3 String arena separation
The string arena (`println_i64.c`) owns heap strings for interpolation and frees them at exit via `atexit`. Lists should NOT use the arena (structured lifetimes differ). The list runtime uses `malloc`/`realloc` and must be freed explicitly or live for the process (if we keep the simple model). For 0.5.3, a process-lifetime list (no free required) is acceptable, matching the current string model but with separate allocation functions. A future ownership model would require explicit `free` or garbage collection — explicitly deferred.
Status: DESIGNED (separate from arena) / BLOCKED (free/ownership deferred)

## 4. Indexing, mutation, length, bounds

### 4.1 Indexing
Current `parser/mod.rs`: no `Expr::Index`. Must add parser production for `expr [ expr ]` (or `items[i]`). HIR: `HirExprKind::Index { list: Box<HirExpr>, index: Box<HirExpr> }` with `ty` = element type (e.g., `I64`). MIR: `MirRvalue::Index { list_local: LocalId, index_local: LocalId }` producing `MirOperand`. Codegen: calls `list_get` runtime function; passes list pointer and index (`i64`); receives `i64` result.

Status: BLOCKED (requires parser, HIR, MIR, codegen, runtime additions)

### 4.2 Mutation
If mutation is supported: `items[i] = 42` is `Expr::Assign { target: ... }` with a new target form (`IndexTarget`). Currently `Assign` uses `String` target (variable name). Must add `HirExprKind::IndexAssign` or extend `Assign` with index target. MIR: `MirStmtKind::Assign` uses `MirRvalue::IndexAssign`. Codegen: calls `list_set`.
Status: BLOCKED

### 4.3 Length
`len(items)` or `items.len` — parser needs new syntax or `len()` function. HIR: `Length { list: Box<HirExpr> }`. MIR: `MirRvalue::Length { list_local: LocalId }`. Codegen: calls `list_len`.
Status: BLOCKED

### 4.4 Bounds checking
The simplest sound approach for 0.5.3: runtime `list_get` checks `index < len`; if out of bounds, prints an error and calls `abort()` (or returns a default). This avoids designing a full exception system (`raise` / `?`) but prevents undefined behavior (reading invalid memory). This aligns with the current `raise` stub (`hir/stmt.rs:43-48`, `mir/lower.rs:314-339`) which prints and aborts.
Status: DESIGNED / BLOCKED (runtime function not implemented)

## 5. `for` loop changes
Current `lower_for` (`mir/lower.rs:670-774`) only supports `Range`. For list iteration, options:
1. Add `for var in list:` parser production; lower to a new `lower_for_list` using runtime iterator or index loop.
2. Keep current parser but add `Range`-only semantics; list iteration deferred.

The minimal sound first pass: add `for var in list_expr:` to parser; lower to a loop that:
- Creates `loop_var` local (`i64` index, not the element).
- Creates `element_var` local (element type, e.g., `I64`).
- Compares index with `list_len` (runtime call).
- Reads element via `list_get` (runtime call).
- Increments index.
- Assigns to loop variable at start of body.
This requires significant new MIR lowering logic but does not require universal iterator framework.
Status: BLOCKED (requires parser + new lowering)

## 6. Smallest sound first implementation (recommended order)

### 6.1 Required compiler changes (in dependency order)
1. `ast.rs`: keep `Type::List`, `Expr::StrLit` placeholder must be replaced by real list literal expression (`Expr::ListLiteral` or reuse `StrLit` but change lowering). RECOMMENDED: add `Expr::ListLiteral(Vec<Expr>, Range<usize>)`.
2. `parser/mod.rs`: change `lbracket_span()` branch to emit `Expr::ListLiteral` instead of placeholder `StrLit`.
3. `hir/types.rs`: add `HirType::List(SymbolId)`.
4. `hir/lower.rs`: change `Type::List` lowering from `Struct(SymbolId)` placeholder to `HirType::List(SymbolId)`.
5. `hir/expr.rs`: add `ListLiteral { items: Vec<Box<HirExpr>> }`, `Index { list: Box<HirExpr>, index: Box<HirExpr> }`, `Length { list: Box<HirExpr> }`.
6. `hir/stmt.rs`: no new statement needed for basic list ops (expressions handle it); mutation requires `IndexAssign` or new `Assign` target form. DEFERRED for first pass.
7. `mir/mod.rs`: add `MirRvalue::ListLiteral { elements: Vec<MirOperand> }`, `MirRvalue::Index { list_local: LocalId, index: MirOperand }`, `MirRvalue::Length { list_local: LocalId }`.
8. `mir/lower.rs`: add lowering for new `HirExprKind` variants.
9. `mir/codegen.rs`: add `List` case in `mir_type_to_llvm`; declare `list_new`, `list_get`, `list_set`, `list_len` in `declare_builtin_functions`; wire new `DefId` sentinels (like `CONCAT_STR_DEF_ID` but for list runtime).
10. `runtime/list.c`: implement functions.
11. `build.rs`: compile `runtime/list.c`.

### 6.2 Required runtime changes
- `runtime/list.c` (new file): `list_new`, `list_len`, `list_get`, `list_set`, `list_index_safe` (optional bounds-checked version).
- ABI: `list_new(size_t cap) -> list*`; `list_len(list*) -> size_t`; `list_get(list*, size_t) -> int64_t`; `list_set(list*, size_t, int64_t)`.
- Allocation: `malloc`/`realloc` (not arena). Separate from `println_i64.c` arena.

### 6.3 Required tests
- Parsing: `let a = [1, 2, 3]`; empty `[]` (optional); nested `[1, [2, 3]]` (deferred).
- Type checking: `List<i64>` inference; `List<bool>` (deferred); `List<str>` (deferred).
- Runtime: create list, read elements, read length, mutation, out-of-bounds behavior.
- End-to-end: `.stn` file compiling to native executable with list output.
- Regression: all existing `native_compilation` tests must pass (`63` tests currently pass).

### 6.4 Dependency / order
```
Parser fix (real list literal) → HIR type (List) → HIR expression (ListLiteral, Index, Length) → MIR rvalue (same variants) → MIR lowering (new branches) → Codegen (new cases + runtime declarations) → Runtime C file → Build update → Tests
```
Before step 1: confirm runtime ABI interface design (documented here, in `.tau/specs/phase0_5_3_list_review.md`).
Before step 6: confirm bounds-checking behavior (runtime abort preferred for 0.5.3).
Before full feature: resolve whether mutation is required (if yes, add mutation semantics before indexing; if no, defer mutation to later phase).

## 7. Architectural problems to solve before implementing

### 7.1 BLOCKED — Runtime ABI interface not fully documented
The assessment (`.tau/specs/phase0_5_scalability_assessment.md`) notes: "runtime ABI interface should become explicit file" (designed, not implemented). Before writing `runtime/list.c`, confirm:
- Whether `list_new` receives `size_t cap` or `i64 cap`.
- Whether `list_get` returns `i64` directly or `i64*` pointer (current `Str` uses pointer; `i64` uses value). For consistency with current `MirOperand` model, `list_get` should return `i64` value (not pointer), matching `Integer`, `Bool`.
- Whether `list_len` returns `i64` or `size_t` (current `Range` uses `i64` for loop variable; `len` as `i64` is simpler).
- Whether out-of-bounds uses runtime abort or returns a default. RECOMMENDED: runtime abort (`printf` + `abort`) to avoid inventing exception system.

### 7.2 BLOCKED — List expression variants missing
`ast::Expr` has no real `ListLiteral`; `hir/expr.rs` has no `ListLiteral`; `mir/mod.rs` has no `MirRvalue::ListLiteral`. These are prerequisites for any list implementation.

### 7.3 BLOCKED — `HirType::List` not implemented
`hir/types.rs` uses placeholder. Changing to real `List(SymbolId)` affects `mir_type_to_llvm`, `ast_type_to_hir`, `hir/lower.rs`, and all type-checking code.

### 7.4 BLOCKED — `for` loop over list not designed
`mir/lower.rs` `lower_for` is hard-coded to `Range`. Adding list iteration requires either:
- A new `lower_for_list` method (recommended), or
- Changing `HirExprKind::For` to carry iterator type information (more invasive).
RECOMMENDED: new `lower_for_list` method, keep `lower_for` unchanged.

### 7.5 BLOCKED — Indexing expression missing
`ast::Expr` needs `Index { list, index }`; `hir/expr.rs` needs `Index`; parser needs `expr [ expr ]`. This is a new parser production (`primary` chain) with new precedence rules.

### 7.6 DESIGNED — Runtime C function names and ABI
Based on `runtime/println_i64.c` conventions:
- Function names: lowercase snake_case (`list_new`, not `ListNew`).
- Return types: `list_new` returns pointer (`void*` / `sat_list*`); `list_get` returns `long long`; `list_len` returns `size_t` (or `long long` if we prefer consistency).
- Arguments: `list*` pointer passed by value (`void*` in C, mapped to `ptr_type` in LLVM).
- Memory: `malloc`/`realloc` (not arena). No `free` required for first pass (process lifetime), but future design should allow it.

### 7.7 DESIGNED — Supported first-pass element types
Only `i64` (number) is safe. `bool`, `f64`, `str`, `Unit`, `Struct`, `Enum` require additional work (`bool` needs `bool_type` mapping; `str` needs string pointer; `Struct` needs struct pointer; nested types need recursive support). RECOMMENDED: restrict first pass to `i64` elements and reject others with compile-time diagnostic (similar to interpolation's type check: `hir/lower.rs:2135-2141`).

### 7.8 DESIGNED — Mutation semantics
If mutation is required for 0.5.3, it requires new HIR/MIR variants (`IndexAssign`). If not required, defer mutation to a later phase. The user's instruction says: "If the existing language semantics allow mutation, implement it correctly." The current parser does not have syntax for `items[i] = 42` (`parser/mod.rs`: no index assignment in `stmt`). Therefore mutation is NOT currently supported by syntax. Defer mutation; first pass supports creation, indexing (read), length, iteration, but not mutation.

## 8. Review conclusion

Every conclusion in `.tau/specs/phase0_5_3_list.md` is consistent with the actual source:
- `Type::List` exists but lowers to placeholder (`ast.rs`, `parser/mod.rs`, `hir/lower.rs`, `hir/types.rs`).
- No real list expression exists (`ast.rs`: no `Expr::ListLiteral`; `parser/mod.rs`: placeholder `StrLit`).
- No HIR expression variants for list construction/index/length (`hir/expr.rs`).
- No MIR rvalue/statement variants (`mir/mod.rs`, `mir/lower.rs`).
- No list runtime functions (`runtime/println_i64.c`; `mir/codegen.rs`: only `println`, `println_str`, `concat_str`, `str_i64`).
- `BuiltinRegistry` exists but has no list entries (`hir/symbol.rs`).
- `for` loop only supports `Range` (`mir/lower.rs`).
- Module/resolver architecture is sound and does not block lists (`module.rs`).
- String arena is separate from list allocation model; lists should use separate `malloc`-based runtime.
- The proposed minimal ABI (pointer + len + cap; `i64` elements; `malloc`; runtime abort for out-of-bounds) is compatible with the existing pipeline and does not require redesigning unrelated subsystems.

No rustc source is reused. Normal Rust crates (`logos`, `chumsky`, `inkwell`, `miette`, etc.) are used normally.

No scope creep into closures, Python interop, package manager, standard library, GC, borrow checker, or universal framework.

The review confirms: the design is sound; the prerequisites are documented; full implementation requires the ordered steps listed above; and no hidden architectural blockers prevent a safe first-pass `List<i64>` implementation once the prerequisites (ABI confirmation, expression variants, runtime C file) are completed.
