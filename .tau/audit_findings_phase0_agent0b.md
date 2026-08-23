# Phase 0 Audit Findings (Agent 0B)
## Language & Project Architecture

### File: `crates/stnx/src/ast.rs`, `parser/mod.rs`, `lexer/mod.rs`, `hir/mod.rs`, `hir/symbol.rs`, `hir/lower.rs`, `hir/function.rs`, `crates/stnx/tests/semantic.rs`, `crates/stnx/src/module.rs`, `crates/stnx/tests/test_module_graph.rs`

---

## 1. Grammar Boundary

### 1.1 AST `ItemKind::ModDecl` is a unit variant with no data
**File:** `crates/stnx/src/ast.rs:83-86`
```rust
/// `mod foo` — declares a dependency on an external module file.
/// The module name is the item's `name`; the file is loaded later by the
/// module loader (Phase 4 infrastructure in `module.rs`).
ModDecl,
```
`ModDecl` carries no inner data (no path, no alias, no resolution). The module name is only available via `Item.name` (a `String`). This means the AST cannot represent `mod foo::bar` — there is no path field. The parser's `mod_decl()` (`parser/mod.rs:202-206`) only matches `mod <ident>` (a single identifier), not a path. There is no `::` token consumed after the module name.

### 1.2 `use` declarations support paths but `mod` does not
**File:** `crates/stnx/src/parser/mod.rs:200-220`
`use_decl()` parses `use foo::bar::baz` (a full path with `double_colon`) plus optional `as alias`. But `mod_decl()` (`parser/mod.rs:202-206`) only parses `mod <ident>` — no path traversal. This asymmetry means `use io::println` is fully supported in the parser but `mod foo::bar` cannot be parsed. `ItemKind::ModDecl` in `ast.rs:86` is a unit variant that cannot store a multi-segment path. If nested modules are desired, `ItemKind::ModDecl` must be extended.

### 1.3 Parser `item()` accepts `mod` and `use` as top-level-only items
**File:** `crates/stnx/src/parser/mod.rs:116-161`
The `item()` parser wraps `func()`, `struct_item()`, `enum_item()`, `mod_decl()`, and `use_decl()` with an optional `pub` prefix. The `stmt()` parser (`parser/mod.rs:339-438`) does NOT include `mod` or `use`. There are explicit tests confirming `mod` inside a function body is an error (`parser/mod.rs:1462-1471`).

### 1.4 `as` keyword is reserved in the lexer but `use ... as` is fully implemented
**File:** `crates/stnx/src/lexer/mod.rs:57-58` — `as` is a `LexicalToken::As` keyword token.
**File:** `parser/mod.rs:214` — `use_decl()` parses `as <alias>`:
```rust
.then(kw("as").ignore_then(t_ident().map(|(n, _)| n)).or_not())
```
The `as` alias IS functional. The design doc (`docs/audit_notes/module_language_design.md:76-87`) claims `as` is "reserved but grammar not designed." This is a doc contradiction — the grammar is designed and implemented.

### 1.5 No semicolon style is consistently applied
Both `mod` and `use` declarations are semicolon-free, matching Saturnite's no-semicolon style. The parser comment at `parser/mod.rs:114` confirms: "items terminate at the next newline/item."

---

## 2. Program Representation

### 2.1 Dual representation in `Program`: `items` + `functions`
**File:** `crates/stnx/src/ast.rs:22-46`
```rust
pub struct Program {
    pub items: Vec<Item>,
    pub functions: Vec<Function>,
}
```
`Program` maintains a flat `items: Vec<Item>` AND a backwards-compatible `functions: Vec<Function>`. The `from_items()` constructor (`ast.rs:36-45`) derives `functions` by filtering `items` for `ItemKind::Function`. This means `functions` is a redundant copy — a projection of `items`, not an independent store.

**Key consequence:** The HIR lowering in `lower.rs:397-411` has a fallback path that synthesizes `Item`s from the legacy `functions` vector when `items` is empty. This fallback is dead code for any `Program` constructed via `parser::parse()` (which always uses `Program::from_items`), but it is a code smell signaling an incomplete migration.

### 2.2 Struct/Enum definitions exist in two namespaces
- At the **top level**: `ItemKind::StructDef` and `ItemKind::EnumDef` (`ast.rs:73-82`) — parsed by `struct_item()` and `enum_item()` inside `item()`.
- **Inside function bodies**: `Stmt::StructDef` and `Stmt::EnumDef` (`ast.rs:116-127`) — parsed by `stmt()`, local to a function scope.

The HIR lowering (`lower.rs:199-290`) handles BOTH. Both end up in the same `HirProgram.structs` / `HirProgram.enums` vectors with `ModuleId::ROOT`. Local struct definitions leak into the global struct namespace, which could cause collisions.

### 2.3 Only `Program` is re-exported from AST
**File:** `crates/stnx/src/lib.rs:32`
Only `Program` is re-exported from `ast`, not `Item`, `ItemKind`, `Function`, `Stmt`, `Expr`, `Type`, `Visibility`, etc. Consumers must navigate `stnx::ast::` for full AST types. This is an inconsistent export surface.

---

## 3. DefId Architecture

### 3.1 `DefId(u32)` is a flat array index — separate index spaces per kind
**File:** `crates/stnx/src/hir/symbol.rs:39`
```rust
pub struct DefId(pub u32);
```
In `lower.rs`:
- **Functions** get `DefId(0), DefId(1), ...` from `func_def_id` counter (`lower.rs:413-433`)
- **Structs** get `DefId(0), DefId(1), ...` from `structs.len()` (`lower.rs:220`)
- **Enums** get `DefId(0), DefId(1), ...` from `enums.len()` (`lower.rs:238`)
- **Use/Mod declarations** get `DefId` from `next_def_id()` (`lower.rs:265,276`)

**This is a collision hazard.** Functions, structs, and enums all start their DefId space at 0. The `DefTable` (`symbol.rs:123-168`) registers entries keyed by position, but `DefEntry` includes a `DefKind` field to disambiguate. However, the MIR layer does NOT use `DefTable` — it uses `DefId.0` as a direct array index:

**File:** `crates/stnx/src/mir/lower.rs:497-501`
```rust
let ret_ty = self.sigs.get(def_id.0 as usize)
```
**File:** `crates/stnx/src/mir/lower.rs:486-519` — `lower_call` uses `def_id.0` as an array index into `sigs`.
**File:** `crates/stnx/src/mir/mod.rs:337-342` — `function_name()` does a linear search by `def_id == id`.

The MIR `lower_program` (`mir/lower.rs:31-51`) passes `&sigs` as `Vec<(Vec<HirType>, HirType)>` and accesses `sigs.get(def_id.0 as usize)` (`mir/lower.rs:499`). This assumes `DefId` for functions is a contiguous 0-based index into the functions vector. Since `func_def_id` starts at 0 and increments by 1, this is currently correct — BUT only because there is no cross-module DefId collision yet. Once modules introduce structs and enums that share the same DefId(0) space, the MIR `sigs` array would mis-index.

### 3.2 `next_def_id()` allocates DefIds from the symbol interner space
**File:** `crates/stnx/src/hir/lower.rs:147-159`
```rust
fn next_def_id(&mut self) -> DefId {
    let next = self.symbols.next_id().0;
    let sym = self.symbols.intern(&format!("__def_{}", next));
    DefId(sym.0)
}
```
Use/mod declarations get a `DefId` that is also a valid `SymbolId` in the same numeric space. This means DefIds for use/mod declarations could collide with function DefIds. The MIR `sigs` array (`mir/lower.rs:33`) does not consult `DefTable` — it indexes by `def_id.0` directly. This is a latent bug if MIR ever needs to resolve non-function DefIds.

### 3.3 `DefTable` exists but is discarded during HIR-to-MIR lowering
**File:** `crates/stnx/src/hir/symbol.rs:123-168` — `DefTable` with `register`, `lookup`, `iter`.
**File:** `crates/stnx/src/hir/function.rs:143` — `HirProgram.def_table: crate::hir::symbol::DefTable`
**File:** `crates/stnx/src/hir/function.rs:198-205` — `module_of()` consults both `module_paths` and `def_table`.
**File:** `crates/stnx/src/mir/lower.rs:31-51` — `lower_program` does NOT copy `def_table` into `MirProgram`.
**File:** `crates/stnx/src/mir/mod.rs:314-323` — `MirProgram` has `functions`, `symbols`, `structs`, `enums` — NO `def_table`.

The `DefTable` is constructed during HIR lowering (`lower.rs:391-476`) and stored in `HirProgram`, but it is **discarded** when lowering HIR to MIR. The MIR layer has no access to `DefTable`.

### 3.4 `module_paths` is a redundant parallel HashMap
**File:** `crates/stnx/src/hir/function.rs:141`
```rust
/// Maps each item `DefId` to its owning `ModuleId`.
/// (Redundant with `def_table` but provided for O(1) lookup without
/// indexing into the `def_table`'s `DefEntry`.)
pub module_paths: HashMap<DefId, ModuleId>,
```
The comment explicitly states this is redundant with `DefTable`. Two data structures must stay in sync — a maintenance burden.

### 3.5 `DefKind` includes `Module` and `Use` variants in implementation but not in design doc
**File:** `crates/stnx/src/hir/symbol.rs:91-100` — `DefKind` has `Function, Struct, Enum, Module, Use`.
**File:** `docs/audit_notes/module_language_design.md:518-521` — only `Function, Struct, Enum`.
The HIR implementation has evolved beyond the design doc.

---

## 4. Namespace Limitations

### 4.1 Single namespace: types and values share the same space
**File:** `crates/stnx/src/module.rs:283-298`
```rust
pub struct ModuleScope {
    pub items: HashMap<SymbolId, DefId>,
    pub imports: HashMap<SymbolId, DefId>,
    pub parent: Option<ModuleId>,
}
```
`items` maps `SymbolId -> DefId` with no type discrimination. A struct named `Foo` and a function named `Foo` in the same module would collide. The design doc (`module_language_design.md:250-253`) acknowledges this.

### 4.2 No bridge between `ModuleScope` and `LowerScope`
`LowerScope` (`lower.rs:67-96`) handles lexical variable scoping. `ModuleScope` (`module.rs:290-298`) handles module-level items. The `LowerScope.lookup_variable()` (`lower.rs:89-95`) does NOT check `ModuleScope` for items — there is no integration between the two scope systems. Local variable name resolution and module-level name resolution are completely separate code paths.

### 4.3 `ModuleScope::lookup` does NOT walk the parent chain
**File:** `crates/stnx/src/module.rs:325-334` — `lookup()` does not walk parent.
**File:** `crates/stnx/src/module.rs:336-351` — `lookup_with_parent()` is a free function taking `&[ModuleScope]` rather than a method on `ModuleScope`. This is an unusual API design.

### 4.4 No `use` path resolution to actual definitions
The `HirUseDecl` (`hir/function.rs:92-103`) stores `path: Vec<SymbolId>` and `alias: SymbolId`, but the target `DefId` is never resolved. The `module_id` in `HirModDecl` (`hir/function.rs:111-122`) is `None` — "resolved by the module graph in Phase 6" (`lower.rs:280`). The module loader uses text-based scanning exclusively (`module.rs:512,548`) and ignores the parsed AST for mod discovery.

### 4.5 No cross-module name resolution
`HirUseDecl` stores the path but never resolves it. The `ModuleScope` has an `imports` map (`module.rs:293`) but it is never populated during lowering — `lower.rs` never calls `ModuleScope::define_import()`. The HIR lowering (`lower.rs:162-502`) does not consult `ModuleScope` for name resolution; it uses `LowerScope` (lexical variables) and `function_sigs` (a flat `HashMap<SymbolId, FunctionSig>`) for function name resolution (`lower.rs:874`).

### 4.6 `module.rs` comment says "mod keyword is not yet in the lexer"
**File:** `crates/stnx/src/module.rs:493-496`
> "The `mod` keyword is not yet in the lexer (Phase 5 adds it), so this method currently scans the source text for `mod <ident>` declarations using a lightweight text-based approach."

**Reality:** The lexer has `Mod` (`lexer/mod.rs:51-52`), the parser handles `mod` (`parser/mod.rs:202-206`), and the token type exists (`lexer/token.ts:25`). This comment is stale from a pre-Phase 5 era.

---

## 5. saturn.toml Usage

### 5.1 `SaturnConfig::from_dir()` exists but is never called by the CLI
**File:** `crates/stnx/src/config.rs:41-58` — `from_dir()` walks a directory to find `saturn.toml`.
**File:** `crates/stnx/src/main.rs:473-545` — `build_run_file()` reads a raw file path directly with `std::fs::read_to_string`. No config lookup.
**File:** `crates/stnx/src/main.rs:547-562` — `check_file()` does the same.

The `Project` type (`module.rs:703-820`) and `Project::discover()` (`module.rs:728-779`) implement upward-walk discovery and config loading. But `main.rs` never calls `Project::discover()`.

### 5.2 `Init` writes `saturn.toml` but Build/Check/Run never read it
**File:** `crates/stnx/src/main.rs:598-604` — `init_project()` writes `saturn.toml` + `src/main.stnx`.
**File:** `crates/stnx/src/main.rs:473-545` — `build_run_file()` and `check_file()` take a `PathBuf` input and never call `Project::discover()` or `SaturnConfig::from_dir()`.

The `saturn.toml` on disk is write-only from the CLI's perspective.

### 5.3 `resolve_output()` uses file stem, not package name
**File:** `crates/stnx/src/main.rs:443`
```rust
let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("out");
```
The output binary is named after the input file's stem, not the package name from `saturn.toml`.

### 5.4 `DependencySpec` is `#[serde(transparent)]` with only a version field
**File:** `crates/stnx/src/config.rs:119-123`
No support for path dependencies, git dependencies, or anything beyond a bare version string.

### 5.5 `lib.rs` re-exports `Project` but `main.rs` doesn't use it
**File:** `crates/stnx/src/lib.rs:83`
`Project` is publicly exported but unused by the CLI. The `test_module_graph.rs` integration tests import and test it, but `main.rs` bypasses it entirely.

---

## 6. Test Gaps

### 6.1 No end-to-end module compilation tests
**File:** `crates/stnx/tests/test_module_graph.rs`
The integration tests cover project discovery, module discovery, missing modules, duplicate modules, and `saturn.toml` parsing. But there are NO tests for:
- End-to-end compilation of a multi-file project (discover -> parse -> lower -> MIR -> codegen)
- `use` declaration resolution across modules
- Cross-module function calls
- `pub` visibility enforcement across module boundaries
- `DefTable` entries for non-root modules
- Integration of `ModuleGraph` with `HirLower` (the `Project::load()` method returns a `Program` but `HirLower::lower_program()` does not accept module graph data)

### 6.2 `discover_modules()` uses text-based `extract_mod_declarations`, not AST
**File:** `crate/stnx/src/module.rs:494-496` (stale comment), `module.rs:512,548`
The text-based scanner will fail to parse:
- `mod` declarations inside string literals
- `mod` declarations on the same line as other code
- `pub mod` with extra whitespace (uses `strip_prefix("pub mod ")` requiring exactly one space)

### 6.3 No test for `use` path resolution or alias
**File:** `crates/stnx/tests/test_module_graph.rs`
Parser tests (`parser/mod.rs:1288-1345`) test parsing `use foo::bar` and `use foo::bar as baz`, but no integration tests verify that imported names actually resolve.

### 6.4 `semantic.rs` tests only test single-function programs
**File:** `crates/stnx/tests/semantic.rs:1-171`
All tests use single-function programs (`fn main() -> i64 { ... }`). No tests mix `mod`, `use`, `pub`, or top-level struct/enum definitions.

### 6.5 Duplicate test function names across integration and unit tests
**File:** `crates/stnx/tests/test_module_graph.rs` and `crate/stnx/src/module.rs`
Many test function names appear in both the integration test file and the inline `#[cfg(test)] mod tests` in `module.rs`. While Rust allows this (separate compilation units), it reflects unclear test architecture boundaries.

---

## 7. Doc Contradictions

### 7.1 `module.rs` comment says "mod keyword is not yet in the lexer"
**Location:** `crates/stnx/src/module.rs:493-496`
States the `mod` keyword is not in the lexer. **Reality:** `lexer/mod.rs:51-52`, `lexer/token.rs:25`, `parser/mod.rs:202` all support `mod`. This comment is stale.

### 7.2 Design doc says `as` is "reserved but grammar not designed"
**Location:** `docs/audit_notes/module_language_design.md:76-87`
States `as` is reserved but grammar not designed. **Reality:** `parser/mod.rs:214` fully implements `as <alias>` in `use_decl()`. The grammar IS designed and implemented.

### 7.3 Design doc says `Program { items: Vec<Item> }` replaces `functions`
**Location:** `docs/audit_notes/module_language_design.md:631-638`
Proposes removing `functions` field entirely. **Reality:** `ast.rs:22-32` keeps BOTH `items` and `functions`. The design doc proposes removal; the implementation keeps backward compatibility.

### 7.4 Design doc `ItemKind` shape differs from implementation
**Location:** `docs/audit_notes/module_language_design.md:618-626` vs `ast.rs:71-93`
Design doc shows `StructDef { fields }` (no `name`/`span`), `ModDecl {}` (empty body), `UseDecl { path }` (no `alias`). Implementation has `StructDef { name, fields, span }`, `ModDecl` (unit variant), `UseDecl { path, alias }`. The docs are stale.

### 7.5 `project_architecture.md` says "single-file compiler" but `module.rs` implements project discovery
**Location:** `docs/audit_notes/project_architecture.md:11-22`
States there is "no project discovery, no `saturn.toml` integration." **Reality:** `module.rs` implements `Project::discover()`. The architecture doc describes a state superseded by Phase 4 work.

### 7.6 `project_architecture.md` Section 6.1 claims no `mod`/`use`/`pub` keywords
**Location:** `docs/audit_notes/project_architecture.md:192-195`
> "There are no `mod`, `use`, or `pub` keywords in the lexer or parser."
**Reality:** All three keywords exist in the lexer, token enum, and parser. This statement is completely false. The design doc was written before Phase 5A implementation and never updated.

### 7.7 `project_architecture.md` Section 6.4 says `DefId` stays flat
**Location:** `docs/audit_notes/project_architecture.md:192-195` and `:242`
States `DefId` stays flat and proposes future "qualified `DefId` paths." **Reality:** A `DefTable` already exists (`hir/symbol.rs:123-168`) mapping `DefId -> (ModuleId, local_index, DefKind)`. The doc's future proposal already exists.

### 7.8 `hir/mod.rs` doc says semantic analysis is "absorbed" by lowering
**Location:** `crates/stnx/src/hir/mod.rs:24`
> "`lower` — `lower()` function: AST → HIR (absorbs `semantic::analyze`)"
**Reality:** `semantic.rs:16-26` still exists as a public module with `analyze()` and `analyze_and_lower()`. The `semantic::analyze` function was NOT absorbed — it delegates to `hir::lower::lower_unit()`. The doc comment in `hir/mod.rs` is aspirational/incorrect.

### 7.9 Design doc references `config.rs` fields that don't exist
**Location:** `docs/audit_notes/project_architecture.md:26-34` (table), `:115-142`
The design doc references `config.rs:41-58` for `from_dir()`, `config.rs:81-92` for `Package`, `config.rs:119-131` for `DependencySpec`. These line numbers are approximately correct. However, the design doc's Section 11 summary table (`project_architecture.md:511`) lists `semver` as needed in Phase 14, while `DependencySpec` (`config.rs:119-123`) only stores a version `String` — no semver validation. This is consistent (not a contradiction), but the design doc's claim that `DependencySpec` "has no resolver, no fetcher, no vendor directory" (`project_architecture.md:328`) is accurate and matches the implementation.

### 7.10 `saturn.toml` on disk has edition "2026" but design doc mentions "2026" as the only known edition
**Location:** Repo root `saturn.toml` (created by `init_project`, `main.rs:600`) vs `config.rs:99` default
```rust
fn default_edition() -> String { "2026".to_string() }
```
This is consistent — no contradiction. But the design doc (`project_architecture.md:131`) says "edition = \"2026\" is the only known edition" while `Package` has no `edition` enum — it is a free-form `String`. A user could write `edition = "2027"` and it would parse without error, despite the design doc stating "2026" is the only known edition.

---

## 8. Key Findings Summary

1. **AST/Parser is ahead of HIR for modules**: The AST (`ast.rs:71-93`), parser (`parser/mod.rs:200-220`), and lexer (`lexer/mod.rs:51-58`) all support `mod`, `use`, `pub`, and `as`. The HIR lowering (`lower.rs:162-502`) records these as `HirUseDecl` / `HirModDecl` but does NOT resolve them. The `module.rs` module discovery layer uses a **text-based scanner** (`extract_mod_declarations`) instead of the AST, despite the AST being available.

2. **DefId collision risk is real and unmitigated in MIR**: Functions get `DefId(0..N)` and structs get `DefId(0..M)` from separate counters in `lower.rs`. The `DefTable` disambiguates, but MIR (`mir/lower.rs:499`) uses `DefId.0` as a direct array index into a function-only `sigs` vector. This works today only because modules are not yet compiled end-to-end.

3. **CLI is disconnected from project/module infrastructure**: `main.rs` `build_run_file()` (`main.rs:473-545`) and `check_file()` (`main.rs:547-562`) take a raw `PathBuf` and read it directly. `Project::discover()` (`module.rs:728-779`) exists and is fully tested (`test_module_graph.rs`) but is never called from the CLI. The `saturn.toml` is write-only.

4. **Stale comments are pervasive**: `module.rs:493-496` claims `mod` is not in the lexer; `hir/mod.rs:24` claims `semantic::analyze` was absorbed; design docs describe a pre-Phase-5 state. The codebase is significantly ahead of its own documentation.
