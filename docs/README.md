# Saturnite Documentation Index

> **Canonical reference:** The top-level [`README.md`](../README.md) and the
> documents in this directory's **Current documentation** section below are the
> authoritative sources for Saturnite syntax, features, and architecture.
>
> **⚠️  Legacy documentation** (in [`legacy/`](./legacy/)) is archived for
> historical reference only. These files describe prior design audits,
> roadmaps, or stale architecture from **before** the native syntax migration.
> They may contain incorrect, outdated, or superseded claims. Do **not** use
> them as a reference for current language features, syntax, or compiler
> behavior. Each legacy file carries an explicit banner at the top.

---

## Current documentation

| Document | Description |
|---|---|
| [`SATURNITE_SYNTAX.md`](./SATURNITE_SYNTAX.md) | **Canonical language syntax reference** — native syntax (indentation-based blocks, `module`, `give`, `say`, `raise`, `main:`, `use python:`/`use rust:`/`use native:`). |
| [`INTEROPERABILITY.md`](./INTEROPERABILITY.md) | Rust and Python interoperability model — supported boundary, ABI rules, fixtures. |
| [`SATURNITE_0_4_ARCHITECTURE.md`](./SATURNITE_0_4_ARCHITECTURE.md) | Compiler architecture — pipeline, MIR, HIR, module system, codegen overview. |
| [`SATURNITE_CRATE_DEPENDENCY_AUDIT.md`](./SATURNITE_CRATE_DEPENDENCY_AUDIT.md) | Dependency audit — all crate dependencies, versions, purposes, risk levels. |
| [`DEPENDENCIES.md`](./DEPENDENCIES.md) | Dependency model — `saturn.toml` configuration, version requirements, future resolver plan. |
| [`README.md`](../README.md) | Top-level project README — quick start, language reference, CLI reference, project layout. |
| [Toolchain reference](../crates/saturn/src/lib.rs) | The `saturn` toolchain — project-oriented build, profiles, project discovery. |

---

## Legacy documentation (archived, do not use as canonical reference)

All files in [`legacy/`](./legacy/) are historical and superseded.
See the banner at the top of each file for details.

| Document | Why it's legacy |
|---|---|
| `SATURNITE_1_0_ROADMAP.md` | Prospective release roadmap — Phase 0–9 plan, not yet executed. |
| `SATURNITE_1_0_ARCHITECTURE.md` | Prospective 1.0 target architecture — future-facing design, not current. |
| `SATURNITE_SYNTAX_MIGRATION.md` | Pre-implementation audit for the 0.5 native syntax migration. |
| `SATURNITE_ACTUAL_ARCHITECTURE.md` | Phase 1 forensic audit — describes the state at commit `35f6132`, predating some current features. |
| `SATURNITE_ACTUAL_ARCHITECTURE_AUDIT_2026.md` | Phase 1 follow-up audit (2026) — stale relative to current code. |
| `SATURNITE_RUST_FORENSIC_AUDIT.md` | Forensic audit of Saturnite vs. rustc — evidence-gathering for reuse analysis. |
| `SATURNITE_RUST_SIDE_BY_SIDE_2026.md` | Side-by-side comparison with rustc — analytical reference, not implementation guide. |
| `SATURNITE_CODE_REUSE_ANALYSIS.md` | Forensic code reuse analysis — pre-implementation, superseded. |
| `SATURNITE_CODE_LEVEL_REUSE_2026.md` | Code-level reuse investigation — analytical reference only. |
| `SATURNITE_RUST_REUSE_PLAN.md` | Rust reuse plan — analytical, not a code-modification guide. |
| `SATURNITE_DEPENDENCY_MODEL.md` | Phase 13 design — dependency model as design proposal (partially superseded by `DEPENDENCIES.md`). |
| `SATURNITE_FINAL_VERIFICATION.md` | Phase 18-19 final verification — stale (claims 123 tests; actual is 116+). |
| `SATURNITE_FINAL_VERIFICATION_AUDIT_2026.md` | Final verification audit — environment-blocked, superseded. |
| `SATURNITE_INCREMENTAL_COMPILATION.md` | Phase 14 incremental compilation — design proposal, not implemented. |
| `SATURNITE_LICENSE_COMPATIBILITY_2026.md` | License compatibility matrix — forensic, not current policy. |
| `SATURNITE_MIR_DESIGN.md` | Phase 15 MIR design proposal — pre-implementation, superseded by actual MIR. |
| `SATURNITE_POST_MODULE_ARCHITECTURE_AUDIT.md` | Post-module-architecture audit — stale relative to current code. |
| `SATURNITE_SCALABILITY.md` | Scalability assessment — design-only, not implemented. |
| `SATURNITE_VS_RUST_SIDE_BY_SIDE_COMPARISON.md` | Architecture comparison — analytical, not canonical guide. |
| `SATURNITE_AGENT_STRATEGY.md` | Multi-agent execution strategy — meta-planning, not implementation reference. |
| `SECURITY_AUDIT.md` | Security audit — forensic assessment, findings may be superseded. |
| `SOUNDNESS_AND_SAFETY_ANALYSIS.md` | Soundness analysis — forensic, may be superseded. |
| `STAGE_B_PYTHON.md` | Phase 9 Python interop boundary — contract-defined, full execution deferred. |
| `STAGE_C_HARDENING.md` | Phase 9-13 hardening notes — design notes, not canonical. |
| `THIRD_PARTY_PROVENANCE.md` | Phase 8 provenance system — design proposal, not implemented. |
| `PHASE9_DESIGN.md` | Phase 9 interop design — design-only, deferred items not implemented. |
| `PHASE9_FINAL_REPORT.md` | Phase 9 final report — report, not current spec. |
| `PHASE9_INTEROP.md` | Phase 9 interop architecture — report, not current spec. |
| `CAMPAIGN_FINAL_REPORT.md` | Interop campaign final report — multi-stage report. |
| `PERFORMANCE_AND_OPTIMIZATION_ANALYSIS.md` | Performance analysis — analytical, not current guidance. |
| `SCALABILITY_THOUGHT_EXPERIMENTS.md` | Scalability thought experiments — speculative, not implemented. |
| `RUST_ACTUAL_ARCHITECTURE.md` | Rust compiler forensic architecture — reference only. |
| `RUST_ACTUAL_ARCHITECTURE_AUDIT_2026.md` | Rust architecture audit — reference only. |
| `SATURNITE_0_3_ARCHITECTURE_REVIEW.md` | 0.2-era review — pre-0.3, fully superseded. |
| `SATURNITE_0_3_HIR_DESIGN.md` | 0.3-era HIR design — superseded by actual HIR implementation. |
| `SATURNITE_0_4_ARCHITECTURE_AUDIT.md` | 0.4 audit — falsely claims "no MIR" and "no module system"; fully debunked. |
| `infra.md` | Forensic evidence — stale pre-MIR architecture claims. |
| `module_language_design.md` | Module system design audit — pre-MIR, may be stale. |
| `pipeline.md` | Compiler pipeline evidence fragment — stale pre-MIR claims. |
| `project_architecture.md` | Project/build architecture audit — pre-0.4 state, stale. |
