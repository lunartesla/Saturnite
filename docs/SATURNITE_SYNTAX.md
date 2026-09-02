# Saturnite Native Syntax (0.5)

> **Status:** Implemented. Both legacy brace-style syntax (`fn f() -> i64 { ... }`,
> `return x`, `println(x)`, `i64`/`str`/`bool`) and the new colon-indented syntax
> described below are accepted. This document defines the new syntax, its grammar,
> semantics, and how it interacts with the existing compiler pipeline.
>
> **Implementation note:** the native colon-indented blocks are desugared at the
> **token level** into brace blocks by `crates/stnx/src/lexer/prepare.rs` (which
> wraps the indent pre-pass). The brace-based parser and all downstream stages
> (HIR, resolver, type checking, MIR, LLVM) are untouched for block structure.

The native syntax is **human-first**: it borrows Python's readability (indentation,
`:`, `say`, `give`, list literals) while keeping Saturnite's static typing, the
existing AST/HIR/MIR/LLVM pipeline, and the existing module/generics/MIR
infrastructure. It is **not** a Rust clone and **not** a Python clone.

---

## 1. Compatibility

| Construct             | Legacy (still works)            | Native (new)                            |
|-----------------------|---------------------------------|-----------------------------------------|
| Function declaration  | `fn name() -> T { ... }`        | `fn name() -> T:` + indented body       |
| Return                | `return expr`                   | `give expr` (also accepts `return`)     |
| Print                 | `println(expr)`                 | `say expr` (also accepts `println`)     |
| Entry point           | `fn main() -> i64 { ... }`      | `main:` + indented body                 |
| Type names            | `i64`, `str`, `bool`            | `number`, `text`, `bool`                |
| Module decl           | `mod name`                      | `module name` (both accepted)           |
| Block delimiters      | `{ ... }`                       | indented block after `:`                |
| Loop body             | `{ ... }`                       | indented block after `:`                |
| Struct body           | `{ ... }`                       | indented block after `:`                |

Both syntaxes may appear in the same file. The lexer emits identical tokens for
both (a `:` followed by a newline is treated like an opening brace by the new
branch in the parser); the brace form continues to be supported by the existing
production.

---

## 2. Lexical additions

`crates/stnx/src/lexer/token.rs` — added keyword tokens:

| TokenKind   | Spelled | Meaning                                            |
|-------------|---------|----------------------------------------------------|
| `Module`    | `module`| full-word module declaration alias for `mod`       |
| `Give`      | `give`  | synonym for `return` (mirrors early-return)        |
| `Say`       | `say`   | synonym for `println` (statement-level print)      |
| `Raise`     | `raise` | error raise (stub: lowers to abort intrinsic)      |
| `Pipe`      | `\|>`   | pipeline operator (desugars to nested call)        |
| `Text`      | `text`  | type alias for `str`                               |
| `Number`    | `number`| type alias for `i64`                               |
| `Indent`    | (synth) | emits when a line's indent > previous line's       |
| `Dedent`    | (synth) | emits when a line's indent < previous line's       |
| `Newline`   | (synth) | acts as a soft statement terminator                |

`Indent`/`Dedent`/`Newline` are emitted by a **pre-parse token filter**
(`crates/stnx/src/lexer/indent.rs`) that walks the byte offsets of the existing
token stream and tracks an indent stack. The combinator parser consumes them
just like `LBrace`/`RBrace`.

`text`/`number` map to existing `Str`/`I64` at the parser boundary; the HIR
never sees them.

---

## 3. Grammar (illustrative)

### 3.1 Modules / imports

```text
module inventory_manager              # top-of-file module name (advisory)

use collections: List, Map           # multi-import under colon
use math as M                        # legacy alias form
```

### 3.2 Structs

```text
struct Item:
    name: text
    price: number
    quantity: number = 0             # default value (parser-only; lowers to init expr)
```

### 3.3 Functions

```text
fn total_value(items: List<Item>) -> number:
    total = 0
    for item in items:
        total += item.price * item.quantity
    give total

fn restock(item: Item, amount: number) -> Item:
    if amount <= 0:
        raise "restock amount must be positive"
    item.quantity += amount
    give item
```

Function bodies use indentation. The indentation level must be strictly greater
than the function header's column. A `dedent` to the function-header level
closes the body.

### 3.4 Entry point

```text
main:
    catalog = [ ... ]
    say "..."
```

`main:` lowers to a synthetic `fn main() -> i64 { ... }` AST node with empty
parameters and `i64` return type. The `return 0` at the end is implicit.

### 3.5 If / elif / else

```text
if amount <= 0:
    raise "..."
elif amount > 100:
    say "large"
else:
    give item
```

### 3.6 Loops

```text
for item in items:
    say item.name

for i in 0..10:
    say i

while x < 100:
    x += 1
```

`0..10` is the inclusive-exclusive range; `0...10` is inclusive-inclusive (already
in the grammar).

### 3.7 Closures and pipelines

```text
give items
    |> filter(x -> x.price < limit)
    |> sort_by(x -> x.price)
```

- `x -> x.price < limit` is a single-parameter closure taking `x`.
- `(x, y) -> x + y` is a multi-parameter closure.
- `a |> f(x)` desugars to `f(a, x)`. `a |> f` desugars to `f(a)`. Left-associative.

Closures are **lambda-lifted** at HIR lowering: the closure expression becomes a
fresh `HirFunction` whose name is mangled (`__closure_<n>`) and whose parameters
include any captured variables. The closure expression site becomes a
`HirExprKind::Call` referencing the synthetic function. This avoids adding a
first-class function type while still passing closures through the existing
pipeline.

### 3.8 String interpolation

```text
say "Total inventory value: {total_value(catalog)}"
```

`"{...}"` segments inside a string literal are parsed into `Expr::InterpolatedStr`,
which lowers to a chain of `concat_str(...)` runtime calls. Each literal segment
is a `StrLit`; each `{...}` is a resolved expression. `Str` expressions concatenate
straight to the result; `I64` expressions are converted by a runtime `str_i64` before
concatenation. All other types are rejected with a clear compile diagnostic.
String interpolation is implemented in 0.5.1 (see §5 HIR additions).

### 3.9 Named arguments

```text
Item(name: "Widget", price: 4.50, quantity: 10)
cheap_items(catalog, limit: 5.00)
```

Named arguments are desugared to positional arguments at AST→HIR lowering by
re-ordering them against `FunctionSig.param_names` (which records the parameter
order during Pass 1).

### 3.10 Types

```text
number  # alias for i64
text    # alias for str
bool
List<T>, Map<K, V>
```

---

## 4. AST additions

All additions are **additive** — no existing variant changes:

- `Stmt::Give(Option<Expr>, Range<usize>)` — alias for `Stmt::Return` at lowering
- `Stmt::Say(Expr, Range<usize>)` — alias for `Stmt::Println` at lowering
- `Stmt::Raise(Expr, Range<usize>)` — error raise; lowers to a `Raise` intrinsic
  (deferred: in 0.5 lowers to `Println(msg); exit(1)` to keep the pipeline
  honest)
- `ItemKind::MainBlock(Vec<Stmt>, Range<usize>)` — `main:` block, lowered to a
  synthetic `Function { name: "main", ... }`
- `ItemKind::ModuleDecl(String, Range<usize>)` — `module name` header (advisory;
  parser metadata only)
- `Expr::Pipeline { lhs, rhs, span }` — desugars at lowering to nested calls
- `Expr::Closure { params, body, span }` — desugars at lowering to a synthetic
  top-level `HirFunction`
- `Expr::InterpolatedStr(Vec<StrPart>, Range<usize>)` — lowers to nested
  `concat_str` calls
- `Expr::Call` gains `named_args: Vec<(String, Expr)>` (parallel to `args`) for
  the transition

`CallArg` is the helper struct: `CallArg::Positional(Expr)` or
`CallArg::Named { name, value }`.

---

## 5. HIR additions

Minimal — only what's strictly required:

- `HirStmtKind::Raise(HirExpr)` — for `raise`. Lowers to MIR as
  `println(msg); exit(1)` (a runtime abort intrinsic).
- `HirStmtKind::StrInterp(Vec<InterpSegment>)` — preserves interpolation
  structure for diagnostics; codegen emits a chain of `concat_str` calls.
  (Implemented in 0.5.1: `concat_str`, `str_i64`, and the `Str`/`I64` interpolation
  conversion pipeline are now live in the compiler and runtime.)
- `FunctionSig.param_names: Vec<SymbolId>` — added so the lower pass can reorder
  named arguments.

No changes to `HirType` (text/number aliases are parsed away before HIR), no
changes to `HirExprKind::Call` (positional only at HIR level), no first-class
function type.

---

## 6. Pipeline impact

| Stage        | Change                                                          |
|--------------|-----------------------------------------------------------------|
| Lexer        | Add keywords; add `Indent`/`Dedent`/`Newline` pre-pass          |
| Parser       | New `module_decl`, `main_block`, `pipeline_expr`, `closure_expr`, `interpolated_str`, named-arg parsing |
| AST          | Additive new variants only                                      |
| HIR lowering | Desugar pipeline → calls, closure → synthetic function, interpolation → concat chain, named args → positional reorder |
| Resolver     | Unchanged (closures don't carry captures)                       |
| Type check   | Unchanged (no new type variants)                                |
| MIR lower    | Unchanged                                                        |
| MIR codegen  | `Raise` lowers to abort intrinsic; rest unchanged               |

---

## 7. Errors and diagnostics

- Missing indented block after `:` → `ParseError::MissingIndentedBlock { colon_span }`
- Mismatched indent levels → `ParseError::MismatchedIndent { found, expected }`
- `give` outside function → already enforced by HIR lowering (no top-level
  statements)
- Unknown named arg → existing call signature check emits the diagnostic

All errors flow through the existing `miette::Diagnostic` chain.

---

## 8. Deferred (not in 0.5)

- True first-class function values (`HirType::Fn`, `HirExprKind::Closure` as
  values) — closures work via lambda-lifting; real values are a later phase.
- Error semantics for `raise` — currently a stub that prints and aborts.
- `match` / pattern matching — not in this phase.
- Async/await — not in this phase.