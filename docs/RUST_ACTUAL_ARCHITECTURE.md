# The Actual Architecture of the Rust Compiler (rustc)

A forensic architecture report based on direct inspection of the rustc source tree (`compiler/`).
This document supplements the rustc-dev-guide with concrete type definitions and data-flow
extracted from the actual code, organized into ten architectural sections.

---

## 1. Compiler Pipeline

The Rust compiler is structured as a series of crates, each responsible for a distinct phase.
The main driver entry points and the crate-level pipeline are:

```
rustc_driver_impl::main()                       <-- binary entry (src/lib.rs)
  -> rustc_driver_impl::run_compiler()          <-- sets up args, Config, callbacks
    -> rustc_interface::run_compiler()          <-- builds Session, CodegenBackend, Compiler
      -> rustc_interface::Compiler::enter()     <-- drives the compilation pipeline
        -> passes::parse()                      <-- lex + parse (rustc_lexer, rustc_parse)
        -> passes::configure_and_expand()       <-- macro expansion (rustc_expand)
        -> passes::resolve()                    <-- name resolution (rustc_resolve)
        -> rustc_ast_lowering::hier_ext()       <-- AST -> HIR lowering
        -> rustc_hir_analysis::analyze()        <-- WF, typeck (rustc_hir_analysis)
        -> rustc_middle::query system           <-- demand-driven analysis
          -> rustc_mir_build::build_mir_inner_impl()  <-- HIR -> MIR (rustc_hir_lowering)
          -> rustc_mir_transform passes             <-- MIR optimization (rustc_mir_transform)
          -> rustc_borrowck::do_mir_borrowck()        <-- borrow checking
          -> rustc_codegen_ssa::codegen_crate()       <-- codegen (rustc_codegen_ssa/llvm)
            -> rustc_codegen_llvm                     <-- LLVM backend
        -> link (back/link.rs)                    <-- final linking
```

### Key Entry Points

- **`rustc_driver_impl::main()`** (line 1665): The binary's main function. Calls
  `run_compiler()` which orchestrates the entire compilation.

- **`rustc_driver_impl::run_compiler()`** (line 173): Parses command-line arguments via
  `args::arg_expand_all` and `handle_options`, builds a `SessionOptions`, constructs an
  `interface::Config`, invokes `callbacks.config()`, then calls `interface::run_compiler`.

- **`rustc_interface::Config`** (in `interface.rs`): Contains `opts`, `crate_cfg`,
  `crate_check_cfg`, `input`, `output_file`, `output_dir`, `ice_file`, `file_loader`,
  `lint_caps`, `psess_created`, `track_state`, `register_lints`, `override_queries`,
  `extra_symbols`, and `make_codegen_backend`. This is the configuration blob that flows
  through the entire pipeline.

- **`rustc_interface::Compiler`** (in `interface.rs`): Holds `sess: Session`,
  `codegen_backend: Box<dyn CodegenBackend>`, and `crate_name`. The `Compiler::enter`
  method takes a callback and runs the compilation within the compiler's context.

### Pipeline Phases via `rustc_interface::passes`

File: `compiler/rustc_interface/src/passes.rs`

1. **`parse()`** (line ~80): Calls `rustc_parse::parser::Parser` to produce an
   `ast::Crate`. This is the lex + parse stage using `rustc_lexer` and `rustc_parse`.

2. **`configure_and_expand()`**: Macro expansion via `rustc_expand`. Produces a fully
   expanded `Crate` and a `StrippedCfg`. Also collects lint capabilities.

3. **`create_and_enter_global_ctxt()`**: The critical bridge. Creates the
   `GlobalCtxt<'tcx>` (which wraps the `TyCtxt`) and enters it, registering all query
   providers from `DEFAULT_QUERY_PROVIDERS` and running `analysis()`.

4. **`start_codegen()`** (in `queries.rs`): Called from `Linker::codegen_and_build_linker`.
   Invokes `backend_codegen_crate()` which calls `CodegenBackend::codegen_crate()`
   and then `CodegenBackend::join_codegen()`.

5. **`Linker::link()`**: Calls `CodegenBackend::link()` which calls `link_binary()`
   in `rustc_codegen_ssa/src/back/link.rs`.

### DEFAULT_QUERY_PROVIDERS

File: `compiler/rustc_interface/src/passes.rs`

The `DEFAULT_QUERY_PROVIDERS` static is a `LazyLock<Providers>` that registers all
query provider functions. Each subsystem's `provide()` function is called:

```rust
pub static DEFAULT_QUERY_PROVIDERS: LazyLock<Providers> = LazyLock::new(|| {
    let mut providers = Providers { queries: Default::default() };
    rustc_resolve::provide(&mut providers.queries);
    rustc_hir::provide(&mut providers.queries);
    rustc_hir_analysis::provide(&mut providers.queries);
    rustc_mir_build::provide(&mut providers.queries);
    rustc_mir_transform::provide(&mut providers.queries);
    rustc_borrowck::provide(&mut providers.queries);
    rustc_incremental::provide(&mut providers.queries);
    rustc_codegen_ssa::provide(&mut providers.queries);
    rustc_metadata::provide(&mut providers.queries);
    rustc_typeck::provide(&mut providers.queries);
    providers
});
```

This is the central registration point where every crate's query implementations are
wired into the `Providers` struct, which is a function-pointer table generated by
the `rustc_queries!` macro.

---

## 2. Query System

The query system is the heart of rustc's demand-driven compilation model. It is defined
across `compiler/rustc_middle/src/query/` and `compiler/rustc_middle/src/queries.rs`.

### The `rustc_queries!` Macro

File: `compiler/rustc_middle/src/queries.rs` (2847 lines)

This macro invocation generates the `Providers` struct (a function pointer table for every
query), the `QueryVTable` definitions, the `TaggedQueryKey` enums, and the `QueryState`
instances. Each query is declared with a pattern like:

```rust
analysis(key: ()) {
    // Triggers all type checking and analysis
}

mir_built(key: LocalDefId) -> &'tcx Steal<mir::Body<'tcx>> {
    feedable
}

optimized_mir(key: LocalDefId) -> &'tcx mir::Body<'tcx> {
    cache_on_disk
    separate_provide_extern
}
```

Query modifiers:
- `cache_on_disk`: Results can be serialized and loaded from the incremental cache.
- `arena_cache`: Results are stored in arena-allocated caches, looked up by key.
- `eval_always`: The query is always re-evaluated (never cached), even across
  incremental sessions.
- `feedable`: The query can be "fed" from outside the compiler (e.g., by a driver).
- `separate_provide_extern`: The query has separate provider implementations for local
  vs. extern crates (the `ExternProviders` struct is generated for these).
- `no_hash`: The query result is not hashed into the dependency graph.

### `QueryVTable`

File: `compiler/rustc_middle/src/query/query_api.rs`

```rust
pub struct QueryVTable<'tcx, C: Criterion = ()> {
    pub name: &'static str,
    pub eval_always: bool,
    pub depth_limit: DepthLimit,
    pub feedable: bool,
    pub cache_on_disk_local: bool,
    pub cache_on_disk_extern: bool,
    pub separate_provide_extern: bool,
    pub dep_kind: DepKind,
    pub state: &'static QueryState<C>,
    pub cache: &'static QueryCache<C>,
    pub invoke_provider_fn: fn(&Providers, C) -> Result<...>,
    pub try_load_from_disk_fn: fn(...),
    pub hash_value_fn: fn(&C) -> u64,
    pub handle_cycle_error_fn: fn(...),
    pub format_value: fn(&...) -> String,
    pub create_tagged_key: fn(C) -> TaggedQueryKey,
    pub execute_query_fn: fn(&...),
}
```

This vtable holds all the metadata and function pointers needed to execute a query,
including cycle handling, on-disk caching, and dep graph integration.

### `QueryKey` / `QueryKeyBounds`

File: `compiler/rustc_middle/src/query/keys.rs`

```rust
pub trait QueryKey: Copy + 'static + Hash + Eq + Debug + Clone + Send + Sync {
    type Cache: QueryCache<Index = Self>;
    fn default_span(self, tcx: TyCtxt) -> Span;
    fn key_as_def_id(self) -> Option<DefId>;
    fn as_local_key(tcx: TyCtxt, key: &Self) -> Option<Self>;
    fn canonical_for(self, tcx: TyCtxt) -> Option<DefId>;
}

pub trait QueryKeyBounds: QueryKey + Ord + Hash + Eq {
    // sealed
}
```

Implementations exist for `()`, `ShimKind`, `InstanceKind`, `Instance`, `GlobalId`,
`(Ty, Option<ExistentialTraitRef>)`, `LitToConstInput`, `LocalDefId`, `DefId`, etc.

### `QueryState` / `QueryCache`

```rust
pub struct QueryState<K: QueryKey> {
    pub k: K,
    pub cache: Lock<HashMap<PackedFingerprint, QueryResult>>,
    pub in_cycle: AtomicBool,
}
```

`QueryCache` is implemented by `HashMapCache` (default) and `ArenaCache` (for
`arena_cache` queries).

### `QueryJob` / `QueryLatch` / `QueryWaiter` / `QueryCycle`

File: `compiler/rustc_middle/src/query/job.rs` (120 lines)

```rust
pub struct QueryJob<'tcx> {
    pub id: QueryJobId,
    pub span: Span,
    pub parent: Option<&'tcx QueryJob<'tcx>>,
    pub latch: QueryLatch,
}
```

- `QueryLatch`: Holds a list of `QueryWaiter`s. When a query completes, it notifies
  all waiters via a `Condvar`.
- `QueryWaiter`: Represents a blocked query, holding the parent job's span, a condvar,
  and cycle detection info.
- `QueryCycle`: Accumulates `QueryStackFrame`s to build a cycle error if detected.
- `ActiveKeyStatus`: Tracks whether a key is `Executing`, `Cancelled`, or `Green`.

### `QueryState` Active Key Tracking

```rust
pub struct QueryState<'tcx, K: QueryKey> {
    active: Sharded<(K, ActiveKeyStatus)>,
    ...
}
```

The `QuerySystem` manages the per-query `QueryState` instances, dep graph, and
on-disk cache.

### `QuerySystem`

File: `compiler/rustc_middle/src/query/system.rs`

```rust
pub struct QuerySystem<'tcx> {
    pub arenas: WorkerLocal<QueryArenas<'tcx>>,
    pub dep_kind_vtables: &'tcx [DepKindVTable],
    pub query_vtables: QueryVTables,
    pub side_effects: Lock<FxIndexMap<DepNodeIndex, QuerySideEffect>>,
    pub used_features: Lock<FxHashMap<Symbol, DepNodeIndex>>,
    pub on_disk_cache: Option<OnDiskCache>,
    pub local_providers: Providers,
    pub extern_providers: ExternProviders,
    pub jobs: AtomicU64,
    pub cycle_handler_nesting: Lock<u8>,
}
```

The `QuerySystem` is owned by `GlobalCtxt` and provides the `Providers` used for
query execution.

### `Providers` / `ExternProviders`

```rust
pub struct Providers {
    // One function pointer field per query:
    pub analysis: fn(TyCtxt<'_>, ()) -> !,
    pub mir_built: fn(TyCtxt<'_>, LocalDefId) -> &'tcx Steal<mir::Body<'tcx>>,
    pub optimized_mir: fn(TyCtxt<'_>, LocalDefId) -> &'tcx mir::Body<'tcx>,
    // ... hundreds more
}

pub struct ExternProviders {
    // Same shape, used when separate_provide_extern is set
    pub optimized_mir_ext: fn(TyCtxt<'_>, DefId) -> &'tcx mir::Body<'tcx>,
    // ...
}
```

The `Providers` struct is generated by the `rustc_queries!` macro and contains one
function pointer per query. When a query is executed, the `TyCtxt` dispatches through
the vtable's `invoke_provider_fn`, which calls the appropriate function from
`local_providers` or `extern_providers`.

### Cycle Detection and Dependency Tracking

The cycle detection mechanism works as follows:
1. When a query starts executing, `QueryState::start` registers the active key.
2. When query A reads query B, `QueryState::mark_active` records the read in `TaskDeps`.
3. If query B is already executing, a `QueryWaiter` is created and added to B's latch.
4. When B completes, it calls `QueryLatch::notify_all`, which wakes up all `QueryWaiter`s.
5. If a cycle is detected (a waiter depends on its own ancestor), `QueryCycle` data
   is assembled from the `QueryStackFrame` chain.

### Key Query Dispatch Methods on `TyCtxt`

- `tcx.query(key)`: Returns a `Query` object that can be called to execute the query.
- `tcx.ensure(key)`: Ensures the query result is computed (forces execution).
- `tcx.store_side_effect()`: Records a side effect for the current dep node.

---

## 3. MIR Architecture

MIR (Mid-level IR) is rustc's typed, control-flow-graph intermediate representation.
It is defined in `compiler/rustc_middle/src/mir/`.

### `Body<'tcx>`

File: `compiler/rustc_middle/src/mir/mod.rs`

```rust
pub struct Body<'tcx> {
    /// List of basic blocks, each containing a sequence of statements and a terminator.
    basic_blocks: Box<IndexVec<BasicBlock, BasicBlockData<'tcx>>>,

    /// The "phase" of this MIR body — see `MirPhase` enum.
    phase: MirPhase,

    /// Number of passes that have run on this body.
    pass_count: u32,

    /// The source from which this MIR was built.
    source: MirSource,

    /// Lexical scopes for debuginfo and shadowing.
    source_scopes: IndexVec<SourceScope, SourceScopeData<'tcx>>,

    /// Declarations of locals (including the return place, arguments, temp slots).
    local_decl: IndexVec<Local, LocalDecl<'tcx>>,

    /// Coroutine-related metadata (for async generators).
    coroutine: Option<CoroutineData<'tcx>>,

    /// Debug info for user variables (maps `Local` to multiple debuginfo entries).
    var_debug_info: Vec<VarDebugInfo<'tcx>>,

    /// Span of the HIR from which this body was built.
    span: Span,

    /// Constants that must be evaluated for the body to compile.
    required_consts: Vec<AnonConst>,

    /// Other items mentioned in this body.
    mentioned_items: Option<Vec<DefId>>,

    /// Whether this body is polymorphic (generic, not monomorphized).
    is_polymorphic: bool,

    /// The phase at which this body was built (for injection phase ordering).
    injection_phase: InjectionPhase,

    /// Whether this body contains errors / was tainted.
    tainted_by_errors: Option<ErrorGuarantee>,
}
```

Key methods:
- `Body::new()`: Full constructor with type checking.
- `Body::new_cfg_only()`: Minimal constructor for CFG-only bodies.
- `typing_env()`: Returns the `TypingEnv<'tcx>` for this body.
- `return_ty()`: Returns the type of the return place.
- `yield_ty()`: Returns the yield type for coroutines.

### `MirPhase` / `AnalysisPhase` / `RuntimePhase`

File: `compiler/rustc_middle/src/mir/syntax.rs`

```rust
pub enum MirPhase {
    Built,
    Analysis(AnalysisPhase),
    Runtime(RuntimePhase),
}

pub enum AnalysisPhase {
    Initial,
    PostCleanup,
}

pub enum RuntimePhase {
    Initial,
    PostCleanup,
    Optimized,
}
```

The phase ordering is: `Built` -> `Analysis(Initial)` -> `Analysis(PostCleanup)` ->
`Runtime(Initial)` -> `Runtime(PostCleanup)` -> `Runtime(Optimized)`.
Each pass pipeline sets the phase via `phase_change` in `run_passes_inner`.

### `MirSource`

```rust
pub struct MirSource {
    pub span: Span,
    pub def_id: LocalDefId,
    pub body_kind: BodyKind,
}
```

### `Local` / `LocalDecl` / `LocalKind`

```rust
pub type Local = NonZero<u32>;
// Local index 0 is reserved as a sentinel (INVALID).
pub const RETURN_PLACE: Local = Local::new(1);  // First valid local is the return place
// Arguments follow after RETURN_PLACE.

pub enum LocalKind {
    /// A temporary local (local variable created during MIR construction).
    Temp,
    /// The `n`th argument passed to the function.
    Arg,
    /// The return place (index 0 for the `RETURN_PLACE` constant).
    ReturnPointer,
    /// A user-declared local variable.
    Var,
    /// Storage for a `Const` item.
    Const,
    /// A closure capture.
    ClosureCapture,
    /// A local for `self` in a `Drop` terminator.
    Drop,
    /// A local for a user type annotation.
    UserDummies,
    /// A local for an `Opaque` type.
    Opaque,
    /// A local that doesn't correspond to any user-visible variable.
    Synthetic,
    /// A local for a promoted constant.
    Promoted,
    /// A local for the `this` pointer in an associated function.
    This,
    /// The `n`th upvar of a closure.
    Upvar,
    /// A local for a discriminant.
    Discriminant,
    _Other(LocalInfo),
}

pub struct LocalDecl<'tcx> {
    pub mutability: Mutability,
    pub local_info: ClearCrossCrate<LocalInfo<'tcx>>,
    pub ty: Ty<'tcx>,
    pub user_ty: Option<UserTy>,
    pub source_info: SourceInfo,
    pub visibility_scope: Option<SourceScope>,
    pub init: Option<&'tcx [Span]>,  // spans where this local is initialized
    pub pinned: bool,
    pub replace: Option<...>,
    pub opaque_hide_type: Option<...>,
    pub is_user_ty: bool,
    pub debug: Debuginfo,
}
```

### `BasicBlock` / `BasicBlockData` / `BasicBlocks`

File: `compiler/rustc_middle/src/mir/basic_blocks.rs`

```rust
pub type BasicBlock = NonZero<usize>;
pub const START_BLOCK: BasicBlock = BasicBlock::new(1);  // First real block (0 is sentinel)

pub struct BasicBlockData<'tcx> {
    /// Statements executed before the terminator.
    pub statements: Vec<Statement<'tcx>>,
    /// Debug info statements after the last real statement.
    pub after_last_stmt_debuginfos: Vec<StatementDebuginfo>,
    /// The terminator—always present in a valid basic block.
    pub terminator: Option<Terminator<'tcx>>,
    /// Whether this block is a cleanup block (unwinding path).
    pub is_cleanup: bool,
}

pub struct BasicBlocks<'tcx> {
    basic_blocks: IndexVec<BasicBlock, BasicBlockData<'tcx>>,
    cache: Cache,
}

pub struct Cache {
    predecessors: OnceLock<IndexVec<BasicBlock, SmallVec<[BasicBlock; 4]>>>,
    reverse_postorder: OnceBox<[BasicBlock]>,
    dominators: OnceLock<DominatorTree<BasicBlock>>,
    start_lanes: OnceLock<...>,
}
```

### `SourceInfo` / `SourceScope` / `SourceScopeData`

```rust
pub struct SourceInfo {
    pub span: Span,
    pub scope: SourceScope,
}

pub type SourceScope = NonZero<u32>;

pub struct SourceScopeData<'tcx> {
    pub span: Span,
    pub parent: Option<SourceScope>,
    pub inlined: Option<(DefId, &'tcx [OpTy<'tcx, Ty<'tcx>]>)>,
    pub desugaring: DesugaringKind,
}
```

### `Statement` / `StatementKind`

File: `compiler/rustc_middle/src/mir/statement.rs`

```rust
pub struct Statement<'tcx> {
    pub source_info: SourceInfo,
    pub kind: StatementKind<'tcx>,
    pub debuginfos: Vec<StatementDebuginfo<'tcx>>,
}

pub enum StatementKind<'tcx> {
    /// `place = rvalue`
    Assign(Box<(Place<'tcx>, Rvalue<'tcx>)>),
    /// `place` will be read; mark it as uninitialized after this point (for debug).
    FakeRead(FakeRead, Place<'tcx>),
    /// Set the discriminant of a place to a specific value.
    SetDiscriminant { place: Box<Place<'tcx>>, variant_index: VariantIdx },
    /// Allocate a local's stack slot (`StorageLive`) or deallocate it (`StorageDead`).
    StorageLive(Local),
    StorageDead(Local),
    /// Track that a place was mentioned (for diagnostics like `unused_variables`).
    PlaceMention(Place<'tcx>),
    /// An `as` type ascription.
    AscribeUserType(Box<(Place<'tcx, Ty<'tcx> + 'tcx), UserTypePredicate<'tcx>)>,
    /// Instrumentation for coverage.
    Coverage(CoverageTerm),
    /// An intrinsic call.
    Intrinsic(Box<IntrinsicOp<'tcx>>),
    /// A const evaluation counter (for CTFE step limits).
    ConstEvalCounter,
    /// No-op.
    Nop,
    /// Used by `drop` elaboration to mark `Rvalue::Aggregate` for dropping.
    BackwardIncompatibleDropHint { ... },
}
```

### `Terminator` / `TerminatorKind`

File: `compiler/rustc_middle/src/mir/terminator.rs`

```rust
pub struct Terminator<'tcx> {
    pub source_info: SourceInfo,
    pub kind: TerminatorKind<'tcx>,
    pub attributes: Option<&'tcx [ConstOperand<'tcx>]>,
}

pub enum TerminatorKind<'tcx> {
    /// `goto dest`
    Goto { target: BasicBlock },

    /// `switch value < targets`
    SwitchInt {
        discr: Operand<'tcx>,
        targets: SwitchTargets,
        switch_ty: Ty<'tcx>,
    },

    /// `resume` (unwinding)
    UnwindResume,

    /// `assert cond, then unwind, else goto`
    UnwindTerminate { reason: &'static str },

    /// `return`
    Return,

    /// Unreachable (dead code / noreturn)
    Unreachable,

    /// `dropplace`
    Drop {
        place: Place<'tcx>,
        corrupt_subtype: Option<BasicBlock>,
        target: BasicBlock,
        unwind: UnwindAction,
        replace: Option<...>,
    },

    /// `call fn(args) -> dest`
    Call {
        func: Operand<'tcx>,
        args: Vec<Operand<'tcx>>,
        destination: Place<'tcx>,
        target: BasicBlock,
        unwind: UnwindAction,
        fn_span: Span,
        call_source: CallSource,
    },

    /// `tail call fn(args) -> dest`
    TailCall {
        func: Operand<'tcx>,
        args: Vec<Operand<'tcx>>,
        fn_span: Span,
        call_source: CallSource,
    },

    /// `assert cond, then -> dest, else -> unwind`
    Assert {
        cond: Operand<'tcx>,
        expected: bool,
        region: Option<Operand<'tcx>>,
        panic: UnwindAction,
        target: BasicBlock,
    },

    /// `yield value -> dest`
    Yield {
        value: Operand<'tcx>,
        target: BasicBlock,
        resume: BasicBlock,
        drop: Option<(Place<'tcx>, BasicBlock)>,
    },

    /// `coroutine.drop`
    CoroutineDrop {
        coroutine: Operand<'tcx>,
        ref_place: Place<'tcx>,
        ref_ty: Ty<'tcx>,
        variant: VariantIdx,
        target: BasicBlock,
        unwind: UnwindAction,
    },

    /// `false_edge` (for CFG structure, no real semantics)
    FalseEdge {
        target: BasicBlock,
        unreachable_evil: bool,
    },

    /// `false_unwind` (for CFG structure, no real semantics)
    FalseUnwind {
        target: BasicBlock,
        unwind: UnwindAction,
    },

    /// Inline asm
    InlineAsm { ... },
}

pub enum UnwindAction {
    Cleanup(BasicBlock),
    Continue(BasicBlock),
    Unreachable,
    Terminate,
}

pub struct SwitchTargets {
    /// Target for non-matching values (default case).
    pub otherwise: BasicBlock,
    /// List of (value, target) pairs for specific value matches.
    pub targets: Box<[(u128, BasicBlock)]>,
}
```

### `Place` / `ProjectionElem` / `Rvalue` / `Operand`

File: `compiler/rustc_middle/src/mir/syntax.rs`

```rust
pub struct Place<'tcx> {
    pub local: Local,
    pub projection: &'tcx List<PlaceElem<'tcx>>,
}

pub enum PlaceElem<'tcx> {
    /// `(place)`
    Deref,
    /// `place.field`
    Field(Field, Ty<'tcx>),
    /// `place[index]`
    Index(Box<Place<'tcx>>),
    /// `place[N]` — constant index
    ConstantIndex {
        const_alloc: ConstOperand<'tcx>,
        offset: u32,
        min_length: bool,
    },
    /// `place[..N]` — fixed-length slice
    Subslice { from: bool, len: u32 },
    /// `place as Variant`
    Downcast(Symbol, VariantIdx),
    /// `place[N..M]`
    ArrayIndex(Box<Place<'tcx>>),
    /// Extra data for future use.
    Extra(Box<...>),
    /// `(place as OpaqueType)`
    OpaqueCast(Ty<'tcx>),
    /// Used in `unsafe` for subtype projections
    SubtypeProjection(Ty<'tcx>),
}

pub enum Operand<'tcx> {
    /// `copy of place`
    Copy(Box<Place<'tcx>>),
    /// `move of place`
    Move(Box<Place<'tcx>>),
    /// A constant value.
    Constant(Box<ConstOperand<'tcx>>),
    /// A runtime check (bounds, overflow, etc.)
    RuntimeChecks(RuntimeChecks<'tcx>),
}

pub struct ConstOperand<'tcx> {
    pub span: Span,
    pub user_ty: Option<UserTy>,
    pub const_: Const<'tcx>,
}

pub enum Rvalue<'tcx> {
    Use(Box<Operand<'tcx>>),
    Repeat(Operand<'tcx>, Ty<'tcx>, &'tcx [Const<'tcx>]),
    Ref(BorrowKind, Box<Place<'tcx>>),
    ThreadLocalRef(DefId),
    RawPtr(BorrowType, Box<Place<'tcx>>),
    Cast(CastKind, Box<Operand<'tcx>>, Ty<'tcx>),
    BinaryOp(Box<BinOp>, Box<(Operand<'tcx>, Operand<'tcx>)>),
    UnaryOp(UnOp, Box<Operand<'tcx>>),
    Discriminant(Box<Place<'tcx>>),
    Aggregate(Box<()>, Box<AggregateKind<'tcx>>, &'tcx [&'tcx [Const<'tcx>]]>,
    ThreadLocalRef(DefId),
}
```

### `BorrowKind`

```rust
pub enum BorrowKind {
    Shared,
    Fake(FakeReadKind),
    Mut {
        kind: MutBorrowKind,
    },
}

pub enum FakeReadKind {
    Shallow,
    Deep,
}

pub enum MutBorrowKind {
    Default,
    TwoPhaseBorrow,
    ClosureCapture,
}
```

---

## 4. MIR Optimization Passes

File: `compiler/rustc_mir_transform/src/lib.rs`

### `MirPass` Trait

File: `compiler/rustc_mir_transform/src/pass_manager.rs`

```rust
pub trait MirPass<'tcx> {
    fn name(&self) -> &'static str {
        const { simplify_pass_type_name(std::any::type_name::<Self>()) }
    }
    fn profiler_name(&self) -> &'static str {
        to_profiler_name(self.name())
    }
    fn policy(&self, sess: &Session) -> PassPolicy;
    fn run_pass(&self, tcx: TyCtxt<'tcx>, body: &mut Body<'tcx>);
    fn is_mir_dump_enabled(&self) -> bool { true }
}
```

### `PassPolicy` / `Optimizations`

```rust
pub enum PassPolicy {
    Required,  // Must always run
    Optional {
        generally_enabled: bool,  // Default on/off
        optimization: bool,       // Is this an optimization (disabled by #[optimize(none)])
    },
}
```

- `WithMinOptLevel<T>`: Wraps a pass so it only runs when `sess.mir_opt_level() >= N`.
- `Lint<T>`: Adapter wrapping a `MirLint` (read-only) as a `MirPass` (write-capable),
  disabling MIR dumping.
- `Optimizations`: `Suppressed` (function has `#[optimize(none)]`) or `Allowed`.

### Pass Execution (`run_passes_inner`)

The main loop in `pass_manager.rs`:
1. Validates pass names against `PASS_NAMES`.
2. Checks `should_run_pass()` for each pass (handles `-Zmir-enable-passes` overrides and
   `#[optimize(none)]` suppression).
3. Optionally applies opt-bisect limiting via `-Z mir-opt-bisect`.
4. Dumps MIR before/after the pass (if enabled).
5. Runs the pass under the profiler.
6. Validates MIR after the pass (if `--validate-mir` or `-Z validate-mir`).
7. Lints after the pass (if `-Z lint-mir`).

### Pass Pipelines

Defined in `rustc_mir_transform/src/lib.rs`:

#### Analysis Cleanup Passes (`run_analysis_cleanup_passes`)
Runs after HIR -> MIR lowering and promotion:
```
ImpossibleClauses,
CleanupPostBorrowCk,
RemoveNoopLandingPads,
SimplifyCfg::PostAnalysis,
Derefer,
```

#### Runtime Lowering Passes (`run_runtime_lowering_passes`)
Lowering from analysis MIR to runtime MIR:
```
CriticalCallEdges (AddCallGuards),
PostAnalysisNormalize,
Subtyper (AddSubtypingProjections),
ElaborateDrops,
CheckDropRecursion (Lint),
AbortUnwindingCalls,
AddMovesForPackedDrops,
EraseDerefTemps,
ElaborateBoxDerefs,
StateTransform (coroutine lowering),
KnownPanicsLint (Lint),
```

#### Runtime Cleanup Passes (`run_runtime_cleanup_passes`)
Initial cleanup for runtime MIR:
```
LowerIntrinsics,
RemovePlaceMention,
SimplifyCfg::PreOptimizations,
```

#### Analysis to Runtime Pipeline (`run_analysis_to_runtime_passes`)
Orchsestrates the full transformation from analysis to runtime MIR:
```
1. run_analysis_cleanup_passes -> AnalysisPhase::PostCleanup
2. (optional) PostDropElaboration passes (RemoveUninitDrops, SimplifyCfg::RemoveFalseEdges, CheckLiveDrops)
3. run_runtime_lowering_passes -> RuntimePhase::Initial
4. run_runtime_cleanup_passes -> RuntimePhase::PostCleanup
```

#### Optimization Passes (`run_optimization_passes`)
The full optimization pipeline, applied after borrowck:
```
// UB checks first
CheckAlignment, CheckNull, CheckEnums,
// Pre-inline trimming
LowerSliceLenCalls,
InstSimplify::BeforeInline,
ForceInline,
Inline,
// Post-inline cleanup
RemoveStorageMarkers, RemoveZsts, RemoveUnneededDrops,
UnreachableEnumBranching, UnreachablePropagation,
SimplifyCfg::AfterUnreachableEnumBranching,
MultipleReturnTerminators,
InstSimplify::AfterSimplifyCfg,
SimplifyConstCondition::AfterInstSimplify,
// Optimizations
ReferencePropagation,
Sra (ScalarReplacementOfAggregates),
SimplifyLocals::BeforeConstProp,
DeadStoreElimination::Initial,
GVN (Global Value Numbering),
SimplifyLocals::AfterGVN,
SsaRangePropagation,
MatchBranchSimplification,
DataflowConstProp,
SingleUseConsts,
SimplifyConstCondition::AfterConstProp,
JumpThreading,
EarlyOtherwiseBranch,
SimplifyComparisonIntegral,
SimplifyConstCondition::Final,
// Post-optimization
RemoveNoopLandingPads,
SimplifyCfg::Final,
StripDebugInfo,
CopyProp,
DeadStoreElimination::Final,
DestProp (DestinationPropagation),
SimplifyLocals::Final,
MultipleReturnTerminators,
EnumSizeOpt,
CriticalCallEdges,
ReorderBasicBlocks,
ReorderLocals
```

### `declare_passes!` Macro

This macro declares all pass modules and builds the `PASS_NAMES` static — a set of all
pass name strings used for validation of `-Zmir-enable-passes`.

### Notable Pass Implementations

- **Inline** (`inline`): Performs function inlining based on `#[inline]` attributes and
  heuristics. Uses a call graph (`mir_callgraph_cyclic`) and an inliner (`mir_inliner_callees`).
- **GVN** (`gvn`): Global Value Numbering — eliminates redundant computations and
  identical expressions.
- **SROA** (`sroa`): Scalar Replacement of Aggregates — breaks down aggregate values
  into scalars when accessed directly.
- **DSE** (`dead_store_elimination`): Dead Store Elimination in two variants (Initial, Final).
- **DestinationPropagation** (`dest_prop`): Propagates destinations to eliminate move chains.

---

## 5. HIR (High-level IR)

The HIR is defined in `compiler/rustc_hir/src/` and the identifier types in
`compiler/rustc_hir_id/src/`.

### `HirId` / `OwnerId` / `ItemLocalId`

File: `compiler/rustc_hir_id/src/lib.rs`

```rust
pub struct OwnerId {
    pub def_id: LocalDefId,
}

pub struct HirId {
    pub owner: OwnerId,
    pub local_id: ItemLocalId,
}

pub type ItemLocalId = NonZero<u32>;

pub const CRATE_OWNER_ID: OwnerId = OwnerId { def_id: CRATE_DEF_ID };
pub const CRATE_HIR_ID: HirId = HirId { owner: CRATE_OWNER_ID, local_id: ItemLocalId::ONE };
```

`OwnerId` wraps a `LocalDefId` and represents a definition context (a function body,
a module, etc.). `HirId` adds a `local_id` to disambiguate within that owner. This
two-part ID allows HIR nodes to be looked up without hashing full `DefId`s.

### `Node<'hir>`

File: `compiler/rustc_hir/src/hir.rs`

```rust
pub enum Node<'hir> {
    Param(&'hir Param<'hir>),
    Item(&'hir Item<'hir>),
    ForeignItem(&'hir ForeignItem<'hir>),
    TraitItem(&'hir TraitItem<'hir>),
    ImplItem(&'hir ImplItem<'hir>),
    Variant(&'hir Variant<'hir>),
    Field(&'hir FieldDef<'hir>),
    AnonConst(&'hir AnonConst),
    ConstBlock(&'hir ConstBlock),
    ConstArg(&'hir ConstArg<'hir>),
    Expr(&'hir Expr<'hir>),
    ExprField(&'hir ExprField<'hir>),
    ConstArgExprField(&'hir ConstArgExprField<'hir>),
    Stmt(&'hir Stmt<'hir>),
    PathSegment(&'hir PathSegment<'hir>),
    Ty(&'hir Ty<'hir>),
    AssocItemConstraint(&'hir AssocItemConstraint<'hir>),
    TraitRef(&'hir TraitRef<'hir>),
    OpaqueTy(&'hir OpaqueTy<'hir>),
    TyPat(&'hir TyPat<'hir>),
    Pat(&'hir Pat<'hir>),
    PatField(&'hir PatField<'hir>),
    PatExpr(&'hir PatExpr<'hir>),
    Arm(&'hir Arm<'hir>),
    Block(&'hir Block<'hir>),
    LetStmt(&'hir LetStmt<'hir>),
    Ctor(&'hir VariantData<'hir>),
    Lifetime(&'hir Lifetime),
    GenericParam(&'hir GenericParam<'hir>),
    Crate(&'hir Mod<'hir>),
    Infer(&'hir InferArg),
    WherePredicate(&'hir WherePredicate<'hir>),
    PreciseCapturingNonLifetimeArg(&'hir PreciseCapturingNonLifetimeArg),
    TestBinderForall(&'hir TestBinderForall<'hir>),
    TestBinderExists(&'hir TestBinderExists<'hir>),
    Synthetic,
    Err(Span),
}
```

### `Item` / `ItemKind`

```rust
pub struct Item<'hir> {
    pub owner_id: OwnerId,
    pub kind: ItemKind<'hir>,
    pub span: Span,
    pub vis_span: Option<Span>,
    pub eii: Option<ExternItemInfo>,
}

pub enum ItemKind<'hir> {
    ExternCrate(Option<Symbol>, Ident),
    Use(&'hir UsePath<'hir>, UseKind),
    Static(Mutability, Ident, &'hir Ty<'hir>, BodyId),
    Const(Ident, &'hir Generics<'hir>, &'hir Ty<'hir>, ConstItemRhs<'hir>),
    Fn {
        sig: FnSig<'hir>,
        ident: Ident,
        generics: &'hir Generics<'hir>,
        body: BodyId,
        has_body: bool,
    },
    Macro(Ident, &'hir ast::MacroDef, MacroKinds),
    Mod(Ident, &'hir Mod<'hir>),
    ForeignMod { abi: ExternAbi, items: &'hir [ForeignItemId] },
    GlobalAsm { asm: &'hir InlineAsm<'hir>, fake_body: BodyId },
    TyAlias(Ident, &'hir Generics<'hir>, &'hir Ty<'hir>),
    Enum(Ident, &'hir Generics<'hir>, EnumDef<'hir>),
    Struct(Ident, &'hir Generics<'hir>, VariantData<'hir>),
    Union(Ident, &'hir Generics<'hir>, VariantData<'hir>),
    Trait {
        impl_restriction: &'hir ImplRestriction<'hir>,
        constness: Constness,
        is_auto: IsAuto,
        ...
    },
    TraitAlias(Ident, &'hir Generics<'hir>, &'hir GenericBounds<'hir>),
    Impl(&'hir Impl<'hir>),
    TestBinderConstraints,
}
```

### `Expr` / `ExprKind`

```rust
pub struct Expr<'hir> {
    pub hir_id: HirId,
    pub kind: ExprKind<'hir>,
    pub span: Span,
}

pub enum ExprKind<'hir> {
    ConstBlock(Ty<'hir>, ConstBlock),
    Array(&'hir [Expr<'hir>]),
    Call(&'hir Expr<'hir>, &'hir [Expr<'hir>]),
    MethodCall(MethodCall<'hir>, &'hir [Expr<'hir>]),
    Use(&'hir Path<'hir>),
    Tup(&'hir [Expr<'hir>]),
    Binary(BinOp, &'hir Expr<'hir>, &'hir Expr<'hir>),
    Unary(UnOp, &'hir Expr<'hir>),
    Lit(&'hir Lit, StrStyle),
    Cast(&'hir Expr<'hir>, &'hir Ty<'hir>),
    Type(&'hir Expr<'hir>, &'hir Ty<'hir>),  // type ascription
    DropTemps(&'hir Expr<'hir>),
    Let(&'hir LetExpr<'hir>, &'hir Expr<'hir>),
    If(&'hir Expr<'hir>, &'hir Block<'hir>, Option<&'hir Expr<'hir>>),
    Loop(&'hir Block<'hir>, Option<&'hir Label>),
    Match(&'hir Expr<'hir>, &'hir [Arm<'hir>], MatchSource),
    Closure(CoroutineId, &'hir Closure<'hir>),
    Block(&'hir Block<'hir>, Option<&'hir Label>),
    Assign(&'hir Expr<'hir>, &'hir Expr<'hir>, ToLvalueTarget),
    AssignOp(BinOp, &'hir Expr<'hir>, &'hir Expr<'hir>),
    Field(&'hir Expr<'hir>, Ident),
    Index(&'hir Expr<'hir>, &'hir Expr<'hir>),
    AddrOf(BorrowType, AutoBorrow, &'hir Expr<'hir>),
    Break(Option<Label>, &'hir Expr<'hir>),
    Continue(Option<Label>),
    Ret(&'hir Expr<'hir>),
    Become(&'hir Expr<'hir>),
    InlineAsm(&'hir InlineAsm<'hir>),
    OffsetOf(&'hir Ty<'hir>, &'hir [Ident]),
    Yield(&'hir Expr<'hir>),
    YieldFrom(&'hir Expr<'hir>),
    Err(_Guar),
}
```

### `Pat` / `PatKind`

```rust
pub struct Pat<'hir> {
    pub hir_id: HirId,
    pub kind: PatKind<'hir>,
    pub span: Span,
}

pub enum PatKind<'hir> {
    Missing,
    Wild,
    Binding(BindingAnnotation, HirId, Ident, Option<&'hir Pat<'hir>>),
    Struct(QPath<'hir>, &'hir [PatField<'hir>], bool),
    TupleStruct(QPath<'hir>, &'hir [Pat<'hir>], Option<(&'hir Path<'hir>, usize)>),
    Or(&'hir [&'hir Pat<'hir>]),
    Never,
    Tuple(&[&'hir Pat<'hir>], Option<usize>),
    Deref(&'hir Pat<'hir>),
    Ref(&'hir Pat<'hir>, Mutability),
    Expr(PatExpr<'hir>),
    Range { start: Option<...>, end: Option<...>, limits: RangeLimits },
    Slice(&'hir [Pat<'hir>]),
    Or(&'hir [&'hir Pat<'hir>]),
    _Other,
}
```

### `Stmt` / `StmtKind` / `Block` / `Arm`

```rust
pub struct Stmt<'hir> {
    pub hir_id: HirId,
    pub kind: StmtKind<'hir>,
    pub span: Span,
}

pub enum StmtKind<'hir> {
    Local(&'hir Local<'hir>),
    Item(ItemId),
    Expr(&'hir Expr<'hir>),
    Semi(&'hir Expr<'hir>),
    Empty,
    Let(&'hir LetStmt<'hir>),
}

pub struct Block<'hir> {
    pub stmts: &'hir [Stmt<'hir>],
    pub expr: Option<&'hir Expr<'hir>>,
    pub hir_id: HirId,
    pub rules: BlockCheckMode,
    pub span: Span,
    pub targeted_by_break: bool,
}

pub struct Arm<'hir> {
    pub hir_id: HirId,
    pub span: Span,
    pub pat: &'hir Pat<'hir>,
    pub guard: Option<&'hir Expr<'hir>>,
    pub body: &'hir Expr<'hir>,
}
```

### `DefKind`

File: `compiler/rustc_hir/src/def.rs`

```rust
pub enum DefKind {
    // Type namespace
    Mod, Struct, Union, Enum, Variant, Trait, TyAlias, ForeignTy, TraitAlias,
    AssocTy, TyParam,
    // Value namespace
    Fn, Const { is_type_const: bool }, ConstParam, Static { safety, mutability, nested },
    Ctor(CtorOf, CtorKind),
    AssocFn, AssocConst { is_type_const: bool },
    // Macro namespace
    Macro(MacroKinds),
    // Not namespaced
    ExternCrate, Use, ForeignMod, AnonConst, OpaqueTy, Field, LifetimeParam,
    GlobalAsm, Impl { of_trait: bool }, Closure, CoroutineClosure,
    SyntheticCoroutineBody,
}
```

With methods:
- `descr()`: Human-readable description (e.g. "function", "struct").
- `article()`: "a" or "an".
- `ns()`: The `Namespace` (TypeNS, ValueNS, MacroNS, LifetimeNS).

### `Res` (Resolution)

```rust
pub enum Res<'tcx> {
    Def(DefKind, DefId),
    PrimTy(PrimTy),
    SelfTyParam,
    SelfTyAlias { alias_to: DefId, param_ids: &'tcx [DefId] },
    _Others,
}
```

### `Path` / `PathSegment`

```rust
pub struct Path {
    pub span: Span,
    pub res: Res,
    pub segments: Vec<PathSegment>,
}

pub struct PathSegment<'hir> {
    pub ident: Ident,
    pub hir_id: HirId,
    pub res: Res,
    pub args: Option<&'hir GenericArgs<'hir>>,
    pub infer_args: Option<()>,
}
```

### HIR Map

File: `compiler/rustc_middle/src/hir/map.rs`

HIR map queries are defined as methods on `TyCtxt`:
- `tcx.hir_owner_nodes(def_id)`: Returns `Option<&OwnerNodes>` for a given `LocalDefId`.
- `tcx.hir_owner(def_id)`: Returns `Option<Owner<'hir>>`.
- `tcx.hir_node(hir_id)`: Returns `Node<'hir>` for a `HirId`.
- `tcx.hir_body(body_id)`: Returns `&'hir Body<'hir>` (THIR).
- `tcx.hir_crate_items()`: Returns `ModuleItems`.
- `tcx.hir_module_items(mod_id)`: Returns `Option<&ModuleItems>`.

### `ModuleItems`

File: `compiler/rustc_middle/src/hir/mod.rs`

```rust
pub struct ModuleItems {
    pub submodules: Vec<LocalDefId>,
    pub free_items: Vec<LocalDefId>,
    pub trait_items: LocalDefIdMap<Vec<ItemTreeInfo>>,
    pub impl_items: LocalDefIdMap<IndexVec<...>>,
    pub foreign_items: Vec<LocalDefId>,
    pub body_owners: Vec<LocalDefId>,
    pub proc_macro_decls: Option<LocalDefId>,
    pub eiis: IndexVec<...>,  // existential impl item signatures
}
```

### THIR (Typed HIR Body)

File: `compiler/rustc_middle/src/thir.rs`

```rust
pub struct Thir<'tcx> {
    pub body_type: ThirBody<'tcx>,
    pub attributes: AttrVec,
    pub arms: IndexVec<ArmIndex, Arm<'tcx>>,
    pub blocks: IndexVec<BlockId, Block<'tcx>>,
    pub exprs: IndexVec<ExprId, Expr<'tcx>>,
    pub stmts: IndexVec<StmtId, Stmt<'tcx>>,
    pub params: IndexVec<ParamId, Param<'tcx>>,
    pub struct_destructured_elements: ...
}
```

---

## 6. Codegen Backend

The codegen backend is defined in `compiler/rustc_codegen_ssa/src/` with the LLVM-specific
implementation in `compiler/rustc_codegen_llvm/src/`.

### Backend Traits

File: `compiler/rustc_codegen_ssa/src/traits/backend.rs`

```rust
pub trait CodegenBackend {
    fn name(&self) -> &'static str;
    fn init(&self, _sess: &Session) {}
    fn print(&self, _req: &PrintRequest, _out: &mut String, _sess: &Session) {}
    fn target_config(&self, _sess: &Session) -> TargetConfig { ... }
    fn supported_crate_types(&self, _sess: &Session) -> Vec<CrateType> { ... }
    fn print_passes(&self) {}
    fn print_version(&self) {}
    fn replaced_intrinsics(&self) -> Vec<Symbol> { vec![] }
    fn fallback_intrinsics(&self) -> Vec<Symbol> { vec![] }
    fn thin_lto_supported(&self) -> bool { true }
    fn has_zstd(&self) -> bool { false }
    fn has_mnemonic(&self, _sess: &Session, _mnemonic: &str) -> bool { false }
    fn metadata_loader(&self) -> Box<MetadataLoaderDyn> { ... }
    fn provide(&self, _providers: &mut Providers) {}
    fn target_cpu(&self, sess: &Session) -> String;
    fn codegen_crate<'tcx>(&self, tcx: TyCtxt<'tcx>) -> Box<dyn Any>;
    fn join_codegen(&self, ongoing_codegen: Box<dyn Any>, sess: &Session, ...) -> (CompiledModules, WorkProductMap);
    fn print_pass_timings(&self) {}
    fn print_statistics(&self) {}
    fn print_statistics_json(&self) -> String { String::new() }
    fn link(&self, sess: &Session, compiled_modules: CompiledModules, crate_info: CrateInfo, ...) { ... }
}
```

File: `compiler/rustc_codegen_ssa/src/traits/write.rs`

```rust
pub trait WriteBackendMethods: Clone + 'static {
    type Module: Send + Sync;
    type TargetMachine;
    type ModuleBuffer: ModuleBufferMethods;
    type ThinData: Send + Sync;
    fn supports_parallel(&self) -> bool { true }
    fn thread_profiler() -> Box<dyn Any> { Box::new(()) }
    fn target_machine_factory(&self, sess: &Session, opt_level: config::OptLevel,
        target_features: &[String]) -> TargetMachineFactoryFn<Self>;
    fn optimize_and_codegen_fat_lto(sess: &Session, cgcx: &CodegenContext, ...) -> CompiledModule;
    fn run_thin_lto(cgcx: &CodegenContext, prof: &SelfProfilerRef, ...) -> (Vec<ThinModule<Self>>, Vec<WorkProduct>);
    fn optimize(cgcx: &CodegenContext, prof: &SelfProfilerRef, shared_emitter: &SharedEmitter,
        module: &mut ModuleCodegen<Self::Module>, config: &ModuleConfig);
    fn optimize_and_codegen_thin(cgcx: &CodegenContext, ...) -> CompiledModule;
    fn codegen(cgcx: &CodegenContext, ...) -> CompiledModule;
    fn serialize_module(module: Self::Module, is_thin: bool) -> Self::ModuleBuffer;
}
```

```rust
pub trait ExtraBackendMethods: Send + Sync + DynSend + DynSync {
    type Module;
    fn codegen_allocator<'tcx>(&self, tcx: TyCtxt<'tcx>, module_name: &str,
        methods: &[AllocatorMethod]) -> Self::Module;
    fn compile_codegen_unit(&self, tcx: TyCtxt<'_>, cgu_name: Symbol) -> (ModuleCodegen<Self::Module>, u64);
}
```

```rust
pub trait BackendTypes {
    type Function: CodegenObject;
    type BasicBlock: Copy;
    type Funclet;
    type Value: CodegenObject + PartialEq;
    type Type: CodegenObject + PartialEq;
    type FunctionSignature: CodegenObject + PartialEq;
    type DIScope: Copy + Hash + PartialEq + Eq;
    type DILocation: Copy;
    type DIVariable: Copy;
}
```

### `LlvmCodegenBackend`

File: `compiler/rustc_codegen_llvm/src/lib.rs`

```rust
pub struct LlvmCodegenBackend;

impl LlvmCodegenBackend {
    pub fn new() -> Box<dyn CodegenBackend> {
        Box::new(LlvmCodegenBackend)
    }
}

impl CodegenBackend for LlvmCodegenBackend {
    fn name(&self) -> &'static str { "llvm" }
    fn init(&self, sess: &Session) { llvm_util::init(sess); ... }
    fn provide(&self, providers: &mut Providers) {
        providers.queries.global_backend_features =
            |tcx, ()| llvm_util::global_llvm_features(tcx.sess, false);
    }
    fn print(&self, ...) { ... }
    // codegen_crate, join_codegen, link are inherited from default implementations
}
```

The LLVM backend implements `WriteBackendMethods` and `ExtraBackendMethods` for LLVM-specific
types.

### Codegen Pipeline

File: `compiler/rustc_codegen_ssa/src/base.rs`

```rust
pub fn codegen_crate<B: ExtraBackendMethods + WriteBackendMethods>(
    backend: &B,
    tcx: TyCtxt<'_>,
) -> Box<OngoingCodegen<B>> {
    // 1. Validate target CPU features
    validate_target_cpu_features(tcx);

    // 2. Collect monomorphization items
    let MonoItemPartitions { codegen_units, .. } = tcx.collect_and_partition_mono_items(());

    // 3. Partition CGUs
    //    Each CGU is either a pre-compile module or a post-LTO module
    for cgu in codegen_units {
        match cgu_reuse {
            CguReuse::No => {
                let module = backend.compile_codegen_unit(tcx, cgu.name());
                submit_codegened_module_to_llvm(...);
            }
            CguReuse::PreLto => { submit_pre_lto_module_to_llvm(...); }
            CguReuse::PostLto => { submit_post_lto_module_to_llvm(...); }
        }
    }

    // 4. Return OngoingCodegen for parallel join
}
```

### `OngoingCodegen` / `join_codegen` / `link`

The `OngoingCodegen<B>` struct holds the async codegen join handle. After all CGUs
are submitted, `join_codegen` waits for all codegen threads, runs LTO if requested,
and returns `CompiledModules`. Then `Linker::link()` calls `backend.link()`.

### `Linker` (queries.rs)

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

`Linker::codegen_and_build_linker()` calls `passes::start_codegen()` -> `codegen_crate()`
-> `join_codegen()`. Then `Linker::link()` calls `backend.link()`.

### `FunctionCx`

File: `compiler/rustc_codegen_ssa/src/mir/mod.rs`

```rust
pub struct FunctionCx<'a, 'tcx, Bx: CodegenObject> {
    /// The instance being compiled (for monomorphization).
    pub instance: Instance<'tcx>,
    /// The MIR body.
    pub mir: &'a Body<'tcx>,
    /// Debug info context.
    pub debug_context: Box<DebugInfoBuilder<...>>,
    /// The LLVM function being built.
    pub llfn: Bx::Function,
    /// The codegen context.
    pub cx: &'a Bx,
    /// Function ABI info.
    pub fn_abi: &'a FnAbi<'tcx, Ty<'tcx>>,
    /// The local slot for the exception personality.
    pub personality_slot: Option<Local>,
    /// Cached basic blocks.
    pub cached_llbbs: IndexVec<BasicBlock, Option<Bx::BasicBlock>>,
    /// Cleanup kinds for each basic block.
    pub cleanup_kinds: IndexVec<BasicBlock, CleanupKind>,
    /// Funclet tracking for Windows SEH.
    pub funclets: IndexVec<BasicBlock, Funclet>,
    /// Landing pad tracking.
    pub landing_pads: Vec<LandingPad>,
    /// Unreachable block (all calls unwind to this if no unwind).
    pub unreachable_block: Bx::BasicBlock,
    /// Blocks that must terminate a function.
    pub terminate_blocks: FxHashMap<BasicBlock, ()>,
    /// Cold blocks (for code layout).
    pub cold_blocks: BitSet<BasicBlock>,
    /// Nop landing pads.
    pub nop_landing_pads: bool,
    /// Local variable tracking.
    pub locals: LocalMap<'tcx, LocalRef<...>>,
    /// Debuginfo for each local.
    pub per_local_var_debug_info: Vec<PerLocalVarDebugInfo>,
    /// Cached caller location arguments.
    pub caller_location: Option<&'a Bx::Value>,
}
```

### `ModuleCodegen` / `CompiledModule` / `ModuleKind`

File: `compiler/rustc_codegen_ssa/src/lib.rs`

```rust
pub struct ModuleCodegen<M: WriteBackendMethods> {
    pub name: String,
    pub kind: ModuleKind,
    pub module: M,
    pub size_estimate: u64,
    pub times_run: usize,
    pub cgu_units: Vec<String>,
}

pub struct CompiledModule {
    pub name: String,
    pub kind: ModuleKind,
    pub object: Option<Vec<ObjectCode>>,
    pub bytecode: Option<Vec<u8>>,
    pub dwp: Option<Vec<u8>>,
    pub dep_info: Option<Vec<u8>>,
}

pub enum ModuleKind {
    CodeGen,
    Metadata,
    Allocator,
    MetadataVerification,
    NativeLibraries,
}
```

### `CrateInfo`

```rust
pub struct CrateInfo {
    pub(crate) local_crate_traits: LocalCrateMap<TyCtxt>,
    pub krate: &'static str,
    pub(crate) dep_graph: &DepGraph,
    pub stable_crate_id: StableCrateId,
    pub crate_name: Symbol,
    pub(crate) up_kind: Option<UpKind>,
    pub(crate) crate_disambiguator: u32,
    pub(crate) rmeta_pos: Option<usize>,
    pub exported_symbols: Vec<(String, Vec<SymbolExport>)>,
    pub(crate) native_libraries: Vec<NativeLib>,
    pub(crate) native_dependencies: NativeLibDepGraph,
}
```

### Linking (`link.rs`)

File: `compiler/rustc_codegen_ssa/src/back/link.rs`

Key functions:
- `link_binary()`: Entry point. Selects the linker (cc, lld, etc.) and invokes it.
- `link_natively()`: Links with native libraries (rlibs, dylibs, static libs).
- `ensure_removed()`: Removes old output files.

The linker dispatches to either `collect_obj` (for native linking) or
`llvm_add_bitorun_to_passes` (for LLVM-based linking via `lld`).

---

## 7. Symbol / Interner

File: `compiler/rustc_span/src/symbol.rs`

### `Symbol` / `SymbolIndex`

```rust
pub struct Symbol(SymbolIndex);

rustc_index::newtype_index! {
    #[orderable]
    struct SymbolIndex {}
}

impl Symbol {
    pub const fn new(n: u32) -> Self;
    pub fn intern(str: &str) -> Self {
        with_session_globals(|sg| sg.symbol_interner.intern_str(str))
    }
    pub fn as_str(&self) -> &str {
        with_session_globals(|sg| unsafe {
            std::mem::transmute::<&str, &str>(sg.symbol_interner.get_str(*self))
        })
    }
    pub fn as_u32(self) -> u32;
    pub fn is_empty(self) -> bool;
}
```

`Symbol` wraps a `SymbolIndex` (a `u32`). Symbols are interned: the same string always
maps to the same `Symbol`. The mapping is stored in the `Interner` held by `SessionGlobals`.

### `ByteSymbol`

Like `Symbol` but for arbitrary byte strings (used in some contexts where the content
may not be valid UTF-8). The interner is shared between `Symbol` and `ByteSymbol`.

### `Ident`

```rust
pub struct Ident {
    pub name: Symbol,
    pub span: Span,
}
```

### `Interner` / `InternerInner`

```rust
pub(crate) struct Interner(Lock<InternerInner>);

struct InternerInner {
    arena: DroplessArena,
    indices: HashTable<(&'static [u8], u32)>,
    byte_strs: Vec<&'static [u8]>,
}
```

The interner uses a `DroplessArena` for allocation of interned strings. The `indices`
hash table maps byte slices to `u32` indices (into `byte_strs`). The `intern_inner`
method hashes the byte string, checks the hash table, and if absent, allocates in the
arena and extends the lifetime to `'static` (safe because the interner outlives the
`Symbol`s).

Key methods:
- `Interner::prefill()`: Pre-populates from the `symbols!` macro's keywords and symbols.
- `Interner::intern_str()`: Converts `&str` to `Symbol`.
- `Interner::intern_byte_str()`: Converts `&[u8]` to `ByteSymbol`.
- `Interner::intern_inner()`: Core hashing + arena allocation.
- `Interner::get_str()`: Retrieves the string for a `Symbol`.

### `symbols!` Macro

The `symbols!` macro in `symbol.rs` generates:
1. A `kw` module with keyword `Symbol`s (e.g., `kw::Loop`, `kw::Break`).
2. A `sym` module with pre-interned non-keyword `Symbol`s (e.g., `sym::rustfmt`, `sym::u8`).
3. An `extra_symbols` list for driver-provided symbols.
4. The `Symbol` and `ByteSymbol` `new()` constructors for pre-interned values.

The macro uses `SymbolIndex::from_u32(n)` for each entry, assigning sequential indices.
Keywords are checked by `rustc_lexer`/parser against this list.

### `SessionGlobals`

File: `compiler/rustc_span/src/lib.rs`

```rust
pub struct SessionGlobals {
    symbol_interner: symbol::Interner,
    span_interner: Lock<span_encoding::SpanInterner>,
    metavar_spans: MetavarSpansMap,
    hygiene_data: Lock<hygiene::HygieneData>,
    source_map: Option<Arc<SourceMap>>,
}

scoped_tls::scoped_thread_local!(static SESSION_GLOBALS: SessionGlobals);
```

`SessionGlobals` is stored in a thread-local `scoped_tls` variable. This makes all
symbol and span interning accessible from anywhere via `with_session_globals(|sg| ...)`.
The `symbol_interner` is the single `Interner` instance for the entire compilation.

### `SpanData`

```rust
pub struct SpanData {
    pub lo: BytePos,
    pub hi: BytePos,
    pub ctxt: SyntaxContext,
    pub parent: Option<LocalDefId>,
}
```

`Span` wraps a `SpanData` pointer (via an interned, reference-counted handle). The
`ctxt` field is a `SyntaxContext` (also a `u32`-backed interned ID) for hygiene tracking.

### `DefId` / `LocalDefId` / `CrateNum` / `DefIndex` / `DefPathHash`

File: `compiler/rustc_span/src/def_id.rs`, `compiler/rustc_hir_id/src/definitions.rs`

```rust
#[repr(C)]
pub struct DefId {
    pub index: DefIndex,   // high entropy, low bits on 64-bit LE
    pub krate: CrateNum,
}

pub struct LocalDefId { pub local_def_index: DefIndex }

pub struct StableCrateId(pub(crate) Hash64);

#[derive(StableHash, Encodable, Decodable)]
pub struct DefPathHash(pub Fingerprint);  // combines StableCrateId + local hash

pub struct DefKey {
    pub parent: Option<DefIndex>,
    pub disambiguated_data: DisambiguatedDefPathData,
}

pub struct DisambiguatedDefPathData {
    pub data: DefPathData,
    pub disambiguator: u32,
}

pub struct DefPath {
    pub data: Vec<DisambiguatedDefPathData>,
    pub krate: CrateNum,
}

pub struct Definitions {
    stable_crate_id: StableCrateId,
    def_id_to_key: IndexVec<LocalDefId, DefKey>,
    def_path_hashes: IndexVec<LocalDefId, Hash64>,
    def_path_hash_to_index: DefPathHashMap,
}
```

The `DefId` Hash impl combines `krate` and `index` into a single `u64` on 64-bit
little-endian systems for performance with `FxHash`. The `DefPathHash` is used for
stable hashing in the dependency graph and incremental compilation.

### `DefId` -> `DefPath` Resolution

`Definitions::def_key(def_id)` returns the `DefKey` for a `LocalDefId`.
`DefPath::make(krate, start_index, get_key)` walks up the parent chain from a
`DefIndex` to the crate root, building the full path.
`Definitions::def_path_hash(def_id)` returns the `DefPathHash`.

---

## 8. Incremental Compilation

Files: `compiler/rustc_incremental/src/`, `compiler/rustc_middle/src/dep_graph/`

### Dependency Graph (`DepGraph`)

File: `compiler/rustc_middle/src/dep_graph/graph.rs`

```rust
pub struct DepGraph {
    data: Option<Arc<DepGraphData>>,
    virtual_dep_node_index: DepNodeIndex,
}

pub struct DepGraphData {
    current: CurrentDepGraph,
    previous: Arc<SerializedDepGraph>,
    colors: IndexVec<SerializedDepNodeIndex, DepNodeIndex>,
    previous_work_products: WorkProductMap,
    debug_loaded_from_disk: bool,
    green_edge_buf: Vec<(SerializedDepNodeIndex, SerializedDepNodeIndex)>,
}

pub struct CurrentDepGraph {
    encoder: DepGraphEncoder,
    /// Maps anonymous dep node indices to their positions in the graph.
    anon_node_to_index: HashTable<(DepNode, DepNodeIndex)>,
    /// Reverse mapping from previous to current indices.
    prev_index_to_index: IndexVec<SerializedDepNodeIndex, Option<DepNodeIndex>>,
}
```

### `DepNode` / `DepKind`

File: `compiler/rustc_middle/src/dep_graph/dep_node.rs`

```rust
pub struct DepNode {
    pub kind: DepKind,
    pub key_fingerprint: PackedFingerprint,
}
```

`DepKind` is an enum generated by the `define_dep_nodes!` macro (invoked from
`crate::queries::rustc_with_all_queries!`). Each variant corresponds to a query kind
or a manual dep node (like `CompileCodegenUnit`, `CompileMonoItem`, `Metadata`).

The `DepKindVTable` holds metadata:
```rust
pub struct DepKindVTable {
    pub is_eval_always: bool,
    pub key_fingerprint_style: KeyFingerprintStyle,
    pub can_reconstruct_query: bool,
    pub force_from_dep_node_fn: Option<fn(TyCtxt, DepNode, SerializedDepNodeIndex) -> bool>,
}
```

### `KeyFingerprintStyle`

```rust
pub enum KeyFingerprintStyle {
    /// The fingerprint is derived from the DefPath (stable across compilations).
    DefPathHash,
    /// The fingerprint is derived from a HirId (not stable across recompilations).
    HirId,
    /// No key data — just the DepKind.
    Unit,
    /// Opaque fingerprint — cannot be reconstructed.
    Opaque,
}
```

### Task Dependencies (`TaskDeps` / `TaskDepsRef`)

File: `compiler/rustc_middle/src/dep_graph/graph.rs`

```rust
pub enum TaskDepsRef<'a> {
    /// New dependencies can be added (normal query).
    Allow(&'a Lock<TaskDeps>),
    /// eval_always query — no dep tracking, but emit FOREVER_RED.
    EvalAlways,
    /// Ignore all new dependencies (decoding from disk).
    Ignore,
    /// Panic if a dependency is added (decoding integrity check).
    Forbid,
}

pub struct TaskDeps {
    reads: TaskReads,
    #[cfg(debug_assertions)]
    node: Option<DepNode>,
}

pub enum TaskReads {
    Small { len: usize, buf: [DepNodeIndex; 16] },  // Inline optimization
    Recorded(ReadsRecorder),
}

pub struct ReadsRecorder {
    reads: Vec<DepNodeIndex>,
    epochs: IndexVec<SerializedDepNodeIndex, u8>,
    epoch: u8,
}
```

The `SMALL_READS_MAX` (16) inline buffer avoids heap allocation for queries with
few dependencies. Epoch-based deduplication skips repeated reads.

### Work Products

```rust
pub struct WorkProduct {
    pub cgu_name: String,
    pub saved_files: Vec<String>,
}

pub type WorkProductMap = UnordMap<WorkProductId, WorkProduct>;
```

### Persist Module

File: `compiler/rustc_incremental/src/persist/mod.rs`

The persist module handles saving and loading the dep graph between compilations.
Key functions:
- `save_dep_graph()`: Serializes the current dep graph to disk.
- `save_work_product_index()`: Writes the work product index.
- `load_query_result_cache()`: Loads query results from the on-disk cache.
- `setup_dep_graph()`: Sets up the initial dep graph from a previous compilation.
- `finalize_session_directory()`: Writes the finalization marker.

The persist submodules are:
- `clean.rs`: Removes stale incremental cache files.
- `data.rs`: Data structures for serialization.
- `file_format.rs`: Binary format definitions.
- `fs.rs`: Filesystem helpers.
- `save.rs`: Saving logic.
- `load.rs`: Loading logic.
- `work_product.rs`: Work product tracking.

### `rustc_incremental::provide()`

File: `compiler/rustc_incremental/src/lib.rs`

```rust
pub fn provide(providers: &mut Providers) {
    providers.save_dep_graph = |tcx, _: &mut SaveContext, _: ()| -> bool {
        persist::save_dep_graph(tcx)
    }
}
```

The `save_dep_graph` query is the trigger for serializing the dep graph during
the final compilation phase. Its `eval_always` nature means it runs every time
it's queried (once per compilation at the end).

---

## 9. Module System

The module system spans multiple crates and representations.

### AST Level (`rustc_ast`)

File: `compiler/rustc_ast/src/ast.rs`

At the top level:
```rust
pub struct Crate {
    pub id: NodeId,
    pub attrs: AttrVec,
    pub items: ThinVec<Box<Item>>,
    pub spans: ModSpans,
    pub is_placeholder: bool,
}

pub struct Item<K> {
    pub attrs: AttrVec,
    pub id: NodeId,
    pub span: Span,
    pub vis: Visibility,
    pub kind: K,
    pub tokens: Option<LazyAttrTokenStream>,
}

pub enum ItemKind {
    ExternCrate(Option<Symbol>, Ident),
    Use(Box<UseTree>),
    Static(Box<Ty>, Mutability, Option<ConstBlock>),
    Const(Box<Ty>, ConstItemRhs),
    ConstBlock(Box<Ty>),
    Fn(Box<FnHead>, FnSig, Ident, Generics, Box<Block>),
    Mod(Ident, ModFile),
    ForeignMod(Abi, Safety, Vec<ForeignItem>),
    GlobalAsm(Box<InlineAsm>),
    TyAlias(Box<Ty>, Option<GenericArgs>),
    Enum(Box<EnumDef>, Ident, Generics),
    Struct(VariantData, StructRest, Ident, Generics),
    Union(VariantData, Ident, Generics),
    Trait(TraitDef),
    TraitAlias(Ident, Generics, Box<GenericBounds>),
    Impl(Box<Impl>),
    MacCall(Box<MacCall>),
    MacroDef(MacroDef),
    // 2024 edition
    Delegation(Delegation),
    DelegationMac(DelegationMac),
    TestBinderConstraints,
}
```

### Resolution (`rustc_resolve`)

The resolver (`compiler/rustc_resolve/src/lib.rs`) handles name resolution.
Key types:
- `Resolver`: The main resolver state.
- `NameResolution`: Tracks where a name resolves in different namespaces.
- `BindingTable`: Per-scope identifier-to-resolution mapping.
- `Module`: Represents a module in the resolution graph.

The resolver processes modules by:
1. Building the module tree from `ItemKind::Mod`.
2. Inserting items into the appropriate namespace.
3. Resolving `use` trees to establish aliases.
4. Handling `extern crate` and `extern {}` blocks.

### HIR Level (`rustc_hir`)

After lowering:
- `Node::Crate(&'hir Mod<'hir>)` represents the crate root module.
- `ItemKind::Mod(Ident, &'hir Mod<'hir>)` for each module.
- `ModuleItems` (in `rustc_middle/src/hir/mod.rs`) stores the hierarchical structure:
  submodules, free items, trait items, impl items, etc.

### `rustc_resolve::provide()`

File: `compiler/rustc_resolve/src/lib.rs`

Registers resolution-related queries:
- `resolver_for_lowering`
- `resolution_for_module`
- `def_collections`

### Key HIR Queries

File: `compiler/rustc_middle/src/hir/mod.rs`

The `provide()` function registers:
- `hir_crate_items`: Returns `ModuleItems` for a crate.
- `hir_module_items`: Returns `ModuleItems` for a specific module.
- `hir_owner`: Returns the `Owner` for a `LocalDefId`.
- `hir_owner_nodes`: Returns `&OwnerNodes` for a definition.

### Visibility (`Visibility`)

```rust
pub struct Visibility<'hir> {
    pub kind: VisibilityKind<'hir>,
    pub span: Span,
}

pub enum VisibilityKind<'hir> {
    Public,
    Inherited,
    Ctor(usize),
    Restricted { path: &'hir Path<'hir>, id: HirId },
}
```

The visibility system is resolved during name resolution, where `use` and `mod`
items establish the visibility graph. The HIR representation is later used by
typeck to enforce visibility rules.

### `ModFile` / Module Sources

```rust
pub enum ModFile {
    Path(PathBuf),
    Parse,
    File(PathBuf),
}

pub struct ModSpans {
    pub span: Span,
    pub inline: bool,
    pub lines: Vec<LineSep>,
    pub outer: Vec<Path>,
}
```

---

## 10. License / Provenance

### Repository Origin

This is the `rust-lang/rust` repository — the official Rust compiler source.
The codebase is dual-licensed under the MIT License and the Apache License, Version 2.0.
Individual contributions are copyrighted by their authors; the project is governed by
the Rust Foundation and the Rust Core Team.

### External Dependencies (subtrees/tools)

Several externally-maintained components are integrated as subtrees or submodules:

- **`src/llvm-project/`**: LLVM, Clang, and related projects (Apache 2.0 / MIT).
  This is the largest external dependency — rustc's primary codegen backend
  (`rustc_codegen_llvm`) is built on LLVM. Changes here are managed via SVN
  mirrors and should be made upstream.

- **`src/tools/`**: Contains several sub-projects:
  - `cargo/` — The Rust package manager (MIT/Apache 2.0).
  - `clippy/` — A bunch of lints (MIT/Apache 2.0).
  - `rustfmt/` — Code formatting (MIT/Apache 2.0).
  - `miri/` — Interpreter for detecting UB (MIT/Apache 2.0).
  - `rust-analyzer/` — Language server (MIT/Apache 2.0).
  - `compiletest/` — Compiletest harness (MIT/Apache 2.0).

- **`compiler/rustc_llvm/`**: FFI bindings to LLVM (MIT/Apache 2.0).

### Internal Crate Licenses

All Rust compiler crates under `compiler/` are licensed under the MIT License and
the Apache License, Version 2.0. See `LICENSE-MIT` and `LICENSE-APACHE` at the
repository root.

### Contributing

The file `CONTRIBUTING.md` specifies that changes to subtrees and submodules
(especially `src/tools/` and `src/llvm-project/`) should be routed to their owning
repositories. For `src/llvm-project/`, the `README` in that directory describes the
update process.

### Provenance Tracking

- The `StableCrateId` (defined in `rustc_span/src/def_id.rs`) uniquely identifies a crate
  by hashing its name, `-C metadata` arguments, crate type, and rustc version. This
  enables the compiler to distinguish between different builds of crates with the same
  name.

- `DefPathHash` similarly identifies individual definitions stably across compilation
  sessions, enabling incremental compilation.

- The `SourceMap` (in `rustc_span`) tracks source file origins, macro expansion spans,
  and hygiene contexts — all essential for provenance tracking through macro expansion
  and incremental compilation.

### Copyright Headers

Source files in the `rust-lang/rust` repository typically include a copyright header
pointing to the Rust Foundation and Contributors. Example:

```rust
// Copyright 2012 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, saved or redistributed
// except according to those terms.
```

### Third-party Licenses

The repository also contains license files for third-party dependencies vendored
into the source tree, located in `src/tools/*/LICENSE*`, `src/llvm-project/*/LICENSE*`,
and `LIBRARY_LICENSES_THIRD_PARTY.txt`.
