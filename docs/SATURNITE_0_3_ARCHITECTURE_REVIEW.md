# Saturnite 0.3 Architecture Review

**Status:** Completed as part of the 0.2 release checkpoint.

This document is a forward-looking review performed **before** the start of
Saturnite 0.3 (the language-expansion milestone).  Its purpose is to identify
architectural decisions in 0.2 that would make future features difficult, so
that 0.3 can be planned rather than discovered mid-implementation.

The conceptual pipeline targeted for 0.3 is:

```
Saturnite source -> AST -> HIR -> MIR -> LLVM SSA
```

Saturnite 0.2 does **not** implement HIR or MIR.  The current pipeline is:

```
Saturnite source -> AST -> (semantic analysis) -> LLVM SSA
```

---

## 1. SAFE TO KEEP

These 0.2 designs are fundamentally sound and should be carried forward
into 0.3 without major changes.

### 1.1 Spans on every AST node

Every `Expr` and `Stmt` variant in `crates/stnx/src/ast.rs` carries a
`Range<usize>` byte-span.  The parser (chumsky 0.13 with `SimpleSpan`)
converts token-index spans to byte-offset spans at the boundary.  This means
source-location information is available at every layer from lexing through
parsing.  In 0.3, HIR and MIR nodes can simply inherit or extend these spans.

### 1.2 Variable storage model (SSA + optional alloca)

The `Variable<'ctx>` struct in `codegen/context.rs` stores both an
`ssa_value` and an optional `alloca` pointer.  Immutable variables have
`alloca: None` (pure SSA).  Mutable variables get an `alloca` slot with
`store`/`load`.  This design is correct and is **not** hardwired to "all
mutables live in memory" — the split is explicit: the semantic checker
records `mutable: bool`, and the codegen decides alloca-vs-SSA at that
point.  When 0.3 introduces MIR, the memory-promotion pass can promote
eligible allocas back to SSA form.

### 1.3 Clean pipeline: CodeGenerator -> Module -> ObjectEmitter -> Linker

The separation in `codegen/mod.rs` is clean:

- `CodeGenerator::emit` produces an LLVM `Module`.
- `ObjectEmitter` takes a `Module` + `TargetConfig` and writes an object file
  (or IR text) via `TargetMachine`.
- `Linker` takes the object path + `TargetConfig` and invokes the system linker.

Each stage has a single responsibility.  In 0.3, insert HIR and MIR stages
between AST and `CodeGenerator`.

### 1.4 TargetConfig with triple validation

`TargetConfig` (`target.rs`) validates triples via `Target::from_triple`,
stores architecture/OS/environment, and preserves `opt_level` / `debug_info`
/ `output_kind`.  The `host()` and `from_triple()` constructors are correct.

### 1.5 Linker discovery via `which`

`Linker` (`linker.rs`) uses the `which` crate to locate the linker binary and
fails with a clear diagnostic when it is missing.  Windows/MSVC (`link.exe`)
and Windows/GNU (`gcc`) paths are already handled.

### 1.6 Runtime compilation via `build.rs` + `cc`

The runtime (`runtime/println_i64.c`) is compiled at build time by `cc` and
stored in `OUT_DIR`.  No checked-in architecture-specific `.o` file is used.
If `cc` is unavailable or the source is missing, `build.rs` fails with a clear
diagnostic.

### 1.7 Optimization pipeline

Optimization uses `Module::run_passes` with `PassBuilderOptions` and
pass-manager strings (`default<O1>`, `default<O2>`, `default<O3>`).
Verified to produce measurably different output (opt-3 object is ~37% smaller
than opt-0 for the test program).

### 1.8 miette diagnostic rendering

`LexError` and `ParseError` derive `miette::Diagnostic` with `#[source_code]`
and `#[label(...)]` annotations.  `render_diagnostic` in `main.rs` renders
these through `GraphicalReportHandler`, producing source-span-underlined output.

---

## 2. SHOULD REFACTOR BEFORE 0.3

These 0.2 designs work but contain technical debt that will compound if
0.3 adds richer features.

### 2.1 Semantic errors lack source spans

`CompilerError::Semantic(String)` and `CompilerError::Type(String)` carry
only a message -- no source span.  This means semantic errors (undefined
variable, type mismatch, mutability violation) cannot be rendered with
source-context underlines.  In 0.3, these should become structured error
types similar to `ParseError` (with `src` + `span`).

### 2.2 Scope does not track declaration sites

`Scope` (`semantic.rs`) stores `HashMap<String, (Type, bool)>`.  When an
error like "cannot assign to immutable variable: x" is produced, there is no
information about *where* `x` was declared.  Adding a span to each variable
entry enables "declared here" notes.

### 2.3 Redundant span on Stmt::Expr

`Stmt::Expr(Expr, Range<usize>)` stores both an inner `Expr` (which already
has its own span) and a separate span.  The `stmt_span` helper is used to
synchronize them, but this is error-prone redundancy.  Consider storing only
the `Expr` and deriving the span when needed.

### 2.4 No implicit returns

`Stmt::Expr` in the codegen discards the expression value.  The function
always falls through to a default `return 0` / `return 0.0` / `return void`.
This works for 0.2 (all existing tests use explicit `return`), but 0.3 will
likely want implicit returns for ergonomics.

### 2.5 Duplicated pipeline setup

`CodeGenerator::generate_ir_string` and `CodeGenerator::emit` both repeat
the same four-step setup (create context -> declare builtins -> declare
functions -> generate functions).  Extract a `build_module` helper.

### 2.6 CLI error handling inconsistency

The `build_run_file` helper (used by `saturnite run`) uses `anyhow!` for all
error types, bypassing `render_diagnostic`.  Only the `build` subcommand uses
`render_diagnostic` for parse/semantic errors.  Error rendering should be
uniform across all CLI paths.

### 2.7 If-expression always returns Unit

The semantic checker's `Expr::If` arm returns `Type::Unit` without checking
that both branches produce compatible types.  In 0.3, if-expressions may
become value-producing expressions, requiring type unification across branches.

### 2.8 Loop variable forced immutable in semantic check

In `check_expr`, the `Expr::For` arm defines the loop variable as
`mutable: false`.  This is correct for 0.2 (the loop variable is managed by
the loop, not user-reassigned), but it prevents future support for
reassigning loop variables inside the body.

---

## 3. SHOULD MOVE INTO FUTURE HIR

These concerns are currently handled inline in the AST -> codegen path.  In
0.3, a HIR layer should absorb them so that semantic analysis and codegen
operate on a cleaner, lower-level-but-still-typed representation.

### 3.1 AST used directly by both semantic analysis and codegen

The AST (`ast.rs`) is consumed directly by `semantic.rs` and then by
`codegen/context.rs`.  There is no intermediate representation.  In 0.3:

- HIR should resolve string-based names to `Symbol` identifiers.
- HIR should attach resolved `Type` to every expression.
- HIR should flatten the AST into a form more amenable to borrow checking
  and optimization.

### 3.2 Function parameters as Vec<(String, Type)>

`Function.params` is `Vec<(String, Type)>` -- plain tuples with no span,
no pattern, no mutability flag.  HIR should introduce a `Param` struct
with a span, a `Symbol` name, a resolved `Type`, and a mutability flag.

### 3.3 Let-statement mutability

`Stmt::Let { name, mutable, ty, value, span }` tracks mutability via a `bool`.
In HIR, this becomes a `LetStmt` with a `Mutability` enum and a typed
initializer expression.

### 3.4 Control-flow expressions (If, For, While)

`Expr::If`, `Expr::For`, and `Expr::While` carry nested AST directly.  In
HIR, these should be lowered to typed control-flow nodes so that borrow
checking and optimization can reason about them before MIR.

### 3.5 Name resolution in Scope

The `Scope` struct performs ad-hoc name resolution during `check_expr` /
`check_stmt`.  In 0.3, HIR construction should own name resolution as a
dedicated pass, producing resolved identifiers.

### 3.6 Error collection

The parser returns the **first** parse error only (subsequent errors are
summarized as "(plus N more error(s))").  HIR construction can collect all
errors and report them together.

---

## 4. SHOULD MOVE INTO FUTURE MIR

These concerns are currently handled directly in LLVM IR generation.  In 0.3,
a MIR layer should own them so that optimizations happen on a typed,
target-independent representation before LLVM sees it.

### 4.1 Alloca-based mutable variables

The codegen's `Variable.alloca` (store/load for mutables, pure SSA for
immutables) is the simplest correct strategy but is expressed as an ad-hoc
decision in the codegen visitor.  MIR should make the memory-vs-SSA decision
explicit and uniform, with a memory-promotion pass that can hoist allocas
back to SSA when safe.

### 4.2 Basic-block structure for loops and conditionals

The for-loop, while-loop, and if/elif/else codegen all manually create
basic blocks (`cond_bb`, `body_bb`, `end_bb`, etc.) and emit branches.
This is MIR-level control flow expressed inline in the codegen visitor.
MIR should own the CFG structure, with a single pass lowering MIR to LLVM.

### 4.3 Type coercions (int-to-bool for conditions)

The if/while codegen converts the condition via `build_int_cast` from the
condition's integer type to `i1`.  In MIR, this coercion should be an
explicit `Coerce` node with a typed representation.

### 4.4 Default return values

When a function body has no explicit `return`, the codegen emits a default
return (`0` for `i64`, `0.0` for `f64`, `void` for unit).  This is a
MIR-level implicit-return concern, not a codegen detail.

### 4.5 `println_i64` declaration

The `declare_builtin_functions` call in `CodeGenContext` hardcodes the
`println_i64` external function.  In MIR, this should be a typed intrinsic
or runtime call resolved during MIR lowering.

### 4.6 Stmt::Expr value discarding

The `Stmt::Expr(e, _)` codegen discards the tail expression value.  In MIR,
the tail expression of a block becomes the block's value (enabling implicit
returns as a MIR-level feature).

### 4.7 Optimization pass strings

The `run_passes("default<O3>", ...)` call is LLVM-specific.  MIR should
have its own optimization pipeline, with LLVM optimizations running as a
backend pass on the lowered MIR.
