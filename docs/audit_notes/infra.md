# Saturnite Infrastructure & Tooling Audit — Evidence Fragment

Workspace crate under audit: `crates/stnx/` (single crate, edition 2021).
Verification method: source inspection (grep/sed on `crates/stnx/src/**`),
`cargo test --workspace` executed, `Cargo.lock` cross-checked.
All classifications: IMPLEMENTED / PARTIALLY IMPLEMENTED / DESIGN ONLY / STALE /
MISSING / BROKEN / DEAD/UNUSED.

---

## 1. MODULE SYSTEM (Section 9) — NO MODULE SYSTEM EXISTS

**Finding: MISSING.** No module/import/visibility support in lexer, parser, or AST.
Programs are strictly single-file sequences of `fn`s (and inline `struct`/`enum`).

Evidence (exact references):
- Lexer has no module keywords. `crates/stnx/src/lexer/mod.rs:8-50` defines
  `LexicalToken` keywords: Fn, Let, Mut, If, Elif, Else, For, While, In, Return,
  I64, F64, Bool, Str, Unit, True, False, Println, Struct, Enum. No Mod/Use/Pub.
- `TokenKind` (`crates/stnx/src/lexer/token.rs:4-61`) has no Mod/Use/Pub.
- `kw_span` (`crates/stnx/src/parser/mod.rs:718-758`) hard-codes the same keyword
  set in a `matches!` — no mod/use/pub branches.
- `is_keyword` (`crates/stnx/src/parser/mod.rs:766-789`) — no mod/use/pub.
- `program()` (`crates/stnx/src/parser/mod.rs:80-85`): `func().repeated().collect()`
  → a program is *only* a list of functions; mod/use items unparseable.
- `type_ann` (`parser/mod.rs:108-120`): only i64/f64/bool/str/unit + Struct(name).
- AST `Program` (`ast.rs:22-25`): `{ functions: Vec<Function> }` — no Module variant;
  `Type` (`ast.rs:5-18`) has no Module.
- `DefId` (`hir/symbol.rs:17-22`) + `SymbolInterner` (`symbol.rs:24-55`) are a
  *flat* string→id map and array index — not a hierarchical namespace; they index
  single-file definitions, not module paths.

**Minimum architecture needed (design notes):**
- *Lexer:* add Mod, Use, Pub, As tokens; extend `kw_span`/`is_keyword` (3 dup sites).
- *AST:* `Program { items: Vec<Item> }`, Item = Module{...} | Use{...} | Function |
  StructDef | EnumDef, each with `Visibility { pub: bool }`.
- *HIR:* `HirProgram { modules: HashMap<ModuleId, HirModule> }`, per-module
  `Namespace` with type + value child maps; `DefId` gains a module-path component.
- *Resolver:* `Resolver { module_stack, scope_stack }` for `mod foo {}` descent and
  `use foo::bar` imports + recorded import edges.
- *Module loader:* resolve `mod name;` to read `name.stnx`/`name/mod.stnx` relative
  to the current module dir, recursively lex->parse->lower. Tie to `saturn.toml`
  `[lib] path` / `[[bin]]` (see Section 2 — `from_dir` exists but is unused today).

---

## 2. CONFIG / `saturn.toml` (Section 10) — DESERIALIZED ONLY, NOT IN PIPELINE

**Finding: DESIGN ONLY for anything beyond deserialization.** The config is a plain
TOML->struct mirror; it never participates in compilation.

Supported fields (from `crates/stnx/src/config.rs`):
- `SaturnConfig` (`config.rs:27-35`): `package: Package` (default),
  `dependencies: BTreeMap<String, DependencySpec>` (default).
- `Package` (`config.rs:81-92`): `name: String`, `version: String` (default
  `"0.1.0"`), `edition: String` (default `"2026"`); `#[serde(deny_unknown_fields)]`.
- `DependencySpec` (`config.rs:119-131`): `#[serde(transparent)]`, field
  `version: String` only. `FromStr` (`config.rs:125-132`) just clones the string.
  -> **No** version-range parsing, **no** resolution, **no** source
  (path/git/registry), **no** fetching, **no** vendored/local-registry.

`from_dir` (`config.rs:41-58`) reads `saturn.toml` from a directory and falls back
to a synthesized minimal config — but **it is never called** outside `config.rs`.
Grep for callers: `grep -rn "from_dir\|SaturnConfig" src/` returns only
`lib.rs:75` (a re-export). `main.rs` **never imports or uses `SaturnConfig`**.

Build pipeline does NOT consult config (`crates/stnx/src/main.rs`):
- `Build` reads the input as a raw file path (`main.rs:259`:
  `std::fs::read_to_string(&input)`), lexes/parses/lowers/codegens. No project dir,
  no `target/` lookup, no `saturn.toml` read. The `config` matches in `main.rs`
  (lines 207-270, 479-514, 603) are all `TargetConfig` — a *different* type
  (target triple/opt level), not `SaturnConfig`.
- `Init` only *writes* `saturn.toml` (`main.rs:571-577`) via `format!`+`write`,
  never parsing it back.

Contrast with `docs/SATURNITE_DEPENDENCY_MODEL.md` (Phase 13):
- Doc claims version-requirement semantics (`"1.0.*"`, `>=0.1, <0.3`, `*`)
  (doc lines 32-37) and a Resolution Flow
  `DependencySpec -> Resolver -> vendored/fetched crates` (doc lines 39-52).
- Reality: `DependencySpec` holds a bare `String`; no resolver exists; no `vendor/`
  or `~/.cache/saturn/` (doc lines 56-59) is ever created; nothing is fetched.
  -> Dependency *resolution* is **MISSING** (documented as future Phase 14+).

---

## 3. CLI AUDIT (Section 11)

Executable name: **file on disk is `stnx`** (`crates/stnx/Cargo.toml:2`
`name = "stnx"`; no `[[bin]]` override -> binary named after package). The clap
display name is **`saturnite`** (`crates/stnx/src/main.rs:10`
`#command(name = "saturnite")`). README is inconsistent: uses `./target/release/stnx`
for the file path (`README.md:19-22`) but `saturnite build ...` for the command
(`README.md:88-104`). Following README literally -> "command not found".

Commands (all in `crates/stnx/src/main.rs`; `Commands` enum at lines 39-148,
dispatched at lines 149-151 / 365-417):

| Command | Variant lines | Flags / args | Handler |
|---|---|---|---|
| `Build` | `42-98`; dispatched `153+` | `input` (opt path), `-o/--output`, `--target`, `--emit-ir`, `--emit-object`, `--emit-exe`, `--print-target`, `--debug`, `--release`, `--opt-level`, `--json`, `--verbose`, `--no-link`, `--save-temps` | `build_run_file` (`459`) |
| `Check` | `100-109` | `input` (req path), `--target` | `check_file` (`520`): lex->parse->analyze only, no codegen |
| `Run` | `112-117` | `input`, `--debug/--release/--target` | builds to `std::env::temp_dir()` then `Command::status` (`371-396`) |
| `Doctor` | `~132-134`; dispatched `398` | none | `run_doctor` (`598`): host triple, linker via `which`, runtime `.a` presence |
| `Init` | `134-148`; dispatched `403` | `name` (pos), `--in-place`, `--pkg-version` | `init_project` (`539`): writes `saturn.toml` + `src/main.stnx` |

CLI loads `saturn.toml`? **NO** — build/check/run take a raw `PathBuf` source; `Init`
only *writes* the toml. Project discovery / workspace resolution? **NO** — none.

JSON output: **IMPLEMENTED** (not partial). `main.rs:329` `if json { ... }`, builds
`ArtifactInfo` (`main.rs:331`) + `BuildReport` (`main.rs:349`), emits via
`serde_json::to_string_pretty(&report)` (`main.rs:354`). Types at `main.rs:632-646`
(`#[derive(serde::Serialize)]`). Gap: only the *success* path emits JSON — error
paths (`?` early-return, `render_diagnostic`) return before building a report and
`success` is hard-coded `true`.

Cross-compilation guard (`main.rs:270-285`, `464-479`): `--target` != host triple
-> hard error "not yet supported in Saturnite 0.2" (stale "0.2" label on 0.3 code).
README still advertises `--target` as functional (`README.md:91`).

---

## 4. FUNCTIONS AUDIT (Section 8) — see `hir/lower.rs` + `codegen/context.rs`

- **Multiple functions:** YES — `HirProgram.functions: Vec<HirFunction>`
  (`crates/stnx/src/hir/function.rs:51`); codegen iterates all (`codegen/mod.rs:56,60`).
- **Parameters:** YES — AST `Vec<(String, Type)>` (`ast.rs:30`);
  HIR `Vec<(SymbolId, HirType)>` (`function.rs:21`).
- **Return types:** YES — `return_type: Type`/`HirType` (AST `ast.rs:31`; HIR
  `function.rs:22`). Implicit returns allowed (defaults to `0`/`0.0`/`void` via
  codegen default-return; `SATURNITE_0_3_ARCHITECTURE_REVIEW.md:233`).
- **Recursion / forward refs:** YES — two-pass. `CodeGenerator::generate_ir_string`
  (`codegen/mod.rs:51-66`) and `emit` (`mod.rs:84-90`) first loop calls
  `declare_function` for **all** funcs (populating the LLVM module symbol table),
  then a second loop calls `generate_function` — so forward references resolve.
  Lowering mirrors this: `lower.rs:143-190` collects all signatures in Pass 1.
  `test_forward_function_reference` exists in both `semantic.rs:98-101` and
  `codegen.rs:117-122`.
- **Function symbols:** YES — interned `SymbolId` (`SymbolInterner`,
  `hir/symbol.rs:24-55`); `HirFunction.name: SymbolId` (`function.rs:20`).
- **Name resolution:** by **string lookup** in a `HashMap`. Lowering interns the
  call name and looks it up in `function_sigs: HashMap<SymbolId, FunctionSig>`
  (`lower.rs:25,39,583-588`). Codegen resolves `DefId->String` via
  `func_names: HashMap<DefId, String>` populated in `declare_function`
  (`context.rs:31,58`), looked up by `resolve_func_name` (`context.rs:152-153`).
- **ABI:** C ABI (default) via inkwell: `fn_type(&param_types, false)`
  (`context.rs:66`); `println_i64` declared with C signature at
  `runtime/println_i64.c:4`.
- **Function pointers:** NO — `HirType` (`hir/types.rs:14-29`) has
  I64/F64/Bool/Str/Unit/Struct/Enum only; no `Fn` variant. `HirExprKind`
  (`hir/expr.rs:26-118`) has no callable-literal variant.
- **Closures:** NO — no `Closure`/`FnPointer` variant in `HirExprKind`.
- **External/`extern` functions:** NO — the only external symbol is the built-in
  `println_i64` (`context.rs:46-50`), special-cased by string match (`context.rs:463`).
  No `extern` grammar; no foreign-function declarations.
- **Capability check:**
  - `add(i64,i64)->i64`: **YES** (param + i64 return; exercised by
    `native_compilation.rs` arithmetic tests).
  - `factorial` (recursive): **YES** (two-pass declare-then-generate handles it).
  - `apply(fn(i64)->i64, i64)->i64`: **NO** — no function-pointer parameter type.

---

## 5. INCREMENTAL COMPILATION (Section 12) — DESIGN ONLY

- `docs/SATURNITE_INCREMENTAL_COMPILATION.md` header: "Status: Design Proposal"
  (`SATURNITE_INCREMENTAL_COMPILATION.md:3`).
- **No implementation in source:** `grep -rni "fingerprint|incremental"
  crates/stnx/src/` -> zero hits. (The only "cache" hit is a parser comment about
  chumsky `.memoized()` at `parser/mod.rs:253` — parser-internal memo, not
  incremental compilation.)
- **No caching layer:** no `fingerprint` module; no `target/incremental/` writer;
  no `HirProgram`/`HirFunction` serialization (confirmed: `grep -rn "Serialize"
  crates/stnx/src/hir/` -> NO hits). The doc's Phase 14b ("Add Serialize/Deserialize
  to HirProgram") is not done.
- Serialize is only on config types (`config.rs:27,81,119`) and main.rs report types
  (`main.rs:631,641`).
- **Rate: 0 = design only.**

---

## 6. TESTS (Section 16) — 116 tests, 0 failures (verified)

`cargo test --workspace` actual result:

| Test binary | Passed |
|---|---|
| lib (`config.rs` unit tests) | 7 |
| `src/main.rs` unittests | 0 |
| `tests/codegen.rs` | 14 |
| `tests/diagnostics.rs` | 6 |
| `tests/lexer.rs` | 17 |
| `tests/native_compilation.rs` | 47 |
| `tests/semantic.rs` | 28 |
| `tests/test_full_compile.rs` | 1 |
| `tests/test_ir_only.rs` | 1 |
| `tests/test_native_only.rs` | 1 |
| `tests/test_target_machine.rs` | 1 |
| Doc-tests | 0 |
| **Total** | **116, 0 failures** |

Coverage map (matches actual): Lexer **17**; parser **0** (no direct parser tests);
semantic **28**; HIR **0** (indirect); MIR **0** (not implemented); codegen **14**;
native execution **47**; diagnostics **6**; CLI **0**; config **7**; project mode **0**.

**STALE doc:** `docs/SATURNITE_FINAL_VERIFICATION.md:60` claims "123 tests, 0
failures" — **wrong** (actual 116). The +7 is a double-count: line 87 claims
"Phase 0-9: 116 tests" (which *already includes* the 7 config tests) and line 88
claims "Phase 10: 7 new tests" — adding them again yields 123. The doc is also
internally inconsistent: its own table (lines 49-59) sums to **126** (7+14+17+6+
47+28+4+1+1+1), not 123.

**Dead/stale test dirs:** root-level `tests/` (`tests/codegen.rs`,
`tests/lexer.rs`, `tests/semantic.rs`) **is git-tracked but orphaned** — the root
`Cargo.toml` has no `[package]` (only `[workspace]`), so `cargo test --workspace`
does **not** compile them. Real tests live in `crates/stnx/tests/`.
`examples/hello.stn` uses the stale `.stn` extension while `Init` writes `.stnx`
(`main.rs:579,581`).

**Invariants checked by tests:**
- `lexer.rs`: token kinds for int/float/str/ident/keywords; overflow->Error;
  `..` vs `.` vs `...` lexing; invalid char (`$`, `@`) -> LexError mentioning the char.
- `codegen.rs`: IR string assertions — ret i1/i64/double, ret void/no-ret-i64,
  for_cond/for_body/icmp, br i1 count >=2 for elif, `define i64 @main`, forward
  refs present.
- `semantic.rs`: return-type match/mismatch; `println` arg must be i64; range
  start/end must be i64; arg count ("expects N args") and arg-type mismatch;
  immutable-assignment rejection; struct/enum construction type + field/variant
  existence; forward references OK.
- `diagnostics.rs`: parse-error span non-zero length & offset within source; parse
  message contains "expected"/"unexpected"; lex error mentions bad char;
  undefined-variable, immutable-assignment, return-type-mismatch messages.
- `native_compilation.rs`: end-to-end compile->execute; asserts exit codes & stdout
  (arithmetic=20, vars=42/30, mut+augassign=10/39, for-range 0..4 + sum 45,
  inclusive 1..5 sum 15, while sum 10, if/else stdout "100", recursion factorial,
  struct round-trip, enum tags inactive=1/active=99); ELF header `\x7fELF` on
  object files; target init + invalid-triple errors.
- `config.rs`: full/minimal/empty/serde-roundtrip/multi-dep TOML parse and
  invalid-TOML rejection (deserialization only — no pipeline behavior).

---

## 7. DEPENDENCIES (Section 17)

`crates/stnx/Cargo.toml:7-24` direct deps (all verified used):
logos 0.16 -> lexer; chumsky 0.13 (+memoization) -> parser; inkwell 0.9
(+llvm21-1-prefer-dynamic) -> codegen+target; miette 7 (+fancy) -> error.rs +
main.rs GraphicalReportHandler (`main.rs:651`); thiserror 2 -> error.rs; clap 4
(+derive) -> main.rs; serde 1 (+derive) -> config.rs + main.rs; serde_json 1 ->
main.rs to_string_pretty; **toml 0.8** -> config.rs (`config.rs:62,216`);
anyhow 1 -> main.rs; which 5 -> linker.rs.

Build-dep: `cc 1` (`Cargo.toml:21`) -> `build.rs:28` (compiles
`runtime/println_i64.c`). Dev-dep: `tempfile 3` (`Cargo.toml:24`) ->
`tests/common/mod.rs:19`, `tests/test_target_machine.rs:9`.

`Cargo.lock`: **toml = 0.8.23**; 122 total packages.

**Workspace drift:** root `Cargo.toml:11-23` `[workspace.dependencies]` lists every
direct dep *except `toml`* — `toml` is declared only in the crate `Cargo.toml:16`,
not centralized. Minor but real inconsistency.

**STALE doc:** `README.md` dependency table (`README.md:116-129`) lists 12 crates
and **omits `toml`**. (The task premise that
`docs/SATURNITE_CRATE_DEPENDENCY_AUDIT.md:37` omits toml is *incorrect* — that doc
correctly lists `toml 0.8`. The README is the stale one.)

**Hand-rolled infra, no replacement needed:** `SymbolInterner`
(`hir/symbol.rs:24-55`) works; `DefId`/`SymbolId` are `u32` wrappers. No
`string-interner`/`lasso` crate is needed yet.

---

## 8. CRATE RECOMMENDATIONS (Section 22)

Based on actual gaps, preferring smallest footprint:
- **None needed for 0.3.** All current needs are met. Hand-rolled
  `SymbolInterner` is adequate (defer `string-interner` until profiling shows
  intern churn).
- **`semver`** (~$9KB): *only* when Phase 13 dependency resolution ships —
  `DependencySpec.version: String` cannot express `>=0.1, <0.3` or `1.0.*`.
  Premature today.
- **`petgraph`**: when the module graph lands (Section 1), for import-cycle
  detection and topological emit ordering. Premature today.
- **Filesystem walk** (project discovery): std `read_dir`/walk-up suffices — no
  new crate.
- Incremental (Phase 14): reuse existing `serde` for HIR serialization — no new
  crate; possibly `xxhash-rust` or `blake3` for faster hashing than SHA-256
  (doc line 26), but std `DefaultHasher` is fine for now.

---

## 9. DO NOT IMPLEMENT YET (Section 23) — premature for 0.4

Per the actual single-file architecture, the following would be premature:
1. **Module system** (mod/use/pub + path resolution + recursive module loader +
   workspace crate graph) — needs Phase 10->13 groundwork.
2. **Dependency resolution/fetching** (Phase 13) — `DependencySpec` is a string
   stub; nothing consumes dependencies. Implement *after* modules exist to import.
3. **MIR layer** (Phase 15) — design only (`SATURNITE_MIR_DESIGN.md`); HIR->LLVM
   directly today.
4. **Incremental compilation** (Phase 14) — design only; HIR not even
   `Serialize`-able.
5. **Cross-compilation** — runtime is host-only (`build.rs` cc targets host;
   `main.rs:270-285` rejects non-host triples).
6. **Function pointers / closures / higher-order fns** — no Fn type in HIR.
7. **External `extern` FFI blocks** — no grammar, no higher-FFI.
8. **Python interop** (Phase 13 pyo3) — pure design doc, gated feature.

---

## 10. RISKS (Section 18)

- **CRITICAL — `saturn.toml` is a silent no-op:** config deserializes fine but is
  never read by build/check/run (`main.rs` uses raw file paths; `from_dir`
  `config.rs:41-58` is uncalled). `Init` writes a config whose `dependencies`/
  `edition`/`name` have **zero** effect on compilation. High foot-gun risk.
- **CRITICAL — executable vs. command-name mismatch:** on-disk binary is `stnx`
  (`Cargo.toml:2`); clap name is `saturnite` (`main.rs:10`); README mixes both
  (`README.md:19` vs `:88`). Following README `saturnite build ...` -> command not
  found.
- **CRITICAL — dependency resolution is a stub:** `DependencySpec` is a bare
  `String` (`config.rs:121-122`); no resolver, no fetch, no vendor, no error on
  missing deps. `SATURNITE_DEPENDENCY_MODEL.md:39-52` documents a resolver that
  does not exist.
- **CRITICAL — `toml` dep not centralized:** declared only in the crate
  `Cargo.toml`, absent from `[workspace.dependencies]` (root `Cargo.toml:11-23`).
- **HIGH — stale documentation:** `SATURNITE_FINAL_VERIFICATION.md` claims 123
  tests (actual 116) and contradicts its own table (126).
  `SATURNITE_DEPENDENCY_MODEL.md` describes implemented resolution that is missing.
  README omits `toml` from the deps table and advertises cross-compilation as
  working.
- **HIGH — test-coverage gaps:** parser (0 direct tests), HIR (0), CLI (0),
  project mode (0). Config tests only cover deserialization, not that `Init`-
  generated projects build. Parse-error regressions are only caught indirectly
  via integration tests, making root-cause hard.
- **HIGH — orphaned, tracked test files:** root `tests/{codegen,lexer,semantic}.rs`
  are git-tracked but never compiled (no `[package]` at workspace root) ->
  confusion / dead weight.
- **MEDIUM — no MIR layer:** HIR->LLVM directly. Blocks mid-level opts (const
  folding, CSE, DCE, copy-propagation, tail-call) and backend flexibility
  (Cranelift). `SATURNITE_MIR_DESIGN.md` is aspirational.
- **MEDIUM — `from_dir` dead code:** synthesizes a config from the dir name
  (`config.rs:50-57`) but is never called — masks the missing-pipeline problem.
- **MEDIUM — cross-compilation advertised but disabled:** `main.rs:270-285`,
  `464-479` reject non-host targets; README still lists `--target` as functional.
- **LOW — duplicated keyword tables:** `LexicalToken` enum (`lexer/mod.rs:8-50`),
  `kw_span` matches (`parser/mod.rs:718-758`), and `is_keyword`
  (`parser/mod.rs:766-789`) are 3 manual copies — adding `mod`/`use`/`pub`
  requires edits in all 3 or a keyword parses but isn't recognized.
- **LOW — untested example API:** `examples/debug_parse.rs` consumes
  `program_debug`/`func_debug`/`params_debug`/`block_debug`/`ret_type_debug`
  (`parser/mod.rs:628-677`); example-only API not exercised by the test suite.
