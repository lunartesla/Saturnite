# THIRD-PARTY PROVENANCE SYSTEM (Phase 8)

> How Saturnite will track every piece of reused code (especially
> from rustc), so that the license obligations, attributions, and
> NOTICE requirements are satisfied across the lifetime of the
> project.

This is a **design document**, not a working system. The system
itself is the next step (per Phase 12 — a Provenance Tracker agent
that maintains the `provenance/` directory).

---

## 1. Goals

1. **Trace every reused file** back to its upstream origin (project,
   repository, commit, path, license, copyright holders).
2. **Preserve per-file attribution** even after many edits.
3. **Make license audits trivial** — a single command should list
   every third-party file and its obligations.
4. **Survive refactors** — a 2-year-later refactor that moves a
   derived file should not lose its provenance.
5. **Be human-editable** — the source of truth is a plain Markdown
   file, not a database.

---

## 2. The provenance record

Every derived file (or every block of derived code) is associated
with a `ProvenanceRecord` containing:

| Field | Type | Example | Required? |
|---|---|---|---|
| `id` | string | `rustc_interned_v1` | yes |
| `component` | string | `rustc_data_structures::Interned` | yes |
| `upstream_project` | string | `rust-lang/rust` | yes |
| `upstream_repository` | URL | `https://github.com/rust-lang/rust` | yes |
| `upstream_commit` | SHA | `3b8ee6c0ca55afb08e2e130003227a3195394425` | yes |
| `upstream_path` | path | `compiler/rustc_data_structures/src/intern.rs` | yes |
| `upstream_lines` | range | `1-180` (optional, recommended) | recommended |
| `saturnite_path` | path | `crates/stnx/src/intern.rs` | yes |
| `saturnite_lines` | range | `1-90` (optional) | recommended |
| `original_license` | SPDX | `MIT OR Apache-2.0` | yes |
| `copyright_holders` | list of strings | `["The Rust Project Developers (https://thanks.rust-lang.org)"]` | yes |
| `modifications` | list of strings | `["Removed #[rustc_pass_by_value] attribute.", "Removed StableHash impl."]` | yes |
| `import_date` | date (ISO 8601) | `2026-09-01` | yes |
| `dependencies` | list of `ProvenanceId` | `[]` (this record has no upstream deps) | yes (may be empty) |
| `notices_required` | list of strings | `["Preserve MIT and Apache-2.0 license texts."]` | yes |
| `license_files_retained` | list of paths | `["LICENSES/MIT.txt", "LICENSES/Apache-2.0.txt"]` | yes |
| `attribution_requirements` | list of strings | `["Include 'The Rust Project Developers' in NOTICE."]` | yes |
| `source_redistribution_required` | bool | `false` | yes |
| `reviewer` | string | `Dimitar.Simovski` (for `AGENTS.md` compliance) | yes |
| `notes` | string | `""` | no |

---

## 3. File layout

```
docs/
├── THIRD_PARTY_PROVENANCE.md     # this file (design)
├── provenance/
│   ├── README.md                  # top-level index
│   ├── rustc_interned_v1.md       # one file per record
│   ├── rustc_mir_dataflow_v1.md
│   ├── rustc_target_json_schema_v1.md
│   ├── rustc_compiletest_runner_v1.md
│   ├── rustc_lexer_reference_v1.md
│   ├── third_party_unicode_crates_io_v1.md
│   └── ...
└── audit/                         # historical audits
    ├── 2026-08-28/
    │   ├── SATURNITE_ACTUAL_ARCHITECTURE_AUDIT_2026.md
    │   ├── RUST_ACTUAL_ARCHITECTURE_AUDIT_2026.md
    │   ├── SATURNITE_RUST_SIDE_BY_SIDE_2026.md
    │   ├── SATURNITE_CODE_LEVEL_REUSE_2026.md
    │   ├── SATURNITE_LICENSE_COMPATIBILITY_2026.md
    │   └── SATURNITE_RUST_FORENSIC_AUDIT.md
    └── ...
```

Each `provenance/<id>.md` file is a single ProvenanceRecord
rendered as a Markdown frontmatter + body.

---

## 4. Template

```markdown
---
id: rustc_interned_v1
component: rustc_data_structures::Interned
upstream:
  project: rust-lang/rust
  repository: https://github.com/rust-lang/rust
  commit: 3b8ee6c0ca55afb08e2e130003227a3195394425
  path: compiler/rustc_data_structures/src/intern.rs
  lines: 1-180
saturnite:
  path: crates/stnx/src/intern.rs
  lines: 1-90
license: MIT OR Apache-2.0
copyright_holders:
  - "The Rust Project Developers (https://thanks.rust-lang.org)"
modifications:
  - "Removed #[rustc_pass_by_value] attribute (not available on stable Rust)."
  - "Removed StableHash impl (depends on rustc_data_structures::stable_hash)."
  - "Removed Interner trait (not needed for Saturnite's flat type model)."
import_date: 2026-09-01
dependencies: []
notices_required:
  - "Preserve the MIT and Apache-2.0 license texts in LICENSES/."
  - "Include 'The Rust Project Developers' in the top-level NOTICE."
license_files_retained:
  - LICENSES/MIT.txt
  - LICENSES/Apache-2.0.txt
attribution_requirements:
  - "Each ported file must carry a header noting the original Rust Project copyright."
  - "Do not use 'Rust' or 'Rust Project' to endorse any Saturnite derived product."
source_redistribution_required: false
reviewer: Dimitar.Simovski
---

# `rustc_interned_v1` — port of `rustc_data_structures::Interned`

## What was ported

The `Interned<'a, T>` newtype from
`compiler/rustc_data_structures/src/intern.rs`, plus the `Hash`,
`PartialEq`, `Eq`, `Clone`, `Copy`, `Deref`, and `Debug` impls.

## What was NOT ported

- The `Interner` trait (Saturnite's flat type model does not need
  it).
- The `StableHash` impl (depends on `stable_hash`; Saturnite has
  no StableHash analogue).
- The `private::PrivateZst` field (the auditable-construction
  pattern; Saturnite's `intern!` macro makes the construction
  auditable in a different way).
- The `#[rustc_pass_by_value]` attribute (not available on stable
  Rust; the newtype still works without it).

## Code skeleton

```rust
// crates/stnx/src/intern.rs

// Originally derived from Rust Project Developers
// (https://thanks.rust-lang.org), Apache-2.0 OR MIT.
// Adapted for Saturnite by Dimitar.Simovski in 2026.
//
// Modifications:
//   - Removed #[rustc_pass_by_value] attribute.
//   - Removed StableHash impl.
//   - Removed Interner trait.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::ptr;

pub struct Interned<'a, T>(pub &'a T);

impl<'a, T> Interned<'a, T> {
    #[inline]
    pub const fn new_unchecked(t: &'a T) -> Self {
        Interned(t)
    }
}

impl<'a, T> Clone for Interned<'a, T> { /* ... */ }
impl<'a, T> Copy for Interned<'a, T> {}
impl<'a, T> Deref for Interned<'a, T> { type Target = T; /* ... */ }
impl<'a, T> PartialEq for Interned<'a, T> { /* pointer eq */ }
impl<'a, T> Eq for Interned<'a, T> {}
impl<'a, T> Hash for Interned<'a, T> { /* pointer hash */ }
impl<T: fmt::Debug> fmt::Debug for Interned<'_, T> { /* ... */ }
```

## Notes

- Saturnite's flat `HirType` is not currently interned; this
  port is groundwork for the day generics arrive.
- The reviewer has confirmed that the modifications do not
  alter the semantics of the newtype in any user-visible way.
```

---

## 5. The provenance index

`docs/provenance/README.md` is a flat list of all records:

```markdown
# Third-Party Provenance Index

| ID | Component | License | Imported | Reviewer |
|---|---|---|---|---|
| rustc_interned_v1 | rustc_data_structures::Interned | MIT OR Apache-2.0 | 2026-09-01 | Dimitar.Simovski |
| rustc_mir_dataflow_v1 | rustc_mir_dataflow::framework | MIT OR Apache-2.0 | 2026-12-01 | (pending) |
| rustc_target_json_schema_v1 | rustc_target JSON spec | MIT OR Apache-2.0 (schema) | 2026-12-01 | (pending) |
| rustc_compiletest_runner_v1 | compiletest runner | MIT OR Apache-2.0 | 2027-03-01 | (pending) |
| rustc_lexer_reference_v1 | rustc_lexer (architectural reference) | MIT OR Apache-2.0 | 2026-09-15 | Dimitar.Simovski |
| third_party_unicode_crates_io_v1 | unicode-ident, unicode-width, unicode-general-category | MIT OR Apache-2.0 | 2026-09-15 | Dimitar.Simovski |

## Total reused code: ~2 000 lines (estimate)
## Total dependencies with copyleft: 0
## Items requiring legal review: 0 (Unicode-3.0 was excluded; GCC was excluded)
```

---

## 6. The `provenance-check` CI step (design)

A `xtask` (or shell script) that:

1. Walks `docs/provenance/`.
2. Verifies that every `saturnite.path` in a record exists.
3. Verifies that every file under `LICENSES/` referenced in a
   record actually exists.
4. Verifies that every file with a header comment referencing a
   third-party project has a matching record in `provenance/`.
5. Verifies that no file with a `Copyright (C) FSF` or
   `Copyright (C) Unicode, Inc.` header is present in `src/` (i.e.
   no GPL/Unicode-3.0 code was accidentally merged).
6. Emits a summary `provenance_report.md` for the current commit.

The script is read-only (does not modify any source files).

---

## 7. Workflow for a new port

1. Author opens an issue/PR titled "Port `<component>` from
   `<upstream>`".
2. Author writes a `provenance/<id>.md` record (the frontmatter
   alone is enough to start; the body is filled in as the port
   progresses).
3. Author imports the code into `crates/stnx/src/<path>`, adding
   a header comment with the upstream copyright and a link back to
   the record.
4. Reviewer verifies:
   - The license in the record matches the upstream license in
     the **upstream commit** (not HEAD — upstream may have changed
     the license).
   - The modifications list is complete (every non-trivial change
     is documented).
   - The attribution requirements are met (header comment is
     present, NOTICE is updated, LICENSE file is added).
5. Reviewer signs off in the record's `reviewer` field.
6. `provenance-check` CI step passes.

This satisfies the AGENTS.md "named reviewer" gate (since the
provenance record explicitly names a reviewer) and the
"soundness-sensitive" gate (since the record tracks whether the
port is soundness-relevant).

---

## 8. Items in the current 2026-08-28 audit that would become provenance records

| ID | Component | Class | When |
|---|---|---|---|
| `rustc_lexer_reference_v1` | `rustc_lexer` (architectural reference; not ported code) | A. KEEP / Architecture | 2026-09-15 |
| `rustc_interned_v1` | `rustc_data_structures::Interned` | D. FUSE later | when interned types arrive (0.5+) |
| `rustc_mir_dataflow_v1` | `rustc_mir_dataflow::framework` | D. FUSE later | when 5+ dataflow analyses exist (0.6+) |
| `rustc_target_json_schema_v1` | `rustc_target` JSON schema | D. FUSE later | when 50+ target specs needed (0.6+) |
| `rustc_compiletest_runner_v1` | `compiletest` runner | D. FUSE later | when UI/snapshot tests justified (0.5+) |
| `rustc_codegen_ssa_patterns_v1` | `rustc_codegen_ssa` (architectural reference only) | E. REJECT | n/a |
| `rustc_query_system_reference_v1` | `rustc_middle::query` (architectural reference) | F. DEFER | 0.7+ |
| `third_party_unicode_crates_io_v1` | `unicode-ident`, `unicode-width`, `unicode-general-category` (crates.io) | n/a (Cargo dep) | when Unicode identifiers added |

---

## 9. Items that are NOT ports and therefore do not need a record

- Architectural references — no code is copied.
- Crates.io dependencies — these are tracked in `Cargo.toml` and
  `Cargo.lock`; no separate provenance record is needed for
  standard MIT/Apache-2.0 deps.
- `inkwell` (LLVM bindings) — a Cargo dep; no separate record.
- Saturnite's own original code — no record.

---

## 10. Long-term maintenance

- Every release, the `provenance-check` script must pass.
- Every commit that adds a third-party file must add a record
  in the same commit (enforced by the provenance-check script).
- Every commit that removes a third-party file must also remove
  its record.
- The `provenance/README.md` index is regenerated by the script
  on every release.

This is sufficient to make any future license audit
straightforward: "list all third-party code in Saturnite" is
`cat docs/provenance/README.md`.
