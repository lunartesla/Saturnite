# SATURNITE 1.0 — IMPLEMENTATION ROADMAP (Phase 11)

> A concrete, independently-verifiable phase plan for taking
> Saturnite from 0.4 to 1.0. Each phase has prerequisites,
> affected files, architectural goal, tasks, tests, documentation,
> risks, agents, parallelization, and dependencies.

This is **the plan the user executes**, not the plan to write
more docs. The phases are sized to be completable in
1-2 weeks of focused work each (with parallel agents).

---

## Phase 0 — Architecture cleanup (PREREQUISITE for 1.0)

**Goal**: pay down the technical debt documented in
`docs/audit-findings.md` and the inline `// TODO` comments
before adding new features.

**Prerequisites**: none.

**Affected files**:
- `crates/stnx/src/target.rs` (extract `to_inkwell_opt_level`
  helper, fix duplicated default initialization)
- `crates/stnx/src/mir/codegen.rs` (use the extracted helper;
  fix pass-manager string mapping duplication)
- `crates/stnx/src/main.rs` (use the extracted helper; remove
  duplicated `TargetConfig::host()` construction)
- `crates/stnx/src/hir/lower.rs` (remove the `PRINTLN_DEF_ID`
  sentinel — replace with a real `DefKind::Builtin` registry
  in `hir/symbol.rs`)

**Tasks**:
1. Extract `OptimizationLevel → InkwellOptLevel` mapping into a
   single helper on `TargetConfig` (call sites:
   `target.rs:228`, `mir/codegen.rs:795-810`).
2. Extract `TargetConfig::with_defaults()` private helper
   (call sites: `target.rs:76-97`, `target.rs:99-124`).
3. Add a `BuiltinRegistry` to `hir/symbol.rs` that records
   `DefKind::Function { name: SymbolId, runtime_symbol: &str }`
   entries, then delete the three `PRINTLN_DEF_ID = u32::MAX - 1`
   sentinels.
4. Add a `miette::Diagnostic` unit test for the `LinkError`
   variant.

**Tests**: existing integration tests must pass; add
`tests/opt_level_mapping.rs` that verifies the extracted helper
is used in both `target.rs` and `mir/codegen.rs`.

**Documentation**: update `README.md` to point to the new
helper; add a short note in `docs/audit-findings.md` recording
the resolution.

**Risks**: low (refactor only; no semantic change).

**Agents**:
- 1 implementation agent (refactor)
- 1 test agent (write the regression test)
- 1 review agent (verify no semantic change)

**Parallelization**: none (sequential refactor).

**Dependencies**: none.

---

## Phase 1 — Resolver pass (foundation for multi-module)

**Goal**: extract name resolution from HIR lowering into a
separate pass. This is the foundation for any future privacy,
import, or visibility check.

**Prerequisites**: Phase 0.

**Affected files**:
- `crates/stnx/src/resolver.rs` (NEW — ~500 lines)
- `crates/stnx/src/hir/lower.rs` (consume the resolver output)
- `crates/stnx/src/module.rs` (use the resolver to bind
  `mod foo;` to discovered modules)
- `crates/stnx/src/lib.rs` (export the new module)
- `crates/stnx/src/main.rs` (insert the resolver call in
  the pipeline)

**Tasks**:
1. Design a `Resolution` struct: `def_table: DefTable`,
   `use_resolutions: HashMap<SymbolId, DefId>`,
   `mod_resolutions: HashMap<ModuleId, ModuleId>`,
   `unresolved: Vec<UnresolvedError>`.
2. Write `resolver::resolve_modules(program, graph) ->
   Resolution` as a single pass.
3. Update `hir::lower` to take `&Resolution` instead of
   resolving inline.
4. Add the `mod_resolutions` map to `HirProgram` (or a sibling
   structure).
5. Update `module.rs` to call the resolver after `discover`.

**Tests**:
- `tests/resolver_simple.rs` — single-file program.
- `tests/resolver_modules.rs` — two-file program.
- `tests/resolver_unresolved.rs` — undefined identifier.
- `tests/resolver_use.rs` — `use foo::bar;` (forward declaration).

**Documentation**: update `README.md` pipeline diagram;
add `docs/RESOLVER_DESIGN.md`.

**Risks**: medium (refactor of existing code path; risk of
regressing single-file programs).

**Agents**:
- 1 design agent (write the `Resolution` struct + pipeline
  spec)
- 1 implementation agent (write the resolver)
- 1 integration agent (update `hir::lower` and `module.rs`)
- 2 test agents (positive + negative tests, in parallel)
- 1 review agent

**Parallelization**:
- Design and implementation can run in parallel with the
  tests (tests use the design contract).
- Integration must wait for both.

**Dependencies**: Phase 0.

---

## Phase 2 — Generic types (the "Rust-adapted" port)

**Goal**: introduce generic types (`fn id<T>(x: T) -> T { x }`)
and validate the **A1 port** of the `Interned<'a, T>` newtype.

**Prerequisites**: Phase 1.

**Affected files**:
- `crates/stnx/src/intern.rs` (NEW — port of
  `rustc_data_structures::intern`)
- `crates/stnx/src/hir/types.rs` (add `HirType::Generic`)
- `crates/stnx/src/hir/lower.rs` (handle generic params)
- `crates/stnx/src/mir/lower.rs` (monomorphize or pass through)
- `crates/stnx/src/mir/codegen.rs` (monomorphize at LLVM level)
- `crates/stnx/Cargo.toml` (no new deps)
- `docs/provenance/rustc_interned_v1.md` (NEW — provenance
  record per Phase 8)

**Tasks**:
1. Port `Interned<'a, T>` from
   `rustc_data_structures/src/intern.rs` (40 lines after
   trimming). Record provenance.
2. Add `HirType::Generic(SymbolId)` and `HirType::Apply { base:
   SymbolId, args: Vec<HirType> }`.
3. Lower generic params from AST → HIR.
4. In MIR, monomorphize at the call site (one MIR function per
   instantiation).
5. In codegen, generate the LLVM function per monomorphized
   MIR.

**Tests**:
- `tests/generic_identity.rs` — `fn id<T>(x: T) -> T { x }`.
- `tests/generic_pair.rs` — `fn swap<T, U>(a: T, b: U) -> ...`.
- `tests/generic_struct.rs` — `struct Pair<A, B> { ... }`.
- `tests/generic_no_monomorphize.rs` — verify that
   `id::<i64>(42)` and `id::<i64>(99)` use the same LLVM
  function (compare symbol names).

**Documentation**: update `README.md` to mention generics;
add `docs/GENERICS_DESIGN.md`; add the provenance record.

**Risks**: medium-high. Generics touch every stage of the
pipeline. Soundness-sensitive area (per AGENTS.md).

**Agents**:
- 1 port agent (`Interned` port)
- 1 design agent (generic-type integration)
- 2 implementation agents (HIR changes; MIR changes)
- 1 codegen agent (monomorphization)
- 2 test agents (positive; negative)
- 1 review agent (soundness)

**Parallelization**:
- Port agent can run in parallel with everything else.
- HIR changes must precede MIR changes.
- Codegen must wait for MIR.

**Dependencies**: Phase 1.

---

## Phase 3 — Diagnostics expansion

**Goal**: structured error codes (per `rustc_error_codes`),
suggestions (e.g. "did you mean X?"), subdiagnostics.

**Prerequisites**: none (independent of other phases).

**Affected files**:
- `crates/stnx/src/error.rs` (add codes, subdiagnostics)
- `crates/stnx/src/hir/lower.rs` (emit `E0xxx` codes)
- `crates/stnx/src/parser/mod.rs` (emit `E0xxx` codes)
- `crates/stnx/src/mir/verify.rs` (emit `E0xxx` codes)
- `crates/stnx/src/codegen/linker.rs` (emit `E0xxx` codes)
- `docs/ERROR_CODES.md` (NEW — long-form explanations)

**Tasks**:
1. Define error-code ranges: `E0xxx` (lex), `E1xxx`
   (parse), `E2xxx` (semantic), `E3xxx` (MIR), `E4xxx`
   (codegen), `E5xxx` (link).
2. Add `pub code: ErrCode` to every error struct.
3. Add a `Suggestion` enum (replace / insert / remove) to
   `error.rs`.
4. Add an `edit_distance::suggest` helper for "did you mean?".
5. Add `docs/ERROR_CODES.md` with one section per code.

**Tests**:
- `tests/error_codes.rs` — every error has a non-empty code.
- `tests/suggestions.rs` — common typos get suggestions.
- `tests/long_form.rs` — `stnx --explain E2001` prints the
  long-form explanation.

**Documentation**: add `docs/ERROR_CODES.md`; update
`README.md`.

**Risks**: low (additive).

**Agents**:
- 1 design agent (code ranges, suggestion format)
- 1 implementation agent (error.rs changes)
- 1 documentation agent (ERROR_CODES.md)
- 1 test agent

**Parallelization**: full — all four can run in parallel.

**Dependencies**: none.

---

## Phase 4 — MIR optimization expansion

**Goal**: more MIR optimization passes (DCE, copy propagation,
inline for `#[inline]` functions).

**Prerequisites**: Phase 2 (generics make inlining more
complex).

**Affected files**:
- `crates/stnx/src/mir/opt.rs` (expand)
- `crates/stnx/src/mir/dce.rs` (NEW — dead code elimination)
- `crates/stnx/src/mir/copy_prop.rs` (NEW — copy propagation)
- `crates/stnx/src/mir/inline.rs` (NEW — function inlining)
- `crates/stnx/src/mir/mod.rs` (export the new passes)
- `crates/stnx/src/mir/verify.rs` (verify after each pass)

**Tasks**:
1. Add a `MirOptPass` trait.
2. Implement `DcePass` (live-variable analysis + remove dead
   statements).
3. Implement `CopyPropPass` (copy-propagation lattice).
4. Implement `InlinePass` (inline small functions, marked
   `#[inline]`).
5. Verify after each pass.

**Tests**:
- `tests/opt_dce.rs` — DCE removes unused assignments.
- `tests/opt_copy_prop.rs` — copy propagation eliminates
  trivial copies.
- `tests/opt_inline.rs` — `#[inline]` functions are inlined.
- `tests/opt_soundness.rs` — optimization does not change
  observable behavior (snapshot of stdout for a corpus of
  programs).

**Documentation**: update `docs/SATURNITE_MIR_DESIGN.md`;
add `docs/MIR_OPT_PASSES.md`.

**Risks**: high (optimization correctness is soundness-
sensitive).

**Agents**:
- 1 design agent (MirOptPass trait, pass ordering)
- 1 implementation agent per pass (3 agents in parallel)
- 1 soundness agent (verify across corpus)
- 1 review agent

**Parallelization**:
- 3 pass implementations can run in parallel.
- Soundness agent must wait for all 3.
- Review waits for soundness.

**Dependencies**: Phase 2.

---

## Phase 5 — Compiletest runner (the "Rust-adapted" framework fork)

**Goal**: replace `tempfile` integration tests with compiletest-
style UI / snapshot tests.

**Prerequisites**: Phase 3 (error codes are needed for
`.stderr` snapshots).

**Affected files**:
- `crates/compiletest/` (NEW — a sibling crate, ~1 000 lines
  adapted from `src/tools/compiletest`)
- `crates/compiletest/Cargo.toml`
- `crates/compiletest/src/directives.rs` (Saturnite-specific
  directive parser)
- `crates/compiletest/src/runner.rs` (Saturnite-specific test
  loop)
- `tests/ui/*.stn` (NEW — UI tests, ~50 files)
- `tests/ui/*.stderr` (snapshot)
- `docs/provenance/rustc_compiletest_runner_v1.md` (NEW)

**Tasks**:
1. Fork `src/tools/compiletest` into `crates/compiletest/`,
   trimming to `compile-fail` and `run-pass` test types.
2. Adapt directive parser (Saturnite uses `//~ ERROR E1234`
   instead of rustc's `//~^ ERROR`).
3. Write 50 UI tests covering: lex errors, parse errors,
   type errors, MIR errors, codegen errors, linker errors.
4. Add `cargo run -p compiletest -- --stnx-path target/release/stnx
   tests/ui/`.
5. CI integration (add to `.github/workflows/ci.yml` if
   applicable).

**Documentation**: add `crates/compiletest/README.md`;
update `README.md` test section; add provenance record.

**Risks**: medium (CI integration; the runner must work on
all platforms).

**Agents**:
- 1 port agent (fork compiletest)
- 1 directives agent (Saturnite-specific syntax)
- 5 test authors (in parallel — 10 tests each)
- 1 CI agent

**Parallelization**:
- Port can run in parallel with directive design.
- Test authors can run in parallel.
- CI must wait for all tests.

**Dependencies**: Phase 3.

---

## Phase 6 — JSON target spec adoption (the "Rust-adapted" data format)

**Goal**: adopt the JSON target spec format; load 290+ target
specs as data.

**Prerequisites**: none (additive).

**Affected files**:
- `crates/stnx/src/target/json.rs` (NEW — JSON target spec
  parser, ~200 lines)
- `crates/stnx/src/target/mod.rs` (extend with `Target::from_json`)
- `crates/stnx/src/target/specs/` (NEW — directory of ~290
  JSON files copied from `rustc_target/src/spec/`)
- `crates/stnx/src/target.rs` (deprecate hand-rolled targets in
  favor of the JSON loader)
- `docs/provenance/rustc_target_json_schema_v1.md` (NEW)

**Tasks**:
1. Port the JSON target spec schema from
   `rustc_target/src/json.rs` (200 lines).
2. Write a Saturnite-specific `TargetConfig::from_json` that
   populates the existing `TargetConfig` struct.
3. Copy the 290+ JSON files from
   `rustc_target/src/spec/*.json` to
   `crates/stnx/src/target/specs/`.
4. Test that all 290 targets parse without error.
5. Verify that the existing 9 hand-rolled targets still work.

**Tests**:
- `tests/target_json_parse.rs` — all 290 targets parse.
- `tests/target_json_roundtrip.rs` — JSON → struct → JSON
  produces the same bytes.
- `tests/target_cross_compile.rs` — build a trivial program
  for each of the 290 targets (compile-only; skip linking if
  cross-toolchain not available).

**Documentation**: update `README.md`; add provenance record.

**Risks**: low (additive; existing 9 targets still work).

**Agents**:
- 1 port agent (schema + parser)
- 1 test agent (290 targets)
- 1 documentation agent

**Parallelization**: full parallel.

**Dependencies**: none.

---

## Phase 7 — Package manager foundation

**Goal**: `stnx add`, `stnx remove`, `stnx install` (local
filesystem only; no registry yet).

**Prerequisites**: Phase 1 (resolver must handle `use` paths
into dependencies).

**Affected files**:
- `crates/stnx/src/cli/cmd_add.rs` (NEW)
- `crates/stnx/src/cli/cmd_remove.rs` (NEW)
- `crates/stnx/src/cli/cmd_install.rs` (NEW)
- `crates/stnx/src/package/registry.rs` (NEW — local
  `~/.stnx/registry/` layout)
- `crates/stnx/src/package/lock.rs` (NEW — `stnx.lock` file
  format)
- `crates/stnx/src/main.rs` (wire up the new subcommands)

**Tasks**:
1. Define a `StnxPackage { name, version, source }` struct.
2. Define the `~/.stnx/registry/` layout (one directory per
   package version).
3. Implement `stnx add <name>@<version>` — fetch the tarball,
   verify SHA, place in the registry, update `saturn.toml` and
   `stnx.lock`.
4. Implement `stnx remove <name>` — inverse of `add`.
5. Implement `stnx install` — read `saturn.toml`, install
   everything in `stnx.lock`.
6. Add SHA verification (lock files include the source
   tarball SHA).

**Tests**:
- `tests/pkg_add.rs` — add a local package.
- `tests/pkg_remove.rs` — remove.
- `tests/pkg_install.rs` — install.
- `tests/pkg_lockfile.rs` — `stnx.lock` is updated atomically.

**Documentation**: add `docs/PACKAGE_MANAGER.md`.

**Risks**: medium (filesystem operations; needs careful
atomicity).

**Agents**:
- 1 design agent (lock file format, registry layout)
- 1 implementation agent (CLI)
- 1 implementation agent (lock file)
- 1 test agent
- 1 review agent (security: SHA verification, atomic updates)

**Parallelization**:
- Design must precede both implementation agents.
- Both implementation agents can run in parallel.
- Review waits for all.

**Dependencies**: Phase 1.

---

## Phase 8 — Standard library foundation

**Goal**: a small `saturnite-std` crate with `println`,
`assert`, basic arithmetic, and `Vec`-like collections.

**Prerequisites**: Phase 2 (generics needed for `Vec<T>`).

**Affected files**:
- `crates/saturnite-std/` (NEW — sibling crate)
- `crates/saturnite-std/Cargo.toml`
- `crates/saturnite-std/src/lib.rs`
- `crates/saturnite-std/src/println.rs`
- `crates/saturnite-std/src/vec.rs`
- `crates/saturnite-std/src/assert.rs`
- `Cargo.toml` (workspace member)

**Tasks**:
1. Create the `saturnite-std` crate skeleton.
2. Implement `println(i64)` (calls the existing
   `saturnite_runtime_println_i64`).
3. Implement `Vec<T>` with `push`, `pop`, `len`, `at` (using
   generics).
4. Implement `assert(cond: bool)`.
5. Implement `assert_eq<T>(a: T, b: T)`.
6. Update the example `hello.stn` to use the new std.

**Tests**:
- `tests/std_println.rs`
- `tests/std_vec.rs`
- `tests/std_assert.rs`

**Documentation**: add `crates/saturnite-std/README.md`;
update `README.md`.

**Risks**: low (additive; existing 0.4 runtime continues
to work).

**Agents**:
- 1 implementation agent (the whole crate)
- 1 test agent
- 1 documentation agent

**Parallelization**: full parallel.

**Dependencies**: Phase 2.

---

## Phase 9 — Documentation + 1.0 release

**Goal**: complete user-facing documentation; tag 1.0.

**Prerequisites**: Phases 0-8.

**Affected files**:
- `README.md` (major rewrite)
- `docs/` (consolidate)
- `INSTALL.md` (cross-platform install)
- `LICENSE` (already exists; verify still correct)
- `CHANGELOG.md` (NEW)
- `CONTRIBUTING.md` (NEW — small; explain the AGENTS.md
  policy)
- `RELEASE.md` (NEW — release process)
- `Cargo.toml` (bump to 1.0.0)

**Tasks**:
1. Write `CHANGELOG.md` covering 0.4 → 1.0.
2. Rewrite `README.md` to describe 1.0 features.
3. Write `CONTRIBUTING.md` explaining the provenance system,
   the `provenance-check` CI step, and the AGENTS.md policy.
4. Update `INSTALL.md` for cross-platform (Linux, macOS, Windows,
   via WSL or MSVC).
5. Run a final `provenance-check` and ensure the index is
   up to date.
6. Bump version to 1.0.0.
7. Tag the commit.

**Tests**: existing test suite must pass.

**Documentation**: this phase IS documentation.

**Risks**: low.

**Agents**:
- 1 documentation agent (everything)
- 1 release agent (version, tag)

**Parallelization**: full parallel.

**Dependencies**: Phases 0-8.

---

## Summary: the 10-phase plan

| Phase | Goal | Duration (est.) | Major risk |
|---|---|---|---|
| 0 | Architecture cleanup | 1 week | low |
| 1 | Resolver pass | 2 weeks | medium |
| 2 | Generic types (A1 port) | 3 weeks | **high (soundness)** |
| 3 | Diagnostics expansion | 1 week | low |
| 4 | MIR optimization | 2 weeks | **high (soundness)** |
| 5 | Compiletest runner (A3 port) | 2 weeks | medium |
| 6 | JSON target spec (A2 port) | 1 week | low |
| 7 | Package manager | 3 weeks | medium |
| 8 | Standard library | 2 weeks | low |
| 9 | Documentation + 1.0 | 1 week | low |
| **Total** | | **~18 weeks** | |

**Parallelizable pairs** (can run in parallel):
- Phase 3 (diagnostics) ∥ Phase 0 (cleanup)
- Phase 6 (JSON targets) ∥ anything
- Phase 8 (std) ∥ Phase 5 (compiletest)
- Documentation agents always parallel with implementation.

**Sequential chains** (must run in order):
- Phase 1 → Phase 7 (package manager needs the resolver)
- Phase 2 → Phase 4 (MIR optimization needs generics for inlining)
- Phase 0 → Phase 1 (cleanup first)
- Phase 3 → Phase 5 (compiletest needs error codes)
- All → Phase 9 (release)

---

## What is NOT in the 1.0 roadmap (and why)

- **Borrow checker** (F. DEFER) — 1.0 is a memory-safe-but-not-borrow-checked language.
- **Trait solving** (F. DEFER) — same.
- **Generics beyond types** (no lifetime / const generics) —
  Saturnite does not need them.
- **Const evaluation** (F. DEFER) — too much work for the
  benefit.
- **Procedural macros** (F. DEFER) — too much work; the
  language does not need them at 1.0.
- **Stable MIR / public tool API** (F. DEFER) — no IDE support
  planned at 1.0.
- **Incremental compilation / query system** (F. DEFER) — the
  single-crate model is fast enough; 1.0 is for a single-crate
  use case.
- **GCC backend** (E. REJECT) — copyleft.
- **Vendoring LLVM** (E. REJECT) — Saturnite uses system LLVM
  via `inkwell`.

The 1.0 release is a small, focused language that **works
correctly for what it does**, with a clean license profile, a
clean architecture, and a clear roadmap to 2.0.
