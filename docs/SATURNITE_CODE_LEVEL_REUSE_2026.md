# SATURNITE — CODE-LEVEL REUSE INVESTIGATION (Phase 5)

> For every ADAPT/PORT (C) or FUSE (D) classification in
> `SATURNITE_RUST_SIDE_BY_SIDE_2026.md`, this document locates the
> actual Rust source files, records their dependencies, identifies
> the assumptions they make, and concludes with a brutally
> realistic assessment of what can actually be reused.

The output of Phase 4 was that there are **zero C. ADAPT/PORT**
candidates and **four D. FUSE** candidates (one is "D. FUSE later"
not "D. FUSE now"). This document analyzes each of the four in
detail.

---

## F1. `rustc_data_structures::Interned` (D. FUSE — later, when interned types arrive)

### Files (actual)

- `compiler/rustc_data_structures/src/intern.rs` (180 lines) —
  the `Interned<'a, T>` newtype and the `Interner` trait.
- `compiler/rustc_data_structures/src/stable_hash.rs` (~600 lines)
  — used by `Interned::stable_hash` (a separate concern from
  `Hash`).
- `compiler/rustc_data_structures/src/lib.rs` (re-exports
  `pub mod intern`).

### Definition (verbatim, `intern.rs:35-45`)

```rust
pub struct Interned<'a, T>(pub &'a T, pub private::PrivateZst);
```

Constructed only via `new_unchecked` (line 70):

```rust
pub const fn new_unchecked(t: &'a T) -> Self {
    Interned(t, private::PrivateZst)
}
```

`PartialEq` is **pointer equality** (lines 88-94). `Hash` is
**pointer hash** (lines 100-104). `Copy`, `Deref<Target = T>`,
`Clone`.

### Dependencies

- `rustc_data_structures::stable_hash` (same crate).
- **No `rustc_middle`, `rustc_session`, `rustc_span`, or any other
  compiler crate.** This is a true reusable abstraction.

### Rust-specific assumptions

- Uses `#[rustc_pass_by_value]` (an in-tree attribute; not in
  crates.io or stable Rust). This is a micro-optimization for
  pass-by-value generic params. **Removing this attribute
  produces a fully working version.**
- `PrivateZst` is a private ZST used to make construction auditable
  — it is a small structural change, not a semantics change.
- The `Interner` trait (in the same file) is generic over the
  context — it requires a `TyCtxt`-like context type, **but the
  `Interned<'a, T>` newtype itself does not**.

### What Saturnite would actually port

```rust
pub struct Interned<'a, T>(*const T, PhantomData<...>);
// impl Clone, Copy, Hash, PartialEq, Eq, Debug
// impl Deref, AsRef
// + remove the #[rustc_pass_by_value] attribute
```

This is **~40 lines of Rust code**, MIT/Apache-2.0, with a clear
provenance. The trait machinery (`Interner`, `InternedIntepreter`)
would not be ported in the first pass — Saturnite would just get
the newtype.

### Adaptation difficulty

**Low.** The newtype is intentionally designed to be portable.

### When to do it

When Saturnite introduces interned types for `HirType` (i.e.
when generics are added). At that point, the simplest design is:
- `HirTypeKind` becomes the `T` in `Interned<'a, HirTypeKind>`.
- `HirProgram` owns the `Vec<HirTypeKind>` arena.
- All codegen / MIR uses `HirType = Interned<'_, HirTypeKind>`.

This gives Saturnite pointer-equality type comparison for free
without inventing a newtype.

### Risk

**Low.** The newtype is small, portable, and does not depend on
the rest of rustc.

---

## F2. `rustc_mir_dataflow::framework` (D. FUSE — later, when a dataflow analysis is needed)

### Files (actual)

- `compiler/rustc_mir_dataflow/src/framework/` — directory
  containing the modular framework. Key files (per
  `rustc_mir_dataflow/src/lib.rs:14-19`):
  - `framework/mod.rs` (re-exports the framework API)
  - `framework/graphs.rs` (dataflow graph abstractions)
  - `framework/visitors.rs` (results visitor pattern)
  - `framework/lattice.rs` (JoinSemiLattice trait)
  - `framework/visit.rs` (visit results)
  - `framework/graphviz.rs` (graphviz export)
  - `framework/fmt.rs` (display)

### The framework API (from `lib.rs:9-14`)

```rust
pub use self::framework::{
    Analysis, Backward, Direction, EntryStates, Forward, GenKill, JoinSemiLattice,
    MaybeReachable, Results, ResultsCursor, ResultsVisitor, SwitchTargetIndex, fmt,
    graphviz, lattice, visit_results,
};
```

### Dependencies (per `rustc_mir_dataflow/Cargo.toml`)

- `polonius-engine = "0.13.0"`
- `regex = "1"`
- `rustc_abi` (in-tree)
- `rustc_data_structures` (in-tree)
- `rustc_errors` (in-tree)
- `rustc_graphviz` (in-tree)
- `rustc_hir` (in-tree) — **tied to rustc's HIR**
- `rustc_index` (in-tree)
- `rustc_macros` (in-tree)
- `rustc_middle` (in-tree) — **tied to rustc_middle::mir + ty**
- `rustc_span` (in-tree) — **tied to rustc spans**
- `smallvec`, `tracing`

### Rust-specific assumptions

- The `framework` is **generic** over the `Analysis` trait; the
  trait's associated types reference `rustc_mir::BasicBlock`,
  `rustc_mir::Location`, and `rustc_mir::Statement` /
  `rustc_mir::Terminator`.
- The framework does **not** depend on `ty::TyCtxt` directly, but
  the analyses that **use** the framework (e.g.
  `drop_flag_effects`, `move_paths`) do.

### What Saturnite would actually port

The `framework/` directory's `Analysis` trait, `JoinSemiLattice`,
`Forward`/`Backward` direction markers, `Results` storage, and
`GenKill` shape — re-parameterized to Saturnite's `BlockId` /
`LocalId` / `MirStmt` types. This is a **type-parameter
substitution**, not a code change.

Estimated port: **~2 000 lines of framework code**, mostly the
`framework/` subdirectory and `impls/`-like helpers.

### Adaptation difficulty

**High.** The framework is conceptually portable, but the API
uses rustc's `BasicBlock` / `Location` / `Statement` types
directly. Substituting Saturnite's types would mean either:

(a) **Fork** the framework into a new `stnx-mir-dataflow` crate
with Saturnite's `BlockId` / `LocalId` types — high effort, but
the result is reusable.

(b) **Make the framework generic** over the basic-block / local
types — possible but a non-trivial refactor of `rustc_mir_dataflow`
itself, which Saturnite cannot do (it's in the rustc repo).

(c) **Write a small custom dataflow helper** of ~300 lines that
does only what Saturnite needs (e.g. liveness for register
allocation, or reachability for dead-code elimination). Lower
effort, lower benefit.

### Recommendation

**Defer.** When Saturnite first needs a dataflow analysis
(e.g. a simple liveness analysis for an SSA-conversion pass), do
**(c)**: write a 300-line custom helper. Revisit **(a)/(b)** when
the analysis count exceeds ~5.

### Risk

**Medium.** The framework is small enough to fork but large
enough that "fork" should be a deliberate decision.

---

## F3. `src/tools/compiletest` (D. FUSE — later, when UI/snapshot tests are needed)

### Files (actual)

- `src/tools/compiletest/Cargo.toml` — name `compiletest`,
  version `0.0.0`, edition `2024`, license inherited from
  the `src/tools/**` REUSE blanket (MIT/Apache-2.0).
- `src/tools/compiletest/src/lib.rs` (top of the framework).
- `src/tools/compiletest/src/runtest/` — per-test-type runners
  (codegen, ui, run-make, rustdoc, mir-opt, …).
- `src/tools/compiletest/src/directives/` — `//~ ERROR`, `//@`,
  `//~WARN`, etc.
- `src/tools/compiletest/src/bin/main.rs` — the entry binary.

### Dependencies (per `Cargo.toml`)

- `clap`, `regex`, `diff`, `glob`, `indexmap`, `rayon`,
  `colored`, `home`, `anstyle-svg`, `rustfix`, `miropt-test-tools`,
  `build_helper` (in-tree), `camino`.

### Rust-specific assumptions

- The `runtest` modules invoke the rustc binary by default
  (`config.toml` `rustc_path`). Substituting `stnx` is **one
  configuration change** — no code change.
- The `directives` parser is generic over `//~`, `//@`, etc.
  Saturnite could choose its own directive syntax (e.g. `// expect
  error: foo` or whatever).
- Snapshot diffing uses `diff = "0.1"`.

### What Saturnite would actually port

**Just the runner scaffolding**: the per-test-type loop, the
directive parser, the snapshot diff, the JSON output. The runner
is a Rust binary that takes a config file and a list of test
directories; it does not depend on rustc internals.

The rustc-specific runtest modules (which test rustc-specific
output like `.stderr` for a specific diagnostic) are not directly
portable, but Saturnite's tests would replace them with
Saturnite-specific snapshot files.

### Adaptation difficulty

**Medium.** The binary is small; the bulk of compiletest is the
per-test-type framework. Saturnite would write a 1 000-line
subset for `compile-fail` and `run-pass` tests, leaving the rest
out.

### Recommendation

**Defer until 0.5+** when the test count justifies a UI
framework. Until then, the existing `tempfile`-based integration
tests are fine.

### Risk

**Low.** compiletest is dual-licensed and reasonably self-contained.

---

## F4. JSON target spec format (D. FUSE — later, when cross-target support is expanded)

### Source

The format is defined in `compiler/rustc_target/src/json.rs` (~100
lines) and the `Target` / `TargetOptions` structs in
`compiler/rustc_target/src/spec/mod.rs` (~200 lines). There are
~290 JSON files in `compiler/rustc_target/src/spec/` describing
each supported target.

### License

The JSON files are **data, not code** — they are not subject to
copyright in the US (per the Feist doctrine, factual data is not
copyrightable; JSON specs are essentially declarative facts about
target architectures). The **schema definition** (`json.rs` and
`TargetOptions`) is **MIT/Apache-2.0** (under the REUSE blanket).
The actual `TargetOptions` Rust struct **is** copyrightable and
**is** MIT/Apache-2.0.

### What Saturnite would actually port

- The JSON schema (the fields that go in a `target.json` file).
- The list of common target-spec fields.
- A small parser that reads `target.json` into a `TargetConfig`.

The 290+ JSON files themselves can be **used as data** — Saturnite
could load `aarch64-apple-darwin.json` as a target spec with
**no attribution required** (data, not code), and the parsing
code is reusable.

### Adaptation difficulty

**Low.**

### Recommendation

**D. FUSE — when cross-target support is needed.** Saturnite's
9-hand-rolled targets cover the immediate needs; the JSON format
becomes attractive at the 50+ target scale.

### Risk

**Low.** The JSON files are data; the parser is MIT/Apache-2.0.

---

## C. ADAPT/PORT candidates — there are NONE.

The audit's Phase 4 produced no items classified as "C. ADAPT /
PORT actual Rust source." This is **not an oversight**; it is the
result of the source-level analysis:

- **`rustc_lexer`** is MIT/Apache-2.0 and standalone, but is
  the wrong shape for Saturnite's logos/chumsky pipeline.
- **`rustc_parse`** is a 25 000-LOC hand-written parser tightly
  coupled to `rustc_ast`, `rustc_session::ParseSess`, and the
  `DiagCtxt`.
- **`rustc_ast`** is 4 514 lines defining the Rust AST. It is not
  directly useful for Saturnite's smaller language.
- **`rustc_session`** is too coupled to be extractable.
- **`rustc_hir`** is too coupled to be extractable.
- **`rustc_codegen_llvm`** is tightly coupled to `rustc_middle` /
  `rustc_hir` / `rustc_session` / `rustc_metadata` / 25 other
  `rustc_*` crates.

The only items that can be ported are **small, generic
abstractions** (the `Interned` newtype, the dataflow framework,
compiletest's runner, the JSON target spec schema) — and those
are **D. FUSE**, not **C. ADAPT**.

This is the **honest answer**: the size and coupling of every
rustc subsystem that Saturnite would actually want to use
precludes direct code porting. The audit's primary reuse path
is **architectural reference**, not source code.

---

## What this means for the reuse plan

In Phase 9 (the `SATURNITE_RUST_REUSE_PLAN.md`), the **TAKE/ADAPT
list will be very short** — the four D. FUSE items above, plus
possibly some crates.io re-implementations of rustc data
structures (`indexmap`, `hashbrown`, `smallvec` are already
implicitly present in Saturnite's transitive deps).

The **REIMPLEMENT list will be long** — most of rustc's
subsystems are best understood as architectural references and
re-implemented in Saturnite's idiom.

The **DO-NOT-TOUCH list will be precise** — every GPL/NCSA/Unicode
component is enumerated.
