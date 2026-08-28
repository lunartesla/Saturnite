# Performance and Optimization Analysis: Saturnite vs. Rust

## Executive Summary

Saturnite's compiler pipeline is a minimal, single-threaded MIR pipeline with only **one** optimization pass (constant folding) and a codegen backend that allocates an LLVM stack slot (`alloca`) for every local unconditionally. Rust's `rustc`, by contrast, runs **70+ MIR passes** across multiple phases (lint, cleanup, optimization, codegen preparation), uses **selective allocation** (skipping `alloca` for immediate types), iterates basic blocks in **reverse postorder**, runs on **FxHashMap** (fast deterministic hashing), and parallelizes codegen via a **jobserver-backed work-stealing scheduler**.

The gap is largest in three areas: (1) missing MIR optimization passes that eliminate dead code, redundant computation, and unnecessary memory traffic; (2) codegen that misses LLVM optimization opportunities through poor block ordering and universal alloca allocation; and (3) complete absence of parallelism, which on multi-core machines means leaving 4-8x of available throughput on the table.

**Estimated upside from addressing all gaps: 3-10x compile-time speedup and 10-30% runtime code-quality improvement on typical workloads.**

---

## 1. MIR Optimization Pass Comparison

### Pass Inventory

| Rust MIR Pass | Rust LOC (est.) | Saturnite Equivalent? | ROI | Notes |
|---|---|---|---|---|
| **SimplifyCfg** | 715 (`simplify.rs`) | No | **High** | Eliminates dead blocks, merges trivial blocks, removes unreachable edges. Single most impactful MIR pass for codegen quality. Saturnite walks all blocks in source order, including dead ones, and emits LLVM blocks for unreachable code. |
| **InstSimplify** | 471 (`instsimplify.rs`) | Partially (basic const folding, 163 LOC) | **Medium-High** | Rust's version handles operand reassociation, identity elimination, comparison folding, cast folding, arithmetic simplification on i1/i8/i16/i32/i128/usize/isize/f32/f64/bool, and more. Saturnite's `ConstantFolder` only handles i64, f64, bool on a subset of operations and does not simplify identities, reassociate, or propagate casts. |
| **GVN (Global Value Numbering)** | 2,214 (`gvn.rs`) | No | **High** | Eliminates redundant loads and common subexpressions across basic blocks using value-numbering. Saturnite loads every local from its alloca on every use, creating massive redundancy. |
| **Inline** | 1,410 (`inline.rs`) | No | **High** | Function inlining based on cost model. Critical for generic monomorphization and cross-crate optimization. |
| **DataflowConstProp** | 1,071 (`dataflow_const_prop.rs`) | No | **High** | Propagates constants through control flow via dataflow analysis. Saturnite's folding only works on intra-statement constants. |
| **SROA (Scalar Replacement of Aggregates)** | 435 (`sroa.rs`) | No | **High** | Breaks down struct/aggregate allocations into individual scalar allocas or registers, enabling further optimization. Saturnite allocates struct locals as whole allocas unconditionally. |
| **JumpThreading** | 1,132 (`jump_threading.rs`) | No | **Medium-High** | Threads control flow through conditional branches to reduce block count. |
| **CopyProp (Copy Propagation)** | 194 (`copy_prop.rs`) | No | **Medium** | Eliminates redundant copy assignments. |
| **DeadStoreElimination** | 156 (`dead_store_elimination.rs`) | No | **Medium** | Removes stores whose values are never read. Saturnite stores every parameter and every local assignment to an alloca even if the value is never reloaded. |
| **ReferencePropagation** | 467 (`ref_prop.rs`) | No | **Medium** | Propagates reference values through the MIR, reducing address-of/load pairs. |
| **SimplifyLocals** | 715 (part of `simplify.rs`) | No | **Medium** | Removes unused locals, remaps indices, simplifies after other passes. |
| **SimplifyConstCondition** | 99 (`simplify_branches.rs`) | No | **Medium** | Simplifies branches on known-true/known-false conditions (e.g., `if true { ... }`). |
| **UnreachablePropagation** | 151 (`unreachable_prop.rs`) | No | **Medium** | Propagates unreachable from call sites through the CFG. |
| **UnreachableEnumBranching** | 173 (`unreachable_enum_branching.rs`) | No | **Medium** | Eliminates branches on impossible enum variants. |
| **MatchBranchSimplification** | 547 (`match_branches.rs`) | No | **Medium** | Simplifies match arms by merging, reordering, or eliminating branches. |
| **StripDebugInfo** | 52 (`strip_debuginfo.rs`) | No | **Low** | Removes debug intrinsics before codegen. |
| **PromoteTemps** | 1,105 (`promote_consts.rs`) | No | **Medium** | Promotes eligible temporaries to constants for compile-time evaluation. |
| **RemoveZsts** | 146 (`remove_zsts.rs`) | No | **Low** | Removes zero-sized type operations. |
| **RemoveUnneededDrops** | 46 (`remove_unneeded_drops.rs`) | No | **Low** | Removes drop glue for types with no destructors. |
| **ReorderBasicBlocks** | 152 (`prettify.rs`) | No | **Medium** | Reorders blocks for better instruction cache locality and fall-through. |
| **ReorderLocals** | 152 (`prettify.rs`) | No | **Low** | Reorders locals for better allocasim stack layout. |
| **ElaborateDrops** | 523 (`elaborate_drops.rs`) | No | **High** | Lowers `Drop` terminators into explicit drop glue calls. Critical for memory safety with destructors. |
| **ForceInline** | 1,410 (`inline.rs`) | No | **Medium** | Inlines `#[inline(always)]` / `#[rustc_force_inline]` calls. |
| **RemoveStorageMarkers** | 29 (`remove_storage_markers.rs`) | No | **Low** | Cleans up `StorageLive`/`StorageDead` markers after analysis. |
| **SingleUseConsts** | 206 (`single_use_consts.rs`) | No | **Low** | Inlines constants that are used exactly once. |
| **SimplifyComparisonIntegral** | 218 (`simplify_comparison_integral.rs`) | No | **Medium** | Simplifies integer comparison chains. |
| **DestinationPropagation** | 670 (`dest_prop.rs`) | No | **Medium** | Propagates assignment destinations through `Call` terminators. |
| **EarlyOtherwiseBranch** | 424 (`early_otherwise_branch.rs`) | No | **Low** | Simplifies `if let` patterns in match arms. |
| **LowerIntrinsics** | 346 (`lower_intrinsics.rs`) | No | **Medium** | Lowers `intrinsic::*()` calls to MIR-level implementations. |
| **ScalarReplacementOfAggregates** | 435 | Already covered (SROA) | — | — |
| **MultipleReturnTerminators** | 37 (`multiple_return_terminators.rs`) | No | Low | Detects malformed MIR with multiple terminators per block. |
| **CriticalCallEdges** | 29 (`add_call_guards.rs`) | No | Low | Inserts call probes for stack unwinding safety. |

**Saturnite current state:**

| Saturnite Pass | Lines | Description | Rust Equivalent(s) |
|---|---|---|---|
| `ConstantFolder` | 163 (`mir/opt.rs`) | Folds binary (`Add`, `Sub`, `Mul`, `Div`, `Mod`, `Eq`-`Or`) and unary (`Neg`, `Not`) ops on `i64`, `f64`, `bool`. Only within single statements, no CFG propagation. | `InstSimplify`, `DataflowConstProp` |
| `verify` | ~300 (`mir/verify.rs`) | Structural validation of MIR CFG before codegen. | `SanityCheck`, `Validator` |

**Saturnite gap:** 1 optimization pass vs. Rust's ~44 optimization passes in the main pipeline. Additional passes (lint, cleanup, runtime lowering) bring Rust's total to ~70+. Saturnite also lacks the multi-phase pipeline structure — Rust runs passes in stages: `Initial` (analysis), `PromoteConsts`, `PostAnalysis`, `PreOptimizations`, then the full optimization pipeline, then `Final`.

### Pass Manager Architecture

| Aspect | Rust | Saturnite |
|---|---|---|
| Pass trait | `MirPass<'tcx>` with `policy()`, `run_pass()`, dump support | None — `optimize()` is a free function with a hardcoded loop |
| Policy system | `PassPolicy::Required` vs `Optional { optimization, generally_enabled }`, controllable via `-Zmir-enable-passes` and `#[optimize(none)]` | None — pass always runs unconditionally |
| Opt-level gating | `WithMinOptLevel<T>` wraps passes to gate on `sess.mir_opt_level() >= N` | None — always runs |
| MIR phases | `MirPhase::Analysis` → `Runtime` → `Optimized` tracked via `phase_change` parameter | None — no phase tracking |
| MIR dumping | Per-pass MIR dumps via `is_mir_dump_enabled()` | None |

---

## 2. Codegen Performance Analysis

### Alloca vs. Immediate Storage

**Rust approach** (`compiler/rustc_codegen_ssa/src/mir/mod.rs`, 700+ lines):

Rust uses a `LocalRef` enum with multiple variants:
- `Place(PlaceRef)` — stored in an `alloca` (for types the optimizer might need to inspect, or types that are too large/complex)
- `Operand(OperandRef)` — stored as a direct LLVM SSA value (skips `alloca` entirely)
- `UnsizedPlace` — for unsized types that need indirect storage
- `PendingOperand` / `PendingPlace` — deferred allocation

The decision is made by `non_ssa_locals` analysis: locals whose types are judged "immediate" by `is_llvm_immediate` and are never referenced indirectly get the `Operand` path, avoiding `alloca` + `store` + `load` entirely.

```rust
// Rust: selective alloca (mod.rs:350-360)
if memory_locals.contains(local) {
    LocalRef::Place(PlaceRef::alloca(&mut start_bx, layout))  // only if needed
} else {
    LocalRef::new_operand(layout)  // direct SSA operand — no alloca!
}
```

**Saturnite approach** (`mir/codegen.rs`, 841 lines):

Saturnite allocates an `alloca` for **every** local unconditionally, stores every parameter into the alloca, and loads from the alloca on every use:

```rust
// Saturnite: unconditional alloca for all locals (codegen.rs:146-153)
for local in &func.locals {
    let alloca = self.builder.build_alloca(ty, ...).unwrap();
    self.local_allocas.insert(local.id, (alloca, ty));
}
// And stores params into allocas (codegen.rs:155-159)
for (param_idx, param_lid) in func.param_locals.iter().enumerate() {
    let llvm_param = function_value.get_nth_param(param_idx as u32).unwrap();
    if let Some((alloca, _)) = self.local_allocas.get(param_lid) {
        self.builder.build_store(*alloca, llvm_param).unwrap();  // extra store
    }
}
```

**Impact:** For a function with 10 integer locals, this generates 10 `alloca` instructions, 10 parameter stores, and 10+ `load` instructions per use — all of which the LLVM mem2reg pass must undo. This generates significantly more IR and forces more work on the LLVM backend.

| Metric | Rust | Saturnite | Gap |
|---|---|---|---|
| Alloca per local | Selective (immediate types skip) | Universal (all locals) | **High** |
| Parameter passing | Direct operand when possible | Always stored to alloca | **High** |
| Load/store per use | Zero for operand locals | Load from alloca on every use | **High** |
| `LocalRef` variants | 5 (Place, UnsizedPlace, Operand, PendingOperand, PendingPlace) | Effectively 1 (always alloca) | High |

### Basic Block Ordering

| Aspect | Rust | Saturnite |
|---|---|---|
| Block iteration | `traversal::mono_reachable_reverse_postorder` — computes RPO over reachable blocks only | `for block in &func.blocks` — iterates in vector/source order |
| Dead block handling | `unreached_blocks` DenseBitSet tracks and emits `unreachable` for non-reachable blocks | No dead block detection — emits blocks in source order, including unreachable ones |
| Block merging | `SimplifyCfg` merges trivial blocks before codegen | None — blocks emitted as-is |
| Cold block detection | `find_cold_blocks()` marks `unreachable` and cold paths | None |

**Impact:** Saturnite's source-order block emission means:
1. LLVM sees blocks in non-optimal order, reducing branch prediction quality and icache locality
2. Dead/unreachable blocks are emitted as real code (extra LLVM blocks, extra work)
3. No fall-through optimization opportunities exploited

### LLVM Pass Pipeline

| Aspect | Rust | Saturnite |
|---|---|---|
| Pass configuration | `PassBuilderOptions` with `opt_pass_name()` returning `"default<O0>"` through `"default<O3>"` | Same approach (`opt_pass_name()` in `target.rs`), but the mapping is duplicated inline in `codegen.rs:795-803` instead of calling `target_config.to_inkwell_opt_level()` |
| Opt-level mapping | Centralized in `TargetConfig::to_inkwell_opt_level()` + `opt_pass_name()` | Duplicated in `compile_from_mir_ext()` — the codegen re-implements the same `OptimizationLevel → InkwellOptLevel` and `OptimizationLevel → pass pipeline` mappings that already exist in `target.rs` |
| Tested? | Yes — `target.rs:391-478` has tests for profile → opt-level, pass name, and inkwell mapping | The `opt_pass_name()` method is never called in production code — tests exist but dead code |
| Module merge | Multiple codegen units merged at module level | Single module, no merge step |

---

## 3. Hashing Comparison

### Saturnite

Uses `std::collections::HashMap` (which defaults to `RandomState`, i.e., **SipHash-1-3**) throughout:

```rust
// mir/codegen.rs:24
use std::collections::HashMap;
// mir/codegen.rs:38
local_allocas: HashMap<LocalId, AllocaInfo<'ctx>>,  // RandomState
// mir/codegen.rs:135
let mut llvm_blocks: HashMap<BlockId, ...> = HashMap::new();  // RandomState
// mir/lower.rs:35-36
let mut sigs: HashMap<DefId, (Vec<HirType>, HirType)> = HashMap::with_capacity(...);  // RandomState
// hir/symbol.rs:49
indices: std::collections::HashMap<String, SymbolId>,  // RandomState on String keys
// hir/lower.rs:172
let mut enum_names: HashMap<&str, ()> = HashMap::new();  // RandomState
```

Key issues:
1. **SipHash overhead**: Every insertion, lookup, and deletion pays the SipHash-1-3 cost. For compiler-internal hash maps that don't face adversarial input, this is 5-10x slower than `FxHash` (a simple, fast integer hash).
2. **String-keyed interner lookup**: `SymbolInterner` uses `HashMap<String, SymbolId>`, meaning symbol lookup hashes the full `String` (which itself may allocate). Rust's equivalent typically uses `HashMap<&str, SymbolId>` (no allocation on lookup) or a pre-hash scheme.
3. **No `rustc_hash` dependency**: Saturnite's `Cargo.toml` does not include `rustc-hash`, so there is no `FxHashMap` type available. All hash maps use the slow default.

### Rust

Uses `rustc_data_structures::fx::FxHashMap` (which is `HashMap<K, V, FxBuildHasher>`):

```rust
// rustc_data_structures/src/fx.rs:1
pub use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet, FxHasher};
// rustc_data_structures/src/fx.rs:5-6
pub type FxIndexMap<K, V> = indexmap::IndexMap<K, V, FxBuildHasher>;
pub type FxIndexSet<V> = indexset::IndexSet<V, FxBuildHasher>;
```

`FxBuildHasher` uses `FxHasher` (a fast, non-cryptographic 64-bit hash based on a simple multiply-with-carry RNG). This is:
- **Deterministic**: Same input always produces same hash (important for reproducible builds)
- **Fast**: ~1-2ns per hash vs. ~5-10ns for SipHash
- **Non-cryptographic**: No security guarantees, which is fine for a compiler's internal data structures

Rust also uses `FxIndexMap`/`FxIndexSet` (via the `indexmap` crate) for ordered hash maps, and `DenseBitSet` for bitset-based operations (e.g., tracking reachable blocks).

| Metric | Rust | Saturnite | Gap |
|---|---|---|---|
| Hasher | `FxBuildHasher` (FxHash, ~1-2ns) | `RandomState` (SipHash-1-3, ~5-10ns) | **Medium** |
| Hash map type | `FxHashMap` (custom type alias) | `HashMap` (always std default) | Medium |
| Interner lookup key | `Symbol` (interned, no allocation) | `String` (may allocate on lookup) | **Medium** |
| Ordered map | `FxIndexMap` (indexmap) | Not available | Low |

---

## 4. Parsing Comparison

### Saturnite Parser

Uses **chumsky 0.13**, a parser-combinator library:

```rust
// parser/mod.rs:5-8
use chumsky::error::Simple;
use chumsky::prelude::*;
use chumsky::recursive::Direct;
use chumsky::span::SimpleSpan;

// parser/mod.rs:12
pub type ParserExtra<'a> = extra::Err<Simple<'a, Token, SimpleSpan<usize>>>;

// parser/mod.rs:80
fn program<'a>() -> impl Parser<'a, &'a [Token], Program, ParserExtra<'a>> { ... }

// parser/mod.rs:430
// 1. Use .memoized() on the recursive expr to cache results and detect cycles

// parser/mod.rs:747,765
.memoized()
.memoized()
```

Key characteristics:
- **16 inline test functions** in the parser module (grep count of `fn test_`)
- Uses `.memoized()` on recursive expression parsers to handle left-recursion and avoid exponential blowup
- chumsky's error reporting is comprehensive (Simple error type with spans)
- All parser combinators return `impl Parser<...>` — no explicit trait objects

### Rust Parser

Uses a **hand-written recursive descent** parser (`rustc_parse`), which:
- Is tightly integrated with the lexer (`logos`-like internal scanner)
- Does not use parser-combinator abstractions
- Has full control over error recovery and incremental parsing
- Can produce highly optimized parse trees with pre-allocated `Vec`s

### Comparison

| Aspect | Rust | Saturnite | Gap |
|---|---|---|---|
| Parser type | Hand-written recursive descent | chumsky parser combinators (0.13) | **Medium** |
| Memo | `.memoized()` on recursive exprs | — | chumsky provides memoization for cycle detection |
| Inline tests | ~hundreds across modules | 16 in parser module | Low |
| Error recovery | Sophisticated, span-aware | chumsky Simple error, limited recovery | Medium |
| Performance | Hand-tuned per production rule | Combinator dispatch overhead | **Medium** |
| Binary size | Single compiled parser module | chumsky dependency + generated closure trees | **Low** |

**Notes:**
- chumsky is a good choice for rapid development and correctness, but parser combinators have inherent overhead from closure dispatch and intermediate result construction.
- The `.memoized()` calls on recursive expressions (lines 747, 765) are essential for avoiding exponential time on deeply recursive grammars, but they allocate a memoization cache per parse.
- Rust's hand-written parser avoids this entirely through careful recursion structure.

---

## 5. Parallelization Analysis

### Saturnite

**Fully sequential.** No `rayon` dependency, no threading, no parallel iterators.

Pipeline (`main.rs:271-320`):
```
Source → Lexer → Parser → AST → HIR → verify → optimize → codegen → link
```

Each stage runs on a single thread. Functions within a module are lowered, verified, optimized, and codegen'd one at a time. The only "parallelism" is in the test harness (`tempfile::TempDir` for isolated test execution).

**Confirmed blocking issues from codebase audit:**
1. `SymbolInterner` is passed as `&mut` — shared mutable state blocks HIR/MIR parallelization
2. `LLVMContext` is not thread-safe — shared `module` + `builder` blocks codegen parallelization
3. No `rayon` dependency in `Cargo.toml`

### Rust

Uses a **jobserver-backed work-stealing scheduler** for codegen parallelism:

```rust
// rustc_codegen_ssa/src/back/write.rs:1019-1030
let jobserver_helper = cgcx.parallel.map(|_| {
    let coordinator_send2 = coordinator_send.clone();
    jobserver::client()
        .into_helper_thread(move |token| {
            drop(coordinator_send2.send(Message::Token::<B>(token)));
        })
});
```

Parallelization opportunities identified by Saturnite's own audit:

| Pipeline Stage | Parallelizable? | Saturnite Current | Rust Approach |
|---|---|---|---|
| HIR lowering | Per-function (`par_iter_mut`) after pre-interning | Sequential | `par_iter_mut(functions)` |
| MIR lowering | Per-function (`par_iter`) | Sequential — `MirLower::new` clones interner N times | Per-function |
| MIR verify | Per-function | Sequential | `par_iter` — embarrassingly parallel |
| Constant fold | Per-function | Sequential — stateless, embarrassingly parallel | Would be parallel |
| LLVM codegen | Per-function (per-LLVMContext) | Blocked — `LLVMContext` not thread-safe | Per-function LLVMContext + merge |
| Object emission | Per-function | Blocked | Per-function `TargetMachine::write_to_file` |

### Parallelization Maturity Matrix

| Capability | Rust Status | Saturnite Status | Effort to Match |
|---|---|---|---|
| Per-function parallel codegen | Production (jobserver + work-stealing) | Missing | High (per-LLVMContext isolation + merge) |
| Per-function parallel MIR passes | Per-function (MIR borrowck parallelization) | Missing | Low (add `rayon`, use `par_iter_mut`) |
| Per-function parallel lowering | Partial | Missing | Medium (fix SymbolInterner mutation) |
| Incremental compilation | Full (query system) | None | Very High |
| ThinLTO / FatLTO | Supported | None | High |

---

## 6. Prioritized Roadmap

| Priority | Item | Benefit | Complexity | Est. Size | Key Files |
|---|---|---|---|---|---|
| **1** | **Add rayon + parallelize MIR optimization and verification** | 2-4x compile speed on multi-core; ConstantFolder is already stateless and embarrassingly parallel | Low | Small (1-2 files) | `mir/opt.rs`, `mir/verify.rs`, `main.rs`, `Cargo.toml` |
| **2** | **Implement SimplifyCfg pass** (dead block elimination, block merging, unreachable edge removal) | 20-40% codegen quality improvement; reduces LLVM IR size significantly | Medium | Medium (port ~300 LOC from Rust's `simplify.rs`, adapt to Saturnite's `MirTerminator` set: `Goto`, `SwitchInt`, `Call`, `Return`) | New file `mir/passes/simplify_cfg.rs` |
| **3** | **Selective alloca — skip alloca for immediate types** | 15-30% reduction in LLVM IR; eliminates redundant stores/loads | Medium | Medium (rewrite `codegen.rs:146-173` to use `LocalRef`-style enum) | `mir/codegen.rs` |
| **4** | **Switch all `HashMap` to `FxHashMap` (add `rustc-hash` dep)** | 5-10x faster hash operations for symbol tables, sig maps, block maps | Low | Small (sed-style replace across 7 files) | `mir/codegen.rs`, `mir/lower.rs`, `hir/lower.rs`, `hir/symbol.rs`, `module.rs` |
| **5** | **Block ordering — reverse postorder over reachable blocks** | 10-20% LLVM optimization quality; better branch prediction | Medium | Small (compute RPO traversal, filter to reachable set) | `mir/codegen.rs` |
| 6 | Port **GVN** (Global Value Numbering) | Eliminate redundant loads across blocks | High | Large (2,214 LOC in Rust, would need significant adaptation) | New pass file |
| 7 | Port **InstSimplify** (enhanced constant folding) | Replace basic `ConstantFolder` with full identity/reassociation/cast folding | Medium | Medium (471 LOC → simplified port) | Replace `mir/opt.rs` |
| 8 | Port **CopyProp** + **DeadStoreElimination** | Eliminate copy assignments and dead stores | Medium | Small-Medium (156 + 194 LOC) | New pass file |
| 9 | Port **SROA** (Scalar Replacement of Aggregates) | Decompose struct allocas into scalar slots | High | Medium (435 LOC) | New pass file |
| 10 | Implement **pass manager** (MirPass trait, PassPolicy, opt-level gating) | Enables pass composition, profiling, configurable optimization levels | Medium | Medium | Refactor `mir/opt.rs` + `main.rs` |
| 11 | Parallelize **per-function HIR lowering** (fix SymbolInterner) | 2-4x lowering speed | Medium | Medium-High (requires interner immutability after pre-pass) | `hir/symbol.rs`, `hir/lower.rs`, `mir/lower.rs` |
| 12 | Per-function **LLVMContext isolation** | Parallel codegen across functions | High | Large (architectural: per-function module + llvm-link) | `mir/codegen.rs` |

### Phase Recommendations

**Phase 1 (immediate, 1-2 days):** Items 1 + 4 — both are pure mechanical improvements with no behavioral risk. Adding `rayon` and parallelizing the stateless constant folder is the single highest-ROI change. Switching to `FxHashMap` is a sed-style replace.

**Phase 2 (short-term, 1 week):** Items 2 + 3 + 5 — the three codegen quality improvements. These directly reduce LLVM IR size and improve optimization quality, with moderate implementation complexity.

**Phase 3 (medium-term, 2-4 weeks):** Items 6-10 — the heavy MIR optimization passes. These require porting or reimplementing substantial Rust code, with soundness implications (must be carefully validated against the Saturnite MIR semantics).

**Phase 4 (long-term, 2-3 months):** Items 11-12 — full parallelism, including parallel HIR/MIR lowering and per-function LLVM contexts with module merging. This is a major architectural change.

---

## 7. Key Codebase Locations

### Saturnite
- **MIR optimization:** `C:\Users\atimo\Saturnite\crates\stnx\src\mir\opt.rs` (163 lines, 1 pass)
- **MIR codegen:** `C:\Users\atimo\Saturnite\crates\stnx\src\mir\codegen.rs` (841 lines)
- **Symbol interner:** `C:\Users\atimo\Saturnite\crates\stnx\src\hir\symbol.rs` (186 lines)
- **Parser:** `C:\Users\atimo\Saturnite\crates\stnx\src\parser\mod.rs` (1,456 lines)
- **Pipeline entry:** `C:\Users\atimo\Saturnite\crates\stnx\src\main.rs` (lines 260-320)

### Rust
- **MIR pass list:** `C:\Users\atimo\rust\compiler\rustc_mir_transform\src\lib.rs` (843 lines, `declare_passes!` + `run_optimization_passes`)
- **MIR pass manager:** `C:\Users\atimo\rust\compiler\rustc_mir_transform\src\pass_manager.rs` (463 lines, `MirPass` trait, `PassPolicy`)
- **Codegen context:** `C:\Users\atimo\rust\compiler\rustc_codegen_ssa\src\mir\mod.rs` (700+ lines, `FunctionCx`, `LocalRef`)
- **Fast hashing:** `C:\Users\atimo\rust\compiler\rustc_data_structures\src\fx.rs` (41 lines, `FxBuildHasher`, `FxHashMap`)
- **Parallel codegen:** `C:\Users\atimo\rust\compiler\rustc_codegen_ssa\src\back\write.rs` (jobserver + work-stealing)
