# Saturnite vs Rust Compiler: Side-by-Side Architecture Comparison

**Date:** 2026-08-27
**Sources:** `SATURNITE_ACTUAL_ARCHITECTURE.md` (Phase 1) and `RUST_ACTUAL_ARCHITECTURE.md` (Phase 2)
**Scope:** Eight architectural domains compared section-by-section

---

## 1. Exec Summary

| Domain | Saturnite | Rust | Gap |
|---|---|---|---|
| **Scale** | Single crate, 25 source files | ~150 crates, thousands of files | Architectural maturity |
| **Model** | Linear 8-stage pipeline, eager | Demand-driven query system, lazy | Incremental compilation |
| **Identifiers** | 3 flat `u32` spaces (DefId collapse bug) | `DefId{krate, index}` + `DefPathHash` | Soundness |
| **MIR phases** | Built → Optimized → Codegen | Built → Analysis(Initial/Post) → Runtime(Initial/Post/Optimized) | Phase tracking |
| **MIR passes** | 1 (constant folding) | ~40 (MirPass trait + pipeline) | Optimization breadth |
| **Backend** | Inkwell 0.9, flat context, always-alloca | rustc_codegen_ssa + LLVM, FunctionCx generic, LocalRef enum | Codegen quality |
| **Module system** | ModuleGraph (CLI bypass bug) | rustc_resolve + HIR map | Correctness |
| **Serialization** | 15+ types missing serde | Full Encodable/Decodable | Caching/incremental |

**Three architectural chasms:**

1. **Demand-driven vs. linear.** Rust's query system evaluates on demand, tracks dependencies, and caches results on disk. Saturnite runs a fixed pipeline from top to bottom every invocation. No incremental compilation is possible in Saturnite today (15 types lack `Serialize/Deserialize`).

2. **Identifier soundness.** Saturnite's `DefId(u32)` is assigned from *independent per-kind counters*, so `DefId(0)` is simultaneously a function, a struct, and an enum — a soundness defect (Showstopper #1). Rust's `DefId` couples a `DefIndex` with a `CrateNum`, and stable `DefPathHash` enables cross-session identity.

3. **Backend sophistication.** Saturnite allocates an LLVM `alloca` for every local unconditionally and iterates blocks in vector order. Rust distinguishes `LocalRef::Immediate` (direct SSA) from `LocalRef::Place` (alloca), iterates in reverse postorder, carries full debug info, and computes `FnAbi` for ABI-correct calls.

---

## 2. Pipeline Comparison

### Saturnite 8-stage pipeline

| Stage | Component | Input → Output | Key file |
|---|---|---|---|
| 1 | Source | `.stnx` text → raw string | CLI input |
| 2 | Lexer | raw string → `Vec<Token>` | `lexer/mod.rs` (353 lines) |
| 3 | Parser | `Vec<Token>` → `ast::Program` | `parser/mod.rs` (1457 lines) |
| 4 | HIR Lowering | `ast::Program` → `HirProgram` | `hir/lower.rs` |
| 5 | MIR Lowering | `HirProgram` → `MirProgram` | `mir/lower.rs` |
| 6 | MIR Verify | `MirProgram` → `Result<(), Vec<MirVerifyError>>` | `mir/verify.rs` (204 lines) |
| 7 | MIR Optimize | `MirProgram` → `MirProgram` (mutated) | `mir/opt.rs` (163 lines) |
| 8 | CodeGen + Link | `MirProgram` → executable | `mir/codegen.rs` (841) + `codegen/linker.rs` (200) |

Two entry points exist:
- **Single-file:** `analyze_and_lower(&program)` — called by CLI (Showstopper #2: CLI bypass).
- **Multi-module:** `analyze_and_lower_with_graph(&program, &project.graph)` — exists but **never invoked**.

### Rust multi-crate pipeline

| Phase | Crate | Output | Key entry point |
|---|---|---|---|
| Parse | `rustc_lexer` + `rustc_parse` | `ast::Crate` | `passes::parse()` |
| Expand | `rustc_expand` | expanded `Crate` | `passes::configure_and_expand()` |
| Resolve | `rustc_resolve` | `Resolver` | `passes::resolve()` |
| HIR lowering | `rustc_ast_lowering` | `hir::Crate` | `rustc_ast_lowering::hier_ext()` |
| Analysis | `rustc_hir_analysis` | `TyCtxt` | `analyze()` |
| Query system | `rustc_middle` | demand-driven | `DEFAULT_QUERY_PROVIDERS` registration |
| MIR build | `rustc_mir_build` | `mir::Body` | `build_mir_inner_impl()` |
| MIR transform | `rustc_mir_transform` | optimized `mir::Body` | Pass manager (`run_passes_inner`) |
| Borrowck | `rustc_borrowck` | borrow-checked body | `do_mir_borrowck()` |
| Codegen | `rustc_codegen_ssa` / `rustc_codegen_llvm` | object code | `codegen_crate()` |
| Link | `back/link.rs` | executable | `Linker::link()` |

### Architectural gap analysis

| Aspect | Saturnite | Rust | Divergence |
|---|---|---|---|
| **Pipeline model** | Fixed linear sequence, no feedback loops | Demand-driven queries with cycle detection and on-disk caching | Rust's model enables incremental recompilation — unchanged queries skip re-execution |
| **Macro expansion** | N/A — language has no macros | `rustc_expand` runs before resolution | Saturnite's grammar is a strict subset |
| **Resolution** | Single post-pass inside `hir/lower.rs` (Pass 2) | Dedicated `rustc_resolve` crate with `Resolver`, `NameResolution`, `BindingTable` | Rust separates resolution from lowering; Saturnite conflates them |
| **Type checking** | None — `HirType = MirType` (7 flat variants), no typeck pass | `rustc_typeck` + `rustc_hir_analysis` with full trait solving | Saturnite has no type checker at all; types are syntactic labels |
| **Borrow checking** | N/A | `rustc_borrowck` with full NLL analysis | Saturnite has no borrow checker; no aliasing model |
| **Entry point dispatch** | 5 CLI subcommands (Build, Check, Run, Doctor, Init) all share fixed pipeline | `rustc_driver_impl` registers callbacks for `Config` that can override any pass | Rust's driver is a general-purpose API; Saturnite's CLI is the sole entry point |
| **Intermediate caching** | None — re-lexes, re-parses, re-lowers every invocation | `QuerySystem` with `on_disk_cache: Option<OnDiskCache>`, `QueryCache`, and `arena_cache` queries | Saturnite's 364 tests run in 2 minutes *because* it rebuilds from scratch |
| **Phase boundary validation** | `mir/verify.rs` runs 5 structural checks | Each MIR phase sets `phase` via `phase_change`; validator runs after every pass (`--validate-mir`) | Rust validates MIR correctness after *every* pass, not just once |

**Key divergence points:**
- Saturnite's `Check` subcommand stops at `semantic::analyze` (line 363-381), which does no type checking — it is purely a parse + HIR-lowering check. Rust's `--emit metadata` equivalent runs full type checking and produces `.rmeta` files.
- Rust's pipeline is decomposed into ~15 crates, each independently testable. Saturnite's pipeline lives in one crate; the parser alone is 1457 lines with 57 inline tests.

---

## 3. MIR Layer

### Saturnite MIR (5 files)

| File | Lines | Responsibility |
|---|---|---|
| `mir/mod.rs` | 343 | Type inventory: `MirProgram`, `MirFunction`, `MirBasicBlock`, `MirStmtKind`, `MirRvalue`, `MirOperand`, `MirConst`, `MirBinOp`, `MirLocal`, `BlockId`, `LocalId` |
| `mir/lower.rs` | 734 | `lower_program(hir: &HirProgram) -> MirProgram` — one-function-per-HIR-function lowering |
| `mir/verify.rs` | 204 | 5 structural checks (terminator presence, valid targets, valid LocalIds, valid param locals, valid start block) |
| `mir/opt.rs` | 163 | `ConstantFolder` — single pass, wrapping arithmetic on `MirConst` |
| `mir/codegen.rs` | 841 | LLVM IR generation, `MirCodeGenContext`, `ObjectEmitter`, `Linker` |

**MIR type inventory (flat, untyped):**

| Concept | Saturnite variants | Notes |
|---|---|---|
| `MirRvalue` | 7: `Use`, `Binary`, `Unary`, `StructLit`, `FieldAccess`, `EnumCtor`, `StrLit` | No `Cast`, no `Ref`, no `Repeat`, no `Discriminant`, no `Aggregate` |
| `MirTerminator` | 5: `Goto`, `SwitchInt`, `Call`, `Return`, `Unreachable` | No `Drop`, no `Assert`, no `Yield`, no `UnwindResume`, no `TailCall`, no `InlineAsm` |
| `MirStmtKind` | 2: `LocalDecl`, `Assign` | No `StorageLive`, `StorageDead`, `Nop`, `ConstEvalCounter`, `FakeRead` |
| `MirConst` | 3: `I64`, `F64`, `Bool` | No `Str`, no `Const`, no `ConstAlloc`, no promoted statics |
| `MirLocal` | 1 kind | No `LocalKind` enum — every local is treated identically |

`MirType = HirType` — MIR reuses the 7-variant type system (`I64`, `F64`, `Bool`, `Str`, `Unit`, `Struct(String)`, `Enum(String)`). No lifetime parameters, no generics.

### Rust MIR (multiple files in `compiler/rustc_middle/src/mir/`)

| File | Responsibility |
|---|---|
| `mod.rs` | `Body<'tcx>` — the central MIR container (15 fields) |
| `basic_blocks.rs` | `BasicBlockData`, `BasicBlocks` with `Cache` (predecessors, RPO, dominators) |
| `syntax.rs` | `Place`, `ProjectionElem`, `Operand`, `Rvalue` enums |
| `terminator.rs` | `Terminator`, `TerminatorKind`, `SwitchTargets`, `UnwindAction` |
| `statement.rs` | `Statement`, `StatementKind` |
| `local.rs` | `Local`, `LocalDecl`, `LocalKind` |
| `source.rs` | `SourceInfo`, `SourceScope`, `SourceScopeData` |
| `visit.rs` | `MutVisitor` / `Visitor` traits for MIR traversal |
| `abstract_const.rs` | `Const`, `ConstValue`, `ConstAlloc` |
| `mono.rs` | `Instance`, `InstanceKind`, monomorphization |

**Rust `Body<'tcx>` structure (15 fields):**

```rust
pub struct Body<'tcx> {
    basic_blocks: Box<IndexVec<BasicBlock, BasicBlockData<'tcx>>>,
    phase: MirPhase,
    pass_count: u32,
    source: MirSource,
    source_scopes: IndexVec<SourceScope, SourceScopeData<'tcx>>,
    local_decl: IndexVec<Local, LocalDecl<'tcx>>,
    coroutine: Option<CoroutineData<'tcx>>,
    var_debug_info: Vec<VarDebugInfo<'tcx>>,
    span: Span,
    required_consts: Vec<AnonConst>,
    mentioned_items: Option<Vec<DefId>>,
    is_polymorphic: bool,
    injection_phase: InjectionPhase,
    tainted_by_errors: Option<ErrorGuarantee>,
}
```

**MIR phase ordering:**

```
Built → Analysis(Initial) → Analysis(PostCleanup)
      → Runtime(Initial) → Runtime(PostCleanup) → Runtime(Optimized)
```

Each pass pipeline sets the phase via `phase_change` in `run_passes_inner`. Saturnite has no phase concept — MIR goes `Built → Optimized → Codegen` with no intermediate validation.

### Architectural gap analysis

| Concept | Saturnite | Rust | Gap severity |
|---|---|---|---|
| **Body container** | `MirProgram { functions: Vec<MirFunction>, ... }` — flat Vec | `Body<'tcx>` with `IndexVec`, phase tracking, coroutine metadata, debug info, span | High |
| **Basic blocks** | `MirBasicBlock { id, stmts, terminator }` — Vec, no cache | `BasicBlockData` + `BasicBlocks` with `Cache` (predecessors, RPO, dominators) | High |
| **Block ordering** | Vector order (as written) | Reverse postorder (RPO) — critical for optimization | High |
| **Locals** | `MirLocal { id, ty, mutable, name, kind }` — no kind distinction | `LocalDecl` with `LocalKind` enum (14 variants: Temp, Arg, ReturnPointer, Var, Const, ClosureCapture, Drop, etc.) | Medium |
| **Place model** | N/A — `gen_rvalue` matches directly on operands | `Place<'tcx>` with `ProjectionElem` chain (Deref, Field, Index, Subsume, Downcast) | High |
| **Operand model** | `MirOperand { Const(MirConst) \| Local(LocalId) }` — 2 variants | `Operand<'tcx>` with `Copy(Box<Place>)`, `Move(Box<Place>)`, `Constant`, `RuntimeChecks` | High |
| **Terminators** | 5 variants | 15+ variants including `Drop`, `Assert`, `Yield`, `UnwindResume`, `TailCall`, `InlineAsm` | High |
| **Rvalues** | 7 variants | 13+ variants including `Repeat`, `Ref`, `Cast`, `Discriminant`, `Aggregate` | High |
| **Rvalue operands** | Direct `MirOperand` | `Box<(Operand, Operand)>` — boxed for memory efficiency | Low |
| **Phase tracking** | None | `MirPhase` enum with 5 states; each pass sets phase | Medium |
| **Type parameters** | `MirType = HirType` (no `'tcx` lifetime, no generics) | All MIR types carry `'tcx` lifetime; `is_polymorphic` flag | Critical |
| **Debug info** | None | `var_debug_info: Vec<VarDebugInfo>`, `source_scopes` with `DesugaringKind` | High |
| **Validation** | 5 structural checks, one pass | Validator runs after *every* pass with ~20 checks + type system integration | High |
| **Const model** | `MirConst { I64, F64, Bool }` — 3 primitives | `Const<'tcx>` wrapping `ConstVal` with allocations, promoted statics, `StaticDef` | High |

**Key divergence points:**
- Saturnite's `MirBasicBlock.id` is a `BlockId(u32)` newtype — simple sequential indexing. Rust's `BasicBlock` is a `NonZero<usize>` with `START_BLOCK = BasicBlock::new(1)` (0 is sentinel). Rust's `Cache` struct lazily computes predecessors, RPO, and dominator trees — Saturnite has none of this.
- Saturnite's terminator `Return(Some(operand))` carries an operand. Rust's `Return` terminator is bare — the return value is placed in the `RETURN_PLACE` local (index 1) via an implicit `Assign` before `Return`.
- Saturnite's `SwitchInt` has `branches: Vec<(u128, BlockId)>` + `else_target: BlockId`. Rust's `SwitchTargets` has `otherwise: BasicBlock` + `targets: Box<[(u128, BasicBlock)]>`.

---

## 4. HIR Layer

### Saturnite HIR (7 files in `hir/`)

| File | Lines | Types | Derives |
|---|---|---|---|
| `hir/mod.rs` | 40 | Re-exports only | — |
| `hir/symbol.rs` | 187 | `SymbolId`, `DefId`, `ModuleId`, `SymbolInterner`, `DefTable`, `DefEntry`, `DefKind`, `Visibility` | Most: Debug only — NO serde |
| `hir/function.rs` | 221 | `HirProgram`, `HirFunction`, `StructDef`, `EnumDef`, `HirUseDecl`, `HirModDecl` | `HirProgram`: Debug only — NO serde |
| `hir/expr.rs` | 118 | `HirExpr`, `HirExprKind` | Debug, Clone — NO serde |
| `hir/stmt.rs` | 54-55 | `HirStmt`, `HirStmtKind` | Debug, Clone — NO serde |
| `hir/types.rs` | 57 | `HirType` (7 variants) | serde ✓ |
| `hir/lower.rs` | 734-875 | `lower_program`, `lower_program_with_graph` | — |

**`HirProgram` (10 fields):**

```rust
pub struct HirProgram {
    pub functions: Vec<HirFunction>,
    pub structs: Vec<StructDef>,
    pub enums: Vec<EnumDef>,
    pub symbols: SymbolInterner,
    pub modules: Vec<Module>,
    pub root_module: ModuleId,
    pub module_paths: HashMap<DefId, ModuleId>,
    pub def_table: DefTable,
    pub module_scopes: Vec<ModuleScope>,
    pub use_decls: Vec<HirUseDecl>,
    pub mod_decls: Vec<HirModDecl>,
}
```

**`HirFunction` (inferred):** `{ name, params, return_ty, body, ... }` — body is lowered to `HirExpr`/`HirStmt`.

**`HirExprKind` variants:** `Var`, `Assign`, `Call`, `If`, `For`, `While`, `StructLiteral`, `FieldAccess`, `EnumConstructor`, `Binary`, `Unary`, `Lit`, etc. (no `Match`, no closures, no `let` expressions).

**`HirStmtKind` variants:** `Let`, `Return`, `Println`, `StructDef`, `EnumDef`.

**`HirType` (7 variants):** `I64`, `F64`, `Bool`, `Str`, `Unit`, `Struct(String)`, `Enum(String)`.

**Serialization status:** 15 types across HIR lack `Serialize/Deserialize` (per Section 5 of Phase 1 report). This is Showstopper #3.

### Rust HIR (`compiler/rustc_hir/src/`, ~7 core files)

| File | Types | Notes |
|---|---|---|
| `hir.rs` | `Node<'hir>`, `Crate`, `Expr`, `ExprKind`, `Pat`, `PatKind`, `Stmt`, `StmtKind`, `Arm`, `Block` | `Node` enum has 30+ variants; `ExprKind` has ~35 variants |
| `def.rs` | `DefKind` (35 variants), `CtorKind`, `CtorOf`, `Res` | Namespace-aware: TypeNS, ValueNS, MacroNS, LifetimeNS |
| `hir.rs` | `Item`, `ItemKind` (~25 variants), `Attribute`, `Visibility` | Full macro support, generics, where-clauses |
| `lib.rs` | `Map` trait (HIR map interface), `IntrusiveVar`, `Parent` | Query-backed HIR map |
| `pat.rs` | `Pat`, `PatKind` | Pattern matching with or-patterns, ranges, slices |

**Rust `Node<'hir>` (30+ variants):** `Param`, `Item`, `ForeignItem`, `TraitItem`, `ImplItem`, `Variant`, `Field`, `AnonConst`, `ConstBlock`, `ConstArg`, `Expr`, `ExprField`, `ConstArgExprField`, `Stmt`, `PathSegment`, `Ty`, `AssocItemConstraint`, `TraitRef`, `OpaqueTy`, `TyPat`, `Pat`, `PatField`, `PatExpr`, `Arm`, `Block`, `LetStmt`, `Ctor`, `Lifetime`, `GenericParam`, `Crate`, `Infer`, `WherePredicate`, `PreciseCapturingNonLifetimeArg`, `TestBinderForall`, `TestBinderExists`, `Synthetic`, `Err`.

**Rust `ExprKind` (~35 variants):** `ConstBlock`, `Array`, `Call`, `MethodCall`, `Use`, `Tup`, `Binary`, `Unary`, `Lit`, `Cast`, `Type`, `DropTemps`, `Let`, `If`, `Loop`, `Match`, `Closure`, `Block`, `Assign`, `AssignOp`, `Field`, `Index`, `AddrOf`, `Break`, `Continue`, `Ret`, `Become`, `InlineAsm`, `OffsetOf`, `Yield`, `YieldFrom`, `Err`.

**Rust `ItemKind` (~25 variants):** `ExternCrate`, `Use`, `Static`, `Const`, `ConstBlock`, `Fn`, `Macro`, `Mod`, `ForeignMod`, `GlobalAsm`, `TyAlias`, `Enum`, `Struct`, `Union`, `Trait`, `TraitAlias`, `Impl`, `Delegation`, `DelegationMac`, `TestBinderConstraints`.

**`HirId` / `OwnerId` / `ItemLocalId`:**

```rust
pub struct OwnerId { pub def_id: LocalDefId }
pub struct HirId { pub owner: OwnerId, pub local_id: ItemLocalId }
pub type ItemLocalId = NonZero<u32>;
```

`HirId` adds a `local_id` to disambiguate within a definition owner. Saturnite has no analogous two-part ID — its `DefId` is a flat `u32` with the namespace collapse bug (Section 3.5 of Phase 1).

**Rust `DefKind` (35 variants):** Type namespace: `Mod`, `Struct`, `Union`, `Enum`, `Variant`, `Trait`, `TyAlias`, `ForeignTy`, `TraitAlias`, `AssocTy`, `TyParam`. Value namespace: `Fn`, `Const`, `ConstParam`, `Static`, `Ctor`, `AssocFn`, `AssocConst`. Macro namespace: `Macro`. Non-namespaced: `ExternCrate`, `Use`, `ForeignMod`, `AnonConst`, `OpaqueTy`, `Field`, `LifetimeParam`, `GlobalAsm`, `Impl`, `Closure`, `CoroutineClosure`, etc.

**`Res` (Resolution):** `Def(DefKind, DefId)`, `PrimTy`, `SelfTyParam`, `SelfTyAlias`, `_Others`.

**HIR Map queries (on `TyCtxt`):** `tcx.hir_owner_nodes(def_id)`, `tcx.hir_owner(def_id)`, `tcx.hir_node(hir_id)`, `tcx.hir_body(body_id)`, `tcx.hir_crate_items()`, `tcx.hir_module_items(mod_id)`.

**`ModuleItems`:**

```rust
pub struct ModuleItems {
    pub submodules: Vec<LocalDefId>,
    pub free_items: Vec<LocalDefId>,
    pub trait_items: LocalDefIdMap<Vec<ItemTreeInfo>>,
    pub impl_items: LocalDefIdMap<IndexVec<...>>,
    pub foreign_items: Vec<LocalDefId>,
    pub body_owners: Vec<LocalDefId>,
    pub proc_macro_decls: Option<LocalDefId>,
    pub eiis: IndexVec<...>,
}
```

**THIR (Typed HIR Body):**

```rust
pub struct Thir<'tcx> {
    pub body_type: ThirBody<'tcx>,
    pub attributes: AttrVec,
    pub arms: IndexVec<ArmIndex, Arm<'tcx>>,
    pub blocks: IndexVec<BlockId, Block<'tcx>>,
    pub exprs: IndexVec<ExprId, Expr<'tcx>>,
    pub stmts: IndexVec<StmtId, StmtId, Stmt<'tcx>>,
    pub params: IndexVec<ParamId, Param<'tcx>>,
    pub struct_destructured_elements: ...,
}
```

### Architectural gap analysis

| Concept | Saturnite | Rust | Gap severity |
|---|---|---|---|
| **Node enumeration** | Separate types: `HirExpr`, `HirStmt` — no unified `Node` | `Node<'hir>` enum with 30+ variants (single traversal entry point) | High |
| **`ExprKind` variants** | ~12 (no `Match`, no `Closure`, no `Let` expr, no `AddrOf`, no `OffsetOf`) | ~35 variants including `Match`, `Closure`, `Let`, `Yield`, `InlineAsm` | Critical |
| **`ItemKind` variants** | 5: `Function`, `StructDef`, `EnumDef`, `ModDecl`, `UseDecl` | ~25 variants including `ExternCrate`, `Static`, `Const`, `Fn`, `Macro`, `Mod`, `ForeignMod`, `GlobalAsm`, `TyAlias`, `Enum`, `Struct`, `Union`, `Trait`, `TraitAlias`, `Impl` | Critical |
| **Patterns** | N/A — no pattern matching in HIR | Full `Pat`/`PatKind` with bindings, or-patterns, ranges, slices, struct patterns | Critical |
| **Generics** | N/A — types are flat `String` names | `Generics<'hir>`, `GenericParam`, `WhereClause`, `AssocItemConstraint` | Critical |
| **Attributes** | N/A — no attribute system | Full `AttrVec` on every `Item`, `Stmt`, `Expr` | High |
| **Namespaces** | None — `DefKind` is a single flat enum | 4 namespaces: `TypeNS`, `ValueNS`, `MacroNS`, `LifetimeNS`; `Res` carries resolution | High |
| **HIR IDs** | `DefId(u32)` — flat, collapse bug | `HirId { owner: OwnerId, local_id: ItemLocalId }` — two-part, sound | Critical |
| **Derives** | `Debug` only on nearly all types | Full `Serialize/Deserialize`, `Hash`, `Eq`, `Clone` | Showstopper |
| **Debug info** | None in HIR | `SourceInfo` on every `Expr`, `Stmt`, `Terminator` | High |
| **Lowering phases** | 2-pass (signatures, then bodies) + post-pass | Multi-phase: AST → AST lowering → HIR lowering → THIR (typed HIR body) | High |
| **Body representation** | `HirExpr`/`HirStmt` direct recursion | `BodyId` indirection — bodies stored separately, referenced by ID | Medium |
| **Span provenance** | `Range<usize>` byte spans only | `Span` with `SyntaxContext` (hygiene), `LocalDefId` parent | High |

**Key divergence points:**
- Saturnite's `HirType` is 7 flat variants with `String` names (`Struct(String)`, `Enum(String)`). Rust's `Ty` is an arena-allocated, lifetime-parameterized type with 50+ variants (`Uint`, `Int`, `Float`, `DefFnPtr`, `Instance`, `Closure`, `Generator`, `Placeholder`, `BoundVar`, etc.).
- Saturnite has no `Let` statement as an expression. Rust's `ExprKind::Let(&LetStmt, &Expr)` enables `let` chains. Saturnite's `StmtKind::Let` is statement-only.
- Rust's HIR lowering happens inside the query system — `rustc_ast_lowering::hier_ext()` is demand-driven and cached. Saturnite's `lower_program` is a direct function call with no caching.

---

## 5. Identifier System

### Saturnite: 3 flat u32 spaces

| Type | Representation | Purpose | Derives |
|---|---|---|---|
| `SymbolId` | `u32` | Interned string index | `Serialize/Deserialize` ✓ |
| `DefId` | `u32` | Definition index | `Serialize/Deserialize` ✓ |
| `ModuleId` | `u32` | Module identity | (none) |

**`SymbolInterner`:**

```rust
pub struct SymbolInterner {
    strings: Vec<String>,                          // heap-allocated storage
    indices: HashMap<String, SymbolId>,           // RandomState — NON-DETERMINISTIC
}
```

**Critical defects:**
1. **`DefId` namespace collapse (Showstopper #1):** Functions, structs, and enums each assign `DefId(0)` from independent counters. `DefId(0)` is simultaneously a function, a struct, and an enum.
2. **Non-deterministic hashing:** `HashMap` uses `RandomState` — iteration order varies across runs, breaking reproducible builds.
3. **Double allocation:** Each `intern()` call allocates two `String`s.
4. **No `StableHash`/`StableCompare`:** No support for incremental compilation fingerprints.
5. **`DefTable` indexed by `DefId.0`:** The `entries: Vec<DefEntry>` indexed by `def_id.0` is **unsound** — the same `DefId(0)` maps to three different definition kinds.
6. **Missing serde:** `SymbolInterner`, `DefTable`, `DefEntry`, `DefKind`, `Visibility`, `Module`, `ModuleScope`, `ModuleGraph` all lack `Serialize/Deserialize`.

**`DefTable`:**

```rust
pub struct DefTable {
    entries: Vec<DefEntry>,  // indexed by DefId.0 — UNSOUND
}
```

**Why it doesn't crash today:** MIR lowering uses `sigs: HashMap<DefId, ...>` (lookup by equality, not indexing), and `function_name()` uses `find()` (linear scan, not indexing). Any `DefId`-keyed array index or cache would be catastrophically unsound.

### Rust: structured, globally-unique identifiers

| Type | Representation | Purpose | Derives |
|---|---|---|---|
| `Symbol` | `SymbolIndex(u32)` → `DroplessArena` + `HashTable` | Interned string index | `Eq`, `Ord`, `Hash` |
| `DefId` | `{ index: DefIndex, krate: CrateNum }` | Globally unique definition | `StableHash`, `Encodable`, `Decodable` |
| `LocalDefId` | `{ local_def_index: DefIndex }` | Crate-local definition | — |
| `CrateNum` | `NonZero<u32>` | Crate identity | — |
| `DefIndex` | `u32` via `rustc_index::newtype_index!` | Stable index within crate | — |
| `OwnerId` | `{ def_id: LocalDefId }` | Definition context (function body, module) | — |
| `HirId` | `{ owner: OwnerId, local_id: ItemLocalId }` | HIR node identity | — |
| `ItemLocalId` | `NonZero<u32>` | Local node within owner | — |
| `DefPathHash` | `Fingerprint` | Stable hash for dep graph | `StableHash`, `Encodable`, `Decodable` |
| `StableCrateId` | `Hash64` | Stable crate identity | `Hash`, `PartialEq`, `Eq` |

**`Symbol` interner (`compiler/rustc_span/src/symbol.rs`):**

```rust
pub struct Symbol(SymbolIndex);
rustc_index::newtype_index! {
    #[orderable]
    struct SymbolIndex {}
}

pub struct Interner {
    arena: DroplessArena,
    indices: HashTable<(&'static [u8], u32)>,
    byte_strs: Vec<&'static [u8]>,
}
```

Uses `FxBuildHasher` (deterministic). Pre-populated from `symbols!` macro with pre-interned keywords. Thread-local via `SessionGlobals`:

```rust
scoped_tls::scoped_thread_local!(static SESSION_GLOBALS: SessionGlobals);

pub struct SessionGlobals {
    symbol_interner: symbol::Interner,
    span_interner: Lock<span_encoding::SpanInterner>,
    metavar_spans: MetavarSpansMap,
    hygiene_data: Lock<hygiene::HygieneData>,
    source_map: Option<Arc<SourceMap>>,
}
```

**DefPath resolution:**

```rust
pub struct Definitions {
    stable_crate_id: StableCrateId,
    def_id_to_key: IndexVec<LocalDefId, DefKey>,
    def_path_hashes: IndexVec<LocalDefId, Hash64>,
    def_path_hash_to_index: DefPathHashMap,
}

pub struct DefKey {
    pub parent: Option<DefIndex>,
    pub disambiguated_data: DisambiguatedDefPathData,
}

pub struct DefPath {
    pub data: Vec<DisambiguatedDefPathData>,
    pub krate: CrateNum,
}
```

`DefPath` is the human-readable path; `DefPathHash` is the stable hash used in the dep graph. `Definitions::def_path_hash(def_id)` computes it by walking up the parent chain.

### Architectural gap analysis

| Concept | Saturnite | Rust | Gap severity |
|---|---|---|---|
| **`DefId` structure** | Flat `u32` | `{ index: DefIndex, krate: CrateNum }` — crate-qualified | Critical |
| **Uniqueness** | Per-kind counter → collapse (Showstopper #1) | Globally unique within compilation session | Showstopper |
| **Hashing** | `RandomState` — non-deterministic | `FxBuildHasher` — deterministic, reproducible | Showstopper |
| **String interning** | `HashMap<String, SymbolId>` + `Vec<String>` — double allocation | `DroplessArena` + `HashTable` — zero-allocation after first intern | Medium |
| **Thread model** | Explicit passing (`SymbolInterner` passed around) | Thread-local singleton (`SessionGlobals`) | Medium |
| **Stable identity** | None | `DefPathHash` + `StableCrateId` for cross-session identity | Showstopper |
| **HIR-level ID** | None — uses `DefId` directly | `HirId { owner, local_id }` — two-part for efficient HIR traversal | High |
| **Serde support** | `SymbolId`: ✓; `DefId`: ✓; `ModuleId`: ✗; most containers: ✗ | Full `Encodable`/`Decodable` on all ID types | Showstopper |
| **Sentinel values** | None — `DefId(0)` is a real definition | `Local(0)` is INVALID sentinel; `RETURN_PLACE = Local::new(1)`; `START_BLOCK = BasicBlock::new(1)` | Medium |
| **Index typing** | Raw `u32` in newtypes | `rustc_index::newtype_index!` — compile-time type safety | Medium |
| **Namespace awareness** | None — `DefKind` is a flat enum without namespace info | `DefKind::ns()` returns `Namespace` (TypeNS/ValueNS/MacroNS/LifetimeNS) | Critical |

**Key divergence points:**
- Saturnite's `DefId(0)` collision is a **soundness bug**, not a design choice. Rust's `DefId` is structurally incapable of collision within a crate — `index` and `krate` are separate fields, and `DefIndex` is assigned from a single monotonic counter per crate via `IndexVec`.
- Rust's `DefPathHash` enables incremental compilation: if a `DefId`'s source path hasn't changed, its hash matches the previous compilation and cached query results are reused. Saturnite has no equivalent — it cannot cache or fingerprint anything.
- Saturnite passes `SymbolInterner` explicitly to functions. Rust uses `with_session_globals(|sg| ...)` thread-local access. Saturnite's approach is more testable but requires plumbing through every call site.

---

## 6. Module System

### Saturnite: `module.rs` (1516 lines)

| Type | Fields | Derives | Status |
|---|---|---|---|
| `ModuleId` | `u32` | — | ROOT = `ModuleId(0)` |
| `ModulePath` | `{ segments: Vec<SymbolId> }` | — | No serde |
| `Module` | `{ id, path, file_path, ast: Option<Program>, parent, mod_declarations }` | `Debug` only | No serde |
| `ModuleScope` | `{ items: HashMap<SymbolId, DefId>, imports: HashMap<SymbolId, DefId>, parent: Option<ModuleId> }` | `Debug` | No serde (Showstopper M3) |
| `ModuleGraph` | `{ modules, root, symbol_interner, module_index, imports }` | — | No serde (Showstopper M3) |
| `Project` | `{ config, root, source_root, graph }` | — | No serde |

**Discovery algorithm (`discover_modules`, module.rs:497-575):**
1. Create root module from `root_file` with empty `ModulePath`.
2. Read and parse root source text.
3. **AST-based primary path:** `extract_mod_declarations_from_ast(ast, source)` — walks `Program::items` for `ItemKind::ModDecl`.
4. **Text-based fallback:** `extract_mod_declarations(source)` — line-by-line scan for `mod <ident>`.
5. For each child mod name, resolve file via `resolve_module_file`, recursively discover.
6. `add_module()` assigns `ModuleId` sequentially and indexes by path.

**File resolution (`resolve_module_file`, module.rs:590-611):**
1. `<dir>/<name>.stnx` — single file form
2. `<dir>/<name>/mod.stnx` — directory module form

**Project root discovery (`Project::discover`, module.rs:728-798):**
Walks upward from start path looking for `saturn.toml`. First directory containing it is project root. `source_root = <root>/src/`. If no `saturn.toml` found, synthesizes a config from the directory name.

**Showstopper #2 — CLI bypass:** The CLI calls `Project::discover()` and builds the `ModuleGraph`, but then passes only the root module's `Program` to `analyze_and_lower(&program)` (single-file path), NOT `analyze_and_lower_with_graph(&program, &project.graph)` (multi-module path). Child module ASTs are discovered but never lowered to HIR or MIR.

**No cycle detection** in `discover_modules` — circular `mod` imports would cause infinite recursion.

**Test coverage:** `test_module_graph` (41 tests), `test_module_resolution` (3 tests), `test_end_to_end_modules` (2 tests), `test_multi_module_codegen` (3 tests) — total 49 module tests.

### Rust: `rustc_resolve` + HIR map

**AST level (`compiler/rustc_ast/src/ast.rs`):**

```rust
pub struct Crate { pub id: NodeId, pub attrs: AttrVec, pub items: ThinVec<Box<Item>>, ... }
pub struct Item<K> { pub attrs: AttrVec, pub id: NodeId, pub span: Span, pub vis: Visibility, pub kind: K, ... }
pub enum ItemKind { ExternCrate, Use, Static, Const, ConstBlock, Fn, Mod, ForeignMod, GlobalAsm, TyAlias, Enum, Struct, Union, Trait, TraitAlias, Impl, Macro, MacroDef, Delegation, ... }
```

**Resolver (`compiler/rustc_resolve/src/lib.rs`):**
Key types: `Resolver`, `NameResolution`, `BindingTable`, `Module`.

The resolver processes modules by:
1. Building the module tree from `ItemKind::Mod`.
2. Inserting items into the appropriate namespace.
3. Resolving `use` trees to establish aliases.
4. Handling `extern crate` and `extern {}` blocks.

**HIR level (`compiler/rustc_middle/src/hir/map.rs`):**

HIR map queries registered via `rustc_hir::provide()` and accessed on `TyCtxt`:
- `tcx.hir_owner_nodes(def_id)` → `Option<&OwnerNodes>`
- `tcx.hir_owner(def_id)` → `Option<Owner<'hir>>`
- `tcx.hir_node(hir_id)` → `Node<'hir>`
- `tcx.hir_body(body_id)` → `&'hir Body<'hir>` (THIR)
- `tcx.hir_crate_items()` → `ModuleItems`
- `tcx.hir_module_items(mod_id)` → `Option<&ModuleItems>`

```rust
pub struct ModuleItems {
    pub submodules: Vec<LocalDefId>,
    pub free_items: Vec<LocalDefId>,
    pub trait_items: LocalDefIdMap<Vec<ItemTreeInfo>>,
    pub impl_items: LocalDefIdMap<IndexVec<...>>,
    pub foreign_items: Vec<LocalDefId>,
    pub body_owners: Vec<LocalDefId>,
    pub proc_macro_decls: Option<LocalDefId>,
    pub eiis: IndexVec<...>,
}
```

**Resolution queries (`rustc_resolve::provide()`):**
- `resolver_for_lowering`
- `resolution_for_module`
- `def_collections`

**Visibility (`compiler/rustc_hir/src/hir.rs`):**

```rust
pub struct Visibility<'hir> { pub kind: VisibilityKind<'hir>, pub span: Span }
pub enum VisibilityKind<'hir> { Public, Inherited, Ctor(usize), Restricted { path: &'hir Path<'hir>, id: HirId } }
```

**Module file sources (`ModFile`):**

```rust
pub enum ModFile { Path(PathBuf), Parse, File(PathBuf) }
pub struct ModSpans { pub span: Span, pub inline: bool, pub lines: Vec<LineSep>, pub outer: Vec<Path> }
```

### Architectural gap analysis

| Concept | Saturnite | Rust | Gap severity |
|---|---|---|---|
| **Resolution crate** | Single `module.rs` (1516 lines), no separate resolver | `rustc_resolve` — dedicated resolver with `NameResolution`, `BindingTable` | High |
| **AST-level module** | `ItemKind::ModDecl` — simple AST node | `ItemKind::Mod(Ident, ModFile)` + `ModSpans` with line separators, inline flags | Medium |
| **Text-based fallback** | `extract_mod_declarations(source)` — line-by-line scan | None — pure AST-based | Medium |
| **File resolution** | `<dir>/<name>.stnx` or `<dir>/<name>/mod.stnx` | `ModFile::Path`, `ModFile::Parse`, `ModFile::File` — more flexible | Low |
| **Project discovery** | `Project::discover()` — walk up for `saturn.toml` | `rustc_driver` — `--crate-name`, `-L` flags, sysroot | Low |
| **Cycle detection** | **None** — infinite recursion on circular `mod` | Full cycle detection in resolver | Critical |
| **Namespace resolution** | None — flat `DefId` space | 4 namespaces (TypeNS, ValueNS, MacroNS, LifetimeNS) with `Res` | Critical |
| **Use resolution** | `HirUseDecl` stored in `HirProgram.use_decls` | `UseTree` in AST → resolved `Use` in HIR with full path resolution | High |
| **Visibility** | `Visibility { is_public: bool }` — boolean only | `VisibilityKind` with `Public`, `Inherited`, `Ctor(usize)`, `Restricted { path, id }` | High |
| **Macro visibility** | N/A | `pub use` re-exports, `pub(crate)`, `pub(in path)` | N/A |
| **HIR map** | N/A — direct function calls | Query-backed `TyCtxt` methods (`hir_node`, `hir_body`, `hir_owner`) | Showstopper |
| **Demand-driven** | No — always discovers all modules eagerly | `resolver_for_lowering` query is demand-driven | High |
| **Serde support** | `Module`, `ModuleScope`, `ModuleGraph` — all missing serde | Full `Serialize/Deserialize` on module types | Showstopper |
| **CLI integration** | **Bypassed** — `analyze_and_lower(&program)` not `_with_graph` (Showstopper #2) | Full integration — `passes::hier_ext()` lowers all modules | Showstopper |

**Key divergence points:**
- Saturnite discovers modules eagerly (recursively parse all files at startup) but then **discards** the `ModuleGraph` — the CLI calls `analyze_and_lower(&program)` which takes only the root `Program`, not the graph. Rust discovers and resolves lazily via queries — `resolver_for_lowering` is called on demand, and modules are only fully expanded when their items are needed.
- Rust's resolver handles `extern crate` self-import, `pub use` re-exports, glob imports, and macro namespace resolution. Saturnite's resolver is a single-pass `HashMap<SymbolId, DefId>` — it cannot handle any of these.
- Saturnite's `ModuleScope` uses `HashMap<SymbolId, DefId>` for items and imports. Rust's `BindingTable` is a separate type per scope with shadowing support — later bindings shadow earlier ones, and the resolver tracks this through `BindingTableStack`.

---

## 7. Optimization

### Saturnite: constant folding only

| Component | File | Lines | Scope |
|---|---|---|---|
| `ConstantFolder` | `mir/opt.rs` | 163 | Single pass |
| `fold_rvalue()` | `mir/opt.rs` | ~30 | Matches `Binary` and `Unary` rvalues |
| `fold_binop()` | `mir/opt.rs` | ~40 | Matches `MirBinOp` (13 variants) |
| `fold_i64()` | `mir/opt.rs` | ~20 | Wrapping arithmetic; div-by-zero returns `None` |
| `fold_f64()` | `mir/opt.rs` | ~10 | IEEE 754 semantics |
| `fold_bool()` | `mir/opt.rs` | ~10 | Logical operations |

**What is NOT folded:**
- `Unary { op: Not, operand: Const(Bool(true)) }` → not handled (fold_unop exists but not applied to all unary ops)
- `StructLit` with all-const fields → not folded (struct constants not propagated)
- Dead code after `Unreachable` → not eliminated
- Redundant `alloca` + `store` + `load` sequences → not simplified
- Constant expressions in `SwitchInt` branches → not propagated

**No CFG-level optimizations:**
- No block merging, no unreachable block elimination, no critical edge splitting, no jump threading, no edge redirection.

**No data-flow optimizations:**
- No copy propagation, no dead store elimination, no liveness analysis, no available expressions, no reaching definitions.

**Arithmetic model:**
- All integer operations use **wrapping** semantics (`wrapping_add`, `wrapping_sub`, `wrapping_mul`, `wrapping_div`, `wrapping_rem`, `wrapping_shl`, `wrapping_shr`, `wrapping_and`, `wrapping_or`, `wrapping_xor`, `wrapping_shl`, `wrapping_shr`). Overflow does not produce `None` — it wraps silently. Division by zero returns `None`, deferring the panic to runtime.

**Entry point (`optimize`):**

```rust
pub fn optimize(program: &mut MirProgram) {
    for func in &mut program.functions {
        ConstantFolder::run(func);
    }
}
```

**LLVM IR-level optimization:**
- If `opt_level` is non-`None`, Saturnite creates a `TargetMachine` and runs `opt_pass_name()` — a single LLVM IR pass pipeline. The exact pass list is unspecified in the report.

**Test coverage:** 0 direct MIR optimization tests. Only `native_compilation` (63 end-to-end tests) indirectly observe folding.

### Rust: ~40 MIR passes via `MirPass` trait

**Pass trait (`compiler/rustc_mir_transform/src/pass_manager.rs`):**

```rust
pub trait MirPass<'tcx> {
    fn name(&self) -> &'static str;
    fn profiler_name(&self) -> &'static str;
    fn policy(&self, sess: &Session) -> PassPolicy;
    fn run_pass(&self, tcx: TyCtxt<'tcx>, body: &mut Body<'tcx>);
    fn is_mir_dump_enabled(&self) -> bool { true }
}
```

**Policy system:**

```rust
pub enum PassPolicy {
    Required,            // Must always run
    Optional { generally_enabled: bool, optimization: bool },
}
```

- `WithMinOptLevel<T>`: Runs only when `sess.mir_opt_level() >= N`.
- `Lint<T>`: Read-only adapter for `MirLint`.
- `Optimizations`: `Suppressed` (function has `#[optimize(none)]`) or `Allowed`.

**Pass execution (`run_passes_inner`):**
1. Validates pass names against `PASS_NAMES`.
2. Checks `should_run_pass()` — handles `-Zmir-enable-passes` overrides and `#[optimize(none)]`.
3. Applies opt-bisect limiting via `-Z mir-opt-bisect`.
4. Dumps MIR before/after the pass.
5. Runs the pass under the profiler.
6. **Validates MIR after the pass** (`--validate-mir` or `-Z validate-mir`).
7. Lints after the pass (`-Z lint-mir`).

**Full optimization pipeline (4 phases, ~40 passes):**

**Phase 1: Analysis Cleanup Passes (`run_analysis_cleanup_passes`):**
```
ImpossibleClauses, CleanupPostBorrowCk, RemoveNoopLandingPads, SimplifyCfg::PostAnalysis, Derefer
```

**Phase 2: Runtime Lowering Passes (`run_runtime_lowering_passes`):**
```
CriticalCallEdges (AddCallGuards), PostAnalysisNormalize, Subtyper (AddSubtypingProjections),
ElaborateDrops, CheckDropRecursion (Lint), AbortUnwindingCalls, AddMovesForPackedDrops,
EraseDerefTemps, ElaborateBoxDerefs, StateTransform (coroutine lowering), KnownPanicsLint (Lint)
```

**Phase 3: Runtime Cleanup Passes (initial cleanup):**
```
LowerIntrinsics, RemovePlaceMention, SimplifyCfg::PreOptimizations
```

**Phase 4: Analysis-to-Runtime Orchestration:**
```
1. run_analysis_cleanup_passes → AnalysisPhase::PostCleanup
2. (optional) PostDropElaboration: RemoveUninitDrops, SimplifyCfg::RemoveFalseEdges, CheckLiveDrops
3. run_runtime_lowering_passes → RuntimePhase::Initial
4. run_runtime_cleanup_passes → RuntimePhase::PostCleanup
```

**Phase 5: Optimization Passes (`run_optimization_passes`) — ~30+ passes:**
```
// UB checks
CheckAlignment, CheckNull, CheckEnums
// Pre-inline trimming
LowerSliceLenCalls, InstSimplify::BeforeInline, ForceInline, Inline
// Post-inline cleanup
RemoveStorageMarkers, RemoveZsts, RemoveUnneededDrops,
UnreachableEnumBranching, UnreachablePropagation,
SimplifyCfg::AfterUnreachableEnumBranching, MultipleReturnTerminators,
InstSimplify::AfterSimplifyCfg, SimplifyConstCondition::AfterInstSimplify
// Optimizations
ReferencePropagation, Sra (ScalarReplacementOfAggregates),
SimplifyLocals::BeforeConstProp, DeadStoreElimination::Initial, GVN (GlobalValueNumbering),
SimplifyLocals::AfterGVN, SsaRangePropagation, MatchBranchSimplification,
DataflowConstProp, SingleUseConsts, SimplifyConstCondition::AfterConstProp,
JumpThreading, EarlyOtherwiseBranch, SimplifyComparisonIntegral, SimplifyConstCondition::Final
// Post-optimization
RemoveNoopLandingPads, SimplifyCfg::Final, StripDebugInfo, CopyProp,
DeadStoreElimination::Final, DestProp (DestinationPropagation),
SimplifyLocals::Final, MultipleReturnTerminators, EnumSizeOpt,
CriticalCallEdges, ReorderBasicBlocks, ReorderLocals
```

**Notable pass implementations:**
- **Inline** (`inline`): Function inlining based on `#[inline]` attributes and heuristics. Uses call graph (`mir_callgraph_cyclic`) and inliner (`mir_inliner_callees`).
- **GVN** (`gvn`): Global Value Numbering — eliminates redundant computations.
- **SROA** (`sroa`): Scalar Replacement of Aggregates — breaks down aggregates into scalars.
- **DSE** (`dead_store_elimination`): Dead Store Elimination (Initial + Final variants).
- **DestProp** (`dest_prop`): Propagates destinations to eliminate move chains.
- **CopyProp** (`copy_prop`): Copy propagation.
- **JumpThreading**: Threads control flow through known jumps.
- **ReorderBasicBlocks**: Reorders blocks for better instruction cache locality.
- **ReorderLocals**: Reorders locals for compact memory layout.

### Architectural gap analysis

| Concept | Saturnite | Rust | Gap severity |
|---|---|---|---|
| **Pass framework** | Single function `optimize()` — no trait, no registry | `MirPass` trait + `declare_passes!` macro + `PASS_NAMES` validation | Critical |
| **Pass count** | 1 (constant folding) | ~40 passes across 5 pipeline phases | Critical |
| **Pass policy** | No policy system — always runs if `optimize()` is called | `PassPolicy::Required` vs `Optional` with `generally_enabled`/`optimization` flags | High |
| **Opt level gating** | None | `WithMinOptLevel<T>` — passes gated by `sess.mir_opt_level()` | High |
| **MIR validation** | 5 structural checks, runs once after lowering | Full `Validator` runs after *every* pass; ~20 checks + type system integration | High |
| **CFG optimization** | None — blocks in vector order | `SimplifyCfg` (4 variants: PostAnalysis, PreOpt, AfterUnreachableEnumBranching, Final), RPO traversal, dominator tree | Critical |
| **Copy propagation** | None | `CopyProp` pass | High |
| **Dead store elimination** | None | `DeadStoreElimination` (Initial + Final) | High |
| **Global value numbering** | None | `GVN` pass | High |
| **Function inlining** | None | `Inline` pass with call graph analysis + heuristics | Critical |
| **Scalar replacement** | None | `Sra` (SROA) pass | High |
| **Jump threading** | None | `JumpThreading` pass | High |
| **Block reordering** | None | `ReorderBasicBlocks` + `ReorderLocals` | Medium |
| **Dataflow analysis** | None | `DataflowConstProp`, `SsaRangePropagation`, `MatchBranchSimplification` | Critical |
| **Const condition simplification** | None | `SimplifyConstCondition` (3 variants: AfterInstSimplify, AfterConstProp, Final) | High |
| **Inline asm lowering** | N/A — no InlineAsm in MIR | `EarlyInlineAsm`, `LateInlineAsm` passes | N/A |
| **Drop elaboration** | N/A — no destructors | `ElaborateDrops` pass + `CheckDropRecursion` lint | N/A |
| **Coroutine lowering** | N/A — no async | `StateTransform` pass for coroutine state machines | N/A |
| **LLVM IR passes** | Single unspecified pass pipeline | `opt_pass_name()` with configurable pipeline; ThinLTO/SplitLTO support | Medium |
| **Test coverage** | 0 direct tests (only end-to-end) | Extensive per-pass snapshot tests with `--bless` | High |
| **`#[optimize(none)]` support** | None | `Optimizations::Suppressed` — skips all optional passes | High |

**Key divergence points:**
- Saturnite's constant folder treats *all* integer overflow as silent wrapping. Rust's `Const` evaluation traps on overflow in debug mode and uses checked arithmetic — the constant folder never folds an operation that could trap at runtime.
- Rust's `DataflowConstProp` pass propagates known-constant values through the CFG using a full dataflow analysis engine (`rustc_dataflow`). Saturnite's folder only looks at single rvalues in isolation — it cannot propagate a constant from one block to a use in another.
- Rust's `MatchBranchSimplification` pass rewrites match expressions with known discriminant values. Saturnite has no pattern matching in MIR at all — `SwitchInt` is the only branching mechanism.
- Rust validates MIR correctness after *every* pass via the `Validator` pass. Saturnite validates once after lowering and trusts the optimizer to preserve invariants.

---

## 8. Backend

### Saturnite: Inkwell 0.9 + LLVM 21

| Component | File | Lines | Structure |
|---|---|---|---|
| `MirCodeGenContext` | `mir/codegen.rs` | 5 fields | Flat struct |
| `ObjectEmitter` | `codegen/emitter.rs` | 42-43 | Thin wrapper |
| `Linker` | `codegen/linker.rs` | 199-200 | Thin wrapper |
| `TargetConfig` | `target.rs` | 10 fields | Debug only — no Hash, no PartialEq |

**`MirCodeGenContext` (5 fields):**

```rust
struct MirCodeGenContext<'ctx> {
    context: &'ctx LLVMContext,
    module: inkwell::module::Module<'ctx>,
    builder: IRBuilder<'ctx>,
    local_allocas: HashMap<LocalId, AllocaInfo<'ctx>>,  // per-function
}
```

**Local storage:** Always `alloca` for every local. Never uses direct SSA operand. Even a constant `i64` value is stored to an alloca and loaded back.

**Block iteration:** Iterates blocks in **vector order** (not reverse postorder). A missed optimization opportunity documented in the Phase 1 report.

**Type mapping:** `mir_type_to_llvm` — direct `MirType` to LLVM type via `inkwell::types`. No ABI computation, no `FnAbi`, no calling convention handling.

**`generate_function` flow:**
1. Allocate LLVM function via `module.add_function`.
2. Create one LLVM basic block per MIR `MirBasicBlock` (eager, all at once).
3. Allocate an **alloca** for every `MirLocal` (always alloca, never immediate).
4. Store parameters into their allocas.
5. Iterate blocks in vector order, call `gen_stmt()` per statement, `gen_terminator()` per terminator.

**`gen_rvalue` dispatch (7 variants):** `Use`, `Binary`, `Unary`, `StructLit`, `FieldAccess`, `EnumCtor`, `StrLit`.

**`gen_terminator` dispatch (5 variants):** `Goto`, `SwitchInt`, `Call`, `Return`, `Unreachable`.

**`function_name(DefId)`:** O(n) `find()` scan by `DefId` equality, not array indexing.

**Builtins:** `PRINTLN_DEF_ID = DefId(u32::MAX - 1)` — calls `println_i64` from `runtime/println_i64.c` (7 lines of C, compiled to `libsaturnite_runtime.a` via `cc` crate, linked at `Exe` output).

**Linking (`select_linker`):**

| OS | Environment | Linker |
|---|---|---|
| Linux | — | `cc` |
| Darwin | — | `clang` |
| Windows | Msvc | `link.exe` |
| Windows | GNU | `gcc` |
| Other | — | `cc` |

Uses `which::which(linker_name)` to verify linker is on PATH. Pre-flight check via `--version` or `/?`.

**Dependency stack:** `inkwell = 0.9` with `llvm21-1-prefer-dynamic` feature — bundles LLVM 21 as dynamic libraries.

### Rust: `rustc_codegen_ssa` + `rustc_codegen_llvm`

| Component | File | Structure | Notes |
|---|---|---|---|
| `CodegenBackend` trait | `traits/backend.rs` | 15 methods | Pluggable backend interface |
| `WriteBackendMethods` trait | `traits/write.rs` | 12 methods | Backend-specific codegen ops |
| `ExtraBackendMethods` trait | `traits/write.rs` | 4 methods | CGU compilation |
| `BackendTypes` trait | `traits/backend.rs` | 9 associated types | Type-level backend abstraction |
| `FunctionCx` | `mir/mod.rs` | 16 fields | Generic over `Bx: CodegenObject` |
| `ModuleCodegen` | `lib.rs` | 6 fields | Per-CGU codegen state |
| `CompiledModule` | `lib.rs` | 5 fields + `ModuleKind` | Final module output |
| `CrateInfo` | `lib.rs` | 9 fields | Crate metadata for codegen |
| `Linker` | `back/queries.rs` | 7 fields | Full linker with dep graph |
| `LlvmCodegenBackend` | `rustc_codegen_llvm/src/lib.rs` | Struct + impl | LLVM-specific backend |

**Backend traits:**

```rust
pub trait CodegenBackend {
    fn name(&self) -> &'static str;
    fn init(&self, _sess: &Session) {}
    fn target_config(&self, _sess: &Session) -> TargetConfig;
    fn supported_crate_types(&self, _sess: &Session) -> Vec<CrateType>;
    fn codegen_crate<'tcx>(&self, tcx: TyCtxt<'tcx>) -> Box<dyn Any>;
    fn join_codegen(&self, ...) -> (CompiledModules, WorkProductMap);
    fn link(&self, sess: &Session, compiled_modules: CompiledModules, crate_info: CrateInfo, ...);
    // ... 8 more
}
```

**`FunctionCx` (16 fields, generic over `Bx: CodegenObject`):**

```rust
pub struct FunctionCx<'a, 'tcx, Bx: CodegenObject> {
    pub instance: Instance<'tcx>,
    pub mir: &'a Body<'tcx>,
    pub debug_context: Box<DebugInfoBuilder<...>>,
    pub llfn: Bx::Function,
    pub cx: &'a Bx,
    pub fn_abi: &'a FnAbi<'tcx, Ty<'tcx>>,
    pub personality_slot: Option<Local>,
    pub cached_llbbs: IndexVec<BasicBlock, Option<Bx::BasicBlock>>,
    pub cleanup_kinds: IndexVec<BasicBlock, CleanupKind>,
    pub funclets: IndexVec<BasicBlock, Funclet>,
    pub landing_pads: Vec<LandingPad>,
    pub unreachable_block: Bx::BasicBlock,
    pub terminate_blocks: FxHashMap<BasicBlock, ()>,
    pub cold_blocks: BitSet<BasicBlock>,
    pub nop_landing_pads: bool,
    pub locals: LocalMap<'tcx, LocalRef<...>>,
    pub per_local_var_debug_info: Vec<PerLocalVarDebugInfo>,
    pub caller_location: Option<&'a Bx::Value>,
}
```

**`LocalRef` enum (immediate vs. place distinction):**

```rust
pub enum LocalRef<'tcx, T: CodegenObject> {
    /// Direct SSA value — no alloca needed
    Immediate(PlaceValue<T>),
    /// Allocated on stack — needs alloca + load/store
    Place(Alloca<'tcx, T>),
    /// Uninitialized
    Uninit,
    /// Passed by-value from a previous call
    UnsafeSingleSidedPtr { addr: T::Value },
}
```

`PlaceValue` can be either an `Immediate` (direct LLVM value) or a `Deref` (pointer to stack). This distinction lets Rust avoid allocas for constants and simple values.

**Codegen pipeline (`codegen_crate`):**

```rust
pub fn codegen_crate<B: ExtraBackendMethods + WriteBackendMethods>(
    backend: &B, tcx: TyCtxt<'_>,
) -> Box<OngoingCodegen<B>> {
    validate_target_cpu_features(tcx);
    let MonoItemPartitions { codegen_units, .. } = tcx.collect_and_partition_mono_items(());
    for cgu in codegen_units {
        match cgu_reuse {
            CguReuse::No => { backend.compile_codegen_unit(tcx, cgu.name()); }
            CguReuse::PreLto => { submit_pre_lto_module_to_llvm(...); }
            CguReuse::PostLto => { submit_post_lto_module_to_llvm(...); }
        }
    }
    // Parallel CGU compilation via OngoingCodegen join handle
}
```

CGU (Codegen Unit) partitioning — Rust splits the crate into multiple compilation units for parallelism. Saturnite compiles everything as one unit.

**`FnAbi`:** Computed via `rustc_codegen_ssa::abi::get_ffi_fn()` using `rustc_target::abi`. Determines calling convention, parameter passing (by-value vs by-reference), return value handling, stack alignment. Saturnite has no ABI layer — `Call` terminator directly maps MIR operands to LLVM function arguments with no ABI computation.

**Linking (`Linker`):**

```rust
pub struct Linker {
    pub dep_graph: DepGraph,
    pub output_filenames: Arc<OutputFilenames>,
    pub crate_hash: Option<Fingerprint>,
    pub crate_info: CrateInfo,
    pub metadata: Option<EncodedMetadata>,
    pub ongoing_codegen: Box<dyn Any>,
}
```

**LLVM backend dispatch:**
- `link_binary()` — selects linker (cc, lld, etc.) and invokes it.
- `link_natively()` — links with native libraries (rlibs, dylibs, static libs).
- `collect_obj` — for native linking.
- `llvm_add_bitorun_to_passes` — for LLVM-based linking via `lld`.

**ThinLTO / FatLTO:** `optimize_and_codegen_fat_lto()` and `run_thin_lto()` for cross-module optimization. Saturnite has no LTO.

**Debug info:** `FunctionCx::debug_context` is a `Box<DebugInfoBuilder<...>>` with full DWARF debug info for every local, scope, and source position. Saturnite generates **none**.

### Architectural gap analysis

| Concept | Saturnite | Rust | Gap severity |
|---|---|---|---|
| **Backend trait** | None — direct inkwell calls in `codegen.rs` | `CodegenBackend` + `WriteBackendMethods` + `ExtraBackendMethods` traits — pluggable | High |
| **Codegen unit model** | Single unit — entire `MirProgram` per invocation | CGU (Codegen Unit) partitioning — `collect_and_partition_mono_items` splits crate for parallelism | High |
| **Local storage** | Always `alloca` for every local | `LocalRef` enum: `Immediate` (direct SSA) vs. `Place` (alloca) vs. `Uninit` | High |
| **Block ordering** | Vector order (as written in MIR) | Reverse postorder (RPO) with `BasicBlocks::cache().reverse_postorder` | High |
| **Function context** | `MirCodeGenContext` (5 fields, flat, module-level) | `FunctionCx` (16 fields, generic, per-function) | High |
| **ABI handling** | None — direct operand-to-LLVM mapping | `FnAbi` via `rustc_target::abi` — calling convention, param passing, alignment | Critical |
| **Calling convention** | Default LLVM C calling convention | `rustc_abi::FnAbi` with `Conv` enum (C, System, Rust, Cold, Preserve, …) | Critical |
| **Debug info** | None | Full `DebugInfoBuilder` with `VarDebugInfo`, `SourceScope`, DWARF | Critical |
| **Cleanup/funclet** | None | `funclets: IndexVec<BasicBlock, Funclet>` for Windows SEH | N/A |
| **Unwinding** | None — `Unreachable` is the only cleanup path | `landing_pads: Vec<LandingPad>`, `unwind_block` tracking, `UnwindAction` per terminator | Critical |
| **LTO** | None | FatLTO (`optimize_and_codegen_fat_lto`) and ThinLTO (`run_thin_lto`) | High |
| **Parallel codegen** | None — sequential function iteration | `OngoingCodegen` with async join handle, `submit_codegened_module_to_llvm` for concurrent CGUs | High |
| **Linker abstraction** | `Linker` (2 fields) — thin wrapper, system linker only | `Linker` (7 fields) with `DepGraph`, `WorkProductMap`, `EncodedMetadata` — full link management | High |
| **Linker selection** | `(OS, Environment)` → 4 hardcoded linkers | `linker::select_native_linker()` with target spec `linker_flavor` (46 variants) | Medium |
| **Link-time optimization linking** | Uses `which::which` pre-flight check | `llvm_add_bitorun_to_passes` for `lld` integration | Medium |
| **Dependency stack** | `inkwell 0.9` (LLVM 21 dynamic) | `rustc_codegen_llvm` (in-tree FFI bindings to LLVM) | Medium |
| **`#[optimize(none)]`** | N/A — always runs constant folding | `Optimizations::Suppressed` — skips all optional passes in MIR + LLVM | High |
| **Cross-module inlining** | None | ThinLTO imports, `Inline` pass with call graph | High |
| **Monomorphization** | None — generics not supported | `collect_and_partition_mono_items` + `Instance` system | N/A |

**Key divergence points:**
- Saturnite's `MirCodeGenContext` is a flat struct holding a module-level `IRBuilder`. Rust's `FunctionCx` is generic over `Bx: CodegenObject` (the `BuilderMethods` trait) — the same code can target LLVM, Cranelift, or any future backend. Saturnite is hardwired to inkwell/LLVM.
- Saturnite's `function_name(DefId)` does an O(n) linear scan. Rust's `Instance` system resolves to a `CrateNum` + `DefPathHash` lookup into a pre-compiled symbol table — O(1) amortized.
- Saturnite's `Unreachable` terminator creates a new `unreachable` block and branches to it. Rust's `TerminatorKind::Unreachable` emits a direct `unreachable` LLVM instruction. More importantly, Rust's `UnwindAction` enum (4 variants: `Cleanup`, `Continue`, `Unreachable`, `Terminate`) on every terminator enables structured exception handling — Saturnite has no unwinding model at all.
- Rust's `Linker` carries `dep_graph: DepGraph` — linking decisions are fingerprinted and cached. Saturnite's `Linker` has no dep graph — it always links from scratch.

---

## Appendix: Critical Showstoppers Recap

| ID | Title | Saturnite location | Architectural domain |
|---|---|---|---|
| #1 | DefId namespace collapse | `hir/lower.rs` — per-kind counters all start at `DefId(0)` | Identifier system |
| #2 | CLI bypass of module system | `main.rs:255` calls `analyze_and_lower` instead of `_with_graph` | Pipeline / Module system |
| #3 | No serialization on 15+ types | `hir/symbol.rs`, `hir/function.rs`, `hir/expr.rs`, `hir/stmt.rs`, `mir/mod.rs`, `module.rs`, `target.rs` | All layers (caching) |
| #4 | Missing Apache-2.0 license file | Repository root | Provenance / licensing |

These four showstoppers are independent and must be resolved before Saturnite can support incremental compilation, multi-module programs, or sound identifier resolution — all of which Rust has had since 1.0.

---

*End of comparison. Compiled from `SATURNITE_ACTUAL_ARCHITECTURE.md` (Phase 1) and `RUST_ACTUAL_ARCHITECTURE.md` (Phase 2). No source files were read directly during this comparison.*
