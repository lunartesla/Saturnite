# SATURNITE — LICENSE COMPATIBILITY MATRIX (Phase 6-7)

> Forensic license / provenance matrix for every rustc component
> that the audit considered for reuse. Built strictly from
> evidence in `/home/dimitar/rust/REUSE.toml`,
> `/home/dimitar/rust/LICENSES/`, `/home/dimitar/rust/COPYRIGHT`,
> `/home/dimitar/rust/.gitmodules`, per-file comments, and
> `/home/dimitar/rust/license-metadata.json`.

This document does **not** render legal opinions beyond what the
licenses' actual text says. Where ambiguity remains, the row is
marked **LEGAL REVIEW REQUIRED**.

---

## 0. The Saturnite distribution target (what we must be compatible with)

| Property | Value | Source |
|---|---|---|
| Project license | `MIT OR Apache-2.0` (dual) | `Cargo.toml:8-9` |
| Author | Dimitar.Simovski (sole) | `LICENSE:2` |
| Runtime | MIT (sole author) | `runtime/println_i64.c` + `LICENSE` |

Saturnite intends to be distributed under `MIT OR Apache-2.0`,
dual-licensed, with the author's own copyright retained. Anything
that is **incompatible** with MIT/Apache-2.0 must be **excluded**,
**separately licensed**, or **replaced**.

---

## 1. The 12 SPDX licenses present in the Rust tree

| # | SPDX ID | License type | Compatible with MIT? | Compatible with Apache-2.0? | Notes |
|---|---|---|---|---|---|
| 1 | `MIT` | Permissive | YES (compatible) | YES (compatible) | Permissive, requires attribution + license preservation |
| 2 | `Apache-2.0` | Permissive + patent grant + NOTICE | YES (compatible) | — (same) | Requires attribution + NOTICE file + patent grant |
| 3 | `Apache-2.0 WITH LLVM-exception` | Permissive + LLVM runtime exception | YES (compatible) | YES (compatible) | The exception allows "binary derivative works" of LLVM to be distributed under any terms, but only when distributing the LLVM components themselves. Source-level reuse is just Apache-2.0 + LLVM-exception. |
| 4 | `BSD-2-Clause` | Permissive | YES (compatible) | YES (compatible) | Requires attribution + license preservation |
| 5 | `ISC` | Permissive | YES (compatible) | YES (compatible) | Functionally equivalent to MIT |
| 6 | `NCSA` | Permissive (UIUC/NCSA Open Source License) | YES (compatible) | YES (compatible) | Permissive with attribution. Used by LLVM-derived code. |
| 7 | `Unicode-3.0` | Special data license | NO (for Unicode Inc. data redistribution in unmodified form) | NO (for unmodified data) | **NOT** a typical OSS license. Permits copying & distribution "free of charge" provided the Unicode Terms of Use are followed. **LEGAL REVIEW REQUIRED** for any code reuse of the unicode data file. |
| 8 | `OFL-1.1` | Font license | N/A (font license, not for code) | N/A | Used for Fira, NanumBarun, SourceCodePro, SourceSerif4. **NOT for code reuse.** |
| 9 | `CC-BY-SA-4.0` | ShareAlike — **copyleft** | NO (share-alike clause) | NO (share-alike clause) | Used for `embedded-book` documentation. **NOT compatible** with MIT/Apache-2.0 for code reuse. The Saturnite docs are not affected (they are separately authored). |
| 10 | `GPL-2.0-only` | Strong copyleft | **NO** | **NO** | Used for `src/gcc/gcc/testsuite/**`. Excluded. |
| 11 | `GPL-3.0-or-later` | Strong copyleft + patent + additional permissions | **NO** | **NO** | Used for `src/gcc/**`. Excluded. |
| 12 | `GCC-exception-3.1` | GCC runtime library exception | Companion to GPL | Companion to GPL | Used for one header. Only relevant if the GPL-licensed GCC code is bundled. |

---

## 2. Component-by-component license matrix

The columns are:

- **Component** — the rustc subtree or file(s) considered.
- **Original license** — the SPDX expression actually present.
- **Reuse possible?** — yes / no / conditional.
- **Conditions** — the obligations that come with reuse.
- **Attribution** — to whom.
- **NOTICE** — whether a NOTICE file is required.
- **Source redistribution** — whether source must be shipped.
- **Risk** — Low / Medium / High.
- **Decision** — what the audit recommends.

### 2.1 The MIT/Apache-2.0 core (the bulk of rustc)

| Component | Original license | Reuse? | Conditions | Attribution | NOTICE | Source | Risk | Decision |
|---|---|---|---|---|---|---|---|---|
| `compiler/rustc_lexer/**` | `MIT OR Apache-2.0` | Yes | (a) preserve license text; (b) preserve copyright; (c) do not use Rust Project / contributor names to endorse derived works | The Rust Project Developers (per `thanks.rust-lang.org`) | No (no NOTICE file) | No (binary redistribution allowed) | Low | **C. KEEP as architectural reference**; do not port |
| `compiler/rustc_data_structures/**` | `MIT OR Apache-2.0` | Yes | (a)+(b)+(c) | The Rust Project Developers | No | No | Low | **D. FUSE later** (only the `Interned` newtype is genuinely portable) |
| `compiler/rustc_data_structures/src/intern.rs` | `MIT OR Apache-2.0` | Yes | (a)+(b)+(c) | The Rust Project Developers | No | No | Low | **D. FUSE later** (port the newtype) |
| `compiler/rustc_mir_dataflow/src/framework/**` | `MIT OR Apache-2.0` | Yes | (a)+(b)+(c) | The Rust Project Developers | No | No | Medium | **D. FUSE later** (fork, retarget to Saturnite types) |
| `compiler/rustc_session/**` | `MIT OR Apache-2.0` | No (practically) | (a)+(b)+(c); + heavily coupled to `Session` / `Config` / `ParseSess` / `DiagCtxt` | The Rust Project Developers | No | No | Very High (coupling) | **E. REJECT (architecture)** |
| `compiler/rustc_ast/**` | `MIT OR Apache-2.0` | No | (a)+(b)+(c); + coupled to `Span`, `NodeId`, `TokenStream` | The Rust Project Developers | No | No | Very High | **E. REJECT (architecture)** |
| `compiler/rustc_hir/**` | `MIT OR Apache-2.0` | No | (a)+(b)+(c); + coupled to `TyCtxt` | The Rust Project Developers | No | No | Very High | **E. REJECT (architecture)** |
| `compiler/rustc_middle/**` | `MIT OR Apache-2.0` | No | (a)+(b)+(c); + 200k+ LOC of interned types tied to `'tcx` | The Rust Project Developers | No | No | Very High | **E. REJECT (architecture)** |
| `compiler/rustc_mir_build/**` | `MIT OR Apache-2.0` | No | (a)+(b)+(c); + coupled to `rustc_hir` | The Rust Project Developers | No | No | Very High | **E. REJECT (architecture)** |
| `compiler/rustc_mir_transform/**` | `MIT OR Apache-2.0` | No | (a)+(b)+(c); + coupled to `rustc_middle::mir` + `TyCtxt` | The Rust Project Developers | No | No | Very High | **E. REJECT (architecture)** |
| `compiler/rustc_const_eval/**` | `MIT OR Apache-2.0` | No | (a)+(b)+(c); + 50k+ LOC interpreter | The Rust Project Developers | No | No | Very High | **F. DEFER** |
| `compiler/rustc_borrowck/**` | `MIT OR Apache-2.0` | No | (a)+(b)+(c); + Polonius coupling | The Rust Project Developers | No | No | Very High | **F. DEFER** |
| `compiler/rustc_traits/**` + `rustc_trait_selection/**` | `MIT OR Apache-2.0` | No | (a)+(b)+(c); + 50k+ LOC of solver | The Rust Project Developers | No | No | Very High | **F. DEFER** |
| `compiler/rustc_codegen_llvm/**` | `MIT OR Apache-2.0` | No | (a)+(b)+(c); + 30+ `rustc_*` deps | The Rust Project Developers | No | No | Very High | **E. REJECT (architecture)**; Saturnite has `inkwell` |
| `compiler/rustc_codegen_cranelift/**` | `MIT OR Apache-2.0` | No | (a)+(b)+(c); + Cranelift coupling | The Rust Project Developers | No | No | Very High | **F. DEFER** (not 0.4-relevant) |
| `compiler/rustc_codegen_ssa/**` | `MIT OR Apache-2.0` | No | (a)+(b)+(c); + `TyCtxt` coupling | The Rust Project Developers | No | No | Very High | **E. REJECT (architecture)** |
| `compiler/rustc_incremental/**` | `MIT OR Apache-2.0` | No | (a)+(b)+(c); + `DepGraph` coupled to queries | The Rust Project Developers | No | No | Very High | **F. DEFER** |
| `compiler/rustc_metadata/**` | `MIT OR Apache-2.0` | No | (a)+(b)+(c); + tied to queries | The Rust Project Developers | No | No | Very High | **F. DEFER** |
| `compiler/rustc_target/**` (Rust code, NOT the JSON files) | `MIT OR Apache-2.0` | Yes (for the JSON schema) / No (for the Rust code) | (a)+(b)+(c); + coupled to `TargetOptions` | The Rust Project Developers | No | No | Low (schema only) | **D. FUSE later** (JSON schema only) |
| `compiler/rustc_abi/**` | `MIT OR Apache-2.0` | No (architecture) | (a)+(b)+(c); + tied to `TargetOptions` | The Rust Project Developers | No | No | Medium | **F. DEFER** |
| `compiler/rustc_errors/**` | `MIT OR Apache-2.0` | No | (a)+(b)+(c); + tied to `DiagCtxt` | The Rust Project Developers | No | No | Very High | **A. KEEP** Saturnite's `miette`-based design |
| `compiler/rustc_span/**` | `MIT OR Apache-2.0` | No | (a)+(b)+(c); + `Span` is 4-byte + `SourceMap` | The Rust Project Developers | No | No | High | **A. KEEP** Saturnite's `Range<usize>` model |
| `compiler/rustc_driver/**` + `rustc_driver_impl/**` + `rustc_interface/**` | `MIT OR Apache-2.0` | No | (a)+(b)+(c); + 1 686 + 4 246 LOC of driver | The Rust Project Developers | No | No | Very High | **A. KEEP** Saturnite's clap-based driver |

### 2.2 The special-license / submoduled components

| Component | Original license | Reuse? | Conditions | Attribution | NOTICE | Source | Risk | Decision |
|---|---|---|---|---|---|---|---|---|
| `compiler/rustc_llvm/llvm-wrapper/SymbolWrapper.cpp` | `Apache-2.0 WITH LLVM-exception AND (Apache-2.0 OR MIT)` | No (architecture) | Preserve license + LLVM exception | The LLVM contributors + The Rust Project Developers | No | No | Medium | **E. REJECT** (Saturnite uses `inkwell`; this is the FFI wrapper for the rustc build of LLVM) |
| `compiler/rustc_middle/src/ptrauth/llvm_siphash/tests.rs` | `Apache-2.0 WITH LLVM-exception AND (Apache-2.0 OR MIT)` | N/A (test code) | — | The LLVM contributors + The Rust Project Developers | No | No | Low | **N/A** (test only) |
| `library/core/src/unicode/unicode_data.rs` | `Unicode-3.0` (1991-2024 Unicode, Inc.) | Conditional | Unicode Terms of Use compliance | Unicode, Inc. | No | Source may be required | High (special) | **E. REJECT** (Saturnite does not need this data; use `unicode-general-category` / `unicode-width` / `unicode-ident` crates from crates.io, which are MIT/Apache-2.0) |
| `library/std/src/sync/mpmc/**` | `MIT OR Apache-2.0` | Yes (later) | (a)+(b)+(c); + attribute Crossbeam | The Crossbeam Project Developers + The Rust Project Developers | No | No | Low | **F. DEFER** (Saturnite 0.4 has no MPMC channel) |
| `library/std/src/sys/sync/mutex/fuchsia.rs` | `BSD-2-Clause AND (MIT OR Apache-2.0)` | Yes (later) | Preserve all three license texts; attribute Fuchsia Authors | The Fuchsia Authors + The Rust Project Developers | No | No | Low | **F. DEFER** (no Fuchsia target) |
| `src/test/rustdoc/auxiliary/enum-primitive.rs` | `MIT` | Yes (for that test) | Preserve MIT; attribute | Anders Kaseorg | No | No | Low | **N/A** (test only) |
| `src/librustdoc/html/static/fonts/**` (Fira, NanumBarun, SourceCodePro, SourceSerif4) | `OFL-1.1` | Yes (for fonts) | OFL-1.1 terms | Mozilla / Telefonica / NAVER / Adobe | No | No | N/A (fonts) | **N/A** (no rustdoc in Saturnite) |
| `src/librustdoc/html/static/css/normalize.css` | `MIT` | Yes | Preserve MIT | Nicolas Gallagher and Jonathan Neal | No | No | Low | **N/A** (no rustdoc) |
| `src/librustdoc/html/static/css/rustdoc.css` | `MIT OR Apache-2.0` | Yes | (a)+(b)+(c) | Ike Ku, Jessica Stokes, Leon Guan + The Rust Project Developers | No | No | Low | **N/A** |
| `src/doc/rustc-dev-guide/mermaid.min.js` | `MIT` | Yes (per-file) | Preserve MIT | Knut Sveidqvist | No | No | Low | **N/A** |
| `library/backtrace/**` | `MIT OR Apache-2.0` | Yes (later) | (a)+(b)+(c); attribute Alex Crichton + backtrace-rs | Alex Crichton + The Rust Project Developers | No | No | Low | **F. DEFER** (no backtrace in Saturnite runtime) |
| `src/doc/embedded-book/**` | `MIT OR Apache-2.0 OR CC-BY-SA-4.0` | No (CC-BY-SA copyleft) | CC-BY-SA-4.0 is **incompatible** | Rust on Embedded Devices WG + The Rust Project Developers | No | No | High | **N/A** (docs) |
| `src/doc/rust-by-example/**` | `MIT OR Apache-2.0` | Yes (for code) | (a)+(b)+(c) | Jorge Aparicio + The Rust Project Developers | No | No | Low | **N/A** (docs) |
| `src/llvm-project/**` (submodule) | `NCSA AND Apache-2.0 WITH LLVM-exception` | No (Saturnite has LLVM via `inkwell`) | NCSA + Apache-2.0 + LLVM-exception | LLVM contributors + Apple + University of Illinois | No | No | High (size) | **E. REJECT** |
| `src/gcc/**` (submodule) | `GPL-3.0-or-later` (bulk), `GPL-2.0-only` (testsuite), `ISC` (some analyzer files), `GCC-exception-3.1` (one header) | **NO** | GPL is copyleft | FSF + contributors | No | **YES** (source required) | **Very High** | **E. REJECT (HARD NO)** |
| `src/tools/cargo/**` (submodule) | `MIT OR Apache-2.0` (cargo's own) | Yes (architecturally) | (a)+(b)+(c) | The Cargo Project Developers | No | No | Low | **B. REIMPLEMENT** (do not vendor cargo; use as architectural reference) |
| `src/tools/rustc-perf/**` (submodule) | `MIT OR Apache-2.0` | Yes (later) | (a)+(b)+(c) | The rustc-perf Developers | No | No | Low | **F. DEFER** |
| `src/tools/enzyme/**` (submodule) | Mostly Apache-2.0 | Yes (later) | Enzyme project license | Enzyme / Modi Labs | No | No | Low | **F. DEFER** |
| `src/doc/{nomicon,reference,book,edition-guide}/**` (submodules) | `MIT OR Apache-2.0` | N/A (docs) | — | respective teams | No | No | Low | **N/A** |

### 2.3 Tools that are in-tree but dual-licensed

| Component | License (per `Cargo.toml`) | Reuse? | Conditions | Risk | Decision |
|---|---|---|---|---|---|
| `src/tools/clippy/**` | `MIT OR Apache-2.0` (`src/tools/clippy/Cargo.toml:6`) | Yes (for code) | (a)+(b)+(c) | Low | **N/A** (no lints in Saturnite yet) |
| `src/tools/rustfmt/**` | `Apache-2.0 OR MIT` (`src/tools/rustfmt/Cargo.toml:5`) | Yes (for code) | (a)+(b)+(c) | Low | **B. REIMPLEMENT** later (Saturnite's `stnx fmt`) |
| `src/tools/compiletest/**` | `MIT OR Apache-2.0` (REUSE blanket) | Yes (for code) | (a)+(b)+(c) | Low | **D. FUSE later** (runner scaffolding) |
| `src/tools/miri/**` | `MIT OR Apache-2.0` (REUSE blanket) | Yes (for code) | (a)+(b)+(c) | High (coupling) | **F. DEFER** (no MIR interpreter in Saturnite) |
| `src/tools/rust-analyzer/**` | `MIT OR Apache-2.0` (REUSE blanket) | Yes (for code) | (a)+(b)+(c) | Very High (coupling) | **N/A** (no IDE support in Saturnite) |
| `src/tools/tidy/**` | `MIT OR Apache-2.0` (REUSE blanket) | Yes (for code) | (a)+(b)+(c) | Low | **B. REIMPLEMENT** later (Saturnite-tidy analogue) |

### 2.4 Standard library

`library/{core,alloc,std,test,coretests,alloctests,compiler-builtins,profiler_builtins,unwind,rtstartup,stdarch,portable-simd,std_detect,windows-sys,windows_link,sysroot,panic_abort,panic_unwind,proc_macro,rustc-std-workspace-*,test}/**` — all under the `library/**` REUSE blanket → **`MIT OR Apache-2.0`**.

| Component | License | Reuse? | Conditions | Risk | Decision |
|---|---|---|---|---|---|
| `library/core/**` (excluding `unicode/unicode_data.rs`) | `MIT OR Apache-2.0` | **No** (language mismatch) | (a)+(b)+(c) | Very High | **E. REJECT (architecture)** — Rust's `core` assumes Rust's type system, ownership, traits, etc. Saturnite cannot reuse it. |
| `library/alloc/**` | `MIT OR Apache-2.0` | No | (a)+(b)+(c) | Very High | **E. REJECT (architecture)** |
| `library/std/**` | `MIT OR Apache-2.0` | No | (a)+(b)+(c) | Very High | **E. REJECT (architecture)** |
| `library/compiler-builtins/**` | `MIT OR Apache-2.0` | No (asm intrinsics tied to specific arch) | (a)+(b)+(c) | High | **F. DEFER** |
| `library/stdarch/**` | `MIT OR Apache-2.0` | No (arch intrinsics) | (a)+(b)+(c) | High | **F. DEFER** |
| `library/portable-simd/**` | `MIT OR Apache-2.0` | No (Rust types) | (a)+(b)+(c) | High | **F. DEFER** |

---

## 3. The license obligations Saturnite WILL inherit

If Saturnite ever takes **any** of the D. FUSE items (F1, F2,
F3, F4), Saturnite must:

1. **Preserve the MIT and Apache-2.0 license texts** in
   `LICENSES/` (per the standard pattern of putting one license
   file per license used).
2. **Preserve the copyright notice** in every ported file's
   header. Conventionally, a port is annotated like:
   ```
   // Originally derived from Rust Project Developers
   // (https://thanks.rust-lang.org), Apache-2.0 OR MIT.
   // Adapted for Saturnite by Dimitar.Simovski in 2026.
   ```
3. **NOT use "Rust" or "Rust Project" to endorse** a Saturnite
   derived product. (Standard MIT and Apache-2.0 "no endorsement"
   clause.)
4. **Document the provenance** in a per-file `ORIGINS.md` or in
   the top-level `THIRD_PARTY_PROVENANCE.md` (Phase 8).
5. **For Apache-2.0 specifically**: include a `NOTICE` file
   (if any notice-worthy events exist; in practice, rustc has no
   NOTICE file, so this is a no-op for the current rust tree).
6. **If submoduled code (e.g. `library/backtrace`)** is reused,
   the submodule's own copyright must be attributed (Alex Crichton
   + backtrace-rs developers).

None of these are onerous. They are normal open-source attribution
hygiene.

---

## 4. The license obligations Saturnite explicitly REJECTS (and why)

| Component | Why rejected |
|---|---|
| `src/gcc/**` (any of it) | **GPL-3.0-or-later** is a strong copyleft. Including even a portion in Saturnite would require Saturnite's whole distribution to be GPL. Saturnite is MIT/Apache-2.0. **HARD NO.** |
| `src/gcc/gcc/testsuite/**` | `GPL-2.0-only` — same issue. |
| `src/gcc/gcc/testsuite/c-c++-common/analyzer/*.c` | `ISC` — compatible, but the file is part of the GCC testsuite; including it would imply including GPL code. **E. REJECT** for that reason. |
| `src/llvm-project/**` | `NCSA AND Apache-2.0 WITH LLVM-exception`. Saturnite already has LLVM via `inkwell`; vendoring the LLVM source is unnecessary. Including the LLVM exception (which grants binary redistribution rights) is a legal complexity Saturnite does not need. **E. REJECT** as a code reuse; **A. KEEP** as a binary-only runtime dep (via `inkwell`). |
| `library/core/src/unicode/unicode_data.rs` | `Unicode-3.0` is a special data license with terms that differ from MIT/Apache-2.0. Saturnite does not need this data file. If/when Unicode data is needed, use the `unicode-general-category`, `unicode-width`, and `unicode-ident` crates (all MIT/Apache-2.0). **E. REJECT** as direct reuse. |
| `src/librustdoc/html/static/fonts/**` | `OFL-1.1` font license — irrelevant for a CLI compiler. **E. REJECT** as a category (not applicable). |
| `src/librustdoc/html/static/css/**` | MIT/Apache-2.0, but irrelevant for a CLI compiler. **E. REJECT** as a category. |
| `src/doc/embedded-book/**` (CC-BY-SA-4.0 portion) | ShareAlike clause is incompatible with MIT/Apache-2.0. Documentation is irrelevant for a compiler distribution anyway. **E. REJECT** as a category. |

---

## 5. The MIT/Apache-2.0 obligations in plain English

Both MIT and Apache-2.0 require:

- **Attribution**: include a copy of the license and the original
  copyright notice in any redistribution.
- **License preservation**: if you modify the code and
  redistribute, the modified files must carry a notice that they
  are modified.

Apache-2.0 adds:

- **NOTICE preservation** (no-op for current rustc).
- **Patent grant**: each contributor grants a patent license for
  their contribution. Recipients may not initiate patent litigation
  against contributors.

For Saturnite's purposes, the obligations are:

- Add `LICENSES/MIT.txt` and `LICENSES/Apache-2.0.txt` to the
  Saturnite repo (already there in spirit — `LICENSE-MIT` and
  `LICENSE-APACHE` exist; but they should be **kept** when any
  rustc-Project code is ported).
- Add a `THIRD_PARTY_PROVENANCE.md` (Phase 8) listing every
  rustc-derived file.
- Mark every ported file with a header noting the original
  copyright.

---

## 6. Items marked **LEGAL REVIEW REQUIRED**

The following are areas where the audit does **not** have
sufficient evidence to render a final reuse decision without
professional legal review:

1. **Unicode-3.0 (`library/core/src/unicode/unicode_data.rs`)** —
   the Unicode license has terms about prohibited uses (e.g.
   "no modification and distribution as part of a fonts,
   rasterization, or rendering system"). Saturnite is unlikely to
   fall afoul, but the audit cannot rule it out without reading
   the full Unicode Terms of Use.

2. **LLVM exception (Apache-2.0 WITH LLVM-exception)** — the
   exception grants rights specifically for "Larger Works" that
   include LLVM components. Saturnite's use of `inkwell` is a
   binary-only link against system LLVM; this is well within the
   exception's intent. The audit believes no review is required
   in practice, but a final answer requires reading the full
   exception text.

3. **GPLv3 implications for embedded GCC headers** — Saturnite
   will not include `src/gcc/**`, so this is moot for Saturnite.
   If a future Saturnite backend (e.g. Cranelift) were
   considered, the boundary is clear.

For the present audit, Saturnite is **safe to proceed** with
items A through D in the side-by-side table, and to **exclude**
all items marked E.

---

## 7. The bottom-line: what is legally safe to use?

| Decision | Items | Provenance | Risk |
|---|---|---|---|
| **SAFE** (any time) | Architectural reference to anything in `compiler/**`, `library/{core,alloc,std}/**` under MIT/Apache-2.0 | The Rust Project Developers | Low (no code copied) |
| **SAFE** (with attribution) | Port the `Interned` newtype from `rustc_data_structures::intern.rs` | The Rust Project Developers, MIT/Apache-2.0 | Low |
| **SAFE** (with attribution) | Adopt the JSON target spec format from `rustc_target` | The Rust Project Developers, MIT/Apache-2.0 (the JSON data itself is not copyrightable) | Low |
| **SAFE** (with attribution, later) | Use `compiletest`'s runner scaffolding as a Saturnite test-runner | The Rust Project Developers, MIT/Apache-2.0 | Low |
| **EXCLUDE** | Any code from `src/gcc/**` | FSF / GPL-3.0-or-later | **REJECTED** |
| **EXCLUDE** | Any code from `src/llvm-project/**` | LLVM contributors, NCSA + Apache-2.0+LLVM-exception | Not needed (Saturnite has LLVM via `inkwell`) |
| **EXCLUDE** | `library/core/src/unicode/unicode_data.rs` | Unicode, Inc., Unicode-3.0 | Use crates.io deps instead |
| **EXCLUDE** | Any code from the doc submodules (nomicon, reference, book, edition-guide, rust-by-example, embedded-book) | Various | N/A (docs) |
| **EXCLUDE** | `library/backtrace/**` (submodule) | Alex Crichton + Rust Project, MIT/Apache-2.0 | Not needed (Saturnite 0.4 has no backtrace) |
| **EXCLUDE** | `library/std/src/sync/mpmc/**` | Crossbeam + Rust Project, MIT/Apache-2.0 | Not needed (Saturnite 0.4 has no MPMC) |
| **EXCLUDE** | `library/std/src/sys/sync/mutex/fuchsia.rs` | Fuchsia Authors + Rust Project, BSD-2-Clause + MIT/Apache-2.0 | Not needed (no Fuchsia target) |

---

## 8. Required NOTICE / attribution file format

If Saturnite ports any rustc-Project code, Saturnite's repository
must add the following to `LICENSES/`:

```
LICENSES/
├── MIT.txt            # copy from rust's LICENSES/MIT.txt
├── Apache-2.0.txt     # copy from rust's LICENSES/Apache-2.0.txt
├── (any others)
```

And the top-level `NOTICE` (or equivalent) should include:

```
This product includes software developed by:
- The Rust Project Developers (https://thanks.rust-lang.org)
  Licensed under the Apache License, Version 2.0 or the MIT License.
  See LICENSES/Apache-2.0.txt and LICENSES/MIT.txt for details.

  Specific source files derived from rustc (commit <SHA>) are
  documented in THIRD_PARTY_PROVENANCE.md.
```

Plus, for any submodule-derived code:

```
- backtrace-rs developers (Alex Crichton et al.)
  See library/backtrace/ for the original sources.
- The Crossbeam Project Developers
  See library/std/src/sync/mpmc/ for the original sources.
- ...
```

These are standard open-source attribution practices.
