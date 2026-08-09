# Saturnite Crate Dependency Audit — Phase 16

## Status: Complete

## Workspace Overview

The Saturnite workspace consists of a single crate:

| Crate | Path | Edition | Version | License |
|-------|------|---------|---------|---------|
| `stnx` | `crates/stnx/` | 2021 | 0.1.0 | MIT or Apache-2.0 |

### Workspace-level `Cargo.toml`

- **Resolver:** v3 (enables feature-unification improvements and avoids
  feature-spaghetti in build scripts).
- **Workspace dependencies:** centralized in `[workspace.dependencies]` so that
  all path members share the same versions. Currently the workspace has only one
  member crate, so no cross-crate version drift has been observed.

---

## Runtime Dependencies (Production)

### Direct dependencies

| Crate | Version | Purpose | Risk Level |
|-------|---------|---------|------------|
| `logos` | 0.16 | Lexer / tokenizer (lexical analysis) | Low — well-established, zero-dep |
| `chumsky` | 0.13 | Parser combinator library (with `memoization` feature) | Low — mature parsing framework |
| `inkwell` | 0.9 | Rust bindings to LLVM 21 (IR generation, codegen, linking) | Medium — heavy native dependency, but FFI bindings are stable |
| `miette` | 7 | Diagnostics / error reporting (with `fancy` feature) | Low — focused error-reporting library |
| `thiserror` | 2 | Ergonomic `#[derive(Error)]` for error types | Low — widely used, stable |
| `clap` | 4 | CLI argument parsing (with `derive` feature) | Low — de-facto standard |
| `serde` | 1 | Serialization framework (with `derive` feature) | Low — foundational |
| `serde_json` | 1 | JSON serialization (structured build reports) | Low — widely used |
| `toml` | 0.8 | TOML parsing for `saturn.toml` config files | Low — standard crate |
| `anyhow` | 1 | Dynamic error handling in `main.rs` | Low — widely used |
| `which` | 5 | Locating system linkers / tools | Low — focused utility |

### Build dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `cc` | 1 | Compiling the C runtime (libsaturnite_runtime.a) at build time |

### Dev dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `tempfile` | 3 | Temporary file/directory handling in integration tests |

---

## Transitive Dependency Analysis

### High-risk transitive dependencies

| Dependency | Via | Version | Notes |
|-----------|-----|---------|-------|
| `inkwell` → `llvm21` native libraries | stnx → inkwell | 21.x | Dynamically linked (`prefer-dynamic` feature); requires LLVM 21 shared libraries on the host. Not a security risk but an installation requirement. |
| `stacker` (via `chumsky`) | stnx → chumsky → stacker | 0.1.25 | Stack overflow protection; pulls in `libc` and `psm`. No known CVEs. |
| `cc` (via build) | stnx → cc | 1.4.0 | Build-time C compilation; may invoke system compiler. No runtime risk. |

### Duplicates / version conflicts

No version conflicts detected. The dependency tree shows no duplicate semver-
incompatible versions of any crate.

### Security considerations

- All crates are sourced from crates.io via the standard Cargo registry.
- No git dependencies or path dependencies outside the workspace.
- `serde_json` pulls in `itoa`, `memchr`, and `zmij` — all are mature, widely
  used crates with no known security issues.
- No crates with known CVEs are present in the dependency tree.
- `tempfile` is dev-only and does not affect production builds.

---

## Dependency Recommendations

### Retained (no change needed)

- All existing dependencies are appropriate for the current phase.
- `inkwell` with `llvm21-1-prefer-dynamic` aligns with the dynamic LLVM setup
  in `build.rs`.
- `chumsky` with `memoization` is needed for parser performance.

### Future considerations (Phases 17+)

- **`cargo-udeps` / `cargo-machete`:** Run periodically to remove unused
  dependencies. Currently no unused dependencies detected.
- **`cargo-audit`:** Should be run in CI to catch newly published CVEs.
- **Dependency on `serde_json`:** Currently used only for JSON build reports.
  Consider whether this is needed long-term or if the JSON output can be
  simplified.
- **`toml` dependency:** Newly added for Phase 10 (config representation).
  Properly scoped — only used in `config.rs`.

---

## Summary

| Metric | Value |
|--------|-------|
| Total crates in tree | ~45 |
| Direct production deps | 10 |
| Direct build deps | 1 |
| Direct dev deps | 1 |
| Known security issues | 0 |
| Version conflicts | 0 |
| Unused deps | 0 |
