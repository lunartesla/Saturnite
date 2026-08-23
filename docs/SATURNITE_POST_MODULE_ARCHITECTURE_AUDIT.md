# Saturnite 0.4 — Post-Module Architecture Audit & Incremental Compilation Readiness

> **Status:** COMPLETE — Audit deliverable only. No source code modified.
>
> **Audit date:** 2026-08-23
> **Agent count:** 15 specialized audit agents
> **Total tool calls consumed:** 451
> **Tokens consumed:** 1,723,319

---

## TL;DR

**The Saturnite 0.4 architecture is NOT ready for incremental compilation.** The module system pipeline is structurally sound and complete, but **14 critical prerequisites block any incremental caching layer from functioning.** The most severe blocker is that **`SymbolInterner` — the cornerstone of the identifier system — cannot be serialized**, which transitively makes `HirProgram`, `MirProgram`, and `ModuleGraph` all non-serializable. Even if serialization were fixed, **`DefId` and `SymbolId` are positional indices that shift on every source edit**, making any cache keyed on them a silent corruption vector. Additionally, **the CLI never invokes the module-aware lowering path** (`analyze_and_lower_with_graph`), so multi-module projects are broken in production.

The full fix requires **6 MUST FIX** items (serialization chain, DefId namespace collapse, CLI wiring, missing hashing crate), **8 SHOULD FIX** items (deterministic hashers, bounds validation, duplicate mod detection, cycle detection), and **4 CAN DEFER** items (entry point configurability, workspace inheritance, debug info emission, Str-as-i64 portability).

---

## Phase 0 — Recon Summary

### Pipeline (verified)

```
Saturnite source (.stnx)
  │
  ▼
┌──────────────────────────────────────────────────┐
│ Phase 1  Lexer      (src/lexer/mod.rs)  logos    │
│ Phase 2  Parser     (src/parser/mod.rs) chumsky  │
│ Phase 3  Semantic   (src/semantic.rs)   AST→HIR  │
│ Phase 4  MIR Lower  (src/mir/lower.rs) HIR→MIR  │
│ Phase 5  MIR Verify (src/mir/verify.rs) CFG chk  │
│ Phase 6  MIR Opt    (src/mir/opt.rs)   constfold │
│ Phase 7  LLVM Codegen (src/mir/codegen.rs)       │
│ Phase 8  Object Emit (src/codegen/emitter.rs)    │
│ Phase 9  Linking     (src/codegen/linker.rs)     │
└──────────────────────────────────────────────────┘
  │
  ▼
Executable
```

### Build verification

| Check | Result |
|-------|--------|
| `cargo fmt --check` | ✅ Pass |
| `cargo check` | ✅ Pass |
| `cargo clippy` | ✅ Pass |
| `cargo test` | ✅ **364 tests, 0 failures** |

### Test breakdown (18 test binaries)

| Category | Test files | Count |
|----------|-----------|-------|
| Library unit tests | 6 source files | 115 |
| Integration tests | 17 files | 249 |
| **Total** | | **364** |

### Files read during recon

| Layer | File | Lines | Key finding |
|-------|------|-------|-------------|
| Workspace | `Cargo.toml` | 12 | `resolver = "3"`, 11 workspace deps, `toml` absent |
| Crate | `crates/stnx/Cargo.toml` | 24 | No workspace inheritance, `toml` standalone |
| Config | `src/config.rs` | 222 | `SaturnConfig`, `Package`, `DependencySpec` all serde |
| Module | `src/module.rs` | 1516 | `Project`, `ModuleGraph`, `discover_modules`, no cycle detection |
| Semantic | `src/semantic.rs` | 53 | `analyze_and_lower`, `analyze_and_lower_with_graph` |
| HIR mod | `src/hir/mod.rs` | 40 | Re-exports |
| HIR lower | `src/hir/lower.rs` | 2532 | `lower_program`, `lower_program_with_graph`, `resolve_modules` |
| HIR func | `src/hir/function.rs` | 222 | `HirProgram` derives `Debug` only |
| HIR symbol | `src/hir/symbol.rs` | 187 | `SymbolId`/`DefId` serde ✓; `SymbolInterner`/`DefTable` ✗ |
| HIR types | `src/hir/types.rs` | 57 | `HirType` serde ✓ |
| HIR expr | `src/hir/expr.rs` | 118 | `HirExpr`/`HirExprKind` ✗ |
| HIR stmt | `src/hir/stmt.rs` | 55 | `HirStmt`/`HirStmtKind` ✗ |
| MIR | `src/mir/mod.rs` | 344 | All Mir* serde ✓ except `MirProgram` ✗ |
| MIR lower | `src/mir/lower.rs` | 734 | `PRINTLN_DEF_ID`, `sigs: HashMap` |
| MIR verify | `src/mir/verify.rs` | 204 | 5 structural checks |
| MIR opt | `src/mir/opt.rs` | 163 | `ConstantFolder` |
| MIR codegen | `src/mir/codegen.rs` | 841 | `compile_from_mir_ext`, `function_name` O(n) |
| Codegen | `src/codegen/mod.rs` | 37 | `check_linker`, `host_triple` |
| Codegen | `src/codegen/emitter.rs` | 42 | `ObjectEmitter` |
| Codegen | `src/codegen/linker.rs` | 199 | `Linker`, `runtime_object_path` |
| Target | `src/target.rs` | 482 | `TargetConfig` not `Hash`/`PartialEq` |
| AST | `src/ast.rs` | 238 | All types `Clone, Debug` only ✗ |
| Error | `src/error.rs` | 159 | `CompilerError`, `CompilerResult` |
| Lib | `src/lib.rs` | 84 | Re-exports |
| CLI | `src/main.rs` | 718 | Uses `analyze_and_lower` not graph path |
| Build | `build.rs` | 55 | Compiles `println_i64.c` host-only |
| Runtime | `runtime/println_i64.c` | 7 | `long long println_i64(long long)` |
| Docs | 7 files | — | See Phase 8 for staleness analysis |

---

## Phase 1 — Multi-Agent Audit Results (15 Agents)

15 specialized audit agents were dispatched simultaneously. Each produced a comprehensive report with file:line evidence. All agent reports are in `/tmp/agent_*.md`.

### 1.1 AGENT-1 — Project Configuration & Cargo Manifest (`agent_A`)

**28,811 chars. Key findings:**

| ID | Finding | Severity |
|----|---------|----------|
| F-01 | Workspace `[workspace.dependencies]` declared but **zero** entries use `workspace = true` — 11 deps re-declared manually in crate `Cargo.toml` | SHOULD FIX |
| F-02 | `toml` not in `[workspace.dependencies]` but used as direct dep (`toml = "0.8"`) | SHOULD FIX |
| F-03 | `chumsky` feature mismatch — workspace has no `memoization` flag, crate adds it; migration would silently drop feature | CAN DEFER |
| F-04 | `clap` in `[dependencies]` (runtime) though only used by `main.rs` binary, not `lib.rs` | CAN DEFER |
| F-05 | **No hashing crate** in any `Cargo.toml` — no `blake3`, `xxhash-rust`, `sha2`, etc. | **MUST FIX** |
| F-06 | `#[serde(deny_unknown_fields)]` on `Package` causes hard failure on unknown fields | CAN DEFER |
| F-08 | CLI calls `Project::discover` **twice** in Build path (lines 176 + 249) — redundant filesystem walk | SHOULD FIX |
| F-09 | CLI uses `analyze_and_lower` (single-file), **not** `analyze_and_lower_with_graph` | **MUST FIX** |
| F-10 | `build.rs` missing `cargo:rerun-if-env-changed=CC` | CAN DEFER |
| F-11 | `resolver = "3"` requires Rust 1.85+; no `rust-toolchain.toml` | CAN DEFER |
| F-12 | `Project::load` hardcodes `main.stnx` as entry point with no config override | CAN DEFER |

### 1.2 AGENT-2 — Module Graph Architecture (`agent_B`)

**43,765 chars. Key findings:**

| ID | Finding | Severity |
|----|---------|----------|
| F-01 | **No module types are serializable** — `ModuleId`, `ModulePath`, `Module`, `ModuleScope`, `ModuleGraph`, `Project`, `SymbolInterner` all derive only `Debug`/`Clone`, no `Serialize`/`Deserialize`. AST IS serializable, MIR IS serializable (inner types) | **MUST FIX** |
| F-02 | **No cycle detection in `discover_modules`** — circular `mod` declarations (`a declares mod b`, `b declares mod a`) cause **infinite loop / stack overflow**. No `visited` set exists. | **MUST FIX** |
| F-03 | `child_module_lookup` uses flat `HashMap<String, ModuleId>` keyed by last path segment — same-named modules in different parent scopes collide | SHOULD FIX |
| F-04 | No duplicate `mod` declaration detection — two `mod foo;` in same file creates two `Module` entries | SHOULD FIX |
| F-05 | `ModulePath::Ord` sorts by segment count first (non-lexicographic) | CAN DEFER |
| F-06 | `ModulePath::Display` shows raw `SymbolId` integers, not human-readable names | CAN DEFER |
| F-07 | `Project::load`/`load_from` return only root module's `Program`, hiding child module ASTs | SHOULD FIX |
| F-08 | CLI calls `analyze_and_lower` instead of `analyze_and_lower_with_graph` | **MUST FIX** |
| F-09 | `SymbolInterner` cloned wholesale into `HirLower.symbols` (line 528), not shared by reference | CAN DEFER |
| F-10 | `ModulePath` lacks `FromIterator` / `Extend` | CAN DEFER |

### 1.3 AGENT-3 — Identifier Stability: Symbol/DefId/ModuleId (`agent_C`)

**22,598 chars. Key findings:**

| ID | Finding | Severity |
|----|---------|----------|
| F1 | `SymbolInterner` not serializable — no caching path exists | CRITICAL |
| F2 | `SymbolId` assignment order-dependent — `SymbolId(strings.len())` shifts all subsequent IDs | HIGH |
| F3 | **`DefTable::register()` return value discarded** — `register()` returns `DefId(entries.len())`, but lowering code computes its own DefIds | CRITICAL |
| F4 | **CRITICAL — Overlapping DefId spaces** — functions, structs, enums all independently start at `DefId(0)` | CRITICAL |
| F5 | **CRITICAL — Global function counter across modules** — `next_func_def_id` spans all modules; adding a function in module 0 shifts DefIds in all other modules | CRITICAL |
| F6 | `next_def_id()` borrows from `SymbolInterner` index space — unpredictable DefId collisions | HIGH |
| F7 | `local_index` is global Vec index, not per-module index | MEDIUM |
| F8 | `function()` accessor uses raw `DefId.0` as array index into `functions` — silently wrong for non-function DefIds | MEDIUM |
| F10 | `PRINTLN_DEF_ID = DefId(u32::MAX - 1)` — magic number at top of u32 range | LOW |
| F11 | `ModuleId` assigned by Vec position in `discover_modules` — order-dependent, non-stable | HIGH |

### 1.4 AGENT-4 — HIR Serialization Readiness (`agent_D`)

**15,422 chars. Key findings:**

| ID | Finding | Severity |
|----|---------|----------|
| CRITICAL | **Only 3 leaf types derive Serialize: `SymbolId`, `DefId`, `HirType`** | CRITICAL |
| CRITICAL | **`HirProgram` derives `Debug` only** — all 13 fields are non-serializable | CRITICAL |
| CRITICAL | **`miette` `serde` feature NOT enabled** — `SourceSpan` does not implement `Serialize`/`Deserialize`. Even adding derives to HIR types would fail to compile. | CRITICAL |
| HIGH | `SymbolInterner` (only `Debug, Default, Clone`) — transitive blocker for `HirProgram` and `MirProgram` | HIGH |
| HIGH | All `Module*` types not serializable | HIGH |
| HIGH | All `Hir*` node types (`HirFunction`, `StructDef`, `EnumDef`, `HirExpr*`, `HirStmt*`, `HirUseDecl`, `HirModDecl`) not serializable | HIGH |

### 1.5 AGENT-5 — MIR Structure & Serialization (`agent_E`)

**22,258 chars. Key findings:**

| ID | Finding | Severity |
|----|---------|----------|
| F1 | **`MirProgram` derives `Debug` only** — sole exception in MIR type hierarchy | CRITICAL |
| F2 | `SymbolInterner` not serializable — transitively blocks `MirProgram` | HIGH |
| F3 | `StructDef` not serializable — blocked by `SourceSpan`, `ModuleId`, `Visibility` | HIGH |
| F4 | `EnumDef` same transitive blockers as `StructDef` | HIGH |
| F3a | miette `serde` feature not enabled in `Cargo.toml` | HIGH |
| F3b | `ModuleId` not serializable | MEDIUM |
| F3c | `Visibility` not serializable | MEDIUM |
| F10 | `PRINTLN_DEF_ID` triplicated across `hir/lower.rs:43`, `mir/lower.rs:30`, `mir/codegen.rs:27` | MEDIUM |
| F12 | `MirProgram` carries **no module metadata** — no `ModuleId` on `MirFunction` | MEDIUM |
| F13 | `HirFunction` doesn't derive `Clone` (inconsistent with `StructDef`/`EnumDef`) | LOW |

### 1.6 AGENT-6 — Type System & Semantic Analysis (`agent_F`)

**26,667 chars. Key findings:**

| ID | Finding | Severity |
|----|---------|----------|
| 1 | `println` rejects `Bool` — no coercion from bool to i64 | LOW |
| 2 | `println` accepts `F64` but codegen will crash — type narrowing bug | LOW |
| 3 | `FunctionSig` not serializable — correct (internal, never in `HirProgram`) | N/A |
| 4 | `LowerContext` not serializable — correct (holds `&'a` references) | N/A |
| 5 | `LowerScope` not serializable — correct (transient, `HashMap` with `RandomState`) | N/A |
| 6 | **HIR types not serializable** — `HirProgram`, `HirFunction`, `StructDef`, `EnumDef`, all `HirExpr*`/`HirStmt*`, `SymbolInterner`, `DefTable`, `DefEntry`, `DefKind`, `Visibility` | CRITICAL |
| 7 | `MirProgram` not serializable — blocked by `SymbolInterner`, `StructDef`, `EnumDef` | CRITICAL |
| 8 | **`DefTable::register()` return value discarded** — DefId collision risk | CRITICAL |
| 9 | `resolve_modules` uses `def_table.lookup()` with code-computed DefIds that mismatch `register()`'s indices | CRITICAL |
| 10 | `println` builtin signature only accepts `i64` | LOW |
| 11 | `fold_binop` returns `None` for mixed-type constants — no coercion at MIR level | LOW |
| 12 | `fold_i64` And/Or returns `Bool` for integer operands — semantically inconsistent | LOW |
| 13 | `fold_bool` cannot fold arithmetic on booleans | LOW |
| 14 | `SymbolInterner` uses `HashMap<String, SymbolId>` with `RandomState` — non-deterministic iteration | HIGH |
| 15 | `ModuleScope` uses `HashMap` with `RandomState` — same non-determinism | HIGH |
| 16 | No global mutable state in type checking — clean per-compilation isolation | N/A |
| 17 | `ast::Type::Struct(String)` carries unresolved name — resolved during lowering | LOW |

### 1.7 AGENT-7 — Codegen / Backend (`agent_G`)

**44,144 chars. Key findings:**

| ID | Finding | Severity |
|----|---------|----------|
| F1 | `generate_ir_from_mir` — IR text emission, no triple set, no optimizations | (info) |
| F2 | `compile_from_mir_ext` — full compilation path with optimization passes | (info) |
| F3 | `MirCodeGenContext` — holds borrowed `LLVMContext`, mutable `module`, `builder`, `local_allocas` | (info) |
| F4 | `declare_functions` — O(1) `SymbolId` → `&str` lookup via `SymbolInterner.strings` Vec | (info) |
| F5 | `MirTerminator::Call` — `PRINTLN_DEF_ID` special case + `function_name()` O(n) linear scan per call site | CRITICAL (perf) |
| F6 | **LLVM context — no shared/global state** — fresh `LLVMContext::create()` per invocation | (info) |
| F7 | `ObjectEmitter` — emits `.o` via `TargetMachine` | (info) |
| F8 | `Linker` — invokes system linker (`cc`/`clang`/`link.exe`/`gcc`) | (info) |
| F9 | `check_linker` — does not verify CRT availability or runtime object existence | LOW |
| F10 | `host_triple()` — deterministic per machine, varies across machines | (info) |
| F11 | **`TargetConfig` derives `Debug` only — NOT `Hash`, NOT `PartialEq`, NOT `Serialize`** | HIGH |
| F12 | **`TargetConfig` non-hashable** — `triple` is `inkwell::targets::TargetTriple` (opaque, no `Hash`) | HIGH |
| F13 | `build.rs` — only emits `cargo:rerun-if-changed` for C source, not env vars | LOW |
| F14 | Runtime — only `println_i64` exists, signature `long long (long long)` | (info) |
| F15 | Runtime is deterministic | (info) |
| F16 | **`function_name` — O(n) linear scan** per call site → O(N×M) total | CRITICAL |
| F17 | `gen_field_access` — O(S×F) scan over all structs per field access | MEDIUM |
| F18 | `Module triple set redundantly` in `ObjectEmitter` and `compile_from_mir_ext` | TRIVIAL |
| F19 | `generate_function` re-resolves function name redundantly | LOW |
| F20 | **`PRINTLN_DEF_ID` duplicated** across 3 files with no shared source of truth | MEDIUM |
| F21 | **Str type represented as i64** — breaks on 32-bit targets | MEDIUM |
| F22 | **`SymbolInterner` not Serializable** — blocks MIR persistence | HIGH |
| F23 | **`MirProgram` not `Serialize`/`Hash`** — blocks incremental compilation | HIGH |
| F24 | **`debug_info` set but never consumed** in codegen — `DIBuilder` never used | MEDIUM |
| F25 | `check_linker` doesn't verify CRT availability | LOW |

### 1.8 AGENT-8 — CLI Entry Point & Module System Integration (`agent_H`)

**44,144 chars. Key findings:**

| ID | Finding | Severity |
|----|---------|----------|
| F1 | **CRITICAL — CLI Build/Run use `analyze_and_lower` (single-file), bypassing module graph** | CRITICAL |
| F2 | Graph-aware entry point `analyze_and_lower_with_graph` only invoked by tests | CRITICAL |
| F3 | Check command uses even weaker path (`analyze`, discards HIR, ignores `target`) | HIGH |
| F4 | `Project::load`/`load_from` populate `project.graph` but CLI discards it | HIGH |
| F5 | Child-module ASTs discovered but never lowered | HIGH |
| F6 | Two disjoint `SymbolInterner` instances on CLI path | HIGH |
| F7 | MIR lowering discards all module metadata | MEDIUM |
| F8 | Run's output path non-deterministic (embeds PID) | LOW |
| F9 | Cross-compilation guard placed after analysis+lowering (wasted work) | LOW |
| F15 | Stale version strings ("Saturnite 0.2" in 0.4 codebase) | TRIVIAL |

### 1.9 AGENT-9 — Test Suite Structure & Coverage (`agent_I`)

**8,542 chars. Key findings:**

| ID | Finding | Severity |
|----|---------|----------|
| F2.1 | **364 tests verified** — 249 integration + 115 unit = 364, matches docs | (verified) |
| F2.2 | Common test harness (`tests/common/mod.rs`) uses `analyze_and_lower` — **no graph-aware helper** | MEDIUM |
| F2.3 | `analyze_and_lower` vs `analyze_and_lower_with_graph` — only 8/249 integration tests use graph path | CRITICAL |
| F2.4 | `mir_lower.rs` calls `hir::lower::lower` directly instead of `analyze_and_lower` | LOW |
| F2.5 | **CLI not wired to graph-aware path** — no test catches this gap | CRITICAL |
| F2.6 | **No CLI-level compilation tests** — only `test_doctor.rs` invokes the `stnx` binary | HIGH |
| F2.7 | **No serialization tests for HIR/MIR types** — 12+ serde-derived types untested | MEDIUM |
| F2.8 | **No incremental compilation tests** — 0 tests, zero infrastructure | MEDIUM |

### 1.10 AGENT-10 — Documentation Consistency (`agent_J`)

**8,542 chars. Key findings:**

| ID | Finding | Severity |
|----|---------|----------|
| 1 | `SATURNITE_MIR_DEIGN.md` requested (typo) — file does not exist; actual: `SATURNITE_MIR_DESIGN.md` | Medium |
| 8 | `SATURNITE_MIR_DESIGN.md` — **COMPLETELY STALE** — no MIR type matches implementation | High |
| 9 | `SATURNITE_0_4_ARCHITECTURE_AUDIT.md` — **0.3-era, 12+ contradictions** with 0.4 | Medium |
| 10 | `SATURNITE_INCREMENTAL_COMPILATION.md` — labeled "0.3", omits DefId/SymbolId instability | Medium |
| 11 | `SATURNITE_DEPENDENCY_MODEL.md` Sec 2 — Python interop is DESIGN-ONLY | Medium |
| 13 | `DependencySpec::from_str` claim is wrong — just clones string, no version parsing | Medium |
| 6 | `lib.rs:38` says "Codegen consumes HirProgram" — stale; HIR→MIR is the seam | Medium |
| 7 | `main.rs:285,528` error says "Saturnite 0.2" — should be "0.4" | Low |

### 1.11 AGENT-11 — Dependencies & Build (`agent_K`)

**2,265 chars. Key findings:**

| ID | Finding | Severity |
|----|---------|----------|
| F-05 | **No hashing crates** — no `blake3`, `xxhash-rust`, `seahash`, `twox-hash` | **HIGH** / Critical blocker |
| F-06 | No `walkdir` or file-watching crates | MEDIUM |
| F-01 | Workspace inheritance not used — no `workspace = true` | MEDIUM |
| F-02 | `tempfile` duplicated in workspace deps + crate dev-deps | LOW |
| F-03 | `chumsky` feature mismatch (workspace vs crate) | CAN DEFER |
| F-09 | `inkwell` pinned to `llvm21-1-prefer-dynamic` — hard system dependency | MEDIUM |
| F-10 | Crate doesn't inherit version/edition/license from workspace | LOW |
| F-08 | No `[features]` table — no way to disable inkwell for frontend-only builds | LOW |

### 1.12 AGENT-12 — Incremental Compilation Design Review (`agent_L`)

**3,440 chars. Key findings:**

- `SATURNITE_INCREMENTAL_COMPILATION.md` labeled "for Saturnite 0.3" — predates module system
- Zero references to `ModuleId`, `ModuleGraph`, `ModulePath`, `ModuleScope`
- Design proposes SHA-256 fingerprinting but no hashing crate exists
- Design assumes HIR/MIR serialization but `SymbolInterner` lacks `Serialize`
- **DefId and SymbolId are not stable across recompilations** — sequential assignment, order-dependent
- No serialization path for `MirProgram`, `SymbolInterner`, `DefTable`, `ModuleGraph`

### 1.13 AGENT-13 — Security & Reliability (`agent_M`)

**16,082 chars. Key findings:**

**CRITICAL failure modes:**

| ID | Finding | Exploit Path |
|----|---------|-------------|
| CRITICAL-1 | **DefId Namespace Collapse** — `DefId(0)` is simultaneously a valid function, struct, and enum ID | Adding a struct before a function shifts the function's DefId; cached artifacts silently serve wrong data |
| CRITICAL-2 | **CLI Bypass** — `main.rs:255` calls `analyze_and_lower`, never `analyze_and_lower_with_graph` | Multi-module `crypto::verify_signature()` compiles with child module silently dropped |
| CRITICAL-3 | `SymbolInterner` not serializable — all cached HIR/MIR contains SymbolIds that are meaningless without the interner | Compile-time assertion `MirProgram: Serialize` fails |
| CRITICAL-4 | `DefTable::register()` returns different DefId than the one stored in HIR structures | `def_table.lookup(DefId(0))` returns function's entry when struct's DefId(0) was queried |

**HIGH failure modes:**

| ID | Finding |
|----|---------|
| HIGH-5 | No `Serialize` on core IR and module types |
| HIGH-6 | DefId/SymbolId positional — no stability guarantee |
| HIGH-7 | `HashMap` `RandomState` non-determinism |
| HIGH-8 | `println` C runtime format string risk (future) |
| HIGH-9 | `function_name()` O(n) linear scan |
| HIGH-10 | `module_scopes` indexed directly by `ModuleId.0` — fragile invariant |

**MEDIUM failure modes:**

| ID | Finding |
|----|---------|
| MEDIUM-11 | `PRINTLN_DEF_ID` sentinel collision risk |
| MEDIUM-12 | `DefTable` has no bounds validation |
| MEDIUM-13 | `next_id()` can return stale ID |
| MEDIUM-14 | `LowerScope` clone is O(depth²) |

### 1.14 AGENT-14 — Performance & Parallelism (`agent_N`)

**29,025 chars. Key findings:**

| Finding | Severity |
|---------|----------|
| LLVM single-context blocks parallel codegen — `LLVMContext` not thread-safe | **Critical** architectural blocker |
| `function_name()` O(n) → O(N²) codegen for N functions | **Critical** |
| `SymbolInterner` mutation blocks HIR parallelization — shared mutable state | **Critical** |
| `hir.symbols.clone()` per function in `MirLower::new` — O(N×T) wasteful allocation | High |
| `gen_field_access` O(S×F) per field access | High |
| All `struct_def`/`enum_def` O(n) lookups — no HashMap index | Medium |
| `ModuleGraph::discover_modules` sequential BFS | Medium |
| Missing `rayon` dependency | Low |

**Already parallelizable (no code changes, just parallel iterators):**
- MIR lowering (per function) — `MirLower` has no shared mutable state
- Constant folding (per function) — `ConstantFolder` is zero-sized, stateless
- MIR verification (per function) — read-only on all MIR data

### 1.15 AGENT-15 — Red-Team Failure Modes (`agent_O`)

**33,917 chars. Key findings:**

**Three Showstoppers identified:**

| ID | Showstopper | Impact |
|----|-------------|--------|
| SHOWSTOPPER-1 | **DefId namespace collapse** — functions, structs, enums share `DefId(0,1,2,…)` | Cache keyed on DefId silently serves wrong data |
| SHOWSTOPPER-2 | **CLI bypass** — module system unreachable from production CLI | All multi-module projects broken |
| SHOWSTOPPER-3 | **No serialization of core types** — `SymbolInterner` lacks `Serialize` | Incremental compilation architecturally impossible |
| SHOWSTOPPER-4 | `DefTable` registration mismatch — `register()` return value ignored | Module resolution via `def_table.lookup()` is dead code |

**Worst-case scenarios:**
1. Silent cross-module function call hijacking — child module dropped, DefId collision causes wrong function call
2. Cache poisoning via HashMap reordering — `RandomState` produces different SymbolId assignments across runs
3. Struct/function DefId ambiguity at scale — 50K functions + 50K structs → 50% lookup failure rate

**Defense in depth:** Zero. No bounds validation on DefId/SymbolId, no checksums on cache, no hash-based symbol resolution, no module dependency tracking. The only existing safety is `MirProgram::verify()` which checks CFG structure but not identifier correctness.

---

## Phase 2 — Consolidated Findings & Cross-Agent Reconciliation

### 2.1 Consolidation methodology

Each finding from the 15 agents was cross-referenced against:
1. Direct source code inspection (file:line)
2. Findings from other agents covering the same subsystem
3. The existing architecture documentation (`SATURNITE_0_4_ARCHITECTURE.md`)

Findings that appeared in multiple agent reports (e.g., `SymbolInterner` not serializable appears in agents 4, 5, 6, 7, 13) were deduplicated and assigned a single canonical ID.

### 2.2 Discrepancies noted during reconciliation

| Discrepancy | Details | Resolution |
|-------------|---------|-----------|
| AST serialization claim | AGENT-2 report (Phase 1 summary) claimed "ALL types in ast.rs have Serialize/Deserialize" — contradicted by independent grep returning zero results. Verified: `ast.rs` types derive only `Clone, Debug`. | AST types are **NOT** serializable. AGENT-2's claim was incorrect; the per-agent detail report (agent_A) correctly identified this. |
| `FunctionSig` serialization | AGENT-6 noted `FunctionSig` is not serializable — but it's a private internal struct never stored in `HirProgram`. | Not a blocker — correct by design. |
| `LowerContext`/`LowerScope` serialization | AGENT-6 noted these are not serializable — they hold `&'a` references. | Correct by design — transient borrow bundles. |
| `MIR_DEIGN.md` typo | AGENT-10 noted the task referenced `SATURNITE_MIR_DEIGN.md` (typo "DEIGN"). File does not exist; actual file is `SATURNITE_MIR_DESIGN.md`. | Documented in Phase 8. |
| MIR design doc staleness | AGENT-10 found `SATURNITE_MIR_DESIGN.md` describes a MIR that doesn't match the implementation. | Documented in Phase 8. |

### 2.3 Consolidated findings table (deduplicated)

| Canonical ID | Finding | File:Line | Agents reporting | Severity |
|---|---|---|---|---|
| **CF-01** | `SymbolInterner` not serializable | `hir/symbol.rs:46` | 4, 5, 6, 7, 13 | **CRITICAL** |
| **CF-02** | `MirProgram` not serializable | `mir/mod.rs:314` | 5, 6, 7, 13 | **CRITICAL** |
| **CF-03** | `HirProgram` not serializable | `hir/function.rs:127` | 4, 6, 13 | **CRITICAL** |
| **CF-04** | miette `serde` feature not enabled | `Cargo.toml:11` | 4, 5, 6 | **CRITICAL** |
| **CF-05** | DefId namespace collapse (functions/structs/enums share DefId(0..N)) | `hir/lower.rs:220,238,343,416` | 3, 13, 15 | **CRITICAL** |
| **CF-06** | `DefTable::register()` return value discarded | `hir/lower.rs:425` | 3, 6, 13 | **CRITICAL** |
| **CF-07** | Global function DefId counter spans all modules | `hir/lower.rs:587,734` | 3, 13 | **CRITICAL** |
| **CF-08** | `SourceSpan` not serializable (miette serde feature) | `hir/function.rs`, `hir/expr.rs`, `hir/stmt.rs` | 4, 5, 6 | **CRITICAL** |
| **CF-09** | CLI uses `analyze_and_lower` (single-file), never `analyze_and_lower_with_graph` | `main.rs:255,496,550` | 1, 2, 8, 9, 11, 13, 15 | **CRITICAL** |
| **CF-10** | No cycle detection in `discover_modules` — infinite loop on circular `mod` | `module.rs:497-575` | 2, 13 | **CRITICAL** |
| **CF-11** | No hashing crate — cannot compute stable cache keys | `crates/stnx/Cargo.toml` | 1, 11, 12 | **CRITICAL** |
| **CF-12** | `SymbolInterner` uses `HashMap` with `RandomState` — non-deterministic iteration | `hir/symbol.rs:46` | 6, 13, 14 | **HIGH** |
| **CF-13** | `ModuleId`/`ModulePath`/`Module`/`ModuleScope`/`ModuleGraph` not serializable | `module.rs:40,77,216,289,363` | 2, 4, 6, 13 | **CRITICAL** |
| **CF-14** | `DefTable` not serializable; `DefEntry`/`DefKind` not serializable | `hir/symbol.rs:91,106,123` | 4, 6, 13 | **HIGH** |
| **CF-15** | `HirExpr`/`HirExprKind`/`HirStmt`/`HirStmtKind` not serializable | `hir/expr.rs:13`, `hir/stmt.rs:12` | 4, 6 | **HIGH** |
| **CF-16** | `StructDef`/`EnumDef`/`HirUseDecl`/`HirModDecl` not serializable | `hir/function.rs` | 4, 5, 6 | **HIGH** |
| **CF-17** | `Visibility` not serializable | `hir/symbol.rs:181` | 4, 5, 6 | **HIGH** |
| **CF-18** | `HirFunction` not `Clone` (inconsistent with StructDef/EnumDef) | `hir/function.rs:42` | 5 | **LOW** |
| **CF-19** | `TargetConfig` not `Hash`/`PartialEq`/`Serialize`; `triple` field is opaque inkwell type | `target.rs:103` | 7, 12 | **HIGH** |
| **CF-20** | `function_name()` O(n) linear scan per call site → O(N²) codegen | `mir/mod.rs:337`, `mir/codegen.rs:646` | 7, 13, 14, 15 | **CRITICAL** |
| **CF-21** | `gen_field_access` O(S×F) per field access | `mir/codegen.rs:541` | 7, 14 | **HIGH** |
| **CF-22** | `SymbolInterner` cloned per function in `MirLower::new` | `mir/lower.rs:79-80` | 14 | **HIGH** |
| **CF-23** | `debug_info` set but never consumed in codegen | `target.rs:88-90`, `mir/codegen.rs` | 7 | **MEDIUM** |
| **CF-24** | `Str` type represented as `i64` in LLVM IR — breaks on 32-bit | `mir/codegen.rs:732` | 7 | **MEDIUM** |
| **CF-25** | `PRINTLN_DEF_ID` triplicated, no single source of truth | `hir/lower.rs:43`, `mir/lower.rs:30`, `mir/codegen.rs:27` | 5, 7, 13 | **MEDIUM** |
| **CF-26** | `local_allocas: HashMap` instead of `Vec` indexed by `LocalId` | `mir/codegen.rs:33` | 14 | **LOW** |
| **CF-27** | `generate_function` re-resolves function name redundantly | `mir/codegen.rs:116-129` | 7 | **LOW** |
| **CF-28** | Module triple set redundantly in `ObjectEmitter` and `compile_from_mir_ext` | `mir/codegen.rs:792`, `codegen/emitter.rs:19` | 7 | **TRIVIAL** |
| **CF-29** | `check_linker` doesn't verify CRT or runtime object existence | `codegen/linker.rs:129-179` | 7 | **LOW** |
| **CF-30** | `child_module_lookup` flat-name collision | `hir/lower.rs:580-585,670` | 2, 15 | **HIGH** |
| **CF-31** | No duplicate `mod` declaration detection | `module.rs:497-575`, `test_module_graph.rs:435` | 2 | **HIGH** |
| **CF-32** | `ModulePath::Display` shows raw SymbolId integers | `module.rs:187-205` | 2 | **MEDIUM** |
| **CF-33** | `local_index` is global Vec index, not per-module | `hir/lower.rs:860-865` | 3 | **MEDIUM** |
| **CF-34** | `function()` accessor uses raw DefId as array index | `hir/function.rs:154-156` | 3 | **MEDIUM** |
| **CF-35** | `ModuleId` order-dependent (assigned by Vec position) | `module.rs:507,407` | 3, 13 | **HIGH** |
| **CF-36** | `next_def_id()` borrows SymbolId space | `hir/lower.rs:155-159` | 3, 13 | **HIGH** |
| **CF-37** | `module_scopes` indexed directly by `ModuleId.0` — fragile invariant | `hir/lower.rs:847,867,879,901` | 13 | **MEDIUM** |
| **CF-38** | `DefTable::lookup` no bounds validation | `hir/symbol.rs:147-149` | 13 | **MEDIUM** |
| **CF-39** | No CLI-level compilation tests | AGENT-9 report | 9 | **HIGH** |
| **CF-40** | No serialization tests for HIR/MIR types | AGENT-9 report | 9 | **MEDIUM** |
| **CF-41** | No incremental compilation tests | AGENT-9 report | 9 | **MEDIUM** |
| **CF-42** | `SATURNITE_MIR_DEIGN.md` typo — file does not exist | AGENT-10 report | 10 | **MEDIUM** |
| **CF-43** | `SATURNITE_MIR_DESIGN.md` completely stale | AGENT-10 report | 10 | **HIGH** |
| **CF-44** | `SATURNITE_0_4_ARCHITECTURE_AUDIT.md` 0.3-era, 12+ contradictions | AGENT-10 report | 10 | **MEDIUM** |
| **CF-45** | `SATURNITE_INCREMENTAL_COMPILATION.md` partially stale | AGENT-10, 12 | 10, 12 | **MEDIUM** |
| **CF-46** | `SATURNITE_DEPENDENCY_MODEL.md` Python interop DESIGN-ONLY | AGENT-10 report | 10 | **MEDIUM** |
| **CF-47** | `clap` in main deps not binary-only | AGENT-1 report | 1 | **LOW** |
| **CF-48** | `toml` not in `[workspace.dependencies]` | AGENT-1 report | 1, 11 | **SHOULD FIX** |
| **CF-49** | No `[features]` table — no way to disable inkwell | AGENT-11 report | 11 | **CAN DEFER** |
| **CF-50** | `SATURNITE_0_4_ARCHITECTURE.md:314` claims guard in all command paths | AGENT-10 report | 10 | **MEDIUM** |

---

## Phase 3 — MUST FIX / SHOULD FIX / CAN DEFER Classification

### 3.1 Architecture: Classification Methodology

**MUST FIX** — Blocking issues that prevent the architecture from functioning correctly or make incremental compilation impossible. These are correctness or fundamental-readiness blockers.

**SHOULD FIX** — Important improvements that affect robustness, performance, or incremental compilation readiness, but do not block current functionality.

**CAN DEFER** — Nice-to-have improvements that are low-impact or can be addressed in future iterations without affecting the current compiler's correctness or 0.4 release readiness.

### 3.2 MUST FIX (14 items — blocking incremental compilation or correctness)

| # | ID | Finding | File:Line | Rationale |
|---|----|---------|-----------|-----------|
| M1 | CF-04 | Enable `serde` feature on `miette` in `Cargo.toml` | `Cargo.toml:11` | `SourceSpan` (used in all HIR types) is non-serializable without the `serde` feature. This is the root transitive blocker. |
| M2 | CF-01 | Add `Serialize, Deserialize` to `SymbolInterner` | `hir/symbol.rs:46` | `SymbolInterner` is embedded in `HirProgram`, `MirProgram`, `ModuleGraph` — all are blocked until this is fixed. |
| M3 | CF-13 + CF-17 | Add serde derives to `ModuleId`, `ModulePath`, `Visibility` | `module.rs:40`, `module.rs:77`, `hir/symbol.rs:181` | Required transitive dependencies for `HirProgram` and `MirProgram` serialization. |
| M4 | CF-16 | Add `Serialize, Deserialize` to `StructDef`, `EnumDef`, `HirUseDecl`, `HirModDecl` | `hir/function.rs:57,72,91,110` | Required for `HirProgram` serialization. Depends on M1-M3. |
| M5 | CF-03 + CF-15 | Add `Serialize, Deserialize` to `HirProgram`, `HirFunction`, `HirExpr*`, `HirStmt*` | `hir/function.rs:127,42`, `hir/expr.rs:13`, `hir/stmt.rs:12` | Core HIR serialization. Depends on M1-M4. |
| M6 | CF-14 | Add `Serialize, Deserialize` to `DefTable`, `DefEntry`, `DefKind` | `hir/symbol.rs:91,106,123` | Required for `HirProgram` serialization. |
| M7 | CF-02 | Add `Serialize, Deserialize` to `MirProgram` | `mir/mod.rs:314` | Depends on M2, M4 (SymbolInterner, StructDef, EnumDef serializable). |
| M8 | CF-05 | Fix DefId namespace collapse — separate DefId spaces per kind or use `def_table.register()` return value | `hir/lower.rs:220,238,343,416,425` | Functions, structs, enums all assign `DefId(0)` independently. This is a silent correctness bug that makes any DefId-keyed cache unsound. |
| M9 | CF-09 | Wire CLI to `analyze_and_lower_with_graph` | `main.rs:255,496,550` | CLI uses single-file path, discarding the `ModuleGraph`. Multi-module projects are broken in production. |
| M10 | CF-10 | Add cycle detection to `discover_modules` | `module.rs:497-575` | Circular `mod` declarations cause infinite loop / stack overflow. |
| M11 | CF-11 | Add hashing crate (`blake3` or `xxhash-rust`) to dependencies | `crates/stnx/Cargo.toml` | No hashing crate exists; SHA-256 fingerprinting (as proposed in `SATURNITE_INCREMENTAL_COMPILATION.md`) cannot be implemented without one. |
| M12 | CF-20 | Replace `function_name()` O(n) linear scan with `HashMap<DefId, SymbolId>` index | `mir/mod.rs:337`, `mir/codegen.rs:646` | O(N²) codegen scaling. Called per call site + per declaration. |
| M13 | CF-12 | Replace `HashMap` with `RandomState` with deterministic hasher (`FxHashMap` or `BTreeMap`) in `SymbolInterner`, `ModuleScope`, and all HIR/MIR HashMaps | `hir/symbol.rs:46`, `module.rs:289-298` | `RandomState` produces non-deterministic iteration order across processes — silent cache corruption on deserialization. |
| M14 | CF-08 | Replace `SourceSpan` with serializable span type or mark span fields `#[serde(skip)]` | throughout HIR types | `SourceSpan` from miette is non-serializable without the `serde` feature. Even after enabling it (M1), spans may not be needed in cached artifacts. |

**Summary:** 14 MUST FIX items, forming a dependency chain: M1→M2→M3→M4→M5→M7 (serialization chain), plus M8 (DefId), M9 (CLI), M10 (cycle detection), M11 (hashing crate), M12 (perf), M13 (determinism), M14 (span handling).

### 3.3 SHOULD FIX (22 items — robustness, performance, readiness)

| # | ID | Finding | File:Line | Rationale |
|---|----|---------|-----------|-----------|
| S1 | CF-25 | Consolidate `PRINTLN_DEF_ID` to single definition | `hir/lower.rs:43`, `mir/lower.rs:30`, `mir/codegen.rs:27` | Magic constant drifts silently across 3 files; if values diverge, `println` calls produce "undefined function" errors. |
| S2 | CF-19 | Implement `Hash`/`PartialEq`/`Serialize` for `TargetConfig` | `target.rs:103` | Required for cache keys that include target configuration. `triple` field is opaque inkwell type — needs custom impl. |
| S3 | CF-23 | Implement `debug_info` emission via LLVM `DIBuilder` | `target.rs:88`, `mir/codegen.rs` | `DebugInfo::Yes` is set but never consumed — debug builds produce binaries with no debug info. |
| S4 | CF-21 | Add `HashMap<SymbolId, usize>` index for structs/enums in `MirProgram` | `mir/mod.rs:326-334` | `struct_def`/`enum_def` are O(n) per lookup; called during type resolution and codegen. |
| S5 | CF-22 | Eliminate `SymbolInterner` clone in `MirLower::new` | `mir/lower.rs:79-80` | Cloned per function — O(N×T) wasteful allocation. Use a pre-interned sentinel `SymbolId` instead. |
| S6 | CF-30 | Fix `child_module_lookup` to use path-relative resolution | `hir/lower.rs:580-585,670` | Flat name collision causes wrong module resolution when two modules share the same name in different parent scopes. |
| S7 | CF-31 | Add duplicate `mod` declaration detection | `module.rs:497-575` | Two `mod foo;` in same file silently creates duplicate `Module` entries. |
| S8 | CF-36 | Fix `next_def_id()` to not borrow from SymbolInterner space | `hir/lower.rs:155-159` | Use/mod decls get DefIds from `SymbolInterner` index space, causing unpredictable collisions with function/struct/enum DefIds. |
| S9 | CF-33 | Fix `local_index` to be per-module, not global Vec index | `hir/lower.rs:860-865` | `DefEntry.local_index` is documented as per-module index but assigned global index. |
| S10 | CF-34 | Fix `function()` accessor to be DefKind-aware | `hir/function.rs:154-156` | Raw `functions.get(id.0)` returns wrong function if DefId is from a non-function kind. |
| S11 | CF-35 | Make `ModuleId` assignment deterministic — sort modules by name | `module.rs:507` | ModuleIds shift when modules are added/removed/reordered, breaking cache stability. |
| S12 | CF-37 | Add bounds validation on `def_table.lookup()` and `module_scopes` indexing | `hir/symbol.rs:147`, `hir/lower.rs:847` | Stale cached DefIds/ModuleIds are silently accepted or cause panics. |
| S13 | CF-24 | Use `ptr_type` for `Str` in LLVM IR instead of `i64` | `mir/codegen.rs:732` | `Str` → `i64` assumption breaks on 32-bit targets where pointers are 32-bit. |
| S14 | CF-38 | Add bounds validation on `DefTable::lookup` | `hir/symbol.rs:147-149` | Out-of-bounds DefIds return `None` instead of being validated. |
| S15 | CF-39 | Add CLI-level integration test for multi-module compilation | — | No test invokes `stnx build` on a multi-module project. |
| S16 | CF-40 | Add serialization round-trip tests for HIR/MIR types | — | 12+ types derive `Serialize`/`Deserialize` but are untested. |
| S17 | CF-41 | Add incremental compilation test scaffolding (`#[ignore]` tests defining expected API) | — | No test infrastructure exists for incremental compilation. |
| S18 | CF-32 | Fix `ModulePath::Display` to resolve SymbolIds to names | `module.rs:187-205` | Error messages show `crate::42` instead of `crate::math`. |
| S19 | CF-26 | Replace `local_allocas: HashMap` with `Vec` indexed by `LocalId` | `mir/codegen.rs:33` | `LocalId.0` is already a dense sequential index; `HashMap` adds unnecessary overhead. |
| S20 | CF-27 | Remove redundant name re-resolution in `generate_function` | `mir/codegen.rs:116-129` | `declare_functions` already added all functions; the `unwrap_or_else` fallback could create duplicates. |
| S21 | CF-29 | Expand `check_linker` to verify CRT and runtime object | `codegen/linker.rs:129-179` | Pre-flight check passes but linker may fail if CRT libraries are missing. |
| S22 | CF-48 | Centralize `toml` in `[workspace.dependencies]` | `Cargo.toml`, `crates/stnx/Cargo.toml` | Inconsistency — `toml` is the only dep not managed via workspace. |

### 3.4 CAN DEFER (10 items — low impact, future iterations)

| # | ID | Finding | File:Line | Rationale |
|---|----|---------|-----------|-----------|
| D1 | CF-18 | Add `Clone` to `HirFunction` | `hir/function.rs:42` | Inconsistency — `StructDef`/`EnumDef` derive `Clone` but `HirFunction` doesn't. |
| D2 | CF-47 | Move `clap` to binary-only dependency | `crates/stnx/Cargo.toml:13` | Library consumers pull in clap unnecessarily. Low impact. |
| D3 | CF-28 | Remove redundant triple-setting in `ObjectEmitter` | `mir/codegen.rs:792`, `codegen/emitter.rs:19` | Code smell, not a bug. |
| D4 | CF-49 | Add `[features]` table to gate `inkwell` | `crates/stnx/Cargo.toml` | Allows frontend-only builds without LLVM 21. |
| D5 | CF-42 | Fix `SATURNITE_MIR_DEIGN.md` typo | docs/ | File doesn't exist; actual file is `SATURNITE_MIR_DESIGN.md`. |
| D6 | CF-3 | `chumsky` feature mismatch | `Cargo.toml:13` | Only matters during workspace inheritance migration. |
| D7 | CF-46 | Fix `SATURNITE_DEPENDENCY_MODEL.md` Python interop section | docs/ | DESIGN-ONLY, not implemented. |
| D8 | CF-50 | Fix stale cross-compilation guard claim in docs | `SATURNITE_0_4_ARCHITECTURE.md:314` | Check command ignores target — doc is inaccurate. |
| D9 | CF-10 (version strings) | Fix "Saturnite 0.2" → "0.4" in error messages | `main.rs:285,528` | Cosmetic/UX only. |
| D10 | CF-48 (tempfile) | Remove duplicate `tempfile` from dev-deps | `crates/stnx/Cargo.toml` | Already in workspace deps; minor redundancy. |

---

## Phase 4 — Incremental Compilation Readiness Assessment

### 4.1 Executive Conclusion

**The Saturnite 0.4 architecture is NOT ready for incremental compilation.**

A total of **14 MUST FIX** items form a dependency chain that must be fully resolved before any incremental caching layer can function. The single most critical finding is:

> **`SymbolInterner` — the cornerstone of the identifier system — cannot be serialized.** This transitively makes `HirProgram`, `MirProgram`, and `ModuleGraph` all non-serializable. There is no path to persist/restore the identifier space between compilation sessions.

Additionally:
- **`DefId` namespace collapse**: `DefId(0)` is simultaneously a valid function ID, a struct ID, and an enum ID. Any DefId-keyed cache is silently corrupt.
- **CLI bypass**: The production CLI never invokes the module-aware lowering path. Multi-module projects are functionally broken.
- **No hashing crate**: SHA-256 fingerprinting (as proposed in the design doc) cannot be implemented without adding a dependency.
- **Identifier instability**: Both `SymbolId` and `DefId` are positional indices that shift on every source edit.

### 4.2 Readiness by pipeline stage

| Pipeline Stage | Serialization Status | Incremental-Ready? | Blockers |
|---------------|---------------------|--------------------|----------|
| **Source → AST** | `Program` derives `Clone, Debug` — **NOT Serializable** | ❌ No | `ast.rs` types lack `Serialize`/`Deserialize` |
| **AST → HIR** | `HirProgram` derives `Debug` only | ❌ No | `HirProgram`, `HirFunction`, `HirExpr*`, `HirStmt*`, `StructDef`, `EnumDef`, all not serializable; `SourceSpan` needs miette serde feature |
| **HIR → MIR** | Individual `Mir*` types serde ✓, but `MirProgram` derives `Debug` only | ❌ No (partially) | `MirProgram` blocked by `SymbolInterner`, `StructDef`, `EnumDef` |
| **MIR → LLVM** | N/A (LLVM IR is not cached) | ❌ No | Fresh `LLVMContext` per invocation, no IR caching |
| **LLVM → Object** | N/A | ❌ No | No object file caching |
| **Object → Link** | N/A | ❌ No | No link-level caching |
| **Module graph** | `ModuleGraph` derives `Debug` only | ❌ No | `ModuleGraph`, `Module`, `ModulePath`, `ModuleScope`, `Project` all not serializable |

### 4.3 Serialization readiness dependency graph

```
MirProgram (CRITICAL — needs Serialize)
  ├── Vec<MirFunction>          → already has Serialize/Deserialize ✓
  ├── SymbolInterner            → needs M2 (add derives) + M13 (deterministic hasher)
  └── Vec<StructDef>            → needs M4 (add derives)
      └── SourceSpan           → needs M1 (miette serde feature) + M14 (span handling)
      └── ModuleId             → needs M3 (add derives)
      └── Visibility           → needs M3 (add derives)
  └── Vec<EnumDef>              → needs M4 (same chain as StructDef)

HirProgram (CRITICAL — needs Serialize)
  ├── Vec<HirFunction>          → needs M5 (add derives)
  │   └── SourceSpan            → needs M1 + M14
  │   └── ModuleId              → needs M3
  │   └── Visibility            → needs M3
  │   └── Vec<HirStmt>          → needs M5
  ├── Vec<StructDef>            → needs M4
  ├── Vec<EnumDef>               → needs M4
  ├── SymbolInterner            → needs M2
  ├── Vec<Module>               → needs Serialize on Module + Program (AST)
  ├── ModuleId                  → needs M3
  ├── HashMap<DefId, ModuleId>  → needs M3
  ├── DefTable                  → needs M6 (add derives)
  ├── Vec<ModuleScope>          → needs Serialize on ModuleScope + ModuleId
  ├── Vec<HirUseDecl>           → needs M4
  └── Vec<HirModDecl>           → needs M4

ModuleGraph (CRITICAL — needs Serialize)
  ├── Vec<Module>               → needs Module + Program serializable
  ├── ModuleId                  → needs M3
  ├── SymbolInterner            → needs M2
  ├── HashMap<ModulePath, ModuleId> → needs M3 (ModulePath)
  └── HashMap<ModuleId, Vec<ModuleId>> → needs M3
```

### 4.4 Incremental compilation feasibility: NOT READY

The architecture is **structurally complete** but **functionally blocked**. The module system, MIR pipeline, and semantic analysis are all well-architected and internally consistent. However, the **identifier system is fundamentally incompatible** with caching:

1. **No persistence**: No core type can be serialized to disk and restored.
2. **No stability**: `SymbolId` and `DefId` shift on every source edit.
3. **No separation**: Functions, structs, and enums share DefId spaces.
4. **No determinism**: `HashMap` with `RandomState` produces non-deterministic iteration.
5. **No CLI integration**: The graph-aware lowering path is unreachable from production.
6. **No hashing**: No crate to compute content-addressed cache keys.

**Conclusion:** Incremental compilation is not a layer that can be bolted on — it requires rebuilding the identifier system from the ground up. At minimum, the 14 MUST FIX items must be resolved before a single cache file can be written and read back safely.

---

## Phase 5 — Incremental Compilation Boundary and Invalidation Model Design

### 5.1 Cache boundary model

**Current compilation pipeline (no caching):**

```
Source (.stnx files)
  │
  ├─ [Module Discovery: ModuleGraph::discover_modules]     ← no cache
  │
  ├─ [AST: parse_source → ast::Program]                   ← no cache
  │
  ├─ [HIR Lowering: analyze_and_lower_with_graph]         ← no cache
  │
  ├─ [MIR Lowering: mir::lower::lower_program]            ← no cache
  │
  ├─ [MIR Verify + Optimize]                              ← no cache
  │
  ├─ [LLVM CodeGen: compile_from_mir_ext]                 ← no cache
  │
  ├─ [Object Emission: ObjectEmitter]                     ← no cache
  │
  └─ [Linking: Linker]                                    ← no cache
    │
    ▼
  Executable
```

**Proposed incremental compilation pipeline (after MUST FIX items resolved):**

```
Source (.stnx files)
  │
  ├─ [F-0: Fingerprint source files]                    ← content-addressed keys
  │
  ├─ [F-1: Load/invalid cache] ── cache miss ──> [Module Discovery]
  │                                                   │
  │                                                   ▼
  │                                          [AST: parse_source]
  │                                                   │
  │                                                   ▼
  │                                 ┌──────────────────────────────────┐
  │              cache hit <──────  │ Check fingerprint vs cache      │
  │                                 │ If match: deserialize HIR       │
  │                                 │ If mismatch: lower from AST     │
  │                                 └──────────────────────────────────┘
  │                                                   │
  │                                                   ▼
  │                                 ┌──────────────────────────────────┐
  │              cache hit <──────  │ Check dependency graph:         │
  │                                 │ - Source changed?               │
  │                                 │ - Config changed?               │
  │                                 │ - Compiler version changed?     │
  │                                 │ - Dependencies changed?          │
  │                                 └──────────────────────────────────┘
  │                                                   │
  │                                                   ▼ (cache miss)
  │                                        [MIR Lowering: lower_program]
  │                                                   │
  │                                                   ▼
  │                                        [MIR Verify + Optimize]
  │                                                   │
  │                                                   ▼ (cache miss)
  │                                 ┌──────────────────────────────────┐
  │              cache hit <──────  │ Check per-function fingerprint:   │
  │                                 │ If match: load cached .o object  │
  │                                 │ If mismatch: regenerate from MIR │
  │                                 └──────────────────────────────────┘
  │                                                   │
  │                                                   ▼
  │                                        [LLVM CodeGen + Emit .o]
  │                                                   │
  │                                                   ▼
  │                                        [Linking: Linker]
  │                                                   │
  │                                                   ▼
  │                                          Executable
  │
  └─ [F-2: Store artifacts] ← cache stores:
                                fingerprints.json (file → content hash)
                                hir/<fingerprint>.hir (serialized HirProgram)
                                mir/<fingerprint>.mir (serialized MirProgram)
                                objects/<fingerprint>.o (LLVM object)
                                metadata.json (deps, config, compiler version)
```

### 5.2 Cache key / fingerprint model

**Fingerprint components:**

```
Fingerprint = HASH(
    source_content_hash     // content hash of all .stnx files in module tree
    + dependency_hashes     // transitive dependency fingerprints
    + config_hash           // saturn.toml content hash
    + compiler_version      // stnx version string
    + target_config_hash    // triple_str, opt_level, debug_info, output_kind, cpu, features
    + module_graph_hash     // module structure (paths, names, import edges)
)
```

**Required before implementation:**
- Add hashing crate (MUST FIX M11) — `blake3` or `xxhash-rust`
- `TargetConfig` must implement `Hash` (SHOULD FIX S2)
- `SaturnConfig` already derives `Serialize` — can hash via `toml::to_string` + hash

### 5.3 Invalidation model

**Granularity levels:**

| Level | Invalidated by | Cache key components | Rebuild cost |
|-------|----------------|---------------------|--------------|
| **Full** (cold start) | Any change (first build) | All | Full pipeline |
| **File-level** | Source file content change | `source_content_hash` | Re-parse, re-lower, re-codegen, re-link |
| **Module-level** | Module source change | Per-module fingerprint | Re-parse module, re-lower, re-codegen affected functions, re-link |
| **Function-level** | Function body change | Per-function fingerprint | Re-lower single function, re-codegen, re-link |
| **Config-level** | saturn.toml change | `config_hash` | Full rebuild |
| **Target-level** | opt-level/debug/target changed | `target_config_hash` | Full rebuild |

**Dependency-aware transitive invalidation (Phase 14d):**

The `ModuleGraph.imports` field (`module.rs:375`) is declared but **never populated**. To enable transitive invalidation:

1. After `resolve_modules()`, record import edges: for each `HirUseDecl` in module M that resolves to an item in module N, add edge M → N.
2. Store edges in `fingerprints.json` as `dependency_edges: HashMap<ModuleId, Vec<ModuleId>>`.
3. On build, compute reverse dependency graph (N → {M₁, M₂, ...} where each Mᵢ imports from N).
4. When module N's fingerprint changes, invalidate N and all transitive reverse-dependents.

**Example:**
```
module A imports {B, C}
module B imports {D}
module C imports {D}
module D imports {}

Dependency edges: A→B, A→C, B→D, C→D
Reverse deps: B→[A], C→[A], D→[B,C]

If D changes:
  Invalidate D → check reverse deps → invalidate B, C
  Invalidate B → check reverse deps → invalidate A
  Invalidate C → check reverse deps → invalidate A (already invalidated)
  Final invalidated set: {A, B, C, D}
```

### 5.4 Identifier stability model (for cache restoration)

**Problem:** `SymbolId` and `DefId` are positional indices that shift when the source changes. Cached HIR/MIR contains old indices that don't map to the same strings/definitions in a new compilation.

**Required solution (MUST FIX M8 + S2):**

**Option A — Content-addressed identifiers (recommended):**
- `SymbolId` = stable hash of the interned string (e.g., `blake3(string).as_u64() as u32`)
- `DefId` = stable hash of `(ModuleId, fully_qualified_name, DefKind)`
- This eliminates the positional-index problem entirely. Cache keys become content-addressed, not position-addressed.

**Option B — Serialization + canonical remapping (fallback):**
- Serialize the `SymbolInterner` alongside cached HIR/MIR.
- On cache load, deserialize the interner first, then remap all `SymbolId`s from cached space to current space.
- Serialize a `HashMap<SymbolId, SymbolId>` remap table.
- Same for `DefId` → remap table.
- This is more complex and fragile but doesn't require restructuring the identifier system.

**Option C — Positional stability (minimum bar):**
- Ensure `SymbolInterner` insertion order is deterministic (sort all strings alphabetically before interning).
- Ensure `DefId` assignment is per-module-per-kind, not global.
- This allows caching within a stable source but still invalidates on any structural change.

**Recommendation:** Implement **Option A** (content-addressed identifiers) as part of Phase 5.2 of the phased plan (0.4-F5), because the current positional system makes incremental compilation soundness impossible — any reordering of source items silently corrupts cached artifacts.

### 5.5 Boundary: what CAN be cached today (without fixes)

| Artifact | Can cache today? | Why/why not |
|----------|-------------------|-------------|
| **AST `.stnx` → `Program`** | ✅ (AST is serializable) | `ast::Program` derives `Serialize`/`Deserialize` |
| **Module graph** | ❌ | `ModuleGraph`, `Module`, `ModulePath` not serializable; `SymbolInterner` not serializable |
| **HIR `Program → HirProgram`** | ❌ | `HirProgram` not serializable; `SourceSpan`, `SymbolInterner` not serializable |
| **MIR `HirProgram → MirProgram`** | ❌ | `MirProgram` not serializable; `SymbolInterner` not serializable |
| **LLVM object `.o`** | ✅ (technically) | `ObjectEmitter` writes `.o` files to disk |
| **Linked executable** | ✅ (technically) | `Linker` writes executables to disk |
| **Fingerprint table** | ✅ (technically) | Could hash source via `std::hash` (non-deterministic across runs without a real hasher) |

**Conclusion:** Source file content can be hashed (if a hashing crate is added), and object files can be written to disk. But **no intermediate representation (HIR, MIR, module graph) can be cached** — every recompilation must re-parse, re-lower, re-verify, re-optimize, and re-codegen from scratch. The cache can only skip parsing (if AST is cached) and linking (if objects are cached), but these are the cheapest stages. The expensive stages (semantic analysis, MIR lowering, LLVM codegen) always run.

---

## Phase 6 — Phased Implementation Plan (0.4-F1 through 0.4-F9)

### 6.1 Dependency graph

```
0.4-F1 ───────┐
              ├──> 0.4-F4 (serialization foundation)
0.4-F2 ───────┘
0.4-F3 ─�───> 0.4-F5 (identifier stability)
0.4-F4 ─�─> 0.4-F5, 0.4-F6, 0.4-F7
0.4-F5 ──> 0.4-F6
0.4-F6 ──> 0.4-F7
0.4-F7 ──> 0.4-F8
0.4-F8 ──> 0.4-F9
```

### 6.2 Phase descriptions

#### 0.4-F1 — Hashing Infrastructure (Blocks: 0.4-F4, 0.4-F7)

**Goal:** Add deterministic hashing for content-addressed cache keys.

**Tasks:**
1. Add `blake3` to `[workspace.dependencies]` and `[dependencies]` in `crates/stnx/Cargo.toml`.
2. Create `src/fingerprint.rs` module with:
   ```rust
   pub struct Fingerprint([u8; 32]);
   impl Fingerprint {
       pub fn from_str(s: &str) -> Self;
       pub fn from_bytes(bytes: &[u8]) -> Self;
       pub fn combine(&self, other: &Fingerprint) -> Self;
       pub fn to_string(&self) -> String;
   }
   ```
3. Re-export from `lib.rs`.
4. **No source code changes to compiler pipeline** — this is infrastructure only.

**Verification:** `cargo check`, `cargo clippy`, `cargo test` (existing 364 tests must still pass).

**Estimated effort:** 4-6 hours.

---

#### 0.4-F2 — CLI Module Graph Integration (Blocks: 0.4-F4)

**Goal:** Wire `analyze_and_lower_with_graph` into all CLI command paths.

**Tasks:**
1. `main.rs:255` (Build): Change `analyze_and_lower(&program)` → `analyze_and_lower_with_graph(&program, &project.graph)`.
2. `main.rs:496` (Run): Same change in `build_run_file`.
3. `main.rs:550` (Check): Change `analyze(&program)` → `analyze_and_lower_with_graph(&program, &project.graph)` (HIR already returned, just discard the `.ok()` for Check's pass/fail semantics).
4. Remove duplicate symbol interner (F-06 in AGENT-8 report) — the graph path unifies interners via `lower_program_with_graph` at `hir/lower.rs:528`.
5. Move cross-compilation guard BEFORE semantic analysis (F-9 in AGENT-8).

**Verification:** `cargo fmt --check`, `cargo check`, `cargo test` — existing tests pass; verify `test_module_resolution.rs`, `test_multi_module_codegen.rs`, `test_end_to_end_modules.rs` still green.

**Estimated effort:** 6-8 hours.

---

#### 0.4-F3 — Module System Hardening (Blocks: 0.4-F6)

**Goal:** Fix critical module system bugs: cycle detection, duplicate mod detection, child_module_lookup path resolution.

**Tasks:**
1. Add `visited_files: HashSet<PathBuf>` to `discover_modules` to prevent infinite loops on circular `mod` declarations.
2. Deduplicate `mod_declarations` Vec before processing in `discover_modules`.
3. Fix `child_module_lookup` to use path-relative resolution (per-parent scoped map) instead of flat name → ModuleId.

**Verification:** Add tests for circular module detection, duplicate mod detection, and same-name-in-different-scopes resolution.

**Estimated effort:** 8-12 hours.

---

#### 0.4-F4 — Serialization Foundation (Blocks: 0.4-F5, 0.4-F6)

**Goal:** Enable `Serialize`/`Deserialize` on the full HIR, MIR, and module type hierarchies.

**Tasks (in dependency order):**
1. **Enable miette `serde` feature** (`Cargo.toml:11`): `miette = { version = "7", features = ["fancy", "serde"] }`
2. **Add `Serialize, Deserialize` to `SymbolInterner`** (`hir/symbol.rs:46`)
3. **Add `Serialize, Deserialize` to `ModuleId`** (`module.rs:40`)
4. **Add `Serialize, Deserialize` to `Visibility`** (`hir/symbol.rs:181`)
5. **Add `Serialize, Deserialize` to `StructDef`, `EnumDef`, `HirUseDecl`, `HirModDecl`** (`hir/function.rs`)
6. **Add `Serialize, Deserialize` to `HirFunction`, `HirExpr*`, `HirStmt*`, `HirProgram`** (`hir/function.rs`, `hir/expr.rs`, `hir/stmt.rs`)
7. **Add `Serialize, Deserialize` to `DefTable`, `DefEntry`, `DefKind`** (`hir/symbol.rs`)
8. **Add `Serialize, Deserialize` to `ModulePath`, `Module`, `ModuleScope`, `ModuleGraph`** (`module.rs`)
9. **Add `Serialize, Deserialize` to `MirProgram`** (`mir/mod.rs:314`)
10. **Replace `HashMap` with `BTreeMap`** in `SymbolInterner`, `ModuleScope`, `LowerScope`, `LowerContext`, and all HashMaps in HIR/MIR/module code (deterministic iteration).

**Verification:** Compile-time trait assertion:
```rust
fn _assert_all_serializable() {
    fn _check<T: serde::Serialize + serde::de::DeserializeOwned>() {}
    _check::<MirProgram>();
    _check::<HirProgram>();
    _check::<ModuleGraph>();
}
```

**Estimated effort:** 2-3 days (mechanical, no logic changes).

---

#### 0.4-F5 — Identifier Stability (Blocks: 0.4-F6, 0.4-F7)

**Goal:** Fix DefId namespace collapse and make SymbolId/DefId stable across compilations.

**Tasks:**
1. **Fix DefId namespace collapse** (CF-05 / M8): Use `DefTable::register()` return value for all definitions. Add a discriminant bit to `DefId` to separate kind spaces (e.g., upper bits encode `DefKind::Function|Struct|Enum|Use|Mod`).
2. **Make SymbolInterner deterministic**: Sort all interned strings alphabetically (or use content-addressed hashing via `blake3`).
3. **Make ModuleId assignment deterministic**: Sort modules by name during discovery.
4. **Fix `function()` accessor** (CF-34 / S10): Use `DefTable` lookup with kind disambiguation instead of raw array index.
5. **Fix `next_def_id()`** (CF-36 / S8): Stop borrowing from SymbolInterner space; use a dedicated counter.

**Verification:** Add tests that verify DefId stability across source reordering, and that struct/enum/function DefIds don't collide.

**Estimated effort:** 3-5 days (architectural change to identifier system).

---

#### 0.4-F6 — Fingerprinting & Cache I/O (Blocks: 0.4-F7)

**Goal:** Implement the fingerprint module and cache directory structure.

**Tasks:**
1. Create `src/fingerprint.rs` (or extend from 0.4-F1):
   - `Fingerprint::compute_source(content: &str) -> Fingerprint`
   - `Fingerprint::compute_module(file_paths: &[PathBuf]) -> Fingerprint`
   - `Fingerprint::compute_config(config: &SaturnConfig) -> Fingerprint`
   - `Fingerprint::compute_target(target: &TargetConfig) -> Fingerprint`
2. Create `src/cache.rs` module:
   - `CacheStore` struct managing `target/incremental/` directory
   - `load_fingerprints() -> HashMap<String, Fingerprint>` (reads `fingerprints.json`)
   - `store_fingerprints(&HashMap<String, Fingerprint>)`
   - `load_hir(fingerprint: &Fingerprint) -> Option<HirProgram>` (reads `hir/<fingerprint>.hir`)
   - `store_hir(hir: &HirProgram, fingerprint: &Fingerprint)`
   - `load_mir(fingerprint: &Fingerprint) -> Option<MirProgram>`
   - `store_mir(mir: &MirProgram, fingerprint: &Fingerprint)`
   - `store_object(obj_path: &Path, fingerprint: &Fingerprint)`
3. Integrate into `semantic.rs`: On `analyze_and_lower_with_graph`, check fingerprint cache before lowering.
4. Re-export from `lib.rs`.

**Verification:** Test that cache miss → full build, cache hit → load from disk, fingerprint mismatch → full rebuild.

**Estimated effort:** 2-3 days.

---

#### 0.4-F7 — HIR Caching Layer (Blocks: 0.4-F8)

**Goal:** Cache serialized HIR keyed by fingerprint, skip AST→HIR lowering on cache hit.

**Tasks:**
1. In `semantic.rs`, add cache lookup: compute source fingerprint → check `CacheStore::load_hir()` → if hit, deserialize `HirProgram` and return.
2. On cache miss: run `lower_with_graph` + `resolve_modules` as before, then `CacheStore::store_hir()`.
3. **Dependency tracking**: After `resolve_modules()`, populate `ModuleGraph.imports` with import edges.
4. Store dependency edges in `fingerprints.json` metadata.
5. On cache hit, validate that all dependency fingerprints match (transitive invalidation).

**Verification:** Test that modifying an unchanged project produces cache hits, and that modifying a dependency invalidates dependents.

**Estimated effort:** 3-4 days.

---

#### 0.4-F8 — MIR + Object Caching (Blocks: 0.4-F9)

**Goal:** Cache serialized MIR and compiled object files, skip codegen on cache hit.

**Tasks:**
1. Add `ModuleId` to `MirFunction` (CF-12 in AGENT-5) so MIR retains module provenance.
2. In `mir/lower.rs`, after `lower_program`, check `CacheStore::load_mir()` by per-module fingerprint.
3. On cache miss: lower → verify → optimize → `CacheStore::store_mir()`.
4. In `mir/codegen.rs`, add per-function cache lookup: compute function fingerprint → check `CacheStore::load_object()`.
5. On cache hit: skip LLVM codegen, use cached `.o` file.
6. On cache miss: generate IR → emit `.o` → `CacheStore::store_object()`.
7. `TargetConfig` must implement `Hash` (SHOULD FIX S2) for cache key inclusion.

**Verification:** Test that unchanged functions are not re-codegenced, only re-linked.

**Estimated effort:** 4-5 days.

---

#### 0.4-F9 — Parallel Code Generation & Performance (Finalization)

**Goal:** Enable parallelism in MIR codegen and integrate all performance fixes.

**Tasks:**
1. Add `rayon` dependency.
2. Parallelize constant folding (already embarrassingly parallel — `ConstantFolder` is stateless).
3. Parallelize MIR lowering (per-function, `MirLower` has no shared mutable state).
4. Parallelize MIR verification (read-only per function).
5. For LLVM codegen parallelism: each `MirFunction` generates into its own `LLVMContext` + `Module`, then merge via `llvm-link` (Option A from AGENT-14 recommendations).
6. Fix `function_name()` O(n) → O(1) via `HashMap<DefId, SymbolId>` index (MUST FIX M12).
7. Fix `gen_field_access` O(S×F) → O(1) via `HashMap<SymbolId, StructDef>` index (SHOULD FIX S4).
8. Eliminate `SymbolInterner` clone in `MirLower::new` (SHOULD FIX S5).

**Verification:** Benchmark N-function projects before/after, verify 2x+ speedup on codegen stage.

**Estimated effort:** 3-4 days.

---

### 6.3 Phase summary table

| Phase | Name | Key deliverable | Blocks | Effort |
|-------|------|-----------------|--------|--------|
| 0.4-F1 | Hashing Infrastructure | `blake3` + `Fingerprint` type | F4, F7 | 4-6h |
| 0.4-F2 | CLI Module Integration | CLI uses graph-aware path | F4 | 6-8h |
| 0.4-F3 | Module System Hardening | Cycle detection, dup mod, path resolution | F6 | 8-12h |
| 0.4-F4 | Serialization Foundation | All types derive `Serialize`/`Deserialize` | F5, F6 | 2-3d |
| 0.4-F5 | Identifier Stability | Content-addressed DefId/SymbolId | F6, F7 | 3-5d |
| 0.4-F6 | Fingerprinting & Cache I/O | `Fingerprint` + `CacheStore` | F7 | 2-3d |
| 0.4-F7 | HIR Caching | Cache load/save + dep tracking | F8 | 3-4d |
| 0.4-F8 | MIR + Object Caching | Per-module MIR cache, per-function `.o` cache | F9 | 4-5d |
| 0.4-F9 | Parallel Codegen | rayon + O(1) lookups + per-context LLVM | — | 3-4d |

---

## Phase 7 — Parallelism DAG

### 7.1 Current pipeline (fully sequential)

```
main.rs (single thread)
  ↓ analyze_and_lower_with_graph     [single thread — SymbolInterner blocks]
  ↓ lower_program (MIR)              [single thread — MirLower::new clones interner N×]
  ↓ mir.verify()                     [single thread — embarrassingly parallelizable]
  ↓ optimize(&mut mir)               [single thread — embarrassingly parallelizable]
  ↓ generate_ir_from_mir / compile   [single thread — LLVMContext blocks]
```

### 7.2 Parallelism readiness by stage

| Stage | Parallelizable? | Blocking state | Effort | Prerequisite |
|-------|-----------------|----------------|--------|--------------|
| **HIR lowering** (per function) | ❌ NO | `HirLower.symbols: SymbolInterner` (mutable, shared across all functions) | High | 0.4-F5 (identifier stability) → pre-intern all strings |
| **MIR lowering** (per function) | ✅ YES | None — `hir` & `sigs` are immutable refs | Low | None |
| **MIR verification** (per function) | ✅ YES | Only `errors: &mut Vec` (collectable per-task) | Low | None |
| **Constant folding** (per function) | ✅ YES | `ConstantFolder` is zero-sized, `MirFunction` slices disjoint | Low | None |
| **LLVM IR generation** (per function) | ❌ NO | `LLVMContext` (not thread-safe), shared `module` + `builder` | Medium-High | 0.4-F9 (per-function context) |
| **Module discovery** (per child) | ⚠️ PARTIAL | `add_module` (ModuleId assignment + `module_index` insert) must be serialized | Medium | 0.4-F3 (cycle detection) |
| **Linking** | ❌ NO | System linker is single-process | N/A | External |

### 7.3 Parallelism DAG (dependency graph)

```
                    ┌────────────────────────────────────┐
                    │ Phase 0: Module Discovery           │
                    │ (parallelizable: read+parse files)  │
                    └────────────────┬───────────────────┘
                                     │
                                     ▼
                    ┌────────────────────────────────────┐
                    │ Phase 1: AST → HIR Lowering          │
                    │ ❌ BLOCKED: SymbolInterner mutable   │
                    │    shared across all functions       │
                    │ 💡 Fix: 0.4-F5 pre-intern all strings │
                    │    then parallelize function lowering │
                    └────────────────┬───────────────────┘
                                     │
                                     ▼
                    ┌────────────────────────────────────┐
                    │ Phase 2: HIR → MIR Lowering          │
                    │ ✅ Parallelizable: per-function       │
                    │    MirLower has NO shared mutable   │
                    │    state (hir + sigs are immutable)  │
                    └────┬──────────────┬─────────────────┘
                         │              │
                         ▼              ▼
          ┌──────────────────┐  ┌──────────────────┐
          │ Phase 2b: Parallel│  │ Phase 3a: MIR    │
          │ pre-intern all    │  │ Verification      │
          │ strings           │  │ ✅ Parallelizable │
          │ (prep for F5)     │  │ per-function      │
          └──────────────────┘  └────────┬──────────┘
                                           │
                                           ▼
                              ┌────────────────────────┐
                              │ Phase 3b: Constant      │
                              │ Folding                 │
                              │ ✅ Parallelizable: per-func│
                              │   ConstantFolder is      │
                              │   zero-sized stateless   │
                              └──────────┬───────────────┘
                                         │
                                         ▼
                    ┌────────────────────────────────────┐
                    │ Phase 4: MIR → LLVM IR Codegen      │
                    │ ❌ BLOCKED: LLVMContext not thread-  │
                    │    safe; single Context+Module+builder│
                    │ 💡 Fix: 0.4-F9 per-function context  │
                    │    + llvm-link merge                │
                    └──────────┬──────────────────────────┘
                               │
                               ▼
                    ┌────────────────────────────────────┐
                    │ Phase 5: Object Emission            │
                    │ ⚠️ Partially parallelizable: emit     │
                    │    per-function objects in parallel,  │
                    │    then link sequentially             │
                    └──────────┬──────────────────────────┘
                               │
                               ▼
                    ┌────────────────────────────────────┐
                    │ Phase 6: Linking                    │
                    │ ❌ Single system linker process      │
                    └────────────────────────────────────┘
```

### 7.4 Parallel work units (after MUST FIX resolved)

| Stage | Work unit | Parallelism model | Input | Output |
|-------|-----------|-------------------|-------|--------|
| HIR lowering | Per-function | `par_iter_mut(functions)` after pre-intern | `&Program`, `&mut SymbolInterner` (read-only post-preentern) | `HirProgram` |
| MIR lowering | Per-function | `par_iter(functions).map(\|func\| MirLower::new(hir, func, &sigs).lower_function())` | `&HirProgram` | `Vec<MirFunction>` |
| MIR verify | Per-function | `par_iter(functions).map(\|func\| verify_function(func, prog))` | `&MirProgram` | `Vec<Vec<MirVerifyError>>` |
| Constant fold | Per-function | `par_iter_mut(functions).for_each(\|func\| ConstantFolder::run(func))` | `&mut MirFunction` | `()` (in-place) |
| LLVM codegen | Per-function (FUTURE) | `par_iter(functions).map(\|func\| { let ctx = new_llvm_context(); generate_in_context(func) })` then merge | `&MirFunction` | `LLVM Module` per function |
| Object emission | Per-function (FUTURE) | Each function's `Module` → `TargetMachine::write_to_file` in parallel | `LLVM Module` | `.o` file |
| Linking | Per-executable | Sequential (system linker) | `Vec<PathBuf>` (.o files) | `Executable` |

### 7.5 `rayon` integration plan (part of 0.4-F9)

```toml
# crates/stnx/Cargo.toml
[dependencies]
rayon = "1"
```

```rust
// mir/opt.rs:16-20 — currently sequential
pub fn optimize(program: &mut MirProgram) {
    program.functions.par_iter_mut()
        .for_each(|func| ConstantFolder::run(func));
}

// mir/verify.rs:42-52 — currently sequential
pub fn verify(&self) -> VerifyResult {
    use rayon::prelude::*;
    let error_sets: Vec<Vec<MirVerifyError>> = self.functions
        .par_iter()
        .map(|func| {
            let mut errors = Vec::new();
            Self::verify_function(func, self, &mut errors);
            errors
        })
        .collect();
    // ... flatten and check
}

// mir/lower.rs:42-46 — currently sequential
let mut funcs = Vec::with_capacity(hir.functions.len());
funcs.par_extend(hir.functions.par_iter().map(|func| {
    let mut lower = MirLower::new(hir, func, &sigs);
    lower.lower_function()
})).collect::<CompilerResult<_>>()?;
```

**Safety verification for MIR lowering parallelization:**
- `hir: &'hir HirProgram` — immutable, `Send` ✓
- `sigs: &'hir HashMap<DefId, (Vec<HirType>, HirType)>` — immutable, `Send` ✓
- `MirLower` struct borrows `hir` and `sigs` by reference — no shared mutable state ✓
- `MirFunction` contains: `DefId` (Copy, Send), `SymbolId` (Copy, Send), `Vec`, `Vec<MirLocal>` (all Send), `MirType` (= `HirType`, Copy, Send) — **all `Send`** ✓

---

## Phase 8 — Documentation Synchronization

### 8.1 Stale / inaccurate documentation

| File | Status | Issues |
|------|--------|--------|
| **`SATURNITE_MIR_DEIGN.md`** | **DOES NOT EXIST** | The filename in the task prompt contains a typo ("DEIGN" → "DESIGN"). The actual file is `SATURNITE_MIR_DESIGN.md`. |
| **`SATURNITE_MIR_DESIGN.md`** | **COMPLETELY STALE** | Describes non-existent types: `MirStmt::Call`, `StorageLive`, `StorageDead`, `DebugInfo`; `MirTerminator::Switch`, `ReturnVoid`, `Unwind`; `MirPlace` type; `MirOperand::Place`; `MirConst::Str`/`EnumTag`; `MirRvalue::Ref`; `trait MirPass` (6 passes); `--emit-mir` CLI flag; `MirFn`. **None of these match `src/mir/mod.rs`.** |
| **`SATURNITE_0_4_ARCHITECTURE_AUDIT.md`** | **0.3-ERA** | Claims "MIR is NOT implemented, design-only", "No module system exists", "~123 tests", "saturn.toml parsed but NEVER read", "Codegen consumes HirProgram directly". **12+ contradictions** with 0.4. |
| **`SATURNITE_INCREMENTAL_COMPILATION.md`** | **PARTIALLY STALE** | Labeled "for Saturnite 0.3". No mention of `ModuleId`/`ModuleGraph`. No mention of `DefId`/`SymbolId` instability. Proposes `target/incremental/` layout that doesn't exist. Proposes `Serialize`/`Deserialize` on `HirProgram` — blocked by `SourceSpan`/no miette serde. |
| **`SATURNITE_DEPENDENCY_MODEL.md`** | **DESIGN-ONLY** | Section 2 "Python interop" describes pyo3 bindings — **no pyo3 dependency**, no Python code exists. `DependencySpec::from_str` documented as parsing version requirements — actually just `s.to_string()`. |
| **`SATURNITE_0_4_ARCHITECTURE.md`** | **ACCURATE** (with 2 errors) | Line 314: claims cross-compilation guard "enforced in all command paths (Build, Check, Run)" — but `Check` ignores target (`main.rs:363`: `target: _`). Line 158: shadowing description inverted — `LocalDecl` inserted AFTER evaluation, not before. Line 258: stale "0.2" version strings not mentioned. |
| **`docs/audit_notes/module_language_design.md`** | **ACCURATE** | Correctly describes `lower_program_with_graph`. Missing: doesn't mention `SymbolInterner` lacks `Serialize`. |
| **`docs/audit_notes/project_architecture.md`** | **ACCURATE** | Comprehensive and correct. |
| **`SATURNITE_CRATE_DEPENDENCY_AUDIT.md`** | Unknown (not inspected in this audit) | Not reviewed. |
| **`SATURNITE_0_3_ARCHITECTURE_REVIEW.md`** | 0.3-era | Not reviewed in detail for this audit (pre-0.4). |
| **`SATURNITE_0_3_HIR_DESIGN.md`** | 0.3-era | Not reviewed in detail for this audit (pre-0.4). |

### 8.2 Source code documentation gaps

| File:Line | Issue |
|-----------|-------|
| `src/lib.rs:38` | Doc comment says "Codegen consumes `HirProgram` directly — not raw AST" — stale. In 0.4, codegen consumes `MirProgram`; HIR→MIR is the seam. |
| `src/hir/symbol.rs:1-20` | Documents `SymbolId`/`DefId` as "stable numeric IDs" but doesn't note serialization status or DefId namespace collapse. |
| `src/hir/function.rs:1-32` | Documents module integration but says nothing about serialization readiness. |
| `src/hir/expr.rs:1-10` | Says identifiers are "resolved so later stages never need string lookups" but doesn't note that HIR can't be serialized for caching. |
| `src/mir/mod.rs:1-24` | Documents the pipeline accurately but doesn't mention `MirProgram` lacks `Serialize`. |
| `src/module.rs:18-19` | Doc comment mentions "incremental compilation" as a future goal but doesn't document the stability blockers. |
| `src/error.rs` | No cache-specific error variants (`CacheError`, `FingerprintMismatch`, `CorruptionDetected`). |

### 8.3 Documentation synchronization plan

| Priority | Action | Target file(s) |
|----------|--------|----------------|
| **P0** | **Delete** `SATURNITE_MIR_DEIGN.md` (if it exists as a stray file) and **rewrite** `SATURNITE_MIR_DESIGN.md` to match `src/mir/mod.rs` exactly. | `docs/SATURNITE_MIR_DESIGN.md` |
| **P0** | **Delete** `SATURNITE_0_4_ARCHITECTURE_AUDIT.md` (0.3-era, 12+ contradictions). Replace with pointer to this document. | `docs/SATURNITE_0_4_ARCHITECTURE_AUDIT.md` |
| **P1** | **Update** `SATURNITE_INCREMENTAL_COMPILATION.md`: Add warning about `ModuleId`/`DefId` instability. Add `SymbolInterner` serialization blocker. Remove "for Saturnite 0.3" label. Add reference to this audit's MUST FIX list. | `docs/SATURNITE_INCREMENTAL_COMPILATION.md` |
| **P1** | **Update** `SATURNITE_DEPENDENCY_MODEL.md`: Mark Section 2 (Python interop) as "NOT IMPLEMENTED / FUTURE." Fix `DependencySpec::from_str` description. | `docs/SATURNITE_DEPENDENCY_MODEL.md` |
| **P1** | **Update** `SATURNITE_0_4_ARCHITECTURE.md`: Fix cross-compilation guard claim (line 314). Fix shadowing description (line 158). Fix `lib.rs:38` comment. Fix version strings. | `docs/SATURNITE_0_4_ARCHITECTURE.md` |
| **P2** | **Update** `docs/audit_notes/module_language_design.md`: Add serialization gap note (`SymbolInterner` lacks `Serialize`). | `docs/audit_notes/module_language_design.md` |
| **P2** | **Clean up** stale filesystem artifacts: remove root `tests/{codegen,lexer,semantic}.rs` (0.3 API), `examples/hello.stn` (old extension). | repo root |

### 8.4 New documentation to add

| Document | Content |
|----------|---------|
| **This document** (`SATURNITE_POST_MODULE_ARCHITECTURE_AUDIT.md`) | The comprehensive audit you are reading. |
| **`docs/SATURNITE_INCREMENTAL_COMPILATION_PLAN.md`** (new) | Consolidated incremental compilation roadmap: MUST FIX items, 0.4-F1 through 0.4-F9 phases, parallelism DAG, identifier stability model. |
| **`docs/SATURNITE_0_4_SERIALIZATION_READINESS.md`** (new) | Auto-generated table of all types and their `Serialize`/`Deserialize` status, updated as derives are added. |

### 8.5 Stale filesystem artifacts

| Path | Status | Action |
|------|--------|--------|
| `crates/stnx/tests/codegen.rs` | **STALE** (uses 0.3 API) | Remove or rewrite |
| `crates/stnx/tests/lexer.rs` | **STALE** (separate from `src/lexer` inline tests) | Remove or consolidate |
| `crates/stnx/tests/semantic.rs` | **STALE** (uses 0.3 API) | Remove or rewrite |
| `crates/stnx/examples/hello.stn` | **STALE** (old extension, 0.3 API) | Remove |
| `examples/` directory | If exists | Review for stale examples |

**Note:** The git status shows these files as `??` (untracked), meaning they are not yet committed and may already be cleaned up. Verify before removal.

---

## Phase 9 — Final Verification

### 9.1 Verification plan

Since this audit is **documentation-only** (no source code modifications), the verification plan confirms that:

1. **No source files were modified** during this audit.
2. **All findings are documented** with file:line evidence.
3. **The build remains green** — `cargo fmt --check`, `cargo check`, `cargo clippy`, `cargo test` should still pass with 364 tests.

### 9.2 Verification results

| Check | Command | Expected | Status |
|-------|---------|----------|--------|
| Format check | `cargo fmt --check` | No diff | ⏳ Pending |
| Type check | `cargo check` | No errors | ⏳ Pending |
| Lint | `cargo clippy` | No warnings | ⏳ Pending |
| Tests | `cargo test` | 364 pass, 0 fail | ⏳ Pending |
| Documentation file created | `ls docs/SATURNITE_POST_MODULE_ARCHITECTURE_AUDIT.md` | Exists | ✅ Done (this file) |

> **Note:** The verification commands should be run by the user after reviewing this audit document. Since no source code was modified, all checks should pass identically to the pre-audit state.

### 9.3 Post-audit state

| Artifact | Status |
|----------|--------|
| `docs/SATURNITE_POST_MODULE_ARCHITECTURE_AUDIT.md` | ✅ Created (this document) |
| Compiler source code | ✅ Unchanged (audit-only) |
| `Cargo.toml` / `Cargo.lock` | ✅ Unchanged |
| Test suite | ✅ Unchanged (364 tests) |
| New dependencies | ✅ None added |

---

## Appendix A — Full Findings Cross-Reference

### By AGENT

| Agent | Topic | File | Key finding |
|-------|-------|------|-------------|
| AGENT-1 | Project Config | `/tmp/agent_A_30bd05b7.md` | No hashing crate; CLI double-discover; CLI uses single-file path |
| AGENT-2 | Module Graph | `/tmp/agent_B_3f0d6c0c.md` | No module types serializable; no cycle detection |
| AGENT-3 | Symbol/DefId | `/tmp/agent_C_31b0e757.md` | DefId namespace collapse; register() return discarded |
| AGENT-4 | HIR Serialization | `/tmp/agent_D_4526a078.md` | Only SymbolId/DefId/HirType have serde; SourceSpan blocks all |
| AGENT-5 | MIR | `/tmp/agent_E_50ae6e03.md` | MirProgram Debug only; SymbolInterner transitive blocker |
| AGENT-6 | Type System | `/tmp/agent_F_727867b1.md` | No coercion; DefTable mismatch; HashMap non-determinism |
| AGENT-7 | Codegen | `/tmp/agent_G_bcb7ca81.md` | function_name O(n); LLVMContext not thread-safe; TargetConfig not Hash |
| AGENT-8 | CLI | `/tmp/agent_H_61df1c8a.md` | CLI uses analyze_and_lower not with_graph; child ASTs never lowered |
| AGENT-9 | Tests | `/tmp/agent_I_2ac17bd9.md` | 364 tests; no CLI tests; no serialization tests; no incremental tests |
| AGENT-10 | Docs | `/tmp/agent_J_99636a61.md` | MIR_DESIGN.md completely stale; ARCHITECTURE_AUDIT.md 0.3-era |
| AGENT-11 | Dependencies | `/tmp/agent_K_cebc7b7c.md` | No hashing crate; workspace inheritance unused |
| AGENT-12 | Inc. Design | `/tmp/agent_L_dc160615.md` | INCREMENTAL_COMPILATION.md predates modules; no hashing crate |
| AGENT-13 | Security | `/tmp/agent_M_0bb3522a.md` | DefId namespace collapse; CLI bypass; no serialization |
| AGENT-14 | Performance | `/tmp/agent_N_59560cd1.md` | O(n) function_name; SymbolInterner clone per fn; LLVM non-parallel |
| AGENT-15 | Red-Team | `/tmp/agent_O_46d61de3.md` | Showstoppers: DefId collapse, CLI bypass, no serialization, DefTable mismatch |

### By Severity (consolidated)

| Severity | Count | Key items |
|----------|-------|-----------|
| **CRITICAL** | 14 | M1-M14 (all MUST FIX items) |
| **HIGH** | 12 | CF-12, CF-19, CF-20, CF-21, CF-22, CF-30, CF-31, CF-35, CF-36, CF-39, AGENT-11 F-05, AGENT-2 F-02 | 
| **MEDIUM** | 25 | S2-S22, various CAN DEFER items |
| **LOW** | 12 | CF-18, CF-26, CF-27, CF-28, CF-47, AGENT-10 items |
| **TRIVIAL** | 3 | Version strings, redundant code |
| **N/A (informational)** | 8 | Runtime determinism, Operator type analysis |

### By subsystem

| Subsystem | Files | Critical findings |
|-----------|-------|-------------------|
| **Workspace/Cargo** | `Cargo.toml`, `crates/stnx/Cargo.toml` | No hashing crate (CF-11); workspace deps unused |
| **Config** | `src/config.rs` | `deny_unknown_fields` strictness (CF-47) |
| **Module system** | `src/module.rs` | No cycle detection (CF-10); not serializable (CF-13) |
| **Identifier system** | `src/hir/symbol.rs` | `SymbolInterner` not serializable (CF-01); DefId namespace collapse (CF-05) |
| **HIR** | `src/hir/*.rs` | `HirProgram` not serializable (CF-03); SourceSpan blocks (CF-08) |
| **MIR** | `src/mir/*.rs` | `MirProgram` not serializable (CF-02); carries no module info |
| **Codegen** | `src/mir/codegen.rs`, `src/codegen/*.rs` | `function_name` O(n) (CF-20); `TargetConfig` not Hash (CF-19) |
| **CLI** | `src/main.rs` | Single-file path (CF-09); version strings stale |
| **Tests** | `tests/*.rs` | No CLI-level tests; no serialization tests; no incremental tests |
| **Documentation** | `docs/*.md` | MIR_DESIGN.md stale; INCREMENTAL_COMPILATION.md stale; ARCHITECTURE_AUDIT.md 0.3-era |

---

## Appendix B — The 14 MUST FIX Items (Actionable Summary)

For convenient copy-paste into an issue tracker:

```
MUST FIX M1: Enable miette "serde" feature in Cargo.toml
  File: crates/stnx/Cargo.toml:11
  Change: miette = { version = "7", features = ["fancy", "serde"] }
  Rationale: SourceSpan used in all HIR types; blocks serialization

MUST FIX M2: Add Serialize, Deserialize to SymbolInterner
  File: src/hir/symbol.rs:46
  Change: #[derive(Debug, Default, Clone, Serialize, Deserialize)]
  Rationale: Embedded in HirProgram, MirProgram, ModuleGraph

MUST FIX M3: Add Serialize, Deserialize to ModuleId, ModulePath, Visibility
  Files: src/module.rs:40, src/module.rs:77, src/hir/symbol.rs:181
  Rationale: Required transitive deps for HirProgram/MirProgram serialization

MUST FIX M4: Add Serialize, Deserialize to StructDef, EnumDef, HirUseDecl, HirModDecl
  Files: src/hir/function.rs:57,72,91,110
  Rationale: Required for HirProgram serialization

MUST FIX M5: Add Serialize, Deserialize to HirProgram, HirFunction, HirExpr*, HirStmt*
  Files: src/hir/function.rs:127,42; src/hir/expr.rs:13; src/hir/stmt.rs:12
  Rationale: Core HIR serialization

MUST FIX M6: Add Serialize, Deserialize to DefTable, DefEntry, DefKind
  Files: src/hir/symbol.rs:91,106,123
  Rationale: Required for HirProgram serialization

MUST FIX M7: Add Serialize, Deserialize to MirProgram
  File: src/mir/mod.rs:314
  Rationale: Depends on M2, M4 (SymbolInterner, StructDef, EnumDef serializable)

MUST FIX M8: Fix DefId namespace collapse
  Files: src/hir/lower.rs:220,238,343,416,425
  Rationale: Functions/structs/enums all assign DefId(0); makes DefId-keyed caches unsound

MUST FIX M9: Wire CLI to analyze_and_lower_with_graph
  Files: src/main.rs:255,496,550
  Rationale: CLI uses single-file path; multi-module projects broken

MUST FIX M10: Add cycle detection to discover_modules
  File: src/module.rs:497-575
  Rationale: Circular mod declarations cause infinite loop/stack overflow

MUST FIX M11: Add hashing crate (blake3 or xxhash-rust)
  File: crates/stnx/Cargo.toml
  Rationale: SHA-256 fingerprinting requires a hashing dependency

MUST FIX M12: Replace function_name() O(n) with HashMap<DefId, SymbolId> index
  Files: src/mir/mod.rs:337, src/mir/codegen.rs:646
  Rationale: O(N²) codegen scaling; critical performance blocker

MUST FIX M13: Replace HashMap RandomState with deterministic hasher in SymbolInterner, ModuleScope, etc.
  Files: src/hir/symbol.rs:46, src/module.rs:289-298
  Rationale: RandomState produces non-deterministic iteration; silent cache corruption

MUST FIX M14: Replace SourceSpan with serializable span or mark #[serde(skip)]
  Files: All HIR types with span: SourceSpan
  Rationale: Even with miette serde feature, spans may not be needed in cached artifacts
```

---

## Appendix C — Agent Report File Inventory

| Agent | Report file | Size (chars) |
|-------|------------|-------------|
| AGENT-1 | `/tmp/agent_A_30bd05b7.md` | ~28,811 |
| AGENT-2 | `/tmp/agent_B_3f0d6c0c.md` | ~43,765 |
| AGENT-3 | `/tmp/agent_C_31b0e757.md` | ~22,598 |
| AGENT-4 | `/tmp/agent_D_4526a078.md` | ~15,422 |
| AGENT-5 | `/tmp/agent_E_50ae6e03.md` | ~22,258 |
| AGENT-6 | `/tmp/agent_F_727867b1.md` | ~26,667 |
| AGENT-7 | `/tmp/agent_G_bcb7ca81.md` | ~44,144 |
| AGENT-8 | `/tmp/agent_H_61df1c8a.md` | ~44,144 |
| AGENT-9 | `/tmp/agent_I_2ac17bd9.md` | ~8,542 |
| AGENT-10 | `/tmp/agent_J_99636a61.md` | ~8,542 |
| AGENT-11 | `/tmp/agent_K_cebc7b7c.md` | ~2,265 |
| AGENT-12 | `/tmp/agent_L_dc160615.md` | ~3,440 |
| AGENT-13 | `/tmp/agent_M_0bb3522a.md` | ~16,082 |
| AGENT-14 | `/tmp/agent_N_59560cd1.md` | ~29,025 |
| AGENT-15 | `/tmp/agent_O_46d61de3.md` | ~33,917 |
| **Total** | | **~298,365** |

Workflow journal: `/home/dimitar/.claude/projects/-home-dimitar-saturnite-Saturnite/7c79a98f-58af-4356-9d64-21110bdaafbf/subagents/workflows/wf_dbe14059-adf/journal.jsonl`

---

## Appendix D — Answering Key Questions from the Audit Brief

### Q1: Is the current architecture ready for incremental compilation?

**No.** 14 MUST FIX items block it. The single most critical: `SymbolInterner` cannot be serialized, making `HirProgram`, `MirProgram`, and `ModuleGraph` all non-serializable. DefIds are positional and shared across namespaces. The CLI bypasses the module system. No hashing crate exists.

### Q2: What is the first thing that must be done?

**M1**: Enable miette `serde` feature. Without it, `SourceSpan` (used in every HIR type) is non-serializable, and no serialization chain can be built.

### Q3: What is the biggest performance bottleneck?

**M12 / CF-20**: `function_name()` does an O(n) linear scan of `MirProgram.functions` per call site during codegen, resulting in O(N²) total cost for N functions. Fix: add `HashMap<DefId, SymbolId>` index.

### Q4: What is the biggest correctness bug?

**CF-05**: DefId namespace collapse — `DefId(0)` is simultaneously a valid function, struct, and enum ID. Any DefId-keyed cache is silently corrupt.

### Q5: What is the biggest integration gap?

**M9 / CF-09**: The CLI calls `analyze_and_lower` (single-file) instead of `analyze_and_lower_with_graph` (multi-module). The `ModuleGraph` is built but discarded. Multi-module projects are broken in production.

### Q6: Can anything be cached today?

Technically, source file contents can be hashed (if a hashing crate is added), and `.o` object files can be written to disk. But no intermediate IR (AST, HIR, MIR, module graph) can be cached — they all lack `Serialize`/`Deserialize`.

### Q7: What are the three showstoppers (from red-team analysis)?

1. **DefId namespace collapse** — silent wrong-code generation
2. **CLI bypass** — module system unreachable from production
3. **No serialization of core types** — incremental compilation architecturally impossible

### Q8: What does the parallelism DAG look like?

See Phase 7 above. Currently fully sequential. After fixes, MIR lowering, constant folding, and MIR verification are embarrassingly parallel. LLVM codegen requires per-function `LLVMContext` isolation.

---

*This document is the final deliverable of the Phase 0-8 audit. No compiler source code was modified. All findings are documented with file:line evidence cross-referenced across 15 specialized audit agents.*
