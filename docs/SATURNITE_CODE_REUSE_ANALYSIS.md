# Saturnite Code Reuse Analysis

## Forensic Comparison: Saturnite vs. Rust Compiler

**Date:** 2026-08-27  
**Purpose:** Line-by-line forensic analysis of Saturnite source code (`C:\Users\atimo\Saturnite\crates\stnx\src`) against the Rust compiler source (`C:\Users\atimo\rust\compiler`) to determine which Rust compiler patterns could be reused, adapted, ported, fused, or rejected for direct adoption.

## Methodology

Each Rust compiler pattern was examined against its Saturnite counterpart (if one exists). Patterns were categorized as:

- **KEEP** — Saturnite's existing implementation is aligned with the Rust approach; no change needed.
- **REIMPLEMENT** — Saturnite has no analogous code; rewrite from scratch following the Rust pattern's design principles.
- **ADAPT/PORT** — Saturnite has a partial or divergent implementation that can be conformed to Rust's design with modification.
- **FUSE** — Two or more Saturnite patterns should be merged into a single Rust-aligned design.
- **REJECT** — Direct adoption would be inappropriate due to licensing, complexity, or architectural mismatch.

---

## 1. MIR Structures

### Rust Reference: `rustc_middle/src/mir/syntax.rs`

The Rust MIR syntax module defines the core IR vocabulary:

- **`Rvalue`** (~25 variants): `Use`, `Binary`, `Unary`, `Aggregate`, `Call`, `Cast`, `Shallow`, `CopyForSurvival`, `AddressOf`, `Len`, `AscribeUserType`, `UnwindTerminate`, `Discard`, `Opaque`, `InlineAsm`, `SetDiscriminant`, `Subdiagnostic`, `ResetDropFlag`, `Intrinsic`, `ConstEvalError`, `Dead`, `MaybeUB`, `PtrMetadata`, `Offset`, `Exhaustion`, `Delayed`, `Unevaluated`.
- **`TerminatorKind`** (~15 variants): `Goto`, `Call`, `SwitchInt`, `Return`, `Suspend`, `Abort`, `Yield`, `GeneratorDrop`, `Cleanup`, `UnwindTerminate`, `Unreachable`, `InlineAsm`, `AscribeUserType`, `Drop`, `Opaque`.
- **`StatementKind`** (~15 variants): `Assign`, `SetProgramDate`, `Nop`, `StorageLive`, `StorageDead`, `Deallocate`, `Put`, `Intrinsic`, `ConstEvalError`, `AscribeUserType`, `Fake`, `Coverage`, `Opaque`, `InlineAsm`, `Move`, `Drop`, `ResetDropFlag`.
- **`Operand`** (4 variants): `Copy(Place)`, `Move(Place)`, `Indirect`, `Constant(Box<ConstOperand>)`.
- **`Place`**: `(Local, &[ProjectionElem])`.
- **`ProjectionElem`**: `Deref`, `Field`, `Index`, `Downcast`, `OpaqueCast`, `Subtype`, `Pointer`, `UnwrapUnsafe`, `UnwrapAligned`.
- **`BinOp`** (13 variants): `Add`, `Sub`, `Mul`, `Div`, `Rem`, `Ge`, `Le`, `Ne`, `Eq`, `Lt`, `Gt`, `Or`, `And`.
- **`UnOp`** (3 variants): `Neg`, `Not`.
- **`ConstValue`**: `Scalar`, `ZeroSized`, `Slice`, `Indirect`.

### Saturnite Counterpart: `mir/mod.rs`

- **`MirRvalue`** (7 variants): `Use(MirOperand)`, `Binary { op, lhs, rhs }`, `Unary { op, operand }`, `StructLit { struct_def, fields }`, `FieldAccess { local, field }`, `EnumCtor { enum_def, variant }`, `StrLit(SymbolId)`.
- **`MirTerminator`** (5 variants): `Goto { target }`, `SwitchInt { scrutinee, ty, branches, else_target }`, `Call { func, args, destination, next }`, `Return(Option<MirOperand>)`, `Unreachable`.
- **`MirStmtKind`** (2 variants): `LocalDecl { local, ty, mutable }`, `Assign { local, rvalue }`.
- **`MirOperand`** (2 variants): `Const(MirConst)`, `Local(LocalId)`.
- **No `Place` or `ProjectionElem`**: locals are flat `LocalId`s; no place projection.
- **`MirBinOp`** (13 variants) and **`MirUnOp`** (2 variants) — nearly identical to Rust.
- **`MirConst`**: `I64`, `F64`, `Bool` — subset of `ConstValue`.

#### Categorization: **ADAPT/PORT**

**Rationale:** Saturnite's MIR type vocabulary was designed for a simpler language subset (no generics, no closures, no coroutines, no inline asm, no drop elaboration). The Rust `Rvalue` variants that handle ownership semantics (`AddressOf`, `Len`, `Aggregate`, `Cast`, `Shallow`, `CopyForSurvival`), unwinding (`UnwindTerminate`), and compiler internals (`Subdiagnostic`, `Offset`, `Exhaustion`, `Delayed`, `Unevaluated`, `PtrMetadata`, `InlineAsm`) have no Saturnite equivalent. However, the **core architectural pattern** of MIR as an enum with a small set of value-producing and control-flow variants is sound and aligned.

**Recommendation:** Saturnite should adopt Rust's pattern of using `IndexVec` for `Local`/`BasicBlock` typed indices rather than raw `u32` newtypes (`LocalId`, `BlockId`). The Rust `Place`+`ProjectionElem` model is the one significant architectural divergence: Saturnite currently flattens struct fields by emitting `FieldAccess` as an Rvalue rather than modeling places with projections. For future support of mutable field assignment (`p.x = 5`), Saturnite should adapt the Rust `Place` + `ProjectionElem` design.

### Key structural differences:

| Rust MIR | Saturnite MIR | Action |
|---|---|---|
| `Local` (typed index via `newtype_index!`) | `LocalId(u32)` newtype | **ADAPT**: Replace `u32` with `rustc_index::newtype_index!` |
| `BasicBlock` (typed index) | `BlockId(u32)` newtype | **ADAPT**: Same as above |
| `Operand::Copy(Place)` / `Operand::Move(Place)` | `MirOperand::Const(MirConst)` / `MirOperand::Local(LocalId)` | **KEEP**: Saturnite's simplification is correct for a value-semantics language |
| `Place` = `(Local, ProjectionElem[])` | No concept; field access is an Rvalue | **ADAPT**: Introduce `Place` + `ProjectionElem` for future mutable field writes |
| `StatementKind::StorageLive/StorageDead` | Absent | **REIMPLEMENT**: No allocation model for locals in Saturnite; if memory-based storage is added, port these concepts |

---

## 2. Symbol Interning

### Rust Reference: `rustc_span/src/symbol.rs`

Rust's symbol interner architecture:

- **`Symbol(SymbolIndex)`**: A thin wrapper around a `newtype_index!`-generated `SymbolIndex` type. `Symbol` is `Copy`, `Clone`, `PartialEq`, `Eq`, `PartialOrd`, `Ord`, `Hash`.
- **`Interner`**: Backed by `InternerInner` which contains:
  - `arena: DroplessArena` — bump-allocates strings to `&'static [u8]`
  - `indices: HashTable<(&'static [u8], u32)>` — hash table for deduplication
  - `byte_strs: Vec<&'static [u8]>` — reverse mapping
- Uses `FxBuildHasher` (deterministic, fast, non-cryptographic).
- **`StableHash`** and **`StableCompare`** impls on `Symbol` for incremental compilation.
- `Symbol::intern` and `Symbol::as_str` are the public API.
- The interner is a **thread-local singleton** stored in `SessionGlobals`.

### Saturnite Counterpart: `hir/symbol.rs`

- **`SymbolId(pub u32)`**: Simple `u32` newtype. Implements `Serialize`/`Deserialize` (Rust `Symbol` does too via `Encodable`/`Decodable`).
- **`SymbolInterner`**: 
  - `strings: Vec<String>` — owned `String` storage
  - `indices: HashMap<String, SymbolId>` — lookup table using `std::collections::HashMap` (which uses `RandomState`, a randomized hasher)
- No `StableHash`/`StableCompare` implementation.
- No arena — strings are heap-allocated `String`s.
- No threading model; the interner is passed explicitly as `&mut SymbolInterner` or `&SymbolInterner`.

#### Categorization: **ADAPT/PORT**

**Rationale:** Saturnite's interner works correctly but diverges from Rust's design in several important ways:

1. **Hashed storage**: Rust uses `FxBuildHasher` (deterministic, fast); Saturnite uses `std::collections::HashMap<String, SymbolId>` with `RandomState` (non-deterministic across runs). This is a **correctness and performance issue** for incremental compilation and reproducible builds.
2. **Memory layout**: Rust uses a `DroplessArena` to bump-allocate `&'static [u8]` slices, avoiding per-string heap allocation. Saturnite allocates heap `String`s for every interned string. The arena approach is significantly more cache-friendly.
3. **Determinism**: Rust's `StableHash` and `StableCompare` on `Symbol` ensure that symbol hashes are deterministic across compilation sessions, which is essential for incremental compilation and build cache reuse. Saturnite has no such mechanism.
4. **Threading**: Rust uses a thread-local singleton (`SessionGlobals`); Saturnite passes the interner explicitly, which is actually **simpler** for a single-threaded or fork-based architecture.

**Recommendation:** 
- Replace `RandomState` with `rustc_hash::FxBuildHasher` (available in the Rust compiler's `rustc_data_structures` crate, or as the standalone `rustc-hash` crate). This is a trivial, high-value change.
- Replace `Vec<String>` with a `DroplessArena`-backed `Vec<&'static [u8]>` if the `rustc_arena` crate is accessible. If not, at minimum switch from `String` to `Box<str>` to reduce over-allocation.
- Add `StableHash`/`StableCompare` implementations if incremental compilation is planned.
- The explicit-passing model is fine and arguably better than the thread-local singleton for testability.

### Code comparison (conceptual):

**Rust interner insertion (conceptual):**
```rust
fn intern_str(&self, str: &str) -> Symbol {
    Symbol::new(self.intern_inner(str.as_bytes()))
}

fn intern_inner(&self, byte_str: &[u8]) -> u32 {
    // HashTable entry lookup with FxBuildHasher
    // Arena allocation for byte slice
}
```

**Saturnite interner insertion:**
```rust
fn intern(&mut self, s: &str) -> SymbolId {
    if let Some(&id) = self.indices.get(s) { return id; }
    let id = SymbolId(self.strings.len() as u32);
    self.indices.insert(s.to_string(), id);  // heap allocation
    self.strings.push(s.to_string());        // another heap allocation
    id
}
```

Saturnite allocates **two** heap `String`s per intern call. Rust allocates zero (arena-allocated `&'static [u8]`).

---

## 3. MIR Verification

### Rust Reference: `rustc_mir_transform/src/validate.rs`

Rust's MIR validator (`Validator` struct, implementing `MirPass<'tcx>`):

- Runs as a `MirPass` in the optimization pipeline (declared in `lib.rs` via `declare_passes!`).
- Checks a wide range of structural invariants:
  - Basic block integrity (terminator exists, is well-formed)
  - Local declaration validity (types are well-formed)
  - Operand validity (no dangling locals, constants have correct types)
  - Assignment target validity (places are well-formed)
  - Call argument count matching
  - Control-flow graph integrity (all targets exist)
  - Type checking of MIR (uses `TyCtxt` and the type system)
  - Drop flag integrity (for languages with destructors)
  - Borrow check compatibility (in analysis-phase MIR)
  - `UnsafeCell` wrapping rules
- Uses `MutatingUseContext`, `PlaceContext`, and the MIR visitor infrastructure from `rustc_middle::mir::visit`.
- Integrates deeply with `rustc_middle::ty::TyCtxt` for type-level validation.

### Saturnite Counterpart: `mir/verify.rs`

- **`MirVerifyError`**: Simple struct with `message: String` and `location: Option<(String, String)>`.
- **`VerifyResult`**: `Result<(), Vec<MirVerifyError>>`.
- **`MirProgram::verify()`**: Iterates over functions, calls `verify_function`.
- **`verify_function`** checks 5 things:
  1. Every block has a real terminator (not `Unreachable` placeholder).
  2. All terminator target blocks exist (`check_terminator_blocks`).
  3. `LocalId` references in operands are valid (`check_local_refs`).
  4. Parameters exist as locals.
  5. `start_block` is valid.
- Uses `HashSet` for collection (no `IndexVec`-based dense bitsets).
- Returns structured errors rather than panicking.

#### Categorization: **KEEP** (with **REIMPLEMENT** for specific gaps)

**Rationale:** Saturnite's verifier covers the fundamental structural invariants that matter for a value-semantics language without destructors, borrow checking, or complex type systems. The 5 checks it performs map directly to the **core subset** of Rust's validator:

- Check 1 (terminator existence) maps to Rust's terminator validity.
- Check 2 (block target existence) maps to Rust's CFG integrity check.
- Check 3 (operand local validity) maps to Rust's operand validity check.
- Check 4 (parameter locals) is specific to Saturnite's layout.
- Check 5 (start block validity) maps to Rust's basic block integrity.

The verifier's design — structured errors returned as a `Vec`, with location metadata, converted to `CompilerError` — is sound and aligned with the Rust compiler's principle of returning recoverable errors rather than panicking.

**What's missing (REIMPLEMENT):**
- Type checking of MIR assignments (verifying Rvalue produces the type declared in the target LocalDecl).
- Operand type consistency (e.g., `MirBinOp::Add` requires matching types).
- Rvalue well-formedness (no `StrLit` where the local is `I64`).
- Integration with a type system via `TyCtxt` — Saturnite doesn't have one.
- Drop flag / destructor validation (not applicable).
- Borrow check compatibility (not applicable — Saturnite has no borrows).

**Recommendation:** Saturnite's current approach is correct for its scope. The verifier should be extended (not ported) with **type-level checks** that are specific to `HirType`: verifying that every `Assign` writes the correct type into the `MirLocal`, that binary operations don't mix incompatible types, and that `EnumCtor` variants exist in the enum definition. These are simpler versions of Rust's checks, adapted to `HirType`'s flat type system.

---

## 4. MIR Optimization

### Rust Reference: `rustc_mir_transform/src/`

Rust's MIR optimization infrastructure:

- **`MirPass<'tcx>` trait**: The base trait for all MIR passes. Defines `name()`, `run_pass(&self, tcx: TyCtxt<'tcx>, body: &mut Body<'tcx>)`, and `qualified()`.
- **`PassPolicy`**: Enum (`Required`, `Optional`) controls whether a pass can be disabled.
- **Pass manager** (`pass_manager.rs`): A macro-based system that registers passes via `declare_passes!`. Each pass is a struct implementing `MirPass`.
- **Optimization pipeline**: ~40 passes organized into phases (BeforeBorrowck, AfterBorrowck, Pre-Runtime, Post-Runtime, etc.).
- Key passes relevant for port/adapt:
  - **`SimplifyCfg`** (`simplify.rs`): CFG simplification — removes no-op blocks, merges chains of `Goto`-only blocks, collapses redundant branches. Uses `CfgSimplifier` struct with `pred_count` analysis.
  - **`SimplifyLocals`** (`simplify.rs`): Removes unused local declarations. Uses `UsedLocals` visitor and `LocalUpdater` (MIR visitor that remaps `Local` indices).
  - **`CopyProp`** (`copy_prop.rs`): Copy propagation — unifies locals that copy each other, then replaces uses.
  - **`DeadStoreElimination`** (`dead_store_elimination.rs`): Eliminates stores that are never read. Uses `MaybeTransitiveLiveLocals` dataflow analysis from `rustc_mir_dataflow`.
  - **`UnreachablePropagation`** (`unreachable_prop.rs`): Propagates `Unreachable` terminators through the CFG.
  - **`InstCombine`** / **`InstSimplify`** (`instsimplify.rs`): Algebraic simplification of MIR rvalues.
  - **`GVN`** (`gvn.rs`): Global value numbering.
  - **`BranchS impl ier`** / **`SimplifyBranches`** (`simplify_branches.rs`/`match_branches.rs`): Simplifies match and branch structures.
  - **`PromoteConsts`** (`promote_consts.rs`): Promotes compile-time-evaluable constants to `&'static`.
  - **`SROA`** (`sroa.rs`): Scalar replacement of aggregates.
  - **`RemoveNoopLandingPads`** (`remove_noop_landing_pads.rs`): Removes exception-handling landing pads that are no-ops.
  - **`DerefSeparator`** (`deref_separator.rs`): Separates `Deref` projections.
  - **`EarlyOtherwiseBranch`** (`early_otherwise_branch.rs`): Early-exit pattern matching.
  - **`Cleanup`**, **`AbortUnwindingCalls`**, **`ElaborateDrops`**, etc.
- **`MirPass` trait** requires `TyCtxt` context, meaning all passes have full type-system access.
- Dataflow analysis uses `rustc_mir_dataflow::Analysis` trait and `ResultsCursor` for fixpoint iteration.

### Saturnite Counterpart: `mir/opt.rs`

- **`optimize(program: &mut MirProgram)`**: Entry point that iterates over functions and runs `ConstantFolder::run(func)` on each.
- **`ConstantFolder`**: A single pass that:
  - Iterates over all blocks and statements.
  - For `MirStmtKind::Assign`, calls `fold_rvalue`.
  - `fold_rvalue` attempts to fold `Binary` and `Unary` rvalues if both/all operands are `MirConst`.
  - Calls `MirConst::fold_binop` / `MirConst::fold_unop`.
  - `fold_i64`, `fold_f64`, `fold_bool` implement the constant folding logic.
- **No pass infrastructure**: No `MirPass` trait, no `PassPolicy`, no pass manager.
- **No dataflow analysis**: Uses no fixpoint iteration, no live-variable analysis.
- **No CFG transformation**: No block merging, no dead block elimination, no jump threading.
- Uses `wrapping_*` arithmetic for overflow (matching Rust's semantics for release builds, but without distinguishing debug vs. release).
- Division by zero is deferred to runtime (returns `None` from the folder).

#### Categorization: **ADAPT/PORT**

**Rationale:** Saturnite has a **functional but minimal** constant folder that is **architecturally aligned** with Rust's approach (operate on MIR before LLVM sees it, fold type-aware operations). However:

1. **No pass manager infrastructure**: Rust's `MirPass` trait + `PassPolicy` + `declare_passes!` macro provides a uniform interface for composing, ordering, and parametrizing passes. Saturnite has a single hardcoded call. **ADAPT**: Introduce a simplified `MirPass` trait that doesn't require `TyCtxt` (Saturnite has no type context at this stage).
2. **CFG simplification is missing entirely**: Saturnite's codegen walks blocks in vector order and generates LLVM blocks for each. It does NOT perform dead block elimination, block merging, or jump threading. Rust's `SimplifyCfg` pass is the single most impactful MIR optimization for code quality. **REIMPLEMENT**: Port a simplified version of `SimplifyCfg` that handles the Saturnite terminator set (`Goto`, `SwitchInt`, `Call`, `Return`).
3. **Copy propagation is missing**: Saturnite allocates an LLVM alloca for every local. Rust's `CopyProp` pass reduces alloca count by unifying copy-related locals. **REIMPLEMENT** a simplified version after `SimplifyLocals`.
4. **Dead store elimination is missing**: Saturnite generates a store for every `Assign`, even if the value is never read. **REIMPLEMENT** using a simple liveness analysis (no need for the full `MaybeTransitiveLiveLocals` dataflow — a simple backward-live-variables pass suffices).
5. **Constant folder type-safety**: The current folder uses `wrapping_*` arithmetic unconditionally. Rust distinguishes between debug (overflow-checking) and release (wrapping) semantics. **ADAPT**: Add a configuration flag for overflow checking.

**Recommendation (ordered roadmap):**

1. **Phase 1 (ADAPT):** Refactor `ConstantFolder` into a `MirPass` trait with `run(func: &mut MirFunction) -> bool` (no `TyCtxt` needed). Create a `MirPassManager` struct that holds a `Vec<Box<dyn MirPass>>` and runs them in sequence.
2. **Phase 2 (REIMPLEMENT):** Port `SimplifyCfg` — a significantly simplified version that handles `Goto` chain collapsing, trivial block merging, and unreachable block removal. Use `HashSet<BlockId>` for validity (already done in verify.rs).
3. **Phase 3 (REIMPLEMENT):** Add `SimplifyLocals` — remove locals that are declared but never read (simple use-count analysis).
4. **Phase 4 (REIMPLEMENT):** Add `CopyProp` — propagate simple copies (`_a = _b; _c = _a`) to eliminate redundant locals.
5. **Phase 5 (REIMPLEMENT):** Add dead store elimination — backward liveness analysis to remove stores to never-read locals.

The Rust `rustc_mir_dataflow` crate is overkill for Saturnite. A simple hand-rolled liveness analysis over the SAT graph (each `MirBasicBlock` is a node, terminator successors are edges) will suffice.

---

## 5. LLVM Code Generation

### Rust Reference: `rustc_codegen_ssa/src/mir/mod.rs`

Rust's codegen is centered on the `FunctionCx` struct:

- **`FunctionCx<'a, 'tcx, Bx: BuilderMethods<'a, 'tcx>>`**: Master context for codegenning a single MIR function. Fields:
  - `instance: Instance<'tcx>` — monomorphized function identity
  - `mir: &'tcx mir::Body<'tcx>` — the MIR body being compiled
  - `debug_context: Option<FunctionDebugContext<...>>` — debug info builder
  - `llfn: Bx::Function` — the LLVM function being built
  - `cx: &'a Bx::CodegenCx` — shared codegen context
  - `fn_abi: &'tcx FnAbi<'tcx, Ty<'tcx>>` — calling convention / ABI info
  - `personality_slot: Option<PlaceRef<...>>` — exception personality storage
  - `cached_llbbs: IndexVec<BasicBlock, CachedLlbb<Bx::BasicBlock>>` — lazy LLVM block creation
  - `cleanup_kinds: Option<IndexVec<...>>` — EH funclet cleanup info
  - `funclets: IndexVec<BasicBlock, Option<Bx::Funclet>>` — MSVC exception handling
  - `landing_pads: IndexVec<BasicBlock, Option<Bx::BasicBlock>>` — EH landing pads
  - `unreachable_block: Option<Bx::BasicBlock>` — cached unreachable block
  - `terminate_blocks: IndexVec<BasicBlock, Option<...>>` — unwinding terminate blocks
  - `cold_blocks: IndexVec<BasicBlock, bool>` — cold block annotations
  - `locals: locals::Locals<'tcx, Bx::Value>` — per-local storage decision (alloca vs. direct operand)
  - `per_local_var_debug_info: Option<...>` — per-local debug info
  - `nop_landing_pads: DenseBitSet<BasicBlock>` — optimized-away landing pads
  - `caller_location: Option<OperandRef<...>>` — `#[track_caller]` support
- **`codegen_mir` function**: Orchestrates the full function codegen:
  - Creates the LLVM function via `cx.get_fn(instance)`.
  - Determines `fn_abi` via `cx.fn_abi_of_instance`.
  - Analyzes reachability (`traversal::mono_reachable`).
  - Sets personality function if needed.
  - Computes cleanup kinds.
  - Allocates `cached_llbbs`.
  - Creates `FunctionCx` struct.
  - Allocates local storage (alloca for non-immediate types, direct operand for immediate types).
  - Iterates blocks in reverse-postorder for codegen.
  - Emits unreachable blocks as `unreachable` instructions.
- **`LocalRef<'tcx, V>` enum**: `Place(PlaceRef)`, `UnsizedPlace(PlaceRef)`, `Operand(OperandRef)`, `PendingOperand` — tracks how each local is represented in LLVM.
- **`CachedLlbb<T>` enum**: `None` / `Some(T)` / `Skip` — lazy block creation with skip semantics.
- Sub-modules: `analyze` (liveness, cleanup kinds, non-SSA locals), `block` (basic block codegen), `constant` (constant value materialization), `locals` (local allocation), `operand` (`OperandRef`/`OperandValue`), `place` (`PlaceRef`), `retag` (pointer retagging), `rvalue` (Rvalue→LLVM), `statement` (Statement→LLVM), `debuginfo`, `intrinsic`, `naked_asm`.

### Saturnite Counterpart: `mir/codegen.rs`

- **`MirCodeGenContext<'ctx>`**: Backend struct holding:
  - `context: &'ctx LLVMContext`
  - `module: inkwell::module::Module<'ctx>`
  - `builder: IRBuilder<'ctx>`
  - `local_allocas: HashMap<LocalId, AllocaInfo<'ctx>>` — per-function local alloca cache
- **`AllocaInfo<'ctx>`**: `(PointerValue<'ctx>, BasicTypeEnum<'ctx>)` — alloca pointer + LLVM type.
- **`generate_function`**: Codegens a single MIR function:
  - Looks up or declares the LLVM function via `module.get_function` / `add_function`.
  - Creates LLVM basic blocks for each MIR block (one per `MirBasicBlock`).
  - Allocates an LLVM alloca for every local (always `alloca`, never direct operand).
  - Stores parameters into their allocas.
  - Iterates blocks in vector order (NOT reverse postorder).
  - Generates statements and terminators via `gen_stmt` and `gen_terminator`.
- **`gen_stmt`**: Handles `LocalDecl` (no-op, alloca already created), `Assign` (gen rvalue, store into local alloca).
- **`gen_rvalue`**: Matches on `MirRvalue` variants:
  - `Use`, `Binary`, `Unary` — materialize operand(s), call `gen_binop`/`gen_unop`.
  - `StructLit` — build LLVM struct, insert values, alloca + store.
  - `FieldAccess` — load struct from local, extract field by index.
  - `EnumCtor` — look up variant index in enum definition, emit as `i64` constant.
  - `StrLit` — emit as LLVM string constant.
- **`materialize_operand`**: `Const(I64/F64/Bool)` → LLVM constant; `Local(lid)` → load from alloca.
- **`gen_binop`/`gen_unop`**: Match on `MirBinOp`/`MirUnOp`, dispatch to `inkwell` builder methods (`build_int_add`, `build_int_compare`, `build_float_add`, etc.).
- **`gen_terminator`**: Handles `Goto` (unconditional branch), `SwitchInt` (switch), `Call` (function call), `Return` (return instruction), `Unreachable` (unreachable instruction).
- **`compile_from_mir_ext`**: Entry point that creates `MirCodeGenContext`, declares builtins, declares functions, generates each function's IR, then runs `PassBuilderOptions` with the configured optimization level.

#### Categorization: **ADAPT/PORT**

**Rationale:** Saturnite's `MirCodeGenContext` is the direct analog of Rust's `FunctionCx` + `CodegenCx` pair. The architectural pattern is aligned: a backend context holds the LLVM module and builder, and per-function state tracks local→LLVM mappings. However, several Rust design patterns are superior and should be adapted:

1. **Per-local storage decisions (Immediate vs. Place)**: Rust's `LocalRef` enum and `locals::Locals` track whether a local is "immediate" (stored directly as an LLVM value) or requires an alloca. Saturnite allocates **every** local as an `alloca`, which means every `Local` access is a load/store pair. **ADAPT**: Introduce a simple analysis to determine if a local is never borrowed/address-taken, and if so, keep it as a direct LLVM SSA value instead of an alloca. This is Saturnite's single biggest codegen performance win.

2. **Reverse-postorder block iteration**: Rust iterates blocks in reverse postorder (`traversal::mono_reachable_reverse_postorder`) for better LLVM optimization. Saturnite iterates in vector order. **ADAPT**: Implement a simple RPO traversal (DFS-based) and iterate blocks in that order.

3. **Lazy basic block creation**: Rust uses `CachedLlbb` enum (`None`/`Some`/`Skip`) to lazily create LLVM blocks only when reached. Saturnite eagerly creates all blocks. This is acceptable for Saturnite's simpler CFG model but **ADAPT** for better compile times.

4. **Calling convention / ABI handling**: Rust uses `FnAbi` for platform-specific ABI mapping (struct return, argument passing, etc.). Saturnite hardcodes `HirType → LLVM type` mapping with no ABI awareness. **REIMPLEMENT**: Saturnite's `mir_type_to_llvm` function is a simplified analog of Rust's `Layout` system. For struct/enum return types, Saturnite should eventually port the concept of `FnAbi`-based calling convention selection.

5. **No exception handling / unwinding**: Saturnite has no `Unwind` in its terminators, no `Cleanup`/`Funclet`/`LandingPad` support. This is correct and aligned — **KEEP** as-is.

6. **No debug info**: Rust has extensive debug info generation (`FunctionDebugContext`, `PerLocalVarDebugInfo`, `debuginfo` module). Saturnite has no debug info support. **REIMPLEMENT** (Phase 6 enhancement, not urgent).

7. **No intrinsic handling**: Rust has `intrinsic` module for compiler intrinsics. Saturnite's `PRINTLN_DEF_ID` sentinel is a simplified analog. **KEEP** for now, but **ADAPT** the pattern of declaring builtins in the module before codegen.

8. **`PassBuilderOptions`**: Saturnite correctly uses `inkwell::passes::PassBuilderOptions` with `opt_pass_name()` from `TargetConfig`. This mirrors Rust's approach of mapping profile → pass pipeline. **KEEP**.

---

## 6. Module System and Name Resolution

### Rust Reference: `rustc_resolve/src/lib.rs` + `rustc_resolve/src/imports.rs`

Rust's name resolution is one of the most complex parts of the compiler:

- **`Resolver<'ra, 'tcx>`**: The main resolver visitor. Fields (dozens of HashMap/Vec/HashMap fields):
  - `tcx: TyCtxt<'tcx>` — shared type context
  - `graph_root: LocalModule<'ra>` — root of the module graph
  - `prelude: Option<Module<'ra>>` — the prelude module
  - `extern_prelude: FxIndexMap<IdentKey, ExternPreludeEntry<'ra>>` — extern crate prelude
  - `partial_res_map: NodeMap<PartialRes>` — unresolved → resolved mapping
  - `import_use_map: FxHashMap<Import<'ra>, Used>` — import usage tracking
  - `local_modules: Vec<LocalModule<'ra>>` — all local modules
  - `local_module_map: FxIndexMap<LocalDefId, LocalModule<'ra>>` — DefId→Module
  - `determined_imports` / `indeterminate_imports` — import resolution state
  - `macro_rules_scopes`, `output_macro_rules_scopes` — macro scoping
  - `per_local_var_debug_info`, etc.
- **`Module<'ra>`**: `Interned<'ra, ModuleData<'ra>>` — interned module reference. `ModuleData` contains:
  - `parent: Option<Module<'ra>>` — parent module
  - `kind: ModuleKind` — ModuleKind enum (SourceFile, Def, etc.)
  - `lazy_resolutions: Resolutions<'ra>` — name→resolution map
  - `glob_importers`, `globs` — glob import tracking
  - `traits` — cached trait list
  - `span: Span` — source span
- **`ResolverArenas<'ra>`**: Typed arena allocation for `ModuleData`, `ImportData`, `NameResolution`, `ast::Path`, `SyntaxExtension`.
- **`resolve_crate`**: Entry point. Orchestrates:
  - `finalize_imports` — resolves all `use` declarations
  - `compute_effective_visibilities` — privacy analysis
  - `lint_reexports` — unused import linting
  - `finalize_macro_resolutions` — macro resolution
  - `late_resolve_crate` — per-module late resolution pass
  - `resolve_main` — entry point detection
  - `check_unused` — unused import warnings
  - `report_errors` — emit all resolution errors
  - `postprocess` — finalize external crate metadata
- **`resolve_imports`**: Fixed-point iteration resolving imports; uses `par_for_each_slice` for parallel resolution. Each `Import` is resolved via `resolve_import` which calls `maybe_resolve_path` → `maybe_resolve_ident_in_module`.
- **`resolve_path`**: Resolves a multi-segment path through the module graph. Uses `ParentScope` for visibility. Returns `PathResult` (Module / NonModule / Indeterminate / Failed).
- **`Namespace`**: Two namespaces — `TypeNS` and `ValueNS` (macros have a third in newer Rust).

### Saturnite Counterpart: `hir/lower.rs` + `hir/symbol.rs` + `module.rs`

Saturnite has a **much simpler** module system:

- **`ModuleGraph`** (in `module.rs`):
  - `modules: Vec<Module>` — all discovered modules
  - `root: ModuleId` — root module
  - `symbol_interner: SymbolInterner` — shared interner
  - `module_index: HashMap<ModulePath, ModuleId>` — path→module lookup
  - `imports: HashMap<ModuleId, Vec<ModuleId>>` — import edges
- **`Module`** struct:
  - `id: ModuleId`, `path: ModulePath`, `file_path: PathBuf`
  - `ast: Option<Program>` — lazily loaded AST
  - `parent: Option<ModuleId>`, `mod_declarations: Vec<String>`
- **`ModuleScope`** (in `module.rs`):
  - `items: HashMap<SymbolId, DefId>` — name→def in this module
  - `imports: HashMap<SymbolId, DefId>` — alias→def for use declarations
  - `parent: Option<ModuleId>` — parent for chain walking
- **`LowerScope`** (in `hir/lower.rs`):
  - `variables: HashMap<SymbolId, VarInfo>` — lexical variable scope
  - `parent: Option<Box<LowerScope>>` — lexical parent chain
- **`HirProgram`** (in `hir/function.rs`):
  - `modules: Vec<Module>`, `root_module: ModuleId`
  - `module_paths: HashMap<DefId, ModuleId>` — def→module mapping
  - `def_table: DefTable` — `DefId → (ModuleId, local_index, DefKind)`
  - `module_scopes: Vec<ModuleScope>` — per-module namespaces
  - `use_decls: Vec<HirUseDecl>` — all use declarations
  - `mod_decls: Vec<HirModDecl>` — all mod declarations
- **`HirLower`** (in `hir/lower.rs`):
  - `symbols: SymbolInterner` — owned symbol table
  - `function_sigs: HashMap<SymbolId, FunctionSig>` — for call resolution
  - `struct_defs: &[StructDef]`, `enum_defs: &[EnumDef]` — type registries
  - `enum_names: HashMap<&str, ()>` — enum name set
- **`resolve_modules`** (in `hir/lower.rs`):
  - Post-pass that walks `use_decls` and `mod_decls`, populates `module_paths`, `def_table`, and resolves `HirModDecl.module_id` from `mod_declarations`.
  - Simple string-based matching (uses `HashMap<SymbolId, DefId>` lookups).

#### Categorization: **KEEP** (with **ADAPT** for future growth)

**Rationale:** Saturnite's module system is **appropriately simplified** for its scope. It correctly implements:

- `mod foo;` → file discovery (`foo.stnx` or `foo/mod.stnx`) ✅
- `use foo::bar` → path resolution with alias tracking ✅
- Parent-chain scope walking (`lookup_with_parent`) ✅
- DefId → ModuleId mapping via `DefTable` ✅
- Per-module name→DefId scopes ✅

**Architectural divergences and recommendations:**

1. **`SymbolId`-keyed scopes vs. `String`-keyed**: Saturnite correctly uses `SymbolId` keys (interned) for all hash map lookups, which aligns with Rust's use of `Symbol` keys. **KEEP**.

2. **No namespace separation**: Rust has `TypeNS` and `ValueNS` (and `MacroNS`). Saturnite uses a single namespace. This is fine for a language without `struct Foo` and `fn Foo` coexisting. **KEEP** — but if Saturnite adds traits or type aliases, **REIMPLEMENT** a dual-namespace model.

3. **`resolve_modules` is a post-pass, not integrated**: Rust integrates resolution into the `Resolver` visitor during AST→HIR lowering. Saturnite does module path resolution as a separate `resolve_modules` post-pass. This is architecturally different but acceptable for a simpler compiler. **ADAPT**: If Saturnite adds visibility checking and full path resolution, integrate the resolution logic into the lowering pass rather than a separate step.

4. **No extern crate / extern prelude**: Saturnite has no `extern crate` support. **KEEP** — no action needed.

5. **No glob imports**: Saturnite's `imports` is `HashMap<ModuleId, Vec<ModuleId>>` — tracks module-to-module edges, but `ModuleScope.imports` is a `HashMap<SymbolId, DefId>` (single-name imports only). No `*` glob support. **REIMPLEMENT** if glob imports are added.

6. **No macro system**: Saturnite has no macros. **REJECT** for reuse — Rust's macro resolution (`macro_rules_scopes`, `output_macro_rules_scopes`, `speculative_flag`) is far beyond Saturnite's scope.

7. **No privacy/visibility enforcement**: Saturnite tracks `Visibility` (Private/Public) on definitions but `resolve_modules` does not enforce cross-module visibility rules. Rust's `compute_effective_visibilities` + `EffectiveVisibilities` struct handles this. **REIMPLEMENT** a simple version: when resolving a cross-module name reference, check that the target's visibility allows access from the referencing module.

8. **`DefTable` as flat Vec**: Saturnite uses `DefEntry` in a `Vec<DefEntry>` indexed by `DefId.0`. Rust uses `LocalDefIdMap` (indexed by `LocalDefId`) for similar purposes. Saturnite's approach is simpler and correct — **KEEP**.

---

## 7. Linking

### Rust Reference: `rustc_codegen_ssa/src/back/link.rs`

Rust's linking is handled by `rustc_codegen_ssa::back::link`:

- **`link_binary`**: The main entry point. Takes a `Box<dyn WriteBuilder>` and a `Linker`. Orchestrates:
  - `should_static_link` / `should_link` — decides whether to link at all
  - `build_subprocess_command` — constructs the linker invocation with all flags
  - `collect_link_dead_code` — garbage collection of unreferenced symbols
  - `collect_native_dependencies` — gathers native library dependencies
  - `codegen_optimize` — runs LLVM optimization passes before linking
  - `codegen_prepare_cmd` — prepares the linker command line
  - `codegen_msvc_link` / `codegen_gnu_link` / `codegen_darwin_link` / `codegen_wasm_link` — platform-specific link command builders
- **Platform-specific link command construction**:
  - **MSVC**: `link.exe` with `/OUT:`, `/DEFAULTLIB:`, `.lib` files, `.obj` files. Uses `CodegenFnAttrFlags` for linker directives.
  - **GNU/Linux**: `cc` or `clang` with `-o`, `-Wl,`, `.o` files. Supports LTO (`-flto`), `-C link-arg`.
  - **Darwin**: `ld64` or `clang` with `-m macosx-version-min`, `-l`, `-framework`. Framework linking via `-framework Name -F path`.
  - **WASI**: `wasm-opt`, `--export` flags, `emcc`-style linking.
  - **Windows/GNU**: `gcc` (MinGW) with GNU-style flags.
- **`collect_native_dependencies`**: Reads `#[link(name = "...", kind = "...")]` attributes, library search paths (`#[link_dir]`), and framework paths.
- **`build_subprocess_command`**: Constructs a `Command` with the chosen linker, all object files, all native libraries, all library paths, and platform-specific flags.
- **Linker selection**: Uses `tcx.sess.opts.linker` if specified, else picks platform default (e.g., `cc` for GNU, `link.exe` for MSVC, `clang` for Darwin).
- **Link dead code**: Uses `--gc-sections` (ELF), `/OPT:REF` (MSVC), `-dead_strip` (Darwin) to garbage-collect unused symbols.
- **Rlib/staticlib/dylib**: Handles all output kinds via the `OutputKind` enum.

### Saturnite Counterpart: `codegen/linker.rs`

- **`Linker<'cfg>`**: Holds `&'cfg TargetConfig`.
- **`link`**: Core method — takes `obj_path` and `output_path`, calls `select_linker` and `build_linker_args`, then spawns `Command::new(linker_path)`.
- **`select_linker`**: Matches `(os, env)` → linker binary name:
  - Linux → `cc`
  - Darwin → `clang`
  - Windows MSVC → `link.exe`
  - Windows GNU → `gcc`
  - Default → `cc`
- **`build_linker_args`**: Constructs platform-specific argument vector:
  - Linux/Darwin: `[obj, -o, output, runtime_obj]`
  - Windows MSVC: `[obj, /OUT:output, /DEFAULTLIB:runtime_obj]`
  - Windows GNU: `[obj, -o, output, runtime_obj]`
- Uses `which::which(linker_name)` to locate the linker on PATH before spawning.
- **`check_linker_available`**: Verifies linker discovery and runs `--version` / `/?` to confirm it works.
- **Runtime object**: `libsaturnite_runtime.a` — provides `println_i64` builtin.

#### Categorization: **KEEP**

**Rationale:** Saturnite's linker is **correctly simplified** for its scope. The design patterns are aligned:

1. **Platform-aware linker selection** via `(os, env)` match — directly mirrors Rust's `codegen_msvc_link` / `codegen_gnu_link` / `codegen_darwin_link` split. **KEEP**.
2. **PATH lookup via `which::which`** — Rust doesn't do this (it relies on the linker being on PATH implicitly), but Saturnite's explicit check is **better** for user experience. **KEEP**.
3. **Linker availability check** (`check_linker_available`) — Rust does not have an equivalent pre-flight check. Saturnite's is superior. **KEEP**.
4. **Single runtime object** vs. Rust's `collect_native_dependencies`: Saturnite only needs one runtime library (`libsaturnite_runtime.a`); Rust must gather all `#[link]` dependencies. Saturnite's simplification is correct. **KEEP**.
5. **No `--gc-sections` / dead code elimination**: Saturnite doesn't pass linker flags for removing unused code. For a single-function-per-program model (or small programs), this is fine. **REIMPLEMENT** if unused-symbol elimination becomes necessary.
6. **No LTO**: Saturnite doesn't support `-flto`. **REJECT** — overkill for current scope.
7. **No framework linking** (Darwin): Saturnite doesn't support macOS frameworks. **REJECT** — no framework usage.
8. **No `.obj`/`.o` path handling for multiple files**: Saturnite links a single object file. Rust handles many. **KEEP** — scalable if Saturnite adds multi-file compilation.

---

## 8. Diagnostic Rendering

### Rust Reference: `rustc_errors/src/`

Rust's diagnostic system is extremely sophisticated:

- **`DiagCtxt`** (in `lib.rs`): Central diagnostic context. Creates `Diag` instances, manages emitters, tracks error counts, supports `--json` output.
- **`Diagnostic<'a, G: EmissionGuarantee>` trait** (in `diagnostic.rs`): Trait implemented by all diagnostic types. `into_diag(self, dcx: DiagCtxtHandle, level: Level) -> Diag<'a, G>`. Uses `#[derive(Diagnostic)]` proc macro.
- **`Diag<'a, G: EmissionGuarantee>`**: The mutable diagnostic builder. Wraps `Option<Box<DiagInner>>`. Implements `Deref<DegInner>` and `DerefMut`. Uses `with_fn!` macro to generate builder methods (`code`, `span`, `note`, `help`, `suggestion`, etc.).
- **`DiagInner`** (in `diagnostic.rs`): The inner state of a diagnostic:
  - `level: Level` — severity (Bug, Fatal, Error, DelayBug, Warning, Note, etc.)
  - `messages: Vec<(DiagMessage, Style)>` — primary messages
  - `code: Option<ErrCode>` — error code (e.g., `E0308`)
  - `lint_id: Option<LintExpectationId>`
  - `span: MultiSpan` — all spans associated with this diagnostic
  - `children: Vec<Subdiag>` — sub-messages (notes, helps, suggestions)
  - `suggestions: Suggestions` — code suggestions
  - `args: DiagArgMap` — format arguments
  - `sort_span: Span` — for sorting diagnostics
  - `is_lint: Option<IsLint>`
  - `long_ty_path: Option<PathBuf>`
  - `emitted_at: DiagLocation`
- **`Level` enum**: `Bug`, `Fatal`, `Error`, `DelayedBug`, `ForceWarning`, `Warning`, `Note`, `OnceNote`, `Help`, `OnceHelp`, `FailureNote`, `Allow`, `Expect`.
- **`Suggestions` enum**: `Enabled(Vec<CodeSuggestion>)` / `Sealed(Box<[CodeSuggestion]>)` / `Disabled` — supports sealing suggestions so they cannot be modified after a point.
- **`Emitter` trait** (in `emitter.rs`): `emit(&mut self, diags: &(DiagInner, Option<Subdiags>), ...)` — pluggable rendering backends.
- **`JsonEmitter`**: JSON output format.
- **`AnnotateSnippetEmitterWriter`**: Rich text rendering with underline/span highlighting.
- **`ErrCode`**: Error codes with documentation (`rustc_error_messages`).
- **`MultiSpan`**: A diagnostic span that can span multiple locations with labels (`SpanLabel`).

### Saturnite Counterpart: `error.rs`

- **`Diagnostic` derive via `miette`**: `LexError` and `ParseError` use `#[derive(Diagnostic)]` from the `miette` crate (NOT `rustc_macros::Diagnostic`).
- **`CompilerError` enum**: Central error type with variants:
  - `Lexer(LexError)` via `#[from]`
  - `Parse(ParseError)` via `#[from]`
  - `Semantic(String)` — plain string
  - `Type(String)` — plain string
  - `Codegen(String)` — plain string
  - `Target(#[from] TargetError)`
  - `Link(#[from] LinkError)`
  - `Io(#[from] std::io::Error)`
  - `Process(String)`
  - `Config(String)`
  - `IrEmissionError { message: String }`
- Constructors: `CompilerError::semantic(msg)`, `CompilerError::codegen(msg)`, `CompilerError::config(msg)`.
- **`CompilerResult<T>`**: `Result<T, CompilerError>`.
- **`From<miette::Report>`**: Converts miette reports to `CompilerError::Semantic`.
- **`TargetError`**: `message + triple: Option<String>`.
- **`LinkError`**: `message + details: Option<String>`.

#### Categorization: **ADAPT/PORT** (do NOT directly port code)

**Rationale:** Saturnite's error model (`thiserror` + `miette`) is a **deliberately different architectural choice** from Rust's (`Diagnostic` trait + `#[derive(Diagnostic)]` from `rustc_macros`). The Saturnite approach is:

- **Simpler**: No `DiagCtxt`, no `Level` enum, no suggestion sealing, no JSON emission.
- **Composition**: Uses `thiserror` for error composition and `miette` for rich rendering (source spans, labels).
- **No error codes**: No `ErrCode` system (e.g., `E0308`).

**Rust patterns worth adapting (NOT porting):**

1. **Error codes**: Saturnite's `CompilerError` variants could benefit from a stable code system (e.g., `SNX001` for lexer errors). This is a small addition — add an `ErrorKind` enum with discriminant codes. **ADAPT**: Add a `#[error_code = "SNX001"]` attribute or a manual `code()` method. Do NOT port Rust's `ErrCode` infrastructure.

2. **Severity levels beyond Error/Warning**: Saturnite only has `CompilerError` (always an error) and `miette::Report` (treated as `Semantic`). Rust's `Level` enum (Warning, Note, Help) enables non-fatal diagnostics. **ADAPT**: Add a severity field to `CompilerError` or introduce a `Severity` enum (`Error`, `Warning`, `Note`). Do NOT port the full `EmissionGuarantee` trait pattern — overkill.

3. **Sub-diagnostics (notes, helps)**: Saturnite's `Semantic(String)` / `Type(String)` etc. are flat messages. Rust's `DiagInner` children allow structured notes. **ADAPT**: Add a `notes: Vec<String>` field to `CompilerError::Semantic` / `Type` if multi-message diagnostics are needed. Do NOT port Rust's `Subdiag` trait system.

4. **`miette` vs `rustc_errors` emitter architecture**: Saturnite uses `miette` (an external crate) for rendering. Rust uses its own `rustc_errors::Emitter` trait with `JsonEmitter` and `AnnotateSnippetEmitterWriter`. Saturnite's choice is **appropriate** for a standalone compiler — `miette` is a well-maintained, ergonomic crate. **REJECT** porting `rustc_errors::Emitter` — would be a regression for a project already using `miette`.

5. **Suggestion infrastructure**: Rust's `CodeSuggestion` + `Suggestions` enum (with `Enabled`/`Sealed`/`Disabled` states) is sophisticated. Saturnite has no suggestions. **REJECT** for now — `miette` has its own suggestion support via `#[suggestion(...)]` attributes.

**Recommendation:** Saturnite should **keep** `thiserror` + `miette`. To get closer to Rust's diagnostic quality:

1. Add error codes to `CompilerError` variants (a simple `code: &'static str` field or a manual `ErrorKind` enum).
2. If non-fatal diagnostics (warnings, notes) are needed, introduce a lightweight `Diagnostic` wrapper around `CompilerError` with a `Level`.
3. Use `miette`'s `#[suggestion]` and `#[help]` attributes for richer diagnostics where `miette` supports them.

---

## Cross-Cutting Observations

### Data Structure Choices

| Concern | Rust | Saturnite | Recommendation |
|---|---|---|---|
| Indexed collections | `IndexVec<I, V>` (rustc_index) | `Vec<V>` with manual `u32` indexing | **ADAPT**: Use `rustc_index::IndexVec` or at minimum wrap in typed index types to prevent mixing `LocalId`/`BlockId`/`DefId` |
| HashMap hasher | `FxBuildHasher` (deterministic, fast) | `std::collections::HashMap` (RandomState) | **ADAPT**: Switch to `rustc_hash::FxBuildHasher` for determinism |
| Dense bitsets | `DenseBitSet<I>` (rustc_index) | `HashSet<I>` | **ADAPT**: Use `DenseBitSet` for verifier and future liveness analysis |
| Arena allocation | `DroplessArena` (rustc_arena) | `String` heap allocations | **ADAPT**: Use arena for symbol strings |
| Deterministic hashing | `StableHash` / `StableHasher` | No stable hashing | **REJECT**: Not needed without incremental compilation |

### Type System Decoupling

Saturnite's `MirType = HirType` (flat enum) is fundamentally simpler than Rust's `TyCtxt`-based type system. This is the correct architectural choice for Saturnite's current scope. **KEEP** — do NOT attempt to port `TyCtxt`, `GenericArgs`, or `Ty` structures.

### Testability

Saturnite's `verify.rs` returns `Result<(), Vec<MirVerifyError>>` — this is the **correct pattern**. Rust's `Validator` uses `span_bug!` which panics. Saturnite's approach is actually **better** for a standalone compiler. **KEEP**.

---

## Summary Table

| Area | Saturnite State | Rust Pattern | Recommendation |
|---|---|---|---|
| MIR types (Rvalue, Terminator, Stmt) | 7-var, 5-var, 2-var enums | ~25-var, ~15-var, ~15-var with Place/ProjectionElem | ADAPT/PORT |
| MIR constants (ConstValue) | `MirConst {I64, F64, Bool}` | `ConstValue {Scalar, ZST, Slice, Indirect}` | KEEP (Saturnite's is correct for its types) |
| MIR BinOp / UnOp | 13 variants, 2 variants | 13 variants, 3 variants | KEEP (identical) |
| MIR locals/blocks | `LocalId(u32)`, `BlockId(u32)` newtypes | `Local`/`BasicBlock` via `newtype_index!` | ADAPT |
| MIR verification | 5 checks, structured errors | Validator pass with ~20 checks, panics on failure | KEEP (Saturnite's approach is better) |
| MIR passes | Single ConstantFolder | ~40 MirPass implementations + PassManager | ADAPT/PORT (introduce MirPass trait) |
| MIR optimization | Constant folding only | Full opt pipeline (cfg, copy prop, DSE, GVN, etc.) | REIMPLEMENT (simplified versions) |
| Symbol interning | Vec<String> + HashMap<RandomState> | DroplessArena + HashTable + FxBuildHasher | ADAPT/PORT |
| Codegen context | MirCodeGenContext (flat) | FunctionCx (generic over BuilderMethods) | ADAPT |
| Local storage | Always alloca | Immediate operands for leaf locals | ADAPT |
| Block iteration | Vector order | Reverse postorder | ADAPT |
| Module system | ModuleGraph + ModuleScope | Resolver + ModuleData (interned) | KEEP |
| Name resolution | LowerScope + ModuleScope parent chain | Resolver.resolve_path with Namespaces | KEEP |
| Import resolution | resolve_modules post-pass | resolve_imports fixed-point iteration | KEEP (Saturnite's is adequate) |
| Linker | Platform-aware cc/clang/link.exe/gcc | Same + dead code stripping + LTO | KEEP |
| Diagnostics | thiserror + miette | rustc_errors DiagCtxt + Diagnostic trait | KEEP (miette is the right choice) |
| Error codes | None | ErrCode (E0308, etc.) | ADAPT (add simple error code system) |
| Severity levels | Binary (Error vs. nothing) | Level enum (Bug, Fatal, Error, Warning, Note, etc.) | ADAPT (add lightweight severity) |

---

## Detailed File-Level Findings

### Saturnite files examined:

| File | Path | Lines | Content |
|---|---|---|---|
| `hir/symbol.rs` | `C:\Users\atimo\Saturnite\crates\stnx\src\hir\symbol.rs` | 187 | `SymbolId`, `DefId`, `SymbolInterner`, `DefKind`, `DefEntry`, `DefTable`, `Visibility` |
| `mir/mod.rs` | `C:\Users\atimo\Saturnite\crates\stnx\src\mir\mod.rs` | 343 | MIR type definitions: `MirRvalue`, `MirTerminator`, `MirStmtKind`, `MirOperand`, `MirConst`, `MirBinOp`, `MirUnOp`, `MirLocal`, `MirBasicBlock`, `MirFunction`, `MirProgram` |
| `mir/lower.rs` | `C:\Users\atimo\Saturnite\crates\stnx\src\mir\lower.rs` | 734 | HIR→MIR lowering: `MirLower` builder, explicit CFG construction, `lower_function`, `lower_expr`, `lower_stmt` |
| `mir/verify.rs` | `C:\Users\atimo\Saturnite\crates\stnx\src\mir\verify.rs` | 204 | `MirVerifyError`, `VerifyResult`, 5 structural checks |
| `mir/opt.rs` | `C:\Users\atimo\Saturnite\crates\stnx\src\mir\opt.rs` | 163 | `ConstantFolder`, `fold_binop`/`fold_unop` on `I64`/`F64`/`Bool` |
| `mir/codegen.rs` | `C:\Users\atimo\Saturnite\crates\stnx\src\mir\codegen.rs` | 841 | `MirCodeGenContext`, `gen_rvalue`, `gen_binop`, `gen_terminator`, `compile_from_mir_ext` |
| `hir/lower.rs` | `C:\Users\atimo\Saturnite\crates\stnx\src\hir\lower.rs` | 2532 | `HirLower`, `LowerScope`, `lower_program`, `lower_program_with_graph`, `resolve_modules` |
| `hir/function.rs` | `C:\Users\atimo\Saturnite\crates\stnx\src\hir\function.rs` | 222 | `HirFunction`, `HirProgram`, `StructDef`, `EnumDef`, `HirUseDecl`, `HirModDecl` |
| `hir/types.rs` | `C:\Users\atimo\Saturnite\crates\stnx\src\hir\types.rs` | 57 | `HirType` enum (I64, F64, Bool, Str, Unit, Struct, Enum) |
| `module.rs` | `C:\Users\atimo\Saturnite\crates\stnx\src\module.rs` | 1516 | `ModuleId`, `ModulePath`, `Module`, `ModuleScope`, `ModuleGraph`, `Project` |
| `target.rs` | `C:\Users\atimo\Saturnite\crates\stnx\src\target.rs` | 482 | `TargetConfig`, `Profile`, `OutputKind`, `Architecture`, `OperatingSystem`, `Environment` |
| `codegen/linker.rs` | `C:\Users\atimo\Saturnite\crates\stnx\src\codegen\linker.rs` | 200 | `Linker`, `select_linker`, `build_linker_args`, `check_linker_available` |
| `error.rs` | `C:\Users\atimo\Saturnite\crates\stnx\src\error.rs` | 159 | `CompilerError`, `CompilerResult`, `LexError`, `ParseError`, `TargetError`, `LinkError` |
| `semantic.rs` | `C:\Users\atimo\Saturnite\crates\stnx\src\semantic.rs` | 54 | Entry points: `analyze`, `analyze_and_lower`, `analyze_and_lower_with_graph` |
| `main.rs` | `C:\Users\atimo\Saturnite\crates\stnx\src\main.rs` | 719 | CLI pipeline: Build/Check/Run/Doctor/Init |

### Rust compiler files examined:

| File | Path | Content examined |
|---|---|---|
| MIR syntax | `compiler/rustc_middle/src/mir/syntax.rs` | `Rvalue` (~25 variants), `TerminatorKind` (~15), `StatementKind` (~15), `Operand` (4), `Place`, `ProjectionElem`, `BinOp` (13), `UnOp` (3) |
| MIR consts | `compiler/rustc_middle/src/mir/consts.rs` | `ConstValue` (Scalar, ZeroSized, Slice, Indirect) |
| MIR transform lib | `compiler/rustc_mir_transform/src/lib.rs` | `declare_passes!` macro, ~40 pass registrations |
| Pass manager | `compiler/rustc_mir_transform/src/pass_manager.rs` | `MirPass` trait, `PassPolicy`, `to_profiler_name` |
| SimplifyCfg | `compiler/rustc_mir_transform/src/simplify.rs` | `CfgSimplifier`, block merging, `pred_count` |
| SimplifyLocals | `compiler/rustc_mir_transform/src/simplify.rs` | `UsedLocals`, `make_local_map`, `LocalUpdater` |
| CopyProp | `compiler/rustc_mir_transform/src/copy_prop.rs` | Copy class unification, `SsaLocals` |
| DeadStoreElim | `compiler/rustc_mir_transform/src/dead_store_elimination.rs` | `MaybeTransitiveLiveLocals`, dataflow liveness |
| Validate | `compiler/rustc_mir_transform/src/validate.rs` | `Validator` pass, structural invariant checking |
| Codegen SSA MIR | `compiler/rustc_codegen_ssa/src/mir/mod.rs` | `FunctionCx` struct, `codegen_mir`, `LocalRef`, `CachedLlbb` |
| Symbol interner | `compiler/rustc_span/src/symbol.rs` | `Symbol`, `SymbolIndex`, `Interner`, `InternerInner` (arena + HashTable) |
| Errors lib | `compiler/rustc_errors/src/lib.rs` | `DiagCtxt`, `Suggestions`, `CodeSuggestion`, `Level` enum |
| Diagnostic trait | `compiler/rustc_errors/src/diagnostic.rs` | `Diagnostic<'a, G>` trait, `Diag<'a, G>`, `DiagInner`, `Subdiag` |
| Resolver | `compiler/rustc_resolve/src/lib.rs` | `Resolver<'ra, 'tcx>`, `ModuleData`, `Module`, `ResolverArenas` |
| Imports resolution | `compiler/rustc_resolve/src/imports.rs` | `resolve_imports` fixed-point, `resolve_import` |
| Path resolution | `compiler/rustc_resolve/src/ident.rs` | `resolve_path`, `PathResult` |
| Data structures | `compiler/rustc_data_structures/src/fx.rs` | `FxHashMap`, `FxIndexMap`, `FxIndexSet`, `FxBuildHasher` |
| Stable hash | `compiler/rustc_data_structures/src/stable_hash.rs` | `StableHashCtxt`, `StableHash`, `StableHasher` |
| HIR ItemKind | `compiler/rustc_hir/src/hir.rs` | `ItemKind` enum (~22 variants), `Impl` struct |

---

## Conclusion

The analysis reveals that Saturnite's architecture is **well-aligned** with the Rust compiler's structural patterns, but operates at a significantly reduced scope. The core decisions — MIR as an explicit CFG IR between HIR and LLVM, symbol interning with numeric IDs, structured verification returning `Result`, a `FunctionCx`-like codegen context, platform-aware linking, and `thiserror`+`miette` diagnostics — are all **correct and appropriate** for Saturnite's goals.

The highest-value adaptations are:

1. **Symbol interner**: Replace `RandomState` with `FxBuildHasher` and `Vec<String>` with arena-backed storage.
2. **MIR pass infrastructure**: Introduce a `MirPass` trait and pass manager to compose constant folding, CFG simplification, and dead local elimination.
3. **Indexed collections**: Use typed indices (via `rustc_index` or local equivalents) to prevent `LocalId`/`BlockId`/`DefId` mixing.
4. **Per-local storage optimization**: Track immediate locals (never address-taken) as direct LLVM SSA values instead of always using `alloca`.
5. **Block iteration order**: Use reverse-postorder instead of vector order for better LLVM optimization.

The patterns that should **not** be ported are those that exist solely to support Rust language features absent in Saturnite: borrow checking infrastructure in MIR validation, exception handling / unwinding in codegen, macro resolution in name resolution, extern crate handling in modules, and the `rustc_errors` diagnostic emitter (Saturnite's `miette` choice is superior for a standalone tool).
