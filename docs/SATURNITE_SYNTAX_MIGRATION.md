# Saturnite 0.5 Native Syntax Migration Audit

> **Purpose:** Pre-implementation audit. Lists what each subsystem of the existing
> compiler requires (or does not require) to absorb the new native Saturnite
> syntax described in `docs/SATURNITE_SYNTAX.md`.

> **Implementation status (post-0.5):** The migration is complete. The native
> syntax is accepted and compiles/runs end-to-end. Key implementation decisions
> made during the phase:
>
> - **Colon blocks are desugared at the token level** (`lexer/prepare.rs`): a `:`
>   followed by a newline becomes an opening brace, and the indent pre-pass's
>   `Indent`/`Dedent`/`Newline` synthetic tokens are consumed to emit closing
>   braces and (for struct/enum bodies) field-line commas. The brace parser is
>   untouched.
> - **`main:`** desugars to `fn main() -> i64 { ... }` at the token level.
> - **Bare `name = expr` (Python-style declaration) was rejected.** It conflicts
>   with the existing immutable-assignment diagnostic (a later `x = 2` must be a
>   reassignment error, not a silently-shadowing declaration). Native code uses
>   explicit `let` for declarations. Bare `x = value` remains an assignment.
> - **String printing is implemented.** `say "..."`/`raise "..."` require a string
>   printer, so a new `println_str` builtin was added: runtime C function,
>   `build.rs`, MIR `PrintlnStr` statement, a second `PRINTLN_STR_DEF_ID`
>   sentinel, and the `HirType::Str` → `i8*` LLVM mapping (strings are now
>   NUL-terminated byte globals instead of raw const arrays).
> - **Named arguments are implemented** (`f(x, b: 2)`): `FunctionSig` gained a
>   `param_names` field, and call-site lowering reorders named args into
>   positional slots.
> - **String interpolation** parses to `Expr::InterpolatedStr`. Runtime rendering
>   is now implemented in 0.5.1 (`.tau/specs/phase0_5_1_string_interpolation.md`):
>   `Str` segments concatenate directly; `I64` segments use `str_i64`; other types
>   get a compile-time diagnostic.
> - **List literals** `[...]` lex and parse; runtime list support is deferred
>   (lowers to a placeholder string).

## Can remain unchanged

- **Resolver** (`crates/stnx/src/resolver.rs`) — `use`/`mod` semantics unchanged;
  closures are lambda-lifted so they don't carry captures; named args are
  reordered before resolution.
- **Type system / `HirType`** (`crates/stnx/src/hir/types.rs`) — `text`/`number`
  aliases map to existing `Str`/`I64` at the parser boundary; no new type
  variants required.
- **MIR lower / verify / opt** (`crates/stnx/src/mir/lower.rs`,
  `verify.rs`, `opt.rs`) — `Raise` is a new `HirStmtKind` that lowers to
  existing MIR terminators + an `abort` intrinsic; everything else reaches MIR
  via existing lowering paths.
- **Module system** (`crates/stnx/src/module.rs`) — `module name` is advisory
  metadata that does not change how modules are loaded.
- **MIR → LLVM codegen** (`crates/stnx/src/mir/codegen.rs`) — unchanged; only
  one new intrinsic (`raise` → abort) needs to be wired.
- **Diagnostics infrastructure** (`crates/stnx/src/error.rs`) — new `ParseError`
  variants plug into the existing `miette::Diagnostic` chain via the same
  pattern as the existing `LexError`/`ParseError`.
- **Object emission / linking** (`crates/stnx/src/codegen/*`) — unchanged.
- **Target config / CLI** — unchanged.

## Requires adaptation

- **Lexer** (`crates/stnx/src/lexer/{token.rs,mod.rs}`) — additive: add
  `Module`, `Give`, `Say`, `Raise`, `Pipe`, `Text`, `Number` keywords; add a
  pre-parse token filter that emits `Indent`/`Dedent`/`Newline` synthetic tokens.
- **AST** (`crates/stnx/src/ast.rs`) — additive: new `Stmt::Give`,
  `Stmt::Say`, `Stmt::Raise`; new `ItemKind::MainBlock`, `ItemKind::ModuleDecl`;
  new `Expr::Pipeline`, `Expr::Closure`, `Expr::InterpolatedStr`; `Expr::Call`
  gains parallel `named_args` field.
- **Parser** (`crates/stnx/src/parser/mod.rs`) — additive: new combinators for
  `module_decl`, `main_block`, `give_stmt`, `say_stmt`, `raise_stmt`, named
  args, pipeline expr, closure expr, interpolated str; `colon_block` for
  indentation-significant blocks.
- **HIR lowering** (`crates/stnx/src/hir/lower.rs`) — one new `HirStmtKind::Raise`;
  one new `HirStmtKind::StrInterp`; `FunctionSig.param_names` field added;
  lambda-lifting pass for closures; pipeline / interpolation / named-arg
  desugaring at the AST→HIR boundary.

## Requires redesign

Nothing. Every new construct either reuses existing types or extends an existing
type with a parallel variant. There is no need to redesign the compiler pipeline.

## Unknown

- **Performance of the indentation pre-pass** — should be O(n) per file; will
  benchmark once implemented.
- **Lambda-lifting name collisions** — synthetic function names use a fresh
  counter; need to verify there is no collision with user names.
- **Interaction between brace blocks and indented blocks in the same function**
  — out of scope for 0.5 (a single function uses one style).
- **Real `Result` / `?` semantics for `raise`** — deferred.

## Subsystem change table

| Subsystem              | Status           | Lines changed (estimate) |
|------------------------|------------------|-------------------------|
| Lexer                  | Adapt            | ~120                    |
| Parser                 | Adapt            | ~250                    |
| AST                    | Adapt            | ~80                     |
| HIR lowering           | Adapt            | ~150                    |
| MIR lower              | Minimal adapt    | ~30                     |
| Resolver               | Unchanged        | 0                       |
| Type system            | Unchanged        | 0                       |
| MIR codegen            | Minimal adapt    | ~20 (raise intrinsic)   |
| Diagnostics            | Adapt (1 variant)| ~30                     |
| Module system          | Unchanged        | 0                       |
| Tests                  | New              | ~600                    |
| Docs                   | New              | ~150                    |

## Risk assessment

- **Lexer**: low. Purely additive. Existing tests must keep passing.
- **Parser**: medium. Most logic is additive, but indentation handling is
  novel for this codebase. The `colon_block` production replaces `block` only
  at the new-style call sites; old brace blocks continue to work.
- **AST**: low. Purely additive.
- **HIR lowering**: medium. Lambda-lifting is a new pass. Named-arg reordering
  requires one new field on `FunctionSig`.
- **MIR**: low. Only one new intrinsic (abort).
- **Tests**: high coverage of new syntax; existing tests must keep passing.

## Compatibility strategy

Both syntaxes (legacy brace-style and native colon-style) are accepted during
the 0.5 transition. The legacy syntax remains the default for existing
examples; new examples may use either. There is no formal deprecation date —
the legacy syntax is preserved for the lifetime of 0.5.

## File plan

| File                                  | Action |
|---------------------------------------|--------|
| `crates/stnx/src/lexer/token.rs`      | Add new keywords |
| `crates/stnx/src/lexer/mod.rs`        | Add new keyword arms; integrate indent pre-pass |
| `crates/stnx/src/lexer/indent.rs`     | **NEW** — indent pre-pass |
| `crates/stnx/src/ast.rs`              | Add new AST variants |
| `crates/stnx/src/parser/mod.rs`       | Add new combinators |
| `crates/stnx/src/hir/stmt.rs`         | Add `Raise`, `StrInterp` |
| `crates/stnx/src/hir/expr.rs`         | (if needed for `StrInterp`) |
| `crates/stnx/src/hir/lower.rs`        | Add `FunctionSig.param_names`; lambda-lift; pipeline / interp / named-arg desugar |
| `crates/stnx/src/hir/function.rs`     | (may need a closure env struct) |
| `crates/stnx/src/mir/lower.rs`        | `Raise` → abort intrinsic |
| `crates/stnx/src/mir/codegen.rs`      | `abort` intrinsic → `llvm.trap` |
| `crates/stnx/src/error.rs`            | New `ParseError` variants |
| `tests/lexer.rs`                      | New tests |
| `tests/semantic.rs`                   | New tests |
| `tests/codegen.rs`                    | New tests |
| `examples/hello_native.stn`           | **NEW** — native-syntax example |
| `examples/inventory.stn`              | **NEW** — realistic end-to-end test |
| `docs/SATURNITE_SYNTAX.md`            | **NEW** (already written) |
| `docs/SATURNITE_SYNTAX_MIGRATION.md`  | **NEW** (this file) |
| `README.md`                           | Minor update — mention new syntax |

## Rollback plan

If a regression is detected in existing functionality, the legacy syntax path
(brace blocks, `return`, `println`, `i64`/`str`/`bool`, `mod`, `use ... as ...`)
must continue to work. Each commit is staged so it can be reverted without
breaking other stages.