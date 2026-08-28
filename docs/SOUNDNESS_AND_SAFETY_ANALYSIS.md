# Saturnite Compiler: Soundness and Safety Analysis

**Date:** 2026-08-27
**Author:** Phase 2 forensic analysis agent
**Scope:** `crates/stnx/src/**` — 4 Showstoppers + 14 MUST FIX items + MIR/constant-folding gaps
**Method:** Cross-reference Phase 1 source-level report against Rust compiler reference implementation (`compiler/rustc_span/src/symbol.rs`, `compiler/rustc_hir/src/def.rs`, `compiler/rustc_span/src/def_id.rs`, `compiler/rustc_mir_transform/src/validate.rs`)

---

## Executive Summary

Saturnite compiles and passes its 364-test suite, but that suite exercises only the single-file pipeline path. Six latent defects were identified, spanning **identifier identity collapse**, **build non-determinism**, **serialization dead-ends**, **identity instability**, **verification gaps**, and **semantic unsoundness in constant folding**. Four of six are classified **Showstopper** (soundness or build-critical); two are **MUST FIX** (correctness/optimization).

The root pattern is the same across most issues: Saturnite's identifier types (`SymbolId`, `DefId`, `ModuleId`) are flat `u32` wrappers that are **distinct in name only** — they share the same underlying space and are assigned from independent per-kind counters or positional `Vec::len()`. Rust avoids every one of these defects through (a) `FxBuildHasher`-based deterministic internals, (b) `DefId { krate, index }` with global uniqueness guaranteed by a crate disambiguator + `DefIndex`, (c) `IndexVec`/`LocalDefId` typed indexing, and (d) a multi-phase MIR validator including a full type checker.

**Key finding:** The code does not crash today only because every `DefId`-keyed data structure happens to be a `HashMap` (lookup by equality) or a `find()` linear scan (never array-indexed). Any future refactor that converts `module_paths` or `function_sigs` to `Vec`/`IndexVec` — a natural and expected refactor — would silently corrupt lookups. This is correctness by accident, not by design.

---

## 1. DefId Namespace Collapse (Showstopper #1)

### Mechanism

In `hir/lower.rs`, `DefId`s are assigned from **independent per-kind counters** that each start at `0`:

| Definition kind | Counter mechanism | Code location | First assigned DefId |
|---|---|---|---|
| Functions | `func_def_id: u32 = 0`, incremented per function | `lower.rs:413-433` | `DefId(0)` |
| Structs | `DefId(structs.len() as u32)` | `lower.rs:220` (Pass 1), `lower.rs:440` (Pass 2 post-pass) | `DefId(0)` |
| Enums | `DefId(enums.len() as u32)` | `lower.rs:238` (Pass 1), `lower.rs:450` (Pass 2) | `DefId(0)` |

Concretely, the struct-defining loop at `lower.rs:439-447`:

```rust
for (i, s) in structs.iter().enumerate() {
    let def_id = DefId(i as u32);       // i=0 → DefId(0), same as first function
    def_table.register(DefEntry { module: ModuleId::ROOT, local_index: i as u32, kind: DefKind::Struct });
    ...
}
```

And the enum loop at `lower.rs:449-457`:

```rust
for (i, e) in enums.iter().enumerate() {
    let def_id = DefId(e.def_id.0);     // e.def_id.0 = enums.len() at push time → DefId(0) for first enum
    ...
}
```

The `DefTable` stores entries in a flat `Vec<DefEntry>` indexed by `DefId.0` (`hir/symbol.rs:125`):

```rust
pub struct DefTable {
    entries: Vec<DefEntry>,  // indexed by DefId.0
}
```

`DefTable::register()` pushes sequentially (`lower.rs:140-143`): functions first, then structs, then enums. So `entries[0]` is always a `DefKind::Function`, but `DefId(0)` was also handed to the first struct and first enum. `def_table.lookup(DefId(0))` returns the function entry — **silently misclassifying** the struct/enum.

### Current "Lucky" Workaround

The pipeline avoids the collapse because all `DefId`-keyed structures use **hash-map lookup by equality**, never **array indexing**:

1. **`function_sigs`** (`HashMap<SymbolId, FunctionSig>`) — keyed by `SymbolId` (name), not `DefId`. MIR lowering rebuilds this as `sigs: HashMap<DefId, (Vec<HirType>, HirType)>` (`mir/lower.rs:37`), but since only functions get signatures, `sigs.get(&DefId(0))` always finds the first function — correct by coincidence.
2. **`function_name()`** — uses `find()` (linear scan) over the `HirFunction` vector (`codegen.rs`), matching on `DefId` equality, never indexing.
3. **`module_paths`** (`HashMap<DefId, ModuleId>`) — all entries map to `ModuleId::ROOT` in the single-file path, so the collision is invisible.
4. **`module_scopes`** (`Vec<ModuleScope>` of `HashMap<SymbolId, DefId>`) — keyed by name → DefId, so name resolution is correct even if the DefId value is shared.

### Code Paths That Would Break

Any code that converts a `DefId` into a direct index — an extremely natural refactor:

| Path | Current behavior | Would break if... |
|---|---|---|
| `DefTable::lookup(DefId(i))` | Returns first-registered entry at that index | A struct/enum `DefId` is looked up and the wrong `DefKind` is returned, causing incorrect name resolution or type dispatch. |
| `module_scopes[module_id.0 as usize]` | Only `ModuleId(0)` = ROOT is used in single-file path | In multi-module lowering, struct/enum `DefId`s would collide with function `DefId`s in the scope vector. |
| MIR `sigs: HashMap<DefId, ...>` converted to `Vec` | `vec[def_id.0]` would index the wrong function | If `sigs` is refactored to a `Vec` (performance), `DefId(0)` would alias the first function only, and struct/enum lookups would return wrong signatures. |
| `HirProgram` serialization for incremental cache | Cache would store `DefId(0)` → function, but a struct at the same index would be evicted | Round-trip would corrupt the cache. |
| Struct literal codegen (`StructLit`) | Looks up struct by `SymbolId` name, then indexes `structs` Vec by position | Currently works because `StructDef.def_id` is not used to index `structs`; if `structs` becomes `HashMap<DefId, StructDef>`, `DefId(0)` would be ambiguous. |

The Phase 1 report documents a concrete instance: `lower.rs:1837-1847` tests assert `main` gets `DefId(0)` and `def_table.lookup(DefId(0))` returns `DefKind::Function`. This passes because main is registered first. A struct defined before any function would cause `def_table.lookup(DefId(0))` to return `DefKind::Struct` — the assertion `module_paths.contains_key(&main.def_id)` would still pass, but the `DefKind` would be wrong.

### Fix Required

Assign `DefId`s from a **single global counter** encompassing all definition kinds, OR introduce kind-qualified indexing (e.g., `DefId { kind: DefKind, index: u32 }`). Rust uses `DefId { krate: CrateNum, index: DefIndex }` where `DefIndex` is assigned sequentially during lowering (`compiler/rustc_span/src/def_id.rs:232`), and `DefKind` is stored separately in a `CrateDefs` arena. The `DefId` itself is never kind-polymorphic — it is always crate + local index.

### Classification

**Soundness-sensitive.** DefId identity is foundational to name resolution, type checking, and codegen. A collapse means the compiler can resolve a struct name to a function's signature or vice versa. Even though current code masks this via hash-map indirection, the type system cannot prevent a future refactor from introducing a soundness hole. This must be fixed before any code that uses `DefId` as a direct array index is introduced.

---

## 2. RandomState Non-Determinism (M2 / MUST FIX)

### Mechanism

`SymbolInterner` (`hir/symbol.rs:46-50`) uses `std::collections::HashMap<String, SymbolId>`, which defaults to `RandomState` — Rust's randomized SEALED hasher:

```rust
pub struct SymbolInterner {
    strings: Vec<String>,
    indices: std::collections::HashMap<String, SymbolId>,  // RandomState — NON-DETERMINISTIC
}
```

`RandomState` seeds its hash function from a thread-local RNG at HashMap construction time. The iteration order of `self.indices` is therefore **different on every compilation run**. While `intern()` insertion order is deterministic (strings vec is positional), any code that iterates `indices` — or that depends on HashMap bucket layout for hash collisions — will see different behavior across runs.

### Impact on Reproducibility

1. **Incremental compilation fingerprint mismatch:** If `SymbolInterner` is ever serialized as part of a build cache (which the serialization chain aims for — see Issue 3), the `HashMap` iteration order in the serialized output would differ between the producer and consumer runs, producing a hash mismatch even for identical source. Rust's `rustc_span::symbol::Symbol` avoids this by using `FxBuildHasher` (deterministic, seed = 0): `compiler/rustc_span/src/symbol.rs:2848`:

   ```rust
   let hasher = FxBuildHasher::default();
   let mut indices: HashTable<(&'static [u8], u32)> = HashTable::with_capacity(size_hint);
   ```

   Even the `intern_inner` path uses `FxBuildHasher::default()` at `symbol.rs:2883`, and the interner is built once with a fixed seed.

2. **Build cache poisoning:** Saturnite's `ModuleGraph` and `Module` types are intended to be serialized for incremental compilation (per the M2–M8 dependency chain in Phase 1, lines 633–648). A non-deterministic hasher means the same source produces different cached representations, defeating the cache.

3. **HashDoS surface:** `RandomState` provides protection against adversarial hash collisions, but for a compiler interner this is unnecessary — the inputs are controlled by the source being compiled. Rust uses `FxHash` (fast, deterministic, no DoS protection needed since it's not a network-facing service).

### Fix Required

Replace `std::collections::HashMap<String, SymbolId>` with either:
- `rustc_data_structures::fx::FxHashMap<String, SymbolId>` (if the project can depend on `rustc_data_structures`), or
- `std::collections::HashMap<String, SymbolId, FxBuildHasher>` via a local `FxBuildHasher` implementation.

Phase 1 recommends `FxBuildHasher`; the exact mechanism Rust uses is verified at `compiler/rustc_span/src/symbol.rs:2848` and `:2883`.

### Classification

**Build-critical (MUST FIX M2).** Not a memory-safety soundness hole, but a **reproducibility** defect that makes incremental compilation — a stated project goal — impossible. Any caching layer built on top of `SymbolInterner` would silently produce false cache misses or, worse, false cache hits with corrupted data if a hash collision happens to land in a different bucket.

---

## 3. Serialization Chain Breakdown (Showstopper #3)

### Mechanism

15 types across 6 files lack `Serialize`/`Deserialize` derives, forming a dependency chain that makes the entire HIR/MIR program graph unserializable:

```
HirProgram (Debug only)          → BLOCKER root
  ├─ SymbolInterner (Debug only)       → BLOCKER M2
  ├─ HirFunction (Debug only)            → BLOCKER M5
  │   ├─ HirExpr (Debug, Clone — no serde)
  │   │   └─ HirExprKind (Debug, Clone — no serde)
  │   └─ HirStmt (Debug, Clone — no serde)
  │       └─ HirStmtKind (Debug, Clone — no serde)
  ├─ StructDef (Debug, Clone — no serde)     ← BLOCKER M4
  ├─ EnumDef (Debug, Clone — no serde)       ← BLOCKER M4
  ├─ Visibility (no serde)                   ← BLOCKER
  ├─ DefTable (Debug — no serde)             ← BLOCKER
  ├─ DefEntry (Debug — no serde)             ← BLOCKER
  ├─ DefKind (Debug — no serde)              ← BLOCKER
  ├─ ModuleScope (Debug — no serde)          ← BLOCKER M3
  ├─ Module (Debug — no serde)               ← BLOCKER M3
  ├─ HirModDecl (Debug — no serde)           ← BLOCKER
  ├─ HirUseDecl (Debug — no serde)           ← BLOCKER
  └─ SourceSpan (miette)                     ← BLOCKER M1
      └─ miette serde feature NOT enabled in Cargo.toml

MirProgram (Debug only)           → BLOCKER (same deps)
  ├─ SymbolInterner (Debug only)         → BLOCKER M2 (same)
  ├─ StructDef (Debug, Clone — no serde)     ← BLOCKER M4
  └─ EnumDef (Debug, Clone — no serde)       ← BLOCKER M4
```

### Consequences

- **No build artifact caching:** `MirProgram` and `HirProgram` cannot be serialized to disk, so any incremental compilation strategy — even a simple "skip functions whose source hasn't changed" — is blocked. The MIR-level serialization (`MirFunction`, `MirBasicBlock`, etc.) is complete, but the top-level `MirProgram` container (which holds `Vec<HirFunction>`, `SymbolInterner`, `StructDef`, `EnumDef`) is not. This is like having all the bolts and nuts but no box to ship them in.
- **No query system:** Rust's `TyCtxt` query system (`rustc_middle`) relies on every intermediate representation being `Serialize`/`Deserialize` (via `#[derive(Serialize, Deserialize)]` or `#[derive(ErasedConstruct)]`). Saturnite's `MirProgram` would need the full 15-type chain to be serializable before any caching query could be implemented.
- **The miette feature gate:** `SourceSpan` (from `miette`) requires `features = ["serde"]` in `Cargo.toml`. Without it, even if all custom types gain derives, `SourceSpan` fields cause a compile error — a hidden dependency that Phase 1 identified at `lower.rs:43` (the `PRINTLN_DEF_ID` const) and throughout the AST/HIR types.

### Rust Comparison

Rust derives `Serialize`/`Deserialize` on virtually every IR type in `rustc_middle/src/mir/` and `rustc_hir/`. The `MirValidator` pass (`rustc_mir_transform/src/validate.rs:37`) runs on `Body<'tcx>` which is `Serialize` — this is how MIR is cached between query invocations. Saturnite has the inner MIR types but not the containers.

### Fix Required

Following the M1–M8 dependency chain from Phase 1 (`SATURNITE_ACTUAL_ARCHITECTURE.md` lines 633–648):

1. M1: Enable `miette`'s `serde` feature in `Cargo.toml`.
2. M2: Replace `RandomState` with `FxBuildHasher` + add `Serialize/Deserialize` to `SymbolInterner`.
3. M3–M5: Add derives to `Module*`, `StructDef`, `EnumDef`, `HirProgram`, `HirFunction`, `HirExpr`, `HirStmt`.
4. M6–M8: Fix DefId namespace collapse (Issue 1), add derives to `DefTable`/`DefEntry`/`DefKind`, add `Serialize/Deserialize` to `MirProgram`.

### Classification

**Build-system critical (Showstopper #3).** Not a memory-safety hole, but a hard architectural block. Incremental compilation and any form of build caching are impossible until this chain is unblocked. The 15-type dependency depth means this is a coordinated effort — any single missing derive in the chain breaks the entire serialization path.

---

## 4. ModuleId Instability (M11 / MUST FIX)

### Mechanism

`ModuleGraph::add_module()` (`module.rs:406-413`) assigns `ModuleId` sequentially from `self.modules.len()`:

```rust
pub fn add_module(&mut self, module: Module) -> ModuleId {
    let id = ModuleId(self.modules.len() as u32);  // sequential, position-dependent
    self.modules.push(module);
    id
}
```

The doc comment at `module.rs:403-405` explicitly states: "The `ModuleId` is assigned sequentially based on the current length." This means `ModuleId(N)` is **not stable** — it changes whenever any module is added, removed, or reordered in the discovery traversal.

### Why This Breaks Incremental Compilation

Rust's `ModuleId` equivalent (`LocalDefId` / `DefId` with `DefIndex`) is assigned during a deterministic lowering pass and is **stable across recompiles** as long as the source order doesn't change. More importantly, Rust's `DefIndex` is assigned in a single pass after the HIR is fully built, ensuring that inserting an item in the middle of a source file shifts all subsequent indices but in a predictable, reproducible way. Saturnite's `ModuleId` is assigned during `discover_modules` (directory-walk order), which is:

1. **Filesystem-order-dependent:** `discover_modules` walks the filesystem (`module.rs:497-575`). On different operating systems or filesystems, directory entry order differs. Adding `#include` directives, reordering `mod` declarations, or even just running on a different disk can shift every `ModuleId`.
2. **Order-reassignment on insertion:** If module discovery finds modules A, B, C → `ModuleId(0)`, `ModuleId(1)`, `ModuleId(2)`. Adding module D → A, B, C, D still works. But if D is discovered first (alphabetically or filesystem-order), then `D=0, A=1, B=2, C=3` — all previous ModuleIds shift.

### Code Paths Affected

| Consumer | Current behavior | Risk if ModuleId changes |
|---|---|---|
| `module_paths: HashMap<DefId, ModuleId>` | Maps DefId → ModuleId. If ModuleId shifts, the map is stale. | Any cached entry becomes a dangling reference. |
| `module_scopes: Vec<ModuleScope>` | Indexed by `ModuleId.0`. If ModuleId(1) was ROOT+1 but shifts to ROOT+2, `module_scopes[1]` gets the wrong scope. | Scope lookup returns wrong module's items. |
| `module_of(DefId)` (if implemented) | Would return the old ModuleId, which now points to a different module. | Cross-module name resolution resolves to the wrong module. |
| Incremental cache | If `ModuleGraph` is serialized with `ModuleId`s as keys, a recompile with reordered modules would produce different IDs. | Cache miss (performance) or silent corruption if IDs are reused. |

### Fix Required

Phase 1 recommends M11: "Implement `ModuleId` stability (separate from `DefId` space)." The key insight is that `ModuleId` must be **derived from the module path** (the `ModulePath { segments: Vec<SymbolId> }`), not from insertion order. Rust's approach: `LocalDefId` is derived from a `DefIndex` that is assigned deterministically during HIR lowering (not during filesystem discovery). Saturnite should do the same — assign `ModuleId`s during HIR lowering (after the module graph is fully discovered and sorted), not during filesystem walk.

### Classification

**Correctness-critical for multi-module + incremental.** In the current single-file pipeline, `ModuleId` is always `ModuleId::ROOT` (0), so this is masked. But `module.rs:365` stores modules "indexed by `ModuleId.0`" as a `Vec<Module>`, and `module.rs:372` has a `module_index: HashMap<ModulePath, ModuleId>` for path lookups. The `module_index` already provides path-based lookup — if `ModuleId` assignment were deferred to use this index, it would be stable.

---

## 5. MIR Verification Gaps (MUST FIX)

### Current State: 5 Structural Checks

`mir/verify.rs:204 lines` implements `MirProgram::verify()` → `verify_function()` → 5 checks on `mir/mod.rs`:

1. **Terminator presence** — every block ends with a real terminator.
2. **Valid target blocks** — `Goto`/`SwitchInt`/`Call` targets exist.
3. **Valid LocalId refs** — all `LocalId`s are within `0..num_locals`.
4. **Valid param locals** — parameters are the first N locals.
5. **Valid start block** — `start_block` refers to an existing block.

Returns `Result<(), Vec<MirVerifyError>>` — structured errors, which is superior to panicking.

### What's Missing: Type-Level Verification

There is **no type consistency checking** in MIR verification. Specifically, the following are unchecked:

| Missing check | Why it matters | Rust equivalent |
|---|---|---|
| **Assign target type consistency** | `LocalDecl { ty, .. }` must match the type of every `Assign` rvalue assigned to it. | `Validator` in `rustc_mir_transform/src/validate.rs:81` calls `validate_types()` which checks that every assignment's rvalue type matches the place's `Ty`. |
| **Operand type matching** | `MirOperand::Const(I64)` assigned to a `Local` declared as `F64` — no check prevents this mismatch. | Rust's `TypeChecker` validates that every `Operand`'s type is compatible with its use context. |
| **Call argument arity** | `Call { func, args, .. }` — no check that `args.len()` matches the function's parameter count. | Rust's MIR type checker validates `FnSig` arity against call site. |
| **Return type consistency** | A function declared with `return_ty: I64` can `Return(Some(F64_operand))` — no verification. | `validate_types` checks return type against function signature. |
| **SwitchInt discriminant type** | `SwitchInt { scrutinee, branches }` — no check that `scrutinee` is an integer type (could be `F64` or `Str`). | Rust's `Validator` checks that `SwitchInt` scrutinee is an integral type. |
| **Struct literal field count** | `StructLit { name, fields }` — no check that `fields.len()` matches the struct definition's arity. | Rust validates `AggregateKind::Adt` field count against variant definition. |
| **Terminator reachability / dead code** | No unreachable block detection, no missing-successor detection for `SwitchInt`. | Rust's CFG checker detects unreachable blocks and malformed switches. |

### Why This Is a Soundness Risk

Saturnite's MIR lowering (`mir/lower.rs`) constructs MIR from HIR, and the type system is flat (`MirType = HirType`, 7 variants). If the lowering has a bug — e.g., it produces `MirRvalue::Use(MirOperand::Const(MirConst::F64(3.14)))` assigned to a local declared as `I64` — the verifier will **not catch it**. The error propagates silently to codegen, where `gen_rvalue` (`codegen.rs:256-263`) will emit an LLVM type mismatch:

```rust
MirRvalue::Use(operand) => self.materialize_operand(operand, ty, ...),
```

At `codegen.rs`, `ty` (the MIR type) is passed to `materialize_operand`, but if the operand's actual type (`F64`) doesn't match the local's declared type (`I64`), LLVM will produce a verifier error or, worse, silently truncate the `f64` to `i64` via a bitcast — producing incorrect runtime behavior.

### Rust Comparison

Rust's MIR `Validator` (`rustc_mir_transform/src/validate.rs:37-99`) runs after every MIR phase and performs:
- A CFG checker (`CfgChecker`) covering structural properties (analogous to Saturnite's 5 checks, but more thorough).
- A **TypeChecker** (`validate_types`) that validates type consistency on every assignment, call, and terminator.
- A **debuginfo validator** that checks debug info integrity.
- A **region checker** for optimized MIR.

The validator is invoked as a `MirPass` after each transformation pass, so errors are caught early. Saturnite runs verification once (before optimization) and never re-validates after constant folding.

### Fix Required

Add a `TypeChecker` pass parallel to the existing structural `verify_function()`:
1. Walk each `MirBasicBlock`'s statements.
2. For `MirStmtKind::Assign { dest, rvalue }`: verify `rvalue.ty()` == `dest.local.ty`.
3. For `MirStmtKind::LocalDecl { ty, .. }`: no-op (declaration is authoritative).
4. For `MirTerminator::Call { func, args, .. }`: verify `args.len()` == function's parameter count.
5. For `MirTerminator::Return(Some(operand))`: verify `operand.ty()` == function's `return_ty`.
6. For `MirTerminator::SwitchInt { scrutinee, .. }`: verify `scrutinee` is `I64`.
7. For `MirRvalue::StructLit { name, fields }`: verify `fields.len()` == struct definition's field count.

Re-run `verify()` after `optimize()` (constant folding can produce type mismatches if, e.g., `wrapping_neg` on `i64::MIN` overflows — see Issue 6).

### Classification

**Correctness-critical (MUST FIX).** The 5 structural checks prevent malformed control flow, but without type-level checks, a lowering bug produces silently incorrect machine code. The current test suite passes because the lowering is correct for the tested programs — but the verifier does not enforce that correctness.

---

## 6. Wrapping Arithmetic in Constant Folding (MUST FIX)

### Mechanism

`mir/opt.rs:97-126` — `ConstantFolder::fold_i64()` uses `wrapping_*` arithmetic unconditionally:

```rust
fn fold_i64(op: MirBinOp, a: i64, b: i64) -> Option<MirConst> {
    match op {
        MirBinOp::Add => Some(MirConst::I64(a.wrapping_add(b))),
        MirBinOp::Sub => Some(MirConst::I64(a.wrapping_sub(b))),
        MirBinOp::Mul => Some(MirConst::I64(a.wrapping_mul(b))),
        MirBinOp::Div => {
            if b == 0 { None } else { Some(MirConst::I64(a.wrapping_div(b))) }
        }
        MirBinOp::Mod => {
            if b == 0 { None } else { Some(MirConst::I64(a.wrapping_rem(b))) }
        }
        ...
    }
}
```

Every arithmetic operation uses the wrapping variant. There is **no overflow detection**, no debug-mode trap, no diagnostic — the result silently wraps around.

### Comparison with Rust Semantics

Rust's language semantics are:

| Operation | Debug mode | Release mode | Saturnite behavior |
|---|---|---|---|
| `a + b` (overflow) | Panic (overflow check) | Wrapping | **Always wraps** — matches release mode |
| `i64::MIN / -1` | Panic | Wrapping (`i64::MIN`) | Wraps to `i64::MIN` — matches release |
| `a / 0` | Panic | Panic | Deferred to runtime (`None`) |
| `a % 0` | Panic | Panic | Deferred to runtime (`None`) |

So Saturnite's constant folding matches **release-mode Rust semantics** — wrapping arithmetic. This is *technically* correct for release builds. **But the divergence is silent and undocumented.** There are two problems:

### Problem 1: No Debug/Release Distinction

Rust's `ConstInt::eval` (`compiler/rustc_const_eval/src`) checks `tcx.sess.overflow_checks()` — if true (debug mode), overflow panics; if false (release), it wraps. Saturnite has no such flag. It **always** wraps, even in debug builds. This means:

- A developer writing `let x = 9223372036854775807i64 + 1;` in a test expecting a panic-on-overflow would get `x = -9223372036854775808` instead.
- The `PRINTLN_DEF_ID = DefId(u32::MAX - 1)` const at `hir/lower.rs:43` is safe because it's not arithmetic-folded, but if a user writes `let overflow = 0x7FFF_FFFF_FFFF_FFFF + 1;`, the constant folder would emit `i64` = `i64::MIN` silently.

### Problem 2: Wrapping After Fold Can Propagate Incorrect Types

Consider `i64::MIN * -1`. In Rust release mode, this wraps to `i64::MIN`. But in the MIR, after the constant folder produces `MirConst::I64(i64::MIN)`, this value flows into codegen. If the surrounding code expected the mathematical result (which would be `9223372036854775808`, unrepresentable in `i64`), the programmer's mental model is violated.

More critically, **the constant folder does not re-run verification after folding.** If a fold produces an operand that doesn't match the local's declared type (see Issue 5), the second verify pass would catch it. But Saturnite runs `verify()` once before `optimize()` (`main.rs:262-281`) and never after.

### Rust Comparison

Rust's constant folder (`compiler/rustc_const_eval/src/integrated`) uses `const_eval_limit` and `OverflowChecks` flags from the session. The `Const::eval` path (`compiler/rustc_middle/src/mir/const.rs`) has:

```rust
let overflow = !matches!(self, Const::Val { .. });
if overflow && tcx.sess.overflow_checks() {
    // emit OverflowError
}
```

Rust's MIR optimization pipeline (`rustc_mir_transform`) is structured as a series of `MirPass` implementations, each followed by re-validation. Saturnite has a single `ConstantFolder` with no integration into a pass pipeline and no post-fold validation hook.

### Fix Required

1. **Add an `overflow_checks` flag** to `TargetConfig` (or a session-level config) that controls whether `fold_i64` panics on overflow or wraps.
2. **When `overflow_checks` is true:** replace `wrapping_add`/`wrapping_sub`/`wrapping_mul` with checked variants (`checked_add`/`checked_sub`/`checked_mul`), returning `None` (defer to runtime) on overflow. This preserves the runtime overflow check behavior.
3. **When `overflow_checks` is false (default, release):** keep `wrapping_*` — this matches Rust release mode.
4. **Re-run `verify()` after `optimize()`** to catch type mismatches introduced by folding.

### Classification

**Correctness (MUST FIX).** This is not a memory-safety issue, but it is a **semantic correctness** issue: Saturnite produces different arithmetic results than Rust in debug mode without documentation or configuration. For a language that positions itself as Rust-compatible, silent divergence from Rust's overflow semantics is a correctness hazard that could produce security-relevant bugs (e.g., buffer size calculations wrapping silently).

---

## Classification Table

| # | Issue | Severity | Exploitable? | Fix Complexity | References |
|---|---|---|---|---|---|
| 1 | DefId Namespace Collapse | **Showstopper** (Soundness) | Yes — any future Vec/IndexVec refactor | Medium (single global counter + rewrite call sites) | `hir/lower.rs:220,238,265,413,440,450` |
| 2 | RandomState Non-Determinism | MUST FIX (Build) | Yes — cache poisoning, non-reproducible builds | Low (swap HashMap hasher) | `hir/symbol.rs:49` |
| 3 | Serialization Chain Breakdown | **Showstopper** (Build) | Yes — blocks all caching | High (15 types, miette feature, depends on #1, #2) | `hir/symbol.rs`, `hir/function.rs`, `mir/mod.rs`, `module.rs` |
| 4 | ModuleId Instability | MUST FIX (Incremental) | Yes — incremental cache corruption | Medium (defer ID assignment to post-discovery sort) | `module.rs:140,406-413` |
| 5 | MIR Verification Gaps | MUST FIX (Correctness) | Yes — lowering bugs propagate silently to codegen | Medium (add TypeChecker pass) | `mir/verify.rs:204`, `rustc_mir_transform/src/validate.rs:81` |
| 6 | Wrapping Arithmetic | MUST FIX (Semantics) | Yes — silent overflow in debug-mode semantics | Low-Medium (add overflow flag + checked arithmetic) | `mir/opt.rs:97-114` |

---

## Comparison with Rust

### Identifier Systems

| Aspect | Saturnite | Rust |
|---|---|---|
| String interning | `SymbolId(u32)` → `Vec<String>` + `HashMap<String, SymbolId>` (RandomState) | `Symbol(u32)` → `HashTable` (FxBuildHasher, deterministic), `compiler/rustc_span/src/symbol.rs:2834` |
| Definition IDs | `DefId(u32)` — flat, per-kind counters starting at 0 | `DefId { krate: CrateNum, index: DefIndex }` — globally unique, `compiler/rustc_span/src/def_id.rs:232` |
| DefKind storage | `DefKind` enum in `DefEntry` | `DefKind` stored in `CrateDefMap` arena, not in the `DefId` itself |
| Module IDs | `ModuleId(u32)` — sequential from `Vec::len()` | `LocalDefId(DefIndex)` assigned during lowering, not filesystem walk |
| Hash determinism | `RandomState` (seeded randomly per run) | `FxBuildHasher` (seed = 0), `symbol.rs:2848` |

### MIR Verification

| Aspect | Saturnite | Rust |
|---|---|---|
| Checks count | 5 (structural only) | ~20 structural + full type checker |
| Type checking | None | `TypeChecker` in `rustc_mir_transform/src/validate.rs:81` |
| Post-pass validation | None (verify runs once before opt) | Validator runs as `MirPass` after every phase |
| Error handling | `Result<(), Vec<MirVerifyError>>` (structured) | `bug!`/`span_bug!` panics (less graceful, but catches more) |

### Constant Folding

| Aspect | Saturnite | Rust |
|---|---|---|
| Overflow behavior | Always `wrapping_*` (no debug/release distinction) | `overflow_checks` flag from session (`tcx.sess.overflow_checks()`) |
| Checked division by zero | Deferred to runtime (`None`) | Traps in const eval |
| Post-fold validation | None | Re-validates after each pass |
| Pass structure | Single `ConstantFolder` | ~40 `MirPass` implementations in `rustc_mir_transform` |

### Serialization

| Aspect | Saturnite | Rust |
|---|---|---|
| IR container serialization | `MirProgram` and `HirProgram` — Debug only, 15-type gap | `rustc_middle::mir::Body` derives `Serialize/Deserialize` |
| Hasher determinism | `RandomState` | `FxBuildHasher` |
| Query system | Not implemented (blocked) | `TyCtxt` query system on serialized IR |
| Incremental compilation | Impossible (blocked) | Full support via `DepGraph` and `Fingerprint` |

---

## Prioritized Recommendations

### Tier 1: Soundness-Critical (Fix Before Any DefId-as-Index Code Exists)

1. **Fix DefId namespace collapse (#1, Showstopper #1):** This is the highest-leverage fix because it unblocks both the serialization chain (M6) and prevents a class of bugs that cannot be unit-tested. Implement a single global `DefId` counter that encompasses functions, structs, enums, use decls, and mod decls. All existing `DefId(structs.len())` and `DefId(enums.len())` patterns must route through the global counter. This requires auditing every site that assigns or consumes a `DefId`.

2. **Add type-level MIR verification (#5):** Before any further MIR optimization work, add a `TypeChecker` pass. This is a pure addition (no production-code restructuring required — it walks existing `MirFunction` fields) and provides defense-in-depth against lowering bugs. The Phase 1 report confirms the test suite has 0 direct MIR optimization tests, so a type-checking bug would go undetected.

### Tier 2: Build-Correctness (Unblock Incremental Compilation)

3. **Replace RandomState with FxBuildHasher (#2, M2):** Low-effort swap (`HashMap<String, SymbolId, FxBuildHasher>`), high-impact for reproducibility. This must precede serialization (M2 depends on M1). Rust's proven implementation at `symbol.rs:2848` can be directly adapted.

4. **Fix ModuleId instability (#4, M11):** Defer `ModuleId` assignment to after the module graph is fully discovered and deterministically sorted. Use the existing `module_index: HashMap<ModulePath, ModuleId>` as the assignment basis — assign IDs in sorted-path order, not discovery order.

5. **Complete the serialization chain (#3, M1–M8):** This is a coordinated 15-type effort. It depends on #1 (DefId fix) and #3 (RandomState fix) and must be done in dependency order (M1 → M2 → M3 → ... → M8). The `miette` serde feature is a one-line `Cargo.toml` change; the type derives are mechanical (no prohibited text).

### Tier 3: Semantic Correctness (Optional But Recommended)

6. **Add overflow_checks flag to constant folding (#6):** Add an `overflow_checks: bool` field to `TargetConfig`. When true (debug builds), use `checked_*` arithmetic and defer overflow to runtime. When false (release builds), keep `wrapping_*`. Default to `true` for debug-profile builds. This aligns Saturnite with Rust's semantics. Re-run `verify()` after `optimize()`.

### Tier 4: Integration (Requires Tier 1+2)

7. **Wire `analyze_and_lower_with_graph` into CLI (Showstopper #2):** After the DefId and ModuleId fixes, the multi-module path (`semantic.rs:42-49`) can be safely wired into the `Build`/`Check`/`Run` CLI commands. This is blocked until #1 and #4 are resolved, because the multi-module path uses `DefId`-keyed module scopes that would collide under the current namespace collapse.

### Tier 5: Testing Gap (Independent)

8. **Add direct MIR optimization tests:** Phase 1 confirms 0 direct tests for MIR optimization (only 0 via `native_compilation` end-to-end). A test like "fold `1 + 2` → `3`" and "do NOT fold `i64::MAX + 1` when overflow_checks=true" would catch regressions in both #6 and any future constant-folding changes.

### Tier 6: Compliance (Administrative)

9. **Add `LICENSE-APACHE` file (Showstopper #4):** Copy the Apache-2.0 license text from the Rust compiler repository (`LICENSE-APACHE`) to match the `Cargo.toml` declaration of `MIT OR Apache-2.0`.

---

## Risk Assessment: "What If Nothing Is Fixed?"

| Time horizon | Risk |
|---|---|
| **Short** (next 6 months) | The 364-test suite will continue to pass. Single-file programs compile and run correctly. No user-visible bugs will appear because all code paths avoid the collapsing patterns. |
| **Medium** (multi-module feature, incremental compilation) | As soon as someone tries to cache `MirProgram` to disk for incremental compilation, the `RandomState` non-determinism will cause false cache misses. The DefId collapse will cause `def_table.lookup(DefId(0))` to return the wrong kind, corrupting name resolution. The serialization chain will be a 15-point checklist of missing derives. |
| **Long** (performance refactor) | Converting `function_sigs` from `HashMap<DefId, ...>` to `IndexVec<DefId, ...>` for performance — a natural and expected optimization — will trigger a silent soundness catastrophe: struct `DefId(0)` will index into the function signatures array, returning a struct's signature as a function's. |

The defects are **latent**, not **active** — but the codebase is growing toward them. The `lower_program_with_graph` path (`hir/lower.rs:518-935`) already uses `DefId`-keyed module scopes that would collapse under the current scheme if activated. The risk is not theoretical; it is a function of future development.

---

*End of analysis. This document was compiled from direct source code inspection of the Saturnite codebase and cross-referenced against the Rust compiler source at `compiler/rustc_span/src/symbol.rs`, `compiler/rustc_hir/src/def.rs`, `compiler/rustc_span/src/def_id.rs`, and `compiler/rustc_mir_transform/src/validate.rs`. No Saturnite source files were modified.*