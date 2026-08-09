# Saturnite 0.4 — Architecture Audit

> Reconnaissance & Planning Document | Date: 2026-08-08
> Repository: `lunartesla/Saturnite`
> Source modified: NO | Dependencies added: NO | Tests changed: NO

## 1. Executive Summary

Saturnite 0.3 is a single-crate compiler (`stnx` binary, clap name `saturnite`) targeting native
machine code via LLVM 21 (inkwell). The pipeline is:

```text
Source(.stn) → Lexer → Parser → AST → HIR Lowering → LLVM IR → Object → Linker → Executable
```

**Key findings verified against source:**

- **HIR IS implemented and wired to codegen.** Codegen consumes `HirProgram` (not `ast::Program`). The 0.3 architecture review's statement that "0.2 does not implement HIR" is correct for 0.2, but HIR is now live.
- **MIR is NOT implemented.** `grep -rn "mir\|Mir\|MIR" crates/stnx/src/` returns zero code hits. Only doc comments mention it. The MIR design doc is design-only.
- **Incremental compilation is NOT implemented.** No fingerprint module, no cache, no serialization. HIR types derive only `Debug`/`Clone`.
- **`saturn.toml` is parsed but NEVER read by the compiler.** `from_dir()` is dead code. `Build`/`Check`/`Run` ignore config entirely.
- **No module system.** `program()` parser = `func().repeated()`. No `mod`/`use`/`pub` keywords in lexer or parser. Structs/enums are trapped inside function bodies.
- **123 tests, 0 failures.** `cargo fmt --check` PASS. `cargo clippy --workspace --tests -- -D warnings` PASS. `cargo check --workspace` PASS.

**Classification summary:**

| Feature | Classification |
|---------|---------------|
| Lexer | IMPLEMENTED |
| Parser | IMPLEMENTED |
| AST (types, exprs, stmts, structs, enums) | IMPLEMENTED |
| HIR lowering (SymbolId/DefId interning, type checking, name resolution) | IMPLEMENTED |
| Codegen (HIR → LLVM IR) | IMPLEMENTED |
| Object emission | IMPLEMENTED |
| Linking | IMPLEMENTED |
| CLI (build/check/run/doctor/init) | IMPLEMENTED |
| Config parsing (`saturn.toml`) | IMPLEMENTED (but not wired in) |
| Runtime (`println_i64.c`) | IMPLEMENTED |
| Cross-compilation | DISABLED (runtime is host-only) |
| Module system | MISSING |
| MIR | DESIGN ONLY |
| Incremental compilation | DESIGN ONLY |
| Dependency resolution/fetching | DESIGN ONLY |
| Python interop | DESIGN ONLY |
| Proper `str` type | PARTIALLY IMPLEMENTED (`str` = `i64` pointer) |
| `println` with string args | MISSING (i64 only) |

---

## 2. Repository State

### 2.1 Workspace

Single crate: `crates/stnx` (name: `stnx`, edition 2021, resolver v3). `Cargo.lock` has 122 packages.
Layout: `Cargo.toml` (workspace root), `saturn.toml` (example config), `README.md`,
`examples/hello.stn`, `tests/{codegen,lexer,semantic}.rs` (DEAD — not compiled), `rust_out`
(DEAD — stale ELF binary), `crates/stnx/` (the sole crate), `docs/` (7 design/review docs +
`audit_notes/`).

### 2.2 Git state

- Branch: `main` | HEAD: `60bfb4f` "version 0.3"
- History: 3 commits (0.3 → 0.2 → initial)
- Working tree: clean (untracked: `docs/audit_notes/`)

### 2.3 Cargo.toml discrepancies

- Package binary name: `stnx` (in `crates/stnx/Cargo.toml:2`); on disk: `target/debug/stnx`
- clap `#[command(name = "saturnite")]` at `main.rs:10` — **mismatch with binary name**
- README uses `stnx` (line 19) and `saturnite` (line 88) — internally inconsistent
- `toml = "0.8"` declared only in `crates/stnx/Cargo.toml:16` — **NOT in `[workspace.dependencies]`**

### 2.4 Test count (verified by `cargo test --workspace`)

| Test binary | Tests | Result |
|-------------|-------|--------|
| `config::tests` (lib unit) | 7 | ✅ |
| `crates/stnx/tests/codegen.rs` | 14 | ✅ |
| `crates/stnx/tests/diagnostics.rs` | 6 | ✅ |
| `crates/stnx/tests/lexer.rs` | 17 | ✅ |
| `crates/stnx/tests/native_compilation.rs` | 47 | ✅ |
| `crates/stnx/tests/semantic.rs` | 28 | ✅ |
| `crates/stnx/tests/test_full_compile.rs` | 1 | ✅ |
| `crates/stnx/tests/test_ir_only.rs` | 1 | ✅ |
| `crates/stnx/tests/test_native_only.rs` | 1 | ✅ |
| `crates/stnx/tests/test_target_machine.rs` | 1 | ✅ |
| **Total** | **123** | **0 failures** |

The root-level `tests/{codegen,lexer,semantic}.rs` files are git-tracked but NOT compiled — no
`[package]` at workspace root. `tests/codegen.rs` uses the old API (`generate_ir(&ast::Program)`)
which no longer compiles since `generate_ir` now takes `&HirProgram`.

### 2.5 Lint/format verification


---

## 3. Documentation Consistency Audit

### 3.1 Documents inventory

| Document | Status | Reality check |
|----------|--------|---------------|
| `README.md` | Mostly accurate but stale | Pipeline omits HIR; binary name inconsistent; cross-compilation advertised as working but disabled |
| `SATURNITE_0_3_ARCHITECTURE_REVIEW.md` | STALE (pre-0.3) | Says "0.2 does not implement HIR" — HIR IS implemented in 0.3 and wired to codegen |
| `SATURNITE_0_3_HIR_DESIGN.md` | DESIGN-ONLY | Proposed HIR types differ from actual (e.g., `HirStmtKind` has `StructDef`/`EnumDef`; `HirExprKind` has `Range`) |
| `SATURNITE_CRATE_DEPENDENCY_AUDIT.md` | LARGELY ACCURATE | Says ~45 packages; actual is 122; `toml` not centralized — not mentioned |
| `SATURNITE_DEPENDENCY_MODEL.md` | DESIGN-ONLY | `DependencySpec::from_str()` just clones string, no semver parsing |
| `SATURNITE_INCREMENTAL_COMPILATION.md` | DESIGN-ONLY | HIR not `Serialize`-able; no caching infrastructure exists |
| `SATURNITE_MIR_DESIGN.md` | DESIGN-ONLY | No MIR code in source |
| `SATURNITE_FINAL_VERIFICATION.md` | STALE | Body says "123 tests" (correct) but table sums to 126 (double-counts); Phase 0-9 says structs/enums complete (true at function level only) |
| `docs/audit_notes/infra.md` | ACCURATE | Correctly identifies most gaps |
| `docs/audit_notes/pipeline.md` | ACCURATE | Correctly identifies pipeline and MIR absence |

### 3.2 Key documentation-vs-reality contradictions

1. **README pipeline (lines 41-63):** Shows `Source → Lexer → Parser → Semantic → LLVM IR → Object → Executable`. Omits HIR (actual: AST → HIR → LLVM).
2. **0.3 ARCHITECTURE_REVIEW (line 16):** Says "0.2 does not implement HIR." HIR IS implemented (`crates/stnx/src/hir/`).
3. **SATURNITE_FINAL_VERIFICATION.md table (48-60):** Sums to 126; actual is 123. "misc: 4" overlaps `test_ir_only`/`test_native_only`/`test_target_machine`/`test_full_compile`.
4. **SATURNITE_DEPENDENCY_MODEL.md (line 48):** Claims `DependencySpec::from_str()` "will extend to support semver ranges." Actual: `config.rs:125-132` just clones the string.
5. **SATURNITE_DEPENDENCY_MODEL.md (line 49):** Describes resolver (Phase 14+). Does NOT exist.
6. **README (line 91):** Lists `--target` as cross-compilation. Actual: non-host targets REJECTED (`main.rs:270-285`).
7. **SATURNITE_CRATE_DEPENDENCY_AUDIT.md (line 107):** Says "~45 packages." Actual: 122 in Cargo.lock.
8. **SATURNITE_0_3_HIR_DESIGN.md:** States AST is 134 lines. Actual: 180 lines.

### 3.3 Architectural decisions to preserve

1. **Span-on-every-node** — AST has `Range<usize>`; HIR converts to `SourceSpan` (`lower.rs:20-22`).
2. **Variable storage** — SSA for immutables, alloca for mutables (`context.rs:20-24`).
3. **Clean codegen layering** — `CodeGenerator → ObjectEmitter → Linker` (`codegen/mod.rs`).
4. **Symbol interning** — `SymbolId`/`DefId` replace string resolution.
5. **Two-pass function declaration** — Signatures collected before bodies (`lower.rs:139-162`).

### 3.4 Architectural decisions to reconsider

1. **Struct/enum inside function bodies** — `program()` parser only accepts `func().repeated()`.
2. **Binary name `stnx` vs CLI name `saturnite`** — User confusion.
3. **`Init` project name bug** — Uses full path as `name` in `saturn.toml`.

---

## 4. Actual Compiler Pipeline

### 4.1 The real pipeline (from `main.rs:262-276`)

```rust
let tokens: Vec<_> = stnx::lexer::Lexer::new(&src).by_ref()
    .collect::<Result<Vec<_>, _>>()?;
let program = stnx::parser::parse(&src, tokens)?;
let hir = stnx::semantic::analyze_and_lower(&program)?;
codegen::compile_with_target(&hir, &output_path, config)?;
```

| Stage | File | Types | Entry | Input | Output |
|-------|------|-------|-------|-------|--------|
| 1. Lexing | `lexer/mod.rs` | `Lexer`, `Token`, `TokenKind` | `Lexer::new(src)` iter | `&str` | `Vec<Token>` (byte spans) |
| 2. Parsing | `parser/mod.rs` | `Program`, `Function`, `Stmt`, `Expr` | `parse(src, tokens)` | `Vec<Token>` | `ast::Program` |
| 3. Lowering | `semantic.rs` → `hir/lower.rs` | `HirProgram`, `HirExpr`, `HirStmt` | `analyze_and_lower()` → `lower()` | `&ast::Program` | `HirProgram` (typed) |
| 4. Codegen | `codegen/mod.rs` + `context.rs` | `CodeGenContext`, `FunctionScope`, `Variable` | `emit()` → `gen_expr`/`gen_stmt` | `&HirProgram` | LLVM `Module` |
| 5. Emission | `codegen/emitter.rs` | `ObjectEmitter`, `TargetMachine` | `emit_object(path)` | `Module` | `.o` file |
| 6. Linking | `codegen/linker.rs` | `Linker` | `Linker::link(obj, output)` | `.o` + `libsaturnite_runtime.a` | Executable |

### 4.2 Is MIR actually implemented?

**Classification: MISSING (Design Only).**

- `grep -rn "mir\|Mir\|MIR" crates/stnx/src/` → **zero code results**.
- Only two **doc comments** mention MIR: `hir/lower.rs:5` and `hir/expr.rs:5`.
- No `mir.rs` file, no `mir/` directory, no `Mir*` types.
- `docs/SATURNITE_MIR_DESIGN.md` exists but is NOT implemented.
- `docs/SATURNITE_FINAL_VERIFICATION.md:124`: `**MIR layer:** Design documented but not yet implemented.`

### 4.3 What is between HIR and LLVM?

**Classification: NOTHING — direct HIR → LLVM IR.**

`gen_stmt()` (`context.rs:156`) and `gen_expr()` (`context.rs:207`) walk HIR directly. The CFG for
control flow is constructed on-the-fly from the HIR tree, NOT stored as a data structure:

- `HirExprKind::If` (`context.rs:~490-590`): Constructs BBs inline (`if_then`, `if_else`, `if_end`,
  `elifN_cond`). **No CFG stored.**
- `HirExprKind::For` (`context.rs:~595-660`): `alloca` for loop var, `for_cond`/`for_body`/`for_end`
  BBs. Uses `IntPredicate::ULT`/`ULE` (**unsigned** — potential bug for negative ranges).
- `HirExprKind::While` (`context.rs:~662-712`): `while_cond`/`while_body`/`while_end` BBs.
- `HirExprKind::Range` (`context.rs:~714-720`): Returns only `start`; `end` is discarded
  (`let _ = end;`).
- Optimization: `mod.rs:112-122` runs LLVM IR passes (`default<O3>` etc.) **after** codegen.

### 4.4 Error handling and span survival

| Stage | Error type | Span? |
|-------|-----------|-------|
| Lexer | `LexError` (miette Diagnostic) | ✅ byte span |
| Parser | `ParseError` (miette Diagnostic) | ✅ byte span |
| Semantic/HIR | `CompilerError::Semantic(String)` | ⚠️ string-only |
| Codegen | `CompilerError::Codegen(String)` | ⚠️ string-only; HIR spans never used |
| CLI display | `render_diagnostic()` only miette-renders Lexer/Parse errors | **Severe — semantic+codegen errors lose span rendering** |

### 4.5 `semantic::analyze` vs `analyze_and_lower`

- `analyze()` (`semantic.rs:16`) → `lower_unit(program)` → `lower(program).map(|_| ())` — returns `Ok(())` or first error. Used by `Check` CLI command (`main.rs:532`).
- `analyze_and_lower()` (`semantic.rs:24`) → `lower(program)` — returns `HirProgram`. Used by `Build` (`main.rs:268`), `Run` (`main.rs:476`), and all test helpers.

---

## 5. AST Audit

Source: `crates/stnx/src/ast.rs` (180 lines)

### 5.1 Type representation

```rust
#[derive(Clone, Debug, PartialEq)]
pub enum Type { I64, F64, Bool, Str, Unit, Struct(String), Enum(String) }
```
No `Eq`, no spans. `Str` and `Unit` are both lowered to `i64` at LLVM level (`context.rs:875`).
`Struct(String)`/`Enum(String)` are unresolved names. The parser produces `Type::Struct(name)` for
ALL user-defined types; HIR disambiguates via enum name set.

### 5.2 Program

```rust
pub struct Program { pub functions: Vec<Function> }
```
Contains ONLY functions. **No top-level struct/enum definitions.** Structs/enums are `Stmt`
variants that can appear **inside function bodies only**.

### 5.3 Function

```rust
pub struct Function {
    pub name: String,                    // unresolved — plain String
    pub params: Vec<(String, Type)>,    // no span, no mutability flag, no pattern
    pub return_type: Type,
    pub body: Vec<Stmt>,
        pub span: Range<usize>,             // function name span only
}
```

### 5.4 Statements — 6 variants

| Variant | Fields |
|---------|--------|
| `Let` | `name: String, mutable: bool, ty: Option<Type>, value: Expr, span` |
| `Expr` | `Expr, Range<usize>` |
| `Return` | `Option<Expr>, Range<usize>` |
| `Println` | `Expr, Range<usize>` |
| `StructDef` | `name: String, fields: Vec<(String, Type)>, span` |
| `EnumDef` | `name: String, variants: Vec<String>, span` |

### 5.5 Expressions — 17 variants

| # | Variant | Fields | Unresolved identifier? |
|---|---------|--------|----------------------|
| 1 | `Integer` | `i64, Range<usize>` | No |
| 2 | `Float` | `f64, Range<usize>` | No |
| 3 | `StrLit` | `String, Range<usize>` | No (HIR interns it) |
| 4 | `Bool` | `bool, Range<usize>` | No |
| 5 | `Unit` | `Range<usize>` | No |
| 6 | `Var` | `String, Range<usize>` | **Yes** → `SymbolId` |
| 7 | `Assign` | `target: String, value: Box<Expr>, span` | **Yes** — target |
| 8 | `AugAssign` | `target: String, op: AugOp, value: Box<Expr>, span` | **Yes** — target |
| 9 | `Binary` | `op: BinOp, lhs: Box<Expr>, rhs: Box<Expr>, span` | No |
| 10 | `Unary` | `op: UnOp, expr: Box<Expr>, span` | No |
| 11 | `Call` | `func: String, args: Vec<Expr>, span` | **Yes** — function name |
| 12 | `If` | `condition, then_branch, elif_branches, else_branch, span` | No |
| 13 | `For` | `var: String, iter: Box<Expr>, body: Vec<Stmt>, span` | **Yes** — loop var |
| 14 | `While` | `condition, body: Vec<Stmt>, span` | No |
| 15 | `Range` | `start, end, is_inclusive, span` | No |
| 16 | `StructLiteral` | `name: String, fields: Vec<(String, Expr)>, span` | **Yes** — struct + fields |
| 17 | `FieldAccess` | `expr: Box<Expr>, field: String, span` | **Yes** — field name |
| 18 | `EnumConstructor` | `name: String, variant: String, span` | **Yes** — enum + variant |

**Operators:** `BinOp` (13 variants), `UnOp` (2: Neg, Not), `AugOp` (4: Add, Sub, Mul, Div).

### 5.6 AST vs. HIR separation

The AST carries unresolved `String` identifiers that HIR resolves to `SymbolId`/`DefId`:
`Var`, `Assign.target`, `AugAssign.target`, `Call.func`, `For.var`, `StructLiteral.name/fields`,
`FieldAccess.field`, `EnumConstructor.name/variant`, `Function.name`, `Function.params`, and
`Type::Struct`/`Type::Enum`. The AST does NOT perform type checking or scope resolution — the
separation is clean. `Stmt::Let.mutable: bool` overlaps into HIR (also tracked as `VarInfo.mutable`)
but this is acceptable since HIR owns the canonical copy.

---

## 6. HIR Audit

Source: `crates/stnx/src/hir/` (6 files, ~1,400 lines)

### 6.1 HIR type table

| Type | File:Line | Derives |
|------|-----------|---------|
| `SymbolId(u32)` | `symbol.rs:15` | `Debug, Clone, Copy, PartialEq, Eq, Hash` |
| `DefId(u32)` | `symbol.rs:22` | `Debug, Clone, Copy, PartialEq, Eq, Hash` |
| `SymbolInterner` | `symbol.rs:29` | `Debug, Default` |
| `HirType` | `types.rs:16` | `Clone, Copy, Debug, PartialEq, Eq` |
| `HirExpr` | `expr.rs:14` | `Debug, Clone` |
| `HirExprKind` | `expr.rs:26` | `Debug, Clone` |
| `HirStmt` | `stmt.rs:13` | `Debug, Clone` |
| `HirStmtKind` | `stmt.rs:21` | `Debug, Clone` |
| `StructDef` | `function.rs:29` | `Debug` |
| `EnumDef` | `function.rs:39` | `Debug` |
| `HirFunction` | `function.rs:18` | `Debug` |
| `HirProgram` | `function.rs:50` | `Debug` |

**Critical:** No `Serialize`/`Deserialize` derives on ANY HIR type. `SymbolInterner` derives only
`Debug, Default`. This blocks incremental compilation (cache serialization).

### 6.2 What HIR contains that AST does not

- **Resolved identifiers:** All `String` → `SymbolId`/`DefId` via `SymbolInterner`.
- **Resolved types:** Every `HirExpr` carries `ty: HirType`.
- **`DefId` on functions:** `HirFunction.def_id: DefId` (array index).
- **Shared symbol table:** `HirProgram.symbols: SymbolInterner`.
- **Source spans:** `SourceSpan` on every `HirExpr` and `HirStmt`.

### 6.3 Name/type resolution

- **Identifiers resolved:** Yes — `Expr::Var(name)` → `self.symbols.intern(name)` → `SymbolId` → `LowerScope::lookup`.
- **Types fully resolved:** Yes — `ast_type_to_hir` (`lower.rs:89-122`).
- **Function signatures resolved:** Yes — `FunctionSig { def_id, param_types, return_type }` in `HashMap<SymbolId, FunctionSig>`.
- **Parameters have SymbolIds:** `HirFunction.params: Vec<(SymbolId, HirType)>`.

### 6.4 Scope representation

`LowerScope` (`lower.rs:56-85`): `HashMap<SymbolId, VarInfo>` + parent chain. Used **only during
lowering** — HIR itself does NOT contain scope info. Codegen re-creates its own `FunctionScope`
(`context.rs:826-862`). **HIR is not a complete scope snapshot.**

### 6.5 Is HIR immutable after construction?

**Yes.** `lower()` produces `HirProgram` with no `&mut` methods on the result. All HIR types are
`&self`-only. Codegen takes `&HirProgram`. `SymbolInterner::lookup(&self)` is read-only.

### 6.6 Is HIR suitable as an incremental compilation boundary?

**PARTIALLY IMPLEMENTED.** HIR is immutable ✅, fully typed ✅, has spans ✅, has symbol table ✅.
But **NOT `Serialize`/`Deserialize`** — blocks caching. `SymbolInterner` would need stable
serialization: `Vec<String>` (stable indices) + `HashMap<String, SymbolId>` (reconstructable).

### 6.7 HIR maturity rating: **3 (solid foundation)

- **Strengths:** Fully typed expressions, resolved identifiers via stable `SymbolId`/`DefId`, source
  spans on every node, clean AST→HIR→LLVM pipeline, two-pass function declaration enabling recursion.
- **Weaknesses:** No `Serialize`/`Deserialize` (blocks incremental), struct/enum defs inside
  function bodies (not top-level items — blocks modules), flat `DefId(u32)` with no module namespace,
  `LowerScope` not preserved in HIR.
- **Not 4:** No serialization, no top-level item model, no module paths, no query/reuse system.

### 6.8 HIR expression variants (18)

Mirrors AST `Expr` (17 variants) but all identifiers resolved to `SymbolId`/`DefId`.
`HirExprKind::Range` returns `HirType::I64`. `If`/`For`/`While` all return `HirType::Unit`
(not branch type — limits future expression-based returns).

### 6.9 Key structural findings in `lower.rs`

1. **Pre-pass for enum names** (`lower.rs:143-155`): Scans ALL function bodies for `Stmt::EnumDef`
   before lowering any function. Needed so `ast_type_to_hir` can disambiguate enum vs struct names.
2. **Struct/enum defs emitted as no-ops** (`lower.rs:~370`): `Stmt::StructDef`/`Stmt::EnumDef` in
   lowering produces `HirStmt { kind: Expr(Unit), .. }` — a no-op. Definitions collected separately
   into `HirProgram.structs`/`enums` in the pre-pass.
3. **`Return` type checking** (`lower.rs:326-346`): Matches `return_type` against expr type.
4. **`For` requires Range** (`lower.rs:~630-645`): `iter_expr.kind` must be `Range` — else error.
5. **`While` creates new scope** (`lower.rs:653-655`): `with_parent(scope.clone())` — but `For` does
   NOT create a new scope (loop variable leaks to parent scope). **Inconsistency.**
6. **`main` must exist** (`lower.rs:166-168`): Error if no `main` function.
7. **`lower_unit`** (`lower.rs:862`): `pub fn lower_unit(program) -> CompilerResult<()>` — exists
   only for the `Check` command (`analyze()` wrapper). Calls `lower(program).map(|_| ())`.
8. **`lower`** (`lower.rs:862`): `pub fn lower(program) -> CompilerResult<HirProgram>` — entry point
   for `analyze_and_lower()`. Creates default `HirLower`, calls `lower_program()`.
9. **`range_to_span`** (`lower.rs:874`): `pub fn range_to_span(range: &Range<usize>) -> SourceSpan`
   — converts AST `Range<usize>` to `miette::SourceSpan`. Used to attach spans to HIR nodes.
10. **No `Mut` prefix on params**: Parser `params()` (`parser/mod.rs:103`) only parses
    `ident().then(type_ann())` — no `mut` keyword. All parameters are immutable in HIR.

---

## 7. Semantic Analyzer Audit

Source: `semantic.rs` (30 lines — thin wrapper) + `hir/lower.rs` (~876 lines — actual implementation)

### 7.1 Architecture

`semantic.rs:16-26`:
```rust
pub fn analyze(program: &Program) -> CompilerResult<()> {
    hir::lower::lower_unit(program)  // → lower(program).map(|_| ())
}
pub fn analyze_and_lower(program: &Program) -> CompilerResult<hir::HirProgram> {
    hir::lower::lower(program)
}
```

**Classification: PARTIALLY IMPLEMENTED — semantic analysis is absorbed into HIR lowering.**

The 0.2 `semantic.rs` had a separate `Scope` struct with `HashMap<String, ...>`. This has been
**replaced entirely** by the HIR lowering pass. The old `Scope` type no longer exists.

### 7.2 What IS handled (in `lower.rs`)

| Feature | How | Location |
|---------|-----|----------|
| Variable scopes | `LowerScope` + parent chain | `lower.rs:56-85` |
| Shadowing | Allowed (overwrites in current scope) | `lower.rs:75-77` |
| Mutability | `VarInfo.mutable`; checked in Assign/AugAssign | `lower.rs:509-512, 525-529` |
| Type checking | `HirExpr.ty` computed; compared against expected | `lower.rs:304-326, 547-566` |
| Function signatures | `FunctionSig { def_id, param_types, return_type }` | `lower.rs:139-160` |
| Return types | Checked against `return_type` | `lower.rs:326-346` |
| Undefined names | `lookup_variable` → None → error | `lower.rs:503-507` |
| Undefined functions | `function_sigs.get()` → None → error | `lower.rs:582-584` |
| Argument validation | Count + type check vs `sig.param_types` | `lower.rs:586-603` |
| Recursion | Two-pass: sigs first, bodies second | `lower.rs:139-162` |
| Forward references | ✅ (both passes) | |
| Range type checking | Both ends must be `I64` | `lower.rs:668-687` |
| `For` requires Range | Checked in `lower_expr` | `lower.rs:~630-645` |
| `main` function required | Enforced | `lower.rs:166-168` |

### 7.3 What is NOT handled

- Unused variable warnings
- Unreachable code detection
- Const evaluation / constant folding (happens in LLVM, not semantic)
- Mutability of `self`/parameters (parser doesn't support `mut` on params)
- Borrow checking

### 7.4 Architectural prerequisites for future features

| Feature | Missing component | Severity |
|---------|------------------|----------|
| Structs (shared across functions) | Struct defs trapped inside function bodies | HIGH |
| Modules | `LowerScope` is per-function; no `ModuleId` namespace | CRITICAL |
| Generics | `HirType` has no type parameters; `FunctionSig` has fixed types | HIGH |
| Traits | No trait definitions, no impl blocks, no method resolution | HIGH |
| References (`&T`) | `HirType` has no reference type; `HirExpr` is `Clone` | HIGH |
| Generics + Traits | Need `HirType::Generic(DefId)` and a trait table | CRITICAL |
| Data-carrying enums | Need `HirType` with variant payloads | HIGH |
| Pattern matching | Need `match` syntax and structural deconstruction | HIGH |
| Ownership system | Need `Place` expressions, `Mutability`, lifetime tracking | HIGH |
| Closures | Need first-class function values and capture tracking | HIGH |
| Const eval | Need MIR-level constants and folding engine | MEDIUM |

### 7.5 Borrow-checker foundations needed

1. **DefId hygiene & namespace** — `DefId` is flat `u32` (function index). Needs module paths.
2. **Type IDs** — `HirType` has no structural identity.
3. **Mutability on all bindings** — params are always immutable in HIR; need `Mutability` on params.
4. **Lifetime/type annotation model** — no concept of reference or lifetime yet.
5. **Place expressions** — HIR expressions are values, not places; no `Place` abstraction.

---

## 8. Type System Audit

### 8.1 Complete type table

| Type | AST repr | HIR repr | LLVM repr | Runtime repr | Size | Copy | Mutable | Args | Return | Collections |
|------|----------|----------|-----------|-------------|------|------|---------|------|--------|-------------|
| `i64` | `Type::I64` | `HirType::I64` | `i64` | 8 bytes signed | 8 | ✅ | ✅ | ✅ | ✅ | ✅ |
| `f64` | `Type::F64` | `HirType::F64` | `double` | IEEE 754 | 8 | ✅ | ✅ | ✅ | ✅ | ✅ |
| `bool` | `Type::Bool` | `HirType::Bool` | `i1` | 1 byte | 1 | ✅ | ✅ | ✅ | ✅ | ✅ |
| `str` | `Type::Str` | `HirType::Str` | `i64` (pointer) | raw pointer | 8 | ✅ | ✅ | ✅ | ✅ | ✅ |
| `unit` | `Type::Unit` | `HirType::Unit` | `i64` (zero) | `i64` (0) | 8 | ✅ | ✅ | ✅ | ✅ | ✅ |
| `struct` | `Type::Struct(String)` | `HirType::Struct(SymbolId)` | opaque `ptr` | struct on stack | variable | ✅* | ✅ | ✅ | ✅ | ✅ |
| `enum` | `Type::Enum(String)` | `HirType::Enum(SymbolId)` | `i64` (tag) | `i64` | 8 | ✅ | ✅ | ✅ | ✅ | ✅ |

*Structs are copyable only if all fields are.

### 8.2 What is NOT handled

- Arrays `[T; N]` — no `HirType` variant, no array literal syntax
- Slices `&[T]` — no reference type in `HirType`
- Tuples `(T, U)` — no `HirType::Tuple` variant
- References `&T` / `&mut T` — no reference type in `HirType`
- Function types `fn(i64) -> i64` — no `HirType::Fn`
- Pointers `*T` — no pointer type
- String I/O — `println` only accepts `i64`

### 8.3 Foundations needed for future types

| Type | Prerequisite |
|------|-------------|
| `str` (proper) | Heap allocation, string runtime — needs MIR + runtime |
| Arrays | `HirType::Array(T, N)` — needs MIR for place model |
| Slices | Reference type (`&[T]`) — needs `&T` type |
| Tuples | `HirType::Tuple` — depends on reference type |
| References | `HirType::Ref` — foundational for arrays, slices, tuples |
| Function pointers | `HirType::Fn` — independent but low priority |

**Recommendation:** `str` (proper) and references should be Phase 2 of 0.4 after MIR is stable.

---

## 9. Function System Audit

### 9.1 Current capabilities

| Capability | Status | Evidence |
|------------|--------|----------|
| Multiple functions | ✅ | `Program { functions: Vec<Function> }` |
| Parameters | ✅ | `HirFunction.params: Vec<(SymbolId, HirType)>` |
| Return types | ✅ | `HirFunction.return_type: HirType` |
| Recursion | ✅ | Two-pass: signatures collected before bodies (`lower.rs:139-162`) |
| Forward references | ✅ | Same two-pass mechanism |
| Function symbols | ✅ | `SymbolId` via `SymbolInterner`; `DefId` = array index |
| Name resolution | ✅ | `function_sigs: HashMap<SymbolId, FunctionSig>` |
| External functions | ❌ | No `extern` blocks; only builtin `println` (`PRINTLN_DEF_ID`) |
| Function pointers | ❌ | No `Fn` type in `HirType`; calls resolved by `DefId` at compile time |
| Closures | ❌ | No closure syntax in parser |
| ABI handling | ❌ | Default C ABI only; no `extern "ABI"` |
| Variadic functions | ❌ | `fn_type(&param_types, false)` — always fixed-arity |

### 9.2 How a function becomes LLVM IR

1. **Declaration** (`context.rs:52-69`): `declare_function` creates LLVM function with return + param types via `type_to_llvm`. Name from `func_names: HashMap<DefId, String>`.
2. **Definition** (`context.rs:71-113`): `generate_function` creates `entry` BB, binds params to `FunctionScope` (immutable SSA), calls `gen_stmt` on each body statement.
3. **Parameter binding** (`context.rs:86-91`): `get_nth_param(i)` → `scope.insert_immutable(param)`. Parameters always immutable.
4. **Name mapping** (`context.rs:46-50`): `declare_builtin_functions()` adds `println_i64`.

### 9.3 Can current design support these signatures?

| Signature | Works? |
|-----------|--------|
| `fn add(a: i64, b: i64) -> i64` | ✅ All features exist |
| `fn factorial(n: i64) -> i64` | ✅ Recursion works |
| `fn apply(f: fn(i64) -> i64, x: i64) -> i64` | ❌ `HirType` cannot represent `fn(i64) -> i64`; parser would reject |

### 9.4 ABI correctness

LLVM function types use default C ABI. The `println` builtin maps to `println_i64` with default C calling convention (matches the C runtime in `runtime/println_i64.c`). For 0.4, ABI correctness is sufficient but should be made explicit.

---

## 10. Module System Audit

**Classification: MISSING.**

### 10.1 What does NOT exist

- No `mod` keyword in lexer (`lexer/mod.rs:8-50` keyword list — no `Mod`).
- No `use` keyword.
- No `pub` keyword.
- `TokenKind` enum (`token.rs:4-61`) — no `Mod`, `Use`, `Pub`.
- `is_keyword` (`parser/mod.rs:770-792`) — no `mod`/`use`/`pub`.
- `program()` parser (`parser/mod.rs:80-85`): `func().repeated().collect()` — only functions at top level.
- `Program { functions: Vec<Function> }` — no items vector.
- `SaturnConfig` (`config.rs:27-35`) — has `package` and `dependencies` but no `[lib]`/`[[bin]]` sections, no paths.

### 10.2 Minimum architecture needed

1. **Lexer:** Add `Mod`, `Use`, `Pub` to `LexicalToken` and `TokenKind`.
2. **AST:** `Program { items: Vec<Item> }` where `Item = Function | StructDef | EnumDef | Module | Use`.
3. **Parser:** `program()` → `item().repeated()`. Add `mod name;` and `use path::name;`.
4. **HIR:** `HirProgram { modules: HashMap<ModuleId, HirModule> }` with namespaces.
5. **Resolver:** `Resolver { module_stack, scope_stack }` for `mod foo {}` descent and `use foo::bar` imports.
6. **Module loader:** Resolve `mod foo;` → `foo.stnx`/`foo/mod.stnx` relative to current module dir.

### 10.3 DefId/SymbolId module-path impact

Current `DefId(u32)` is a flat array index. A module path would require `DefId { module: ModuleId, index: u32 }` — a type change. `SymbolId` is fine (path components are just interned strings).

---

## 11. saturn.toml Audit

Source: `crates/stnx/src/config.rs` (222 lines, 7 tests)

### 11.1 Supported fields

```rust
pub struct SaturnConfig {
    pub package: Package,
    pub dependencies: BTreeMap<String, DependencySpec>,
}
pub struct Package { pub name: String, pub version: String, pub edition: String }
pub struct DependencySpec { pub version: String }  // bare string, no parsing
```

### 11.2 Is `saturn.toml` merely deserialization or does it participate in compilation?

**Classification: DESIGN ONLY (deserialization only, never read by compiler).**

- `from_toml_str()` (`config.rs:61`) — works, `toml::from_str`.
- `from_dir()` (`config.rs:41-58`) — defined but **never called** by Build/Check/Run.
- `Build` command (`main.rs:262-276`) — reads source file directly, never touches config.
- `Check` command (`main.rs:530-533`) — same.
- `Run` command (`build_run_file`, `main.rs:476-498`) — same.
- `Init` command — writes `saturn.toml` but never reads it back.

### 11.3 What is NOT supported

- No `[lib]` section (no `path`, no `crate-type`)
- No `[[bin]]` sections
- No `[profile]` / `[profile.release]`
- No `[target]` / `[target.xxx]`
- No `[dependencies]` with source (`path`, `git`, `registry`) — `DependencySpec` is a single `version` string
- No version-range parsing — `DependencySpec::from_str` (`config.rs:125-132`) just clones

### 11.4 Init command bug

`init_project(name, ...)` (`main.rs:540`) uses the `name` argument (which can be a full path like
`/tmp/test_project`) as both the directory path AND `package.name`. Result: `name = "/tmp/test_project"`.
No extraction of the file-name component (`Path::file_name`).

### 11.5 `toml` dependency placement

`toml = "0.8"` is declared in `crates/stnx/Cargo.toml:16` only — **NOT** in `[workspace.dependencies]`
in root `Cargo.toml`. All other dependencies are centralized in the workspace.

---

## 12. CLI Audit

Source: `crates/stnx/src/main.rs` (669 lines)

### 12.1 Commands

| Command | Arguments | Behavior | Uses config? |
|---------|-----------|----------|-------------|
| `Build` | `input, --output, --target, --emit-{ir,object,exe}, --print-target, --debug, --release, --opt-level, --json, --verbose, --no-link, --save-temps` | Full pipeline → link | No |
| `Check` | `input, --target` | Pipeline up to semantic analysis | No |
| `Run` | `input, --debug, --release, --target` | Build to temp dir → execute | No |
| `Doctor` | (none) | Prints target triple, host config, linker, runtime | No |
| `Init` | `name, --in-place, --pkg-version` | Scaffolds `saturn.toml` + `src/main.stn` | Writes config (never reads) |

### 12.2 CLI binary name discrepancy

| Component | Name |
|-----------|------|
| Cargo package | `stnx` (`crates/stnx/Cargo.toml:2`) |
| Binary on disk | `stnx` (confirmed by `cargo run`) |
| clap `#[command(name)]` | `saturnite` (`main.rs:10`) |
| README quick start | `stnx` (`README.md:19`) |
| README CLI reference | `saturnite` (`README.md:88`) |
| Init scaffolding message | `saturnite build src/main.stn` (`main.rs:591`) |

reference will get "command not found". The README is internally inconsistent.

### 12.3 Cross-compilation

`--target` accepts non-host triples in the CLI definition, but `main.rs:270-285` rejects them:
```rust
if !cfg.match_triple_host(&host_triple)? {
    bail!("Cross-compilation is not yet supported...");
}
```
Runtime is host-only (`cc` in `build.rs` compiles `println_i64.c` for host).

### 12.4 No CLI tests

There are **zero** tests for any CLI command. No tests for `Build`, `Check`, `Run`, `Init`, or
`Doctor`. The test suite tests library functions (`compile_src`, `analyze_src`, `ir`) but never
exercises the CLI entry point or argument parsing.

---

## 13. Test Coverage Audit

### 13.1 Coverage map (123 tests total)

| Component | Direct tests | Integration | Total | Notes |
|-----------|-------------|-------------|-------|-------|
| Lexer | 17 | 0 | 17 | Token kinds, spans, errors, overflow |
| Parser | 0 | 0 direct | ~93 indirect | Only via codegen/semantic/native |
| Semantic/HIR | 28 | 0 | 28 | Type checking, mutability, structs, enums |
| HIR (direct) | 0 | 0 | 0 | Only indirectly tested |
| MIR | 0 | 0 | 0 | Does not exist |
| Codegen | 0 | 61 (14 IR + 47 native) | 61 | IR text + native execution |
| Native execution | 0 | 47 | 47 | Full compile + run + check output |
| Diagnostics | 0 | 6 | 6 | Span + message checks |
| Config | 7 (lib) | 0 | 7 | TOML deserialization only |
| CLI | 0 | 0 | 0 | **No CLI tests at all** |

### 13.2 Major untested invariants

1. **Parser correctness** — No direct parser tests; no negative parser tests with spans.
2. **CLI behavior** — `init`, `doctor`, `run`, argument parsing, `--help` — all untested.
3. **Config integration** — `from_dir()` dead code; no test `Init`→build.
4. **Root `tests/` stale** — `tests/codegen.rs` uses old API (`generate_ir(&ast::Program)`).
5. **For-loop negative ranges** — No test for `for i in -5..5`. Uses `ULT`/`ULE` (unsigned) —
   **behavior undefined** for negative bounds.
6. **Enum variant tag values** — No test checks which tag maps to which variant.
7. **Struct layout** — No test verifies field offsets, sizes, nested struct layout.
8. **`--json` output** — No test verifies build report JSON structure.
9. **`--no-link` / `--save-temps`** — No tests.
10. **Release optimization** — No test compares debug vs release, or verifies O2/O3 correctness.

### 13.3 Test file structure

```
crates/stnx/tests/         # 114 integration tests
├── common/mod.rs          # compile_src, compile_to_object, ir_only, analyze_src
├── codegen.rs             # 14 tests (IR text assertions)
├── diagnostics.rs         # 6 tests (span + message checks)
├── lexer.rs               # 17 tests
├── native_compilation.rs  # 47 tests (compile + run + check output)
├── semantic.rs            # 28 tests
├── test_full_compile.rs   # 1 test
├── test_ir_only.rs        # 1 test
├── test_native_only.rs    # 1 test
└── test_target_machine.rs # 1 test
tests/                     # DEAD — git-tracked but NOT compiled
├── codegen.rs             # stale: generate_ir(&ast::Program)
├── lexer.rs               # stale clone
└── semantic.rs            # stale clone
```

---

## 14. Dependency Audit

### 14.1 Direct dependencies (from `Cargo.lock`, 122 total packages)

| Crate | Version | Declared in | Purpose |
|-------|---------|-------------|---------|
| `logos` | 0.16.1 | workspace + crate (lexer) | Tokenizer |
| `chumsky` | 0.13.0 | workspace + crate (parser) | Parser combinator |
| `inkwell` | 0.9.0 | workspace + crate (llvm21-1-prefer-dynamic) | LLVM bindings |
| `miette` | 7.6.0 | workspace + crate (fancy) | Diagnostics |
| `thiserror` | 2.0.19 | workspace + crate | `#[derive(Error)]` |
| `clap` | 4.6.5 | workspace + crate (derive) | CLI parsing |
| `serde` | 1.0.229 | workspace + crate (derive) | Serialization (config, build report) |
| `serde_json` | 1.0.151 | workspace + crate | JSON build report |
| `toml` | 0.8.23 | crate ONLY (not workspace) | `saturn.toml` parsing |
| `anyhow` | 1.0.104 | workspace + crate | CLI error handling |
| `which` | 5.0.0 | workspace + crate | Linker binary discovery |
| `cc` | 1.4.0 | build-dep only | Runtime C compilation (`build.rs`) |
| `tempfile` | 3.27.0 | dev-dep only | Isolated test temp dirs |

### 14.2 All dependencies verified as used

Every direct dependency is used (confirmed via `grep -rn` in source). No unused dependencies.
No version conflicts. No known CVEs (no `cargo-audit` run, but no obviously vulnerable versions).

### 14.3 `toml` not centralized

`toml = "0.8"` is in `crates/stnx/Cargo.toml:16` only. All other deps are in `[workspace.dependencies]`.
Should be moved for consistency.

### 14.4 Crate recommendations for 0.4

| Crate | Needed? | Justification |
|-------|--------:|:---------------|
| `toml` (centralize) | YES (move) | Already used; should be in `[workspace.dependencies]` |
| `hashbrown` | Maybe (Phase 4) | `SymbolInterner` uses `Vec<String> + HashMap`; `indexed_hashmap` could give stable iteration — but current approach is fine |
| `blake3`/`sha2` | Maybe (Phase 6) | For incremental compilation fingerprints |
| `cranelift` | NO | LLVM is the backend; redundant for 0.4 |
| `pyo3` | NO | Python interop is 0.5+ |

---

## 15. Symbol System Audit

Source: `crates/stnx/src/hir/symbol.rs` (56 lines)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SymbolId(u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DefId(u32);

#[derive(Debug, Default)]
pub struct SymbolInterner {
    strings: Vec<String>,
    indices: HashMap<String, SymbolId>,
}
```

### 17.1 What exists

- `SymbolId(u32)` / `DefId(u32)` — cheap copy, array indices.
- `SymbolInterner` — `Vec<String>` + `HashMap<String, SymbolId>`. `intern()`, `lookup()`.
- `Interner` trait (`symbol.rs:41-44`) — `fn intern(&mut self, s: &str) -> SymbolId`.

### 17.2 What is MISSING

- **No `DefId` namespace hierarchy** — flat array index. No module path component.
- **No persistence** — `SymbolInterner` is not `Serialize`/`Deserialize`.
- **Fragile `DefId` indexing** — `HirProgram.function(def_id)` (`function.rs:72`) uses `def_id.0`
  as vec index. If functions are reordered, all `DefId`s shift.
- **No interning for spans** — `SourceSpan` inlined on every HIR node (16 bytes each).

### 17.3 Recommendations

1. Add `Serialize`/`Deserialize` to `SymbolId`/`DefId` (trivial newtype).
2. Add `#[derive(Serialize, Deserialize)]` to `SymbolInterner`.
3. Document invariant: `DefId.0` MUST index into `HirProgram.functions`.
4. For 0.4, keep `SymbolId`/`DefId` as flat indices (module paths deferred to 0.5).

---

## 16. Incremental Compilation Audit

**Classification: DESIGN ONLY (documentation only, no implementation).**

| Component | Exists in source? | Evidence |
|-----------|-------------------|----------|
| Fingerprint module | ❌ | No `fingerprint.rs` |
| Fingerprint computation | ❌ | No SHA-256 / hashing in codebase |
| Cache paths (`target/incremental/`) | ❌ | Never created in code |
| Serialized HIR cache | ❌ | HIR types have no `Serialize`/`Deserialize` |
| Object file caching | ❌ | Every build re-runs full codegen |
| Invalidation logic | ❌ | No cache invalidation code |

The design doc (`SATURNITE_INCREMENTAL_COMPILATION.md`) proposes:
- Fingerprint: `SHA-256(source_content || dependencies_hashes || config_hash)`
- Cache: `target/incremental/fingerprints.json`, `hir/<fp>.hir`, `objects/<fp>.o`

None implemented. HIR is the ideal incremental boundary (immutable, fully resolved, interned) but
must derive `Serialize`/`Deserialize` first (Phase 1 of 0.4). Insertion point: `main.rs`, wrapping
`analyze_and_lower()` in a cache-lookup/store pattern.

---

## 17. MIR Readiness Audit

### 16.1 Zero MIR code exists

Confirmed by `grep -rn "mir\|Mir\|MIR" crates/stnx/src/` → zero code hits. Only two **doc comments**
reference MIR as a future stage (`hir/lower.rs:5`, `hir/expr.rs:5`). `docs/SATURNITE_MIR_DESIGN.md`
is the only specification.

### 16.2 What MIR SHOULD own vs. what currently owns it

| MIR responsibility | Currently in | Status |
|--------------------|-------------|--------|
| Control-flow graph | **Codegen** (`context.rs:490-712`) | INLINE — BBs constructed on-the-fly, not stored |
| Locals (variable allocations) | **Codegen** (`Variable`, `FunctionScope`) | INLINE — no `Place` abstraction |
| Assignments (store/load) | **Codegen** — `build_store`/`build_load` | INLINE |
| Calls (with args + return) | **Codegen** — `build_call` | INLINE |
| Terminators (branch/return) | **Codegen** — build_conditional_branch, build_return | INLINE |
| Structured control flow lowering | **Codegen** — If/For/While handlers | INLINE |
| Coercions (int-to-bool) | **Codegen** — `build_int_cast` | INLINE |
| Optimizations (CSE, DCE) | **LLVM** — `run_passes("default<O3>")` | LLVM-level, not MIR |

### 16.3 Minimum MIR needed for 0.4

| Type | Fields |
|------|--------|
| `MirBasicBlock` | `id: BlockId, stmts: Vec<MirStmt>, terminator: MirTerminator` |
| `MirStmt` | `LocalDecl { local, ty }`, `Assign { place, rvalue }` |
| `MirTerminator` | `Goto { target }`, `Switch { cond, target, else_target }`, `Return(Option<MirOperand>)` |
| `MirOperand` | `Const(i64\|f64\|bool)`, `Local(LocalId)` |
| `MirRvalue` | `Use(MirOperand)`, `Binary { op, lhs, rhs }` |
| `MirFunction` | `def_id, name, blocks: Vec<MirBasicBlock>, locals: Vec<HirType>` |
| `MirProgram` | `functions: Vec<MirFunction>` |
| `ConstFold` pass | One pass: `1 + 2` → `3` |

**Deferred to 0.5+:** `MirPlace`, `StorageLive/Dead`, struct/field/enum rvalues, `MirPass` trait,
`--emit-mir` flag, multi-pass optimization.

---

## 18. Architectural Risks

### CRITICAL (would require major rewrites if deferred)

1. **CLI binary name mismatch** — Binary is `stnx`, clap name is `saturnite`, README inconsistent.
   Users following README get "command not found". **Fix: add `[[bin]] name = "saturnite"`** to crate
   `Cargo.toml`.

2. **`saturn.toml` is a silent no-op** — Parsed but never read by Build/Check/Run. `from_dir()` dead
   code. `DependencySpec` is bare String, no resolver. **High foot-gun risk.**

3. **HIR not serializable** — Blocks incremental compilation. Must add `Serialize`/`Deserialize`.

4. **No module system** — `program()` = `func().repeated()`. Structs/enums trapped in function
   bodies. **Single biggest blocker to a real language.**

5. **`Init` project name bug** — Uses full path as `package.name`. Must extract file-name component.

### HIGH (should be fixed in 0.4)

6. **For-loop unsigned comparisons** — `context.rs:629,631` uses `ULT`/`ULE`. `for i in -5..5`
   produces incorrect behavior. No test covers this.

7. **Range end value discarded** — `context.rs:716`: `let _ = end;`. HIR `Range` carries both
   `start`/`end` but codegen only emits `start`.

8. **Stale root-level tests** — `tests/codegen.rs` uses old API (`generate_ir(&ast::Program)`).
   Tracked but not compiled. Should be deleted or migrated.

9. **Zero CLI tests** — No tests for Build/Check/Run/Init/Doctor or argument parsing.

10. **`While` creates scope but `For` doesn't** — `lower.rs:653` (`While` creates
    `with_parent(scope.clone())`), but `For` does not. Loop variable leaks.

### MEDIUM (address during 0.4)

11. No direct parser tests; no negative parser tests with spans.
12. Three manual keyword tables — adding a keyword requires edits in 3 places.
13. `analyze()` vs `analyze_and_lower()` redundancy.
14. Debug-only parser API (`examples/debug_parse.rs`).
15. `If`/`For`/`While` always return `Unit` — limits expression-based returns.

### LOW

16. `rust_out` stale ELF binary checked into repo root.
17. `println` returns `i64` (always 0) — constrains future expression use.
18. `Type` derives `PartialEq` but not `Eq` — can't be `HashMap` key.

---

## 19. Recommended 0.4 Architecture

### Core Goals (6, max scope)

| # | Goal | Prerequisites | Risk |
|---|------|--------------|------|
| 1 | **CLI/config integration** — `saturn.toml` read by Build/Check/Run; binary name = `saturnite` | None | LOW |
| 2 | **HIR serialization** — `Serialize`/`Deserialize` on all HIR types | Goal 1 | LOW |
| 3 | **Minimum viable MIR** — CFG with basic blocks, locals, assignments, terminators; one optimization pass | Goal 2 | MEDIUM |
| 4 | **Proper `str` type** — real string runtime, `println` accepts str | Goal 3 | MEDIUM |
| 5 | **Top-level struct/enum** — move from function-body stmts to program items | Goal 4 | MEDIUM |
| 6 | **Module system** (`mod`/`use`/`pub`) | Goals 1, 2, 4, 5 | HIGH |

### Phase plan

**Phase 0 — Architecture corrections:** Fix binary name, wire config, delete stale tests, fix Init.
**Phase 1 — HIR serialization:** Add serde derives to HIR types.
**Phase 2 — Minimum MIR:** CFG + one optimization pass; rewrite codegen to consume MIR.
**Phase 3 — Top-level items:** Move structs/enums to program-level items.
**Phase 4 — Proper `str`:** Real string runtime + string I/O.
**Phase 5 — Modules:** `mod`/`use`/`pub` + file-based loading.
**Phase 6 — Incremental compilation:** Cache HIR + objects by fingerprint.

### Dependency graph

```
Phase 0 → Phase 1 → Phase 2 → Phase 3 → Phase 4 → Phase 5 → Phase 6
   ↓                                    ↘
Config (Phase 0)                          → Phase 5 (modules need config paths)
HIR Serialize (Phase 1)                   → Phase 6 (cache needs serialization)
```

---

## 20. DO NOT IMPLEMENT YET List

These features must wait because their architectural prerequisites do not yet exist:

| Feature | Why wait | Prerequisites (Phase) |
|---------|----------|----------------------|
| **Generics** | `HirType` has no type parameters; `FunctionSig` has fixed types; no monomorphization | Top-level items + modules (Phases 3, 5) |
| **Traits** | No trait definitions; no impl blocks; no method resolution | Generics + reference types |
| **Full borrow checker** | No reference type in `HirType`; no lifetime tracking; no `Place` model | Reference type (Phase 4) + MIR `Place` |
| **Package registry / dependency fetching** | `DependencySpec` is bare String; no resolver; no fetcher | Module system (Phase 5) first |
| **Python interop** | No `pyo3`; design-only; heavy native build requirements | Module system (Phase 5) |
| **Cranelift backend** | LLVM is already working with full optimization | Only if WASM/embedded target needed |
| **JIT compilation** | Requires runtime MIR recompilation + symbol resolution | Incremental compilation (Phase 6) |
| **Distributed compilation** | Needs full state serialization + network protocol | Incremental + HIR/MIR serialization |
| **Macro system** | No token-tree representation; no macro expansion engine | Modules + hygiene system |
| **Async/await** | No `Future` type; no executor; no `poll` model | Major language + runtime overhaul |
| **Pattern matching** | No `match` syntax; enums are bare tags | Data-carrying enums (after generics) |
| **`--emit-mir` text output** | Diagnostic, not core compiler feature | Can be added anytime after Phase 2 |
| **Inline assembly** | No `asm!` macro; no assembly AST node | Not needed for 0.4 scope |
| **Data-carrying enums** | Need `HirType` with variant payloads + LLVM tagged unions | Top-level enums + MIR place model |
| **Closures** | Need first-class function values + capture tracking | Fn pointer type + module system |

---

## 21. Crate Recommendations

| Crate | Needed? | Justification |
|-------|--------:|:---------------|
| `toml` (move to workspace) | YES | Already used (`config.rs`); centralize for consistency |
| `serde` derive | Already present | Used for config serialization; extend to HIR in Phase 1 |
| `sha2` or `blake3` | Maybe (Phase 6) | For incremental compilation fingerprints |
| `cranelift` | NO | LLVM is the backend; redundant for 0.4 |
| `pyo3` | NO | Python interop is 0.5+ |
| `hashbrown` | Maybe (Phase 4) | `SymbolInterner` works fine with std `HashMap`; no benefit yet |
| `itertools` | NO | Not needed; keep deps minimal |
| `tempfile` | Already present (dev-dep) | Used in test isolation |

**No new dependencies needed for Phases 0-3.** Phase 4 (`str` runtime) needs only C stdlib.
Phase 5 (modules) needs no new crates. Phase 6 (fingerprints) would add `sha2` or `blake3`.

---

## 22. Final Verdict

### What is Saturnite 0.3 today?

A single-binary compiler that compiles a small Rust-like language to native machine code via LLVM 21.
It has a working lexer/parser/HIR/codegen pipeline with 123 passing tests. The HIR layer is a solid,
fully-resolved intermediate representation with interned symbols, typed expressions, and source spans.
However, it is a **single-file** language with no modules, no MIR, no incremental compilation, and a
`config file` that is parsed but never used.

### What is its strongest foundation?

The **HIR layer** (`crates/stnx/src/hir/`). It is a properly designed IR with: resolved identifiers
(`SymbolId`/`DefId`), types on every expression (`HirType`), source spans on every node
(`SourceSpan`), and a clean separation from AST. Codegen consumes `HirProgram` (not `ast::Program`).
The two-pass function declaration (signatures first, bodies second) enables forward references and
recursion. The `SymbolInterner` is simple and correct.

### What is its biggest architectural weakness?

**The complete absence of a module system.** The parser's `program()` only accepts functions;
there are no `mod`/`use`/`pub` keywords. Structs/enums are trapped inside function bodies. This
blocks every cross-function and cross-file language feature.

**Runner-up:** `saturn.toml` is parsed but silently ignored by the compiler — a critical foot-gun.

### What must 0.4 accomplish?

1. **Wire config into the pipeline** — `saturn.toml` is read by Build/Check/Run. Fix Init's name bug.
2. **Resolve binary naming** — `stnx` binary vs `saturnite` clap name → pick one.
3. **Make HIR serializable** — enables caching (Phase 1).
4. **Introduce minimum viable MIR** — CFG-based IR; rewrite codegen to consume MIR (Phase 2).
5. **Move structs/enums to top-level items** — unblocks type sharing (Phase 3).
6. **Proper `str` type** — real string runtime, `println("hello")` works (Phase 4).
7. **Module system** — `mod`/`use`/`pub` + file loading (Phase 5).

### What 0.4 should NOT attempt?

Generics, traits, full borrow checker, package registry, Python interop, Cranelift, JIT,
distributed compilation, macros, async. These require foundations that don't exist yet.
See Section 20 (DO NOT IMPLEMENT YET list).

### 7 core questions — explicit answers

1. **Does MIR exist?** No — zero MIR code. Design doc only.
2. **Does codegen consume HIR or AST?** HIR (`HirProgram`), directly, with no MIR layer.
3. **Is `saturn.toml` used in compilation?** No — parsed but never read by Build/Check/Run.
4. **Are structs/enums top-level items?** No — they are `Stmt` variants inside function bodies.
5. **Is HIR serializable?** No — derives only `Debug`/`Clone`. No `Serialize`/`Deserialize`.
6. **Is the CLI binary name consistent?** No — binary is `stnx`, clap name is `saturnite`, README uses both.
7. **Are there tests for MIR?** N/A — MIR doesn't exist. Tests cover lexer (17), parser (indirect ~93),
   semantic/HIR (28), codegen (61), diagnostics (6), config (7). 123 total.