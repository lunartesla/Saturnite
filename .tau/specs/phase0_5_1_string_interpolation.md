# Phase 0.5.1 — Runtime String Interpolation

Status: Implemented (Saturnite 0.5.1)

## 4.1 Problem

The native-syntax migration (0.5) accepts interpolated string syntax
(`say "Hello {name}!"`) and parses it into `Expr::InterpolatedStr`, but runtime
rendering was deferred. The HIR lowering pass rejected any interpolation
containing a non-literal segment with:

```
string interpolation expressions are not yet supported at runtime in 0.5;
use plain strings
```

Only all-literal interpolations worked (flattened to a plain `StrLit`). This
made the headlined 0.5 feature unusable for the only interesting case: a runtime
expression (`{name}`) whose value is not known at compile time.

## 4.2 Existing architecture

Before this change the string pipeline looked like:

```
source "Hello {name}!"
 ↓ lexer  →  TokenKind::StrLit("Hello {name}!")
 ↓ parser →  Expr::InterpolatedStr([Literal("Hello "), Expr(name), Literal("!")])
 ↓ HIR    →  rejected (runtime interpolation deferred)
```

What already existed and was reused:

- `HirStmtKind::PrintlnStr(HirExpr)` — `say`/`raise` with a `Str` argument.
  MIR lower maps it to a `Call` to the `PRINTLN_STR_DEF_ID` sentinel; LLVM
  codegen emits a call to the runtime `println_str(const char*)` symbol.
- `HirType::Str` → `i8*` LLVM mapping. Literal strings are NUL-terminated byte
  globals (`build_global_string_ptr`), so a `StrLit` evaluates to an `i8*`.
- `println_i64`/`println_str` in `crates/stnx/runtime/println_i64.c`, compiled
  to `libsaturnite_runtime.a` by `build.rs` and linked for `Exe` output.
- The builtin `DefId` sentinel pattern (`PRINTLN_DEF_ID`, `PRINTLN_STR_DEF_ID`)
  replicated across `hir/lower.rs`, `mir/lower.rs`, and `mir/codegen.rs`.
## 4.3 Intended representation

`"Hello {name}!"` becomes, at runtime, the concatenation:

```
"Hello " + runtime_repr(name) + "!"
```

with each segment lowered individually and combined left-to-right:

```
InterpolatedStr([Literal("Hello "), Expr(name), Literal("!")])
   ↓ lower each segment
"Hello "   (StrLit, i8* global)
name       (Str operand, i8* local / runtime value)
"!"        (StrLit, i8* global)
   ↓ chain of concat_str calls
concat_str(concat_str("Hello ", name), "!")
   ↓ println_str
final string printed with a trailing newline
```

Implementation representation: HIR lowering folds the segments into a nested
`HirExprKind::Call` to a new builtin sentinel `CONCAT_STR_DEF_ID`, whose
signature is `(Str, Str) -> Str`. Non-`Str` numeric segments are first wrapped
in a call to a second builtin sentinel `STR_I64_DEF_ID`, `(I64) -> Str`. This
reuses the existing generic `Call` path in monomorphization, MIR lowering, and
LLVM codegen — no new HIR/MIR form or MIR rvalue is introduced.

A segment is classified by its resolved `HirType`:

| source segment type | runtime value representation | how it becomes `Str` |
|---|---|---|
| `Str`            | `i8*`            | passed straight into `concat_str` |
| `I64`            | `i64`            | converted by runtime `str_i64(i64) -> char*` |
| anything else    | —                | compile-time diagnostic (see §4.7) |

## 4.4 Runtime ABI

New runtime C functions in `crates/stnx/runtime/println_i64.c`:

```c
char *concat_str(const char *a, const char *b);   // (i8*, i8*) -> i8*
char *str_i64(long long value);                    // (i64) -> i8*
```

LLVM declarations added in `declare_builtin_functions`:

- `concat_str`: `i8*(i8*, i8*)`, mapped from `CONCAT_STR_DEF_ID`.
- `str_i64`: `i8*(i64)`, mapped from `STR_I64_DEF_ID`.

`println_str(const char*)` is unchanged and receives the final concatenated
string (an `i8*`).

## 4.5 Ownership

Literal strings are NUL-terminated byte globals owned by the executable for the
whole process (existing model). Interpolated/numeric strings are produced by
`concat_str` / `str_i64`, which `malloc` a fresh buffer.

To avoid per-interpolation leaks while keeping the change minimal, the runtime
owns every heap string it allocates through a **process-wide arena**: each
result pointer is recorded in a growing table and the whole arena is freed once
at process exit via an `atexit` handler. This mirrors the existing "strings live
for the process" model, is visible to leak sanitizers as freed (all heap strings
are either reachable or freed at exit), and removes the need to distinguish
heap-owned from static-owned `i8*` at every call site.

Documented limitation: a program that interpolates inside a long-running loop
accumulates interpolated bytes until exit (no per-iteration reclaim). Saturnite
0.5.1 has no string mutation or destruction semantics, so this is consistent
with the rest of the string subsystem. A real per-string free/reference model is
deferred to a later phase and is NOT part of 0.5.1.

## 4.6 Type conversion

Supported interpolation types:

- `Str` (text) — concatenated directly.
- `I64` (number, incl. `text`/`number` aliases which resolve to `Str`/`I64`) —
  converted with `sprintf("%lld")`-style semantics via `str_i64`.

Not supported (compiler emits a clear diagnostic, never miscompiles):

- `Bool`, `F64`, `Unit`, structs, enums, `List<T>`, generics. There is no
  ToString contract for these in Saturnite 0.5.1. Broader formatting/conversion
  is intentionally excluded from scope.

## 4.7 Error behavior

If an interpolation segment resolves to a type with no supported runtime string
conversion, HIR lowering returns a `CompilerError::semantic` naming the
expression and the type:

```
string interpolation: cannot render a <Type> value; supported types are text and number
```

This is a compile-time error — no invalid or silently-wrong machine code is
generated.

An interpolated string inside `say` must still resolve to `Str`, so `say`
continues to route to `println_str`.

## 4.8 Testing strategy

- **Parser/analysis unit tests** — `Expr::InterpolatedStr` has literal + expr
  parts; unknown/unsupported segment types produce a diagnostic.
- **IR-level regression** — generated LLVM for `say "Hello {name}!"` contains
  calls to `concat_str` (never a hard-coded string literal).
- **End-to-end native tests** — compile and run interpolated programs and
  assert stdout:
  - `say "Hello {name}!"` with `name = "Saturnite"`.
  - multiple segments / adjacent literals / interpolation at start/middle/end.
  - numeric interpolation `say "Age: {age}"`.
  - non-interpolated literal still prints (no regression).
  - unsupported type interpolation fails analysis.
  - an on-disk `.stn` example (`examples/interpolation_demo.stn`) builds, links,
    and runs through the CLI.

There was **no** runtime `concat_str` and no numeric-to-string conversion.
The `concat_str` calls described in `docs/SATURNITE_SYNTAX.md` §3.8 were never
wired end-to-end — the migration report's "runtime rendering is deferred" note
was still accurate.