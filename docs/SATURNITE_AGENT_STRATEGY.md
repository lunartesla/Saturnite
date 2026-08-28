# SATURNITE 1.0 — MULTI-AGENT IMPLEMENTATION STRATEGY (Phase 12)

> How to use parallel agents to execute the Phase 11 roadmap
> without producing spaghetti code, lost context, or overwritten
> work. Built on the principle that **every agent has a
> narrowly-scoped, file-bounded responsibility, with explicit
> coordination through a Phase Coordinator**.

---

## 1. The agent taxonomy

| Agent | Scope | Output | Lifetime |
|---|---|---|---|
| **Research agent** | Reads existing code; produces a design doc or contract. | Markdown | 1 task |
| **Port agent** | Brings rustc code into Saturnite (with provenance). | Rust code + `provenance/<id>.md` | 1 task |
| **Implementation agent** | Writes new Saturnite code. | Rust code | 1 task |
| **Test agent** | Writes tests. | Rust test code | 1 task |
| **Documentation agent** | Writes user-facing docs. | Markdown | 1 task |
| **Review agent** | Verifies a PR/change. | Review report | 1 task |
| **Soundness agent** | Specializes in soundness verification. | Soundness report | 1 task |
| **Phase Coordinator** | Owns a phase; reconciles, merges, integrates. | Working tree | 1 phase |
| **Audit Lead** | Owns the overall roadmap. Reviews phase results. | Markdown | continuous |

The **Phase Coordinator** is the only agent that **merges code
into a working branch**. All other agents produce artifacts that
the coordinator integrates.

---

## 2. Per-phase agent plan

### Phase 0 — Architecture cleanup

- 1 **Implementation agent**: refactor.
- 1 **Test agent**: regression test.
- 1 **Review agent**: verify no semantic change.
- **Phase Coordinator**: merge; ensure the prior audit
  findings are closed.

**No parallelism required** (sequential refactor).

### Phase 1 — Resolver pass

- 1 **Design agent**: `Resolution` struct + pipeline contract.
- 1 **Implementation agent**: `resolver.rs`.
- 1 **Integration agent**: update `hir::lower` and `module.rs`.
- 2 **Test agents** (parallel): positive + negative tests.
- 1 **Review agent**.
- 1 **Phase Coordinator**.

**Parallelism**: design + tests in parallel; integration after
design; review after all.

### Phase 2 — Generic types (A1 port)

- 1 **Port agent**: `Interned` newtype + provenance record.
- 1 **Design agent**: generic-type integration spec.
- 2 **Implementation agents** (sequential, not parallel):
  HIR changes; MIR changes.
- 1 **Codegen agent**: monomorphization.
- 2 **Test agents** (parallel).
- 1 **Soundness agent**: verify monomorphization does not
  produce wrong code.
- 1 **Review agent**.
- 1 **Phase Coordinator**.

**Parallelism**:
- Port agent and design agent run in parallel.
- HIR implementation must precede MIR implementation.
- Codegen waits for MIR.
- All tests wait for implementation.
- Soundness waits for all tests.

### Phase 3 — Diagnostics expansion

- 1 **Design agent**: code ranges, suggestion format.
- 1 **Implementation agent**: `error.rs`.
- 1 **Documentation agent**: `ERROR_CODES.md`.
- 1 **Test agent**.
- 1 **Phase Coordinator**.

**Full parallel** after design is signed off.

### Phase 4 — MIR optimization

- 1 **Design agent**: `MirOptPass` trait, pass ordering.
- 3 **Implementation agents** (parallel): DCE, copy-prop,
  inline.
- 1 **Soundness agent**: corpus-level correctness check.
- 1 **Review agent**.
- 1 **Phase Coordinator**.

**Parallelism**:
- 3 pass implementations run in parallel.
- Soundness waits for all 3.
- Review waits for soundness.

### Phase 5 — Compiletest runner (A3 port)

- 1 **Port agent**: fork compiletest.
- 1 **Directives agent**: Saturnite-specific directive syntax.
- 5 **Test authors** (parallel): 10 tests each.
- 1 **CI agent**: GitHub Actions integration.
- 1 **Phase Coordinator**.

**Parallelism**:
- Port + directives in parallel.
- 5 test authors in parallel.
- CI waits for tests.

### Phase 6 — JSON target spec (A2 port)

- 1 **Port agent**: schema + parser + provenance record.
- 1 **Test agent**: 290 targets.
- 1 **Documentation agent**.
- 1 **Phase Coordinator**.

**Full parallel**.

### Phase 7 — Package manager

- 1 **Design agent**: lock file format, registry layout.
- 2 **Implementation agents** (parallel after design): CLI,
  lock file.
- 1 **Test agent**.
- 1 **Review agent** (security focus).
- 1 **Phase Coordinator**.

### Phase 8 — Standard library

- 1 **Implementation agent**.
- 1 **Test agent**.
- 1 **Documentation agent**.
- 1 **Phase Coordinator**.

**Full parallel**.

### Phase 9 — Documentation + 1.0

- 1 **Documentation agent** (everything).
- 1 **Release agent** (version + tag).
- 1 **Audit Lead** (final review).
- 1 **Phase Coordinator**.

---

## 3. Coordination protocol

Every agent operates on a **dedicated branch**:

```
main
├── phase-0-cleanup
├── phase-1-resolver
├── phase-2-generics
├── phase-3-diagnostics
├── phase-4-mir-opt
├── phase-5-compiletest
├── phase-6-targets
├── phase-7-pkgmgr
├── phase-8-std
└── phase-9-release
```

When an agent completes, it **opens a PR** against the parent
branch (`main` for the first phase; the per-phase branch for
subsequent phases within the same feature).

The **Phase Coordinator**:

1. Reviews the PR.
2. Runs the per-phase test suite.
3. Runs the `provenance-check` script.
4. Runs `cargo fmt --check`, `cargo clippy --workspace --tests -- -D warnings`,
   `cargo test --workspace` on the merged branch.
5. Either merges or sends back for revision.

**The Phase Coordinator never writes code.** It only reviews,
merges, and integrates.

---

## 4. Conflict avoidance

Two agents in the same phase might write to the same file. To
avoid this:

- **Every agent has an explicit file ownership list** in its
  prompt.
- **Cross-cutting files** (`Cargo.toml`, `lib.rs`, `main.rs`)
  are owned by the Phase Coordinator; agents submit a
  "required change" request, and the coordinator makes the
  edit.
- **The `provenance/` directory** is owned by the Audit Lead
  alone; only the Audit Lead adds records.
- **Test files** are owned by Test agents; if a test needs to
  import a new module, the test agent submits a "required
  import" request to the Phase Coordinator.

---

## 5. The "lost work" problem

Agents can produce work that is later overwritten or
abandoned. To mitigate:

- **Every agent's work is a PR**, never a direct push to a
  shared branch.
- **Every PR has a `provenance-check` CI run** before it can
  be merged.
- **The Audit Lead reviews every PR's effect on the
  provenance index** (`docs/provenance/README.md`).
- **Abandoned PRs are explicitly closed** with a reason
  recorded in the Phase Coordinator's notes.

---

## 6. Soundness-sensitive areas (per AGENTS.md)

Per the project's `AGENTS.md` policy:

> Soundness-sensitive areas include ... the query system, type
> checking, trait solving, MIR construction or optimization, borrow
> checking, const evaluation, normalization and semantic caches,
> layout and validity, and codegen.

For Saturnite, this maps to:

- `crates/stnx/src/hir/lower.rs` (type checking)
- `crates/stnx/src/hir/expr.rs` (type checking)
- `crates/stnx/src/mir/lower.rs` (MIR construction)
- `crates/stnx/src/mir/opt.rs` and new MIR opt passes (MIR
  optimization)
- `crates/stnx/src/mir/codegen.rs` (codegen)
- `crates/stnx/src/codegen/emitter.rs` (codegen)
- `crates/stnx/src/codegen/linker.rs` (linking / ABI)

For any agent working on these files:

- The **Soundness agent** must sign off before the Phase
  Coordinator merges.
- The **Soundness agent** writes a `soundness/<phase>.md`
  report documenting: what was changed, what the test
  coverage is, and any unsafe / `unimplemented!` /
  `todo!()` calls introduced.
- The **Audit Lead** reviews the soundness report.

**Implementation of soundness-sensitive changes is NOT
delegated to a single agent.** A soundness-sensitive change
requires:

- A **design agent** producing a written design.
- An **implementation agent** writing the code.
- A **soundness agent** independently verifying.
- A **review agent** doing a final read-through.

This is a 4-agent minimum for soundness work. The cost is
intentional: per the AGENTS.md policy, soundness regressions
are catastrophic, and the multi-agent cost is much smaller
than the cost of a soundness regression.

---

## 7. Provenance discipline

The Audit Lead maintains `docs/provenance/`. Every code port
**must**:

1. Be accompanied by a `provenance/<id>.md` record.
2. Be **added to the `provenance/README.md` index** by the
   Audit Lead.
3. Be **stamped with a header comment** by the port agent
   ("Originally derived from ...").
4. Pass the **`provenance-check` script** (designed in Phase 8).

If a port agent produces a port without a provenance record,
the Phase Coordinator **rejects the PR**.

---

## 8. Communication

Agents communicate through:

- **Markdown files** in the working tree (designs, soundness
  reports, phase notes).
- **PR descriptions** (concise summary, link to design doc,
  link to provenance record).
- **Phase Coordinator's notes** in
  `docs/phase-notes/phase-<N>.md` (a journal of decisions).

Agents **do not** communicate through:

- Direct messages (none in the agent framework).
- External chat.
- Out-of-band comments.

The working tree is the single source of truth.

---

## 9. The "single big phase" anti-pattern

**Do not let one agent own a whole phase.** Every phase must be
broken into the agent types above. If a phase is so large that
it requires more than 3 implementation agents, **split the
phase** (the Audit Lead does this before the phase starts).

For example, Phase 7 (package manager) could be split into:

- Phase 7a: lock file format + atomic updates.
- Phase 7b: CLI subcommands.
- Phase 7c: registry layout.

Each is a smaller, more verifiable unit.

---

## 10. The agent prompt template

Every agent prompt follows this structure:

```
You are a <role> agent. Your task is <task>.

Scope:
- Files you MAY modify: <list>
- Files you MUST NOT modify: <list>
- Files you must REQUEST the Phase Coordinator to modify: <list>

Inputs:
- Design doc: <path> (if applicable)
- Test contract: <path> (if applicable)
- Provenance record: <path> (if applicable)

Deliverables:
- Code at: <paths>
- Tests at: <paths>
- Documentation at: <paths> (if applicable)
- Provenance record at: <path> (if applicable)
- PR description: concise summary

Constraints:
- <list of constraints specific to this task>

Definition of done:
- <list of acceptance criteria>
```

The Phase Coordinator is responsible for filling in the
template. This makes agent behavior **predictable** and
**auditable**.

---

## 11. The Audit Lead's job

The Audit Lead is the only agent that:

- Owns the `docs/provenance/` directory.
- Owns the `docs/audit/` directory.
- Reviews every PR for provenance impact.
- Owns the `provenance-check` script.
- Updates the `provenance/README.md` index.
- Maintains `docs/SATURNITE_RUST_FORENSIC_AUDIT.md` (the
  ongoing audit summary).

The Audit Lead is **not a Phase Coordinator**. The Phase
Coordinators are per-phase; the Audit Lead is continuous.

---

## 12. The cost

The 10-phase roadmap with the agent plan above involves:

- ~3-5 implementation agents per phase.
- ~1-2 test agents per phase.
- ~1 review agent per phase.
- 1 Phase Coordinator per phase.
- 1 Audit Lead (continuous).
- Total: ~50-80 distinct agent invocations.

This is the right cost for a multi-month project. It is
intentionally **not** minimal — the cost is the
discipline that prevents spaghetti code, soundness
regressions, and license leaks.
