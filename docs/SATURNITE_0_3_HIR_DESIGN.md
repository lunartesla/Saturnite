# Saturnite 0.3 HIR Design

## Status: Phase 1 Implementation — Complete

This document records the findings of a comprehensive audit of the Saturnite 0.2
codebase (`crates/stnx/`) and proposes the HIR layer design for 0.3.

The existing forward-looking review lives in
[`SATURNITE_0_3_ARCHITECTURE_REVIEW.md`](SATURNITE_0_3_ARCHITECTURE_REVIEW.md).
This document is ground-truth from reading the actual code; the architecture
review informed the *direction* but is not a substitute for reading the source.

---

## Executive Summary

| Pipeline stage | 0.2 module | 0.3 target |
|---|---|---|
| Source → tokens | `lexer/mod.rs`, `lexer/token.rs` | unchanged |
| Tokens → AST | `parser/mod.rs` (chumsky 0.13) | unchanged |
| AST → semantic check | `semantic.rs` (interleaved) | **extract into HIR** |
| Semantic check → LLVM | `codegen/context.rs` (direct AST walk) | **HIR → MIR → LLVM** |

**Core problem in 0.2:** the AST is the single intermediate representation
shared by both semantic analysis and LLVM codegen.  There is no stage that
produces resolved, typed identifiers.  The codegen re-derives everything
from raw AST string names.  HIR's job is to be the **single authoritative
semantic representation** — the single source of truth that codegen reads
instead of re-resolving.

---

## 1. What information is currently stored in the AST

**File:** `crates/stnx/src/ast.rs` (134 lines)

### Types

```rust
pub enum Type { I64, F64, Bool, Str, Unit }
```
- `Clone, Debug, PartialEq` — no `Eq`, no spans, no type parameters, no generics.
- `Str` and `Unit` are both lowered to `i64` pointers at the LLVM level
  (see `type_to_llvm` in `context.rs`).

### Program

```rust
pub struct Program { pub functions: Vec<Function> }
```

### Function

```rust
pub struct Function {
    pub name: String,                     // plain String — no Symbol/interning
    pub params: Vec<(String, Type)>,      // no span, no mutability flag, no pattern
    pub return_type: Type,                // already resolved by parser
    pub body: Vec<Stmt>,
    pub span: Range<usize>,              // function name span only
}
```

### Statements

```rust
pub enum Stmt {
    Let {
        name: String,
        mutable: bool,
        ty: Option<Type>,               // user-declared annotation; None → inferred
        value: Expr,
        span: Range<usize>,             // union of name span + value span (computed in parser)
    },
    Expr(Expr, Range<usize>),           // bare expression statement
    Return(Option<Expr>, Range<usize>),
    Println(Expr, Range<usize>),
}
```

### Expressions

```rust
pub enum Expr {
    Integer(i64, Range<usize>),
    Float(f64, Range<usize>),
    StrLit(String, Range<usize>),
    Bool(bool, Range<usize>),
    Unit(Range<usize>),
    Var(String, Range<usize>),          // unresolved identifier — just a String
    Assign { target: String, value: Box<Expr>, span: Range<usize> },
    AugAssign { target: String, op: AugOp, value: Box<Expr>, span: Range<usize> },
    Binary { op: BinOp, lhs: Box<Expr>, rhs: Box<Expr>, span: Range<usize> },
    Unary { op: UnOp, expr: Box<Expr>, span: Range<usize> },
    Call { func: String, args: Vec<Expr>, span: Range<usize> },
    If {
        condition: Box<Expr>,
        then_branch: Vec<Stmt>,
        elif_branches: Vec<(Expr, Vec<Stmt>)>,
        else_branch: Option<Vec<Stmt>>,
        span: Range<usize>,
    },
    For {
        var: String,                    // loop variable (always immutable, always I64)
        iter: Box<Expr>,                // expected to be a Range
        body: Vec<Stmt>,
        span: Range<usize>,
    },
    While {
        condition: Box<Expr>,
        body: Vec<Stmt>,
        span: Range<usize>,
    },
    Range {
        start: Box<Expr>,
        end: Box<Expr>,
        is_inclusive: bool,             // true for `...`, false for `..`
        span: Range<usize>,
    },
}
```

### Operator enums

```rust
pub enum BinOp { Add, Sub, Mul, Div, Mod, Eq, Ne, Lt, Gt, Le, Ge, And, Or }
pub enum UnOp { Neg, Not }
pub enum AugOp { Add, Sub, Mul, Div }
```

### What is NOT stored in the AST

- **No resolved type on expressions.**  `Var("x", span)` carries no type;
  the semantic pass looks it up.  `Binary { op, lhs, rhs, .. }` carries no
  resolved operand or result type.
- **No resolved identifier.**  Variable references are bare `String`s.
  Function calls are bare `String`s.  The codegen re-resolves them.
- **No mutability on params.**  `Vec<(String, Type)>` — params are always
  treated as immutable by both semantic analysis and codegen.
- **No source span on param names.**  Param spans are discarded in the parser.
- **No call-site parameter types.**  `Call { func, args }` doesn't store
  what parameter types the resolved function expects.

---

## 2. What information is currently stored in semantic ScopeTable

**File:** `crates/stnx/src/semantic.rs` (370 lines)

### Scope struct

```rust
pub struct Scope {
    variables: HashMap<String, (Type, bool)>,       // name → (type, mutable)
    functions: HashMap<String, (Vec<Type>, Type)>,  // name → (param_types, return_type)
    parent: Option<Box<Scope>>,                     // lexical parent
}
```

### What is stored

| Key | Value type | Contents |
|---|---|---|
| Variable name (String) | `(Type, bool)` | Resolved type + mutability flag |
| Function name (String) | `(Vec<Type>, Type)` | Parameter types + return type |
| Parent scope | `Option<Box<Scope>>` | Chain for lexical lookup |

### Key observations

- **No spans on symbols.**  `define_variable` and `define_function` take
  `&str` name and don't record where the declaration occurred.  When
  `lookup_variable` returns `None`, the error message has no span.
- **No param names stored.**  `functions: HashMap<String, (Vec<Type>, Type)>`
  stores only the *types* of parameters, not their names.  The semantic
  checker discards param names at function registration time (line 77:
  `func.params.iter().map(|(_, t)| t.clone()).collect()`).
- **Full scope cloning.**  `analyze()` does
  `Scope::with_parent(global_scope.clone())` for each function (line 89).
  The entire `HashMap` of all function signatures is cloned into every
  function scope.  This is wasteful.
- **No symbol interning.**  Lookup keys are `String` — every lookup
  allocates/hashes.
- **Builtin `println` hardcoded.**  Line 74:
  `global_scope.define_function("println", vec![Type::I64], Type::Unit)`.
  This is a stringly-typed builtin registration with no concept of
  intrinsics or typed builtins.
- **Scope is consumed, not transformed.**  `analyze()` returns
  `CompilerResult<()>` — it validates but produces no output IR.  The
  semantic information (resolved types, mutability, function signatures)
  is thrown away after the function returns.  The codegen starts from
  scratch with the raw AST.

### Scope lookup methods

- `lookup_variable(name) -> Option<Type>`: walks parent chain, returns
  the variable's resolved type.
- `lookup_variable_mutability(name) -> Option<bool>`: separate walk for
  just the mutability flag — could have been combined with type lookup.
- `lookup_function(name) -> Option<(Vec<Type>, Type)>`: walks parent chain
    for function signatures.

---

## 3. Where identifier resolution occurs

Identifier resolution happens in **two places**, with the second duplicating
the first:

### Primary: `semantic.rs` — `check_expr` / `check_stmt`

| AST node | Resolution call | What it resolves |
|---|---|---|
| `Expr::Var(name, _)` | `scope.lookup_variable(name)` | Variable → Type |
| `Expr::Call { func, .. }` | `scope.lookup_function(func)` | Function → (param_types, return_type) |
| `Expr::Assign { target, .. }` | `scope.lookup_variable(target)` + `scope.lookup_variable_mutability(target)` | Assignment target → type + mutability |
| `Expr::AugAssign { target, .. }` | Same as Assign | |
| `Stmt::Let { name, .. }` | `scope.define_variable(name, resolved, *mutable)` | Defines new variable in current scope |

### Secondary: `codegen/context.rs` — `gen_expr` / `gen_stmt`

The codegen **re-resolves** every identifier because it does not have the
semantic scope:

| AST node | Resolution call | What it resolves |
|---|---|---|
| `Expr::Var(name, _)` | `scope.variables.get(name)` (FunctionScope) | Variable → `Variable<'ctx>` (SSA value + alloca) |
| `Expr::Assign { target, .. }` | `scope.variables.get_mut(target)` | Same, mutable |
| `Expr::AugAssign { target, .. }` | `scope.variables.get(target).cloned()` | Same |
| `Expr::Call { func, .. }` | `self.module.get_function(func)` | Function → `FunctionValue` (LLVM) |

**Critical insight:** The codegen's `FunctionScope`
(`HashMap<String, Variable<'ctx>>`) is a *completely separate* scope table
from the semantic `Scope`.  It stores LLVM values (`BasicValueEnum`,
`PointerValue`) instead of types.  The semantic type information is **not
passed to or reused by** the codegen.

---

## 4. Where type checking occurs

**File:** `crates/stnx/src/semantic.rs`, functions `check_expr` (lines
110–309) and `check_stmt` (lines 311–370).

### Type inference in `check_expr` → returns `CompilerResult<Type>`

| AST node | Type-checking rule |
|---|---|
| `Integer` | Always `Type::I64` |
| `Float` | Always `Type::F64` |
| `StrLit` | Always `Type::Str` |
| `Bool` | Always `Type::Bool` |
| `Unit` | Always `Type::Unit` |
| `Var` | Type from scope lookup (no inference needed) |
| `Binary { op: Add/Sub/Mul/Div/Mod }` | Both operands must have same type; result type = operand type |
| `Binary { op: Eq/Ne/Lt/Gt/Le/Ge }` | Both operands type-checked; result is `Type::Bool` |
| `Binary { op: And/Or }` | Both operands type-checked; result is `Type::Bool` |
| `Unary { op: Neg }` | Operand must be `I64` or `F64`; result type = operand type |
| `Unary { op: Not }` | Result is `Type::Bool` (operand type not checked) |
| `Call { func, args }` | Special case: `println` requires all args to be `I64`, returns `Unit`. Other functions: arg types must match param types, return type = function's return type. |
| `If` | Condition must be `Bool`. Result type is `Unit` (no type unification across branches). |
| `For` | Iter expression type-checked (no constraint). Loop var defined as `I64` (hardcoded). Result is `Unit`. |
| `While` | Condition must be `Bool`. Result is `Unit`. |
| `Range` | Start and end must be `I64`. Result is `I64`. |

### Type checking in `check_stmt`

| AST node | Type-checking rule |
|---|---|
| `Stmt::Let` | If explicit type annotation present, it must match inferred type of value. Variable registered with resolved type. |
| `Stmt::Expr` | Expression type-checked (return value discarded). |
| `Stmt::Return` | If `return e`, `e`'s type must match `return_type`. If `return` (no expr), `return_type` must be `Unit`. |
| `Stmt::Println` | Argument expression must be `I64`. |

### What type checking is NOT done

- **No type inference across branches.**  `if cond { 1i64 } else { 2i64 }`
  returns `Unit`, not `I64`.  The `if` expression's type is hardcoded to
  `Unit` regardless of branch types.
- **No type checking for `Not` operand.**  `!42` is not caught.
- **No integer/float width checking.**  No overflow or precision concerns.

---

## 5. Where mutability is checked

Mutability is enforced in **two places**:

### Primary: `semantic.rs` — `check_expr`

**`Expr::Assign { target, .. }`** (lines ~230–248):
1. Looks up variable type via `scope.lookup_variable(target)`.
2. Checks `!scope.lookup_variable_mutability(target)` → error
   "cannot assign to immutable variable: {target}".

**`Expr::AugAssign { target, op, value, .. }`** (lines ~270–290):
1. Same `lookup_variable_mutability` check.
2. Type mismatch between variable and value checked.

**`Stmt::Let`** (lines ~318–336):
- Defines variable with the `mutable` flag from the AST.
- The `mutable` field is a `bool` parsed from `kw("mut")` in the parser.

### Secondary: `codegen/context.rs` — `gen_stmt` / `gen_expr`

**`Stmt::Let`** (lines ~106–128):
- If `*mutable` → `build_alloca` + `build_store` (stack slot).
- If not mutable → `insert_immutable` (pure SSA).

**`Expr::Assign`** (lines ~341–364):
- If var has `alloca` → `build_store` to alloca slot.
- If var has no `alloca` (immutable) → just updates `ssa_value` in
  FunctionScope.  **This silently allows mutation of immutable variables
  at the codegen level** — it relies on the semantic check having already
  rejected this.

**`Expr::AugAssign`** (lines ~366–420):
- Same alloca-vs-SSA logic as Assign.

### Key observation

The codegen **re-reads the mutable flag** from the AST but stores allocas
in its own `FunctionScope`.  The semantic scope's mutability info is not
reused — it's re-derived from the `Stmt::Let { mutable: bool }` AST field.

---

## 6. Where function signatures are resolved

### Registration: `semantic.rs` — `analyze()` (lines 70–97)

1. `global_scope.define_function("println", vec![Type::I64], Type::Unit)` —
   hardcoded builtin registration (line 74).
2. For each function in `program.functions`:
   `global_scope.define_function(&func.name, param_types, func.return_type.clone())`
   where `param_types` is extracted from
   `func.params.iter().map(|(_, t)| t.clone()).collect()`.
   **Param names are discarded** (line 77).
3. Checks `global_scope.functions.contains_key("main")` — error if no main.

### Call-site checking: `semantic.rs` — `check_expr(Expr::Call)` (lines ~170–200)

1. Special case for `println`: checks all args are `I64`.
2. For other functions: `scope.lookup_function(func)` →
   `(param_types, ret_type)`.  Checks `args.len() == param_types.len()`.
   Checks each arg type against param type.

### Codegen declaration: `codegen/context.rs` — `declare_function` (lines 47–57)

```rust
pub fn declare_function(&mut self, func: &Function) -> CompilerResult<()> {
    let ret_basic = type_to_llvm(self.context, &func.return_type);
    let param_types: Vec<_> = func.params.iter()
        .map(|(_, t)| type_to_llvm(self.context, t).as_basic_type_enum().into())
        .collect();
    let fn_type = ret_basic.as_basic_type_enum().fn_type(&param_types, false);
    self.module.add_function(&func.name, fn_type, None);
    Ok(())
}
```

Re-reads `func.params` and `func.return_type` from the AST.  Calls
`type_to_llvm` to convert each `Type` to `BasicTypeEnum`.  **No caching**.

### Codegen body generation: `generate_function` (lines 59–91)

1. `self.module.get_function(&func.name)` — re-resolves function name to
   LLVM `FunctionValue` (duplicates semantic registration).
2. Maps `func.params` to LLVM parameters via `function_value.get_nth_param(i)`.
3. Inserts each param as an immutable variable in `FunctionScope`.
4. Emits default return at end if no explicit return.

### What is NOT in function signatures

- No param mutability (params are always immutable).
- No param spans (no error reporting at param site).

---

## 7. Where control-flow validation occurs

### Semantic validation: `semantic.rs` — `check_expr`

**`Expr::If`** (lines ~200–235):
- Condition must be `Type::Bool` → error "if condition must be bool".
- `then_branch`, `elif_branches`, `else_branch` statements are all checked
  via `check_stmt`.
- Returns `Type::Unit` (no type unification across branches).
- **No dead-code or reachability analysis.**

**`Expr::For`** (lines ~238–255):
- Iterator expression is type-checked (no type constraint).
- Loop variable `var` is defined as `Type::I64`, `mutable: false`.
- Body statements checked in a child scope.
- Returns `Type::Unit`.
- **No constraint that `iter` is a `Range`.**  This is checked only at
  codegen time (context.rs line ~590: "for loop requires a range expression").

**`Expr::While`** (lines ~257–269):
- Condition must be `Bool`.
- Body checked in a child scope.
- Returns `Type::Unit`.

### Semantic validation: `check_stmt`

**`Stmt::Return`** (lines ~340–357):
- Return expression type must match `return_type`.
- No early-return or unreachable-code analysis.

### Codegen: `codegen/context.rs` — basic block creation

The codegen manually creates basic blocks for all control-flow expressions:

**`Expr::If`** (lines ~470–560):
- Creates `then_bb`, `else_bb`, `end_bb` (or `elifN_cond`, `elifN_body`
  blocks for elif branches).
- Emits `build_conditional_branch`.
- `build_int_cast` condition from int type to `i1`.

**`Expr::For`** (lines ~570–655):
- Creates `for_cond`, `for_body`, `for_end` blocks.
- Uses `alloca` for loop variable.
- Re-derives start/end by re-matching `iter` as `Range` expression.
- Uses `IntPredicate::SLT` or `IntPredicate::SLE` for bounds comparison.

**`Expr::While`** (lines ~660–718):
- Creates `while_cond`, `while_body`, `while_end` blocks.
- `build_int_cast` condition to `i1`.
- `build_conditional_branch`.

### Control-flow validation NOT done

- **No break/continue.**  No `break` or `continue` keywords exist.
- **No unreachable code detection.**
- **No fall-through analysis.**
- **No divergent-branch tracking.**

---

## 8. Which semantic information LLVM codegen currently recomputes

The codegen in `codegen/context.rs` is a **direct AST-to-LLVM visitor**
with no intermediate representation carrying semantic facts.  Everything is
re-derived from the AST:

### Re-derived: function signatures
- `declare_function(func)`: calls `type_to_llvm(self.context, &func.return_type)`
  and maps over `func.params` calling `type_to_llvm` for each param type.
  Converts `ast::Type` → `inkwell::types::BasicTypeEnum` from scratch.
  No caching of the Type→LLVM-Type mapping.
- `generate_function(func)`: re-reads `func.params` to bind LLVM
  `FunctionValue` parameters to names, re-deriving the param count and types.

### Re-derived: variable resolution & types
- `FunctionScope` in `codegen/context.rs` is a separate
  `HashMap<String, Variable<'ctx>>` built from scratch during codegen.
  It stores LLVM values (`BasicValueEnum`, `PointerValue`), not types.
- `gen_expr(Expr::Var)`: looks up `scope.variables.get(name)` — resolves
  variable name to its LLVM value, duplicating the semantic
  `scope.lookup_variable(name)` that already found the same name's type.
- The semantic `Scope` (which knows the type) is **not available** during
  codegen.  Only the `FunctionScope` (which knows the LLVM value) exists.
- When `gen_expr(Expr::Var)` loads from an alloca, it uses
  `var.ssa_value.get_type()` to figure out the LLVM type — re-deriving
  what the semantic check already knew.

### Re-derived: mutability
- `gen_stmt(Stmt::Let)`: checks `*mutable` from the AST to decide
  `build_alloca` vs `insert_immutable`.  The semantic `Scope` already
  stored the mutability flag; the codegen re-reads it from the AST.
- `gen_expr(Expr::Assign)`: re-checks whether the variable has an `alloca`
  to decide `build_store` vs `ssa_value` update.

### Re-derived: function call resolution
- `gen_expr(Expr::Call)`: calls `self.module.get_function(func)` to
  re-resolve the function name to an `FunctionValue`.  The semantic
  `scope.lookup_function(func)` already did this resolution but the
  result was discarded.

### Re-derived: default return values
- In `generate_function` (lines 84–107), if no block terminator exists,
  the codegen re-matches on `func.return_type` to emit:
  - `Type::I64` → `ret i64 0`
  - `Type::F64` → `ret double 0.0`
  - `Type::Bool` → `ret i1 0`
  - `Type::Unit` → `ret void`
  - `Type::Str` → `ret i64 0`
  This is a semantic concern (what is a sensible default?) re-delegated to
  codegen.

### Re-derived: builtin declarations
- `declare_builtin_functions()` hardcodes `println_i64` as
  `fn(i64) -> i64` — the semantic analyzer registered `println` as
  `fn(i64) -> Unit`.  The codegen adds a different return type (I64 vs Unit)
  and a different name (`println_i64` vs `println`).
  The `Stmt::Println` codegen calls `self.module.get_function("println_i64")`
  — the call-to-declaration name mismatch is "papered over" by the parser
    lowering `println(...)` to `Stmt::Println` rather than `Expr::Call`.

---

## 9. Which AST nodes are directly consumed by LLVM codegen

**Every single AST node** is consumed directly by the codegen in
`codegen/context.rs`.  There is no intermediate representation.

### `Program` and `Function`

| Consumer | How |
|---|---|
| `CodeGenerator::emit` / `generate_ir_string` | Iterates `program.functions` |
| `CodeGenerator::declare_function(func)` | Reads `func.name`, `func.return_type`, `func.params` |
| `CodeGenerator::generate_function(func)` | Reads `func.name`, `func.params`, `func.return_type`, `func.body` |

### `Stmt` nodes — consumed in `gen_stmt`

| AST Stmt variant | Codegen action |
|---|---|
| `Stmt::Let { name, mutable, value, .. }` | `gen_expr(value)`; if mutable → `build_alloca`+`build_store`; else → `insert_immutable` |
| `Stmt::Expr(e, _)` | `gen_expr(e)` — result discarded |
| `Stmt::Return(opt, _)` | If Some(e): `gen_expr(e)` + `build_return(Some(val))`; else: `build_return(None)` |
| `Stmt::Println(e, _)` | `gen_expr(e)` + `build_call` to `println_i64` |

### `Expr` nodes — consumed in `gen_expr`

| AST Expr variant | Codegen action |
|---|---|
| `Expr::Integer(n, _)` | `i64_type().const_int(n, true)` |
| `Expr::Float(f, _)` | `f64_type().const_float(f)` |
| `Expr::StrLit(s, _)` | `build_global_string_ptr` + `build_ptr_to_int` (Str → i64 ptr) |
| `Expr::Bool(b, _)` | `bool_type().const_int(b, false)` |
| `Expr::Unit(_)` | `i64_type().const_zero()` (unit lowers to i64 0!) |
| `Expr::Var(name, _)` | `scope.variables.get(name)` — if alloca, `build_load`; else return `ssa_value` |
| `Expr::Assign { target, value, .. }` | `gen_expr(value)`; if alloca → `build_store`; else update `ssa_value` |
| `Expr::AugAssign { target, op, value, .. }` | Load current, compute op, store back, update `ssa_value` |
| `Expr::Binary { op, lhs, rhs, .. }` | `gen_expr(lhs)` + `gen_expr(rhs)`; match op to `build_int_add`/etc. or `build_int_compare`/`build_and`/`build_or` |
| `Expr::Unary { op, expr, .. }` | `gen_expr(expr)`; match op to `build_int_neg`/`build_not` |
| `Expr::Call { func, args, .. }` | `module.get_function(func)`; gen each arg; `build_call` |
| `Expr::If { .. }` | Creates basic blocks; `build_int_cast` condition to i1; `build_conditional_branch`; gen each branch's stmts |
| `Expr::For { var, iter, body, .. }` | Re-parses `iter` as `Range`; creates basic blocks; alloca loop var; SLT/SLE comparison |
| `Expr::While { condition, body, .. }` | Creates basic blocks; `build_int_cast` condition to i1; `build_conditional_branch`; gen body stmts |
| `Expr::Range { start, end, .. }` | `gen_expr(start)`; `gen_expr(end)` — **end value is discarded** (`let _ = end_val`)! Returns just `start_val`. |

### Key observation

The codegen walks the **raw AST** with string-based name lookups.  It does
**not** consult the semantic `Scope` at all — it builds its own
`FunctionScope` of LLVM values.  The only contract between semantic
analysis and codegen is that `analyze()` returns `Ok(())`.

---

## 10. Which existing types can be reused by HIR

### Directly reusable types

| Type | File | Reused as | Notes |
|---|---|---|---|
| `ast::Type` | `ast.rs` | HIR expression types, HIR param types, HIR function return types | `Clone, Debug, PartialEq`.  Perfect as-is. |
| `ast::BinOp` | `ast.rs` | HIR binary operator | `Clone, Copy, Debug, PartialEq`.  Perfect as-is. |
| `ast::UnOp` | `ast.rs` | HIR unary operator | `Clone, Copy, Debug, PartialEq`.  Perfect as-is. |
| `ast::AugOp` | `ast.rs` | HIR augmented assignment operator | `Clone, Copy, Debug, PartialEq`.  Perfect as-is. |
| `error::CompilerError` | `error.rs` | HIR construction errors | Has `Semantic(String)`, `Type(String)` variants.  Needs span-bearing variants (see §H.1). |
| `error::CompilerResult<T>` | `error.rs` | HIR construction result type | `Result<T, CompilerError>`.  Perfect as-is. |

### Concepts to adapt (not reuse verbatim)

| Concept | File | Adaptation needed |
|---|---|---|
| `Scope` | `semantic.rs` | Replace `String` keys with `Symbol` (interned).  Store `DefId` for each binding.  Add span tracking.  The struct shape (HashMap + parent) is sound. |
| `Variable<'ctx>` | `codegen/context.rs` | Currently tied to `inkwell::values::BasicValueEnum` and `PointerValue`.  HIR needs a target-independent version.  MIR would reintroduce the LLVM-tied version. |
| `FunctionScope<'ctx>` | `codegen/context.rs` | Currently `HashMap<String, Variable<'ctx>>`.  HIR needs `HashMap<Symbol, LocalDef>` with type and mutability info, not LLVM values. |

### Types that CANNOT be reused (must be new)

- **`ast::Expr` and `ast::Stmt`**: HIR needs new versions with resolved
  identifiers (Symbol/DefId), attached types, and flattened control flow.
  The AST uses bare `String` for names; HIR must use `Symbol`.
- **`ast::Function`**: HIR version needs a `DefId`, resolved param types
  with names and spans, and a HIR body (not raw AST).
- **`ast::Program`**: HIR version needs function signatures table,
  interning arena for symbols, and resolved function bodies.

---

## Proposed HIR Design

Based on the audit above, HIR should be a new module (`crates/stnx/src/hir.rs`)
that sits between the parser and the codegen.  The HIR lowering pass is the
**single authoritative semantic representation**.  The codegen will consume
HIR instead of the raw AST.

### H.1 Module structure

```
src/
├── ast.rs          (unchanged — parser output only)
├── hir.rs          (NEW — HIR types + lowering from AST)
├── semantic.rs     (MODIFIED — thin wrapper around hir::lower)
├── codegen/        (MODIFIED — consumes HIR, not AST directly)
├── lexer/          (unchanged)
├── parser/         (unchanged)
├── target.rs       (unchanged)
├── error.rs        (extended — span-bearing semantic errors)
└── ...
```

### H.2 Symbol interning

Introduce a `Symbol` type backed by an interner arena:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Symbol(pub u32);

pub struct SymbolTable {
    strings: Vec<String>,
    map: HashMap<String, Symbol>,
}

impl SymbolTable {
    pub fn intern(&mut self, s: &str) -> Symbol { ... }
    pub fn resolve(&self, sym: Symbol) -> &str { ... }
}
```

The `SymbolTable` lives in `HirProgram` and is shared across all HIR nodes.
This eliminates the stringly-typed lookups in both semantic and codegen.

### H.3 Definition IDs

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DefId(pub u32);
```

Each variable, function parameter, and function definition gets a unique
`DefId`.  This allows HIR to resolve `Var(String)` → `Var(Symbol, DefId)`,
removing all ambiguity.

### H.4 HIR node types

Reuse `ast::Type`, `ast::BinOp`, `ast::UnOp`, `ast::AugOp` directly.

```rust
pub struct HirProgram {
    pub symbols: SymbolTable,
    pub functions: Vec<HirFunction>,
}

pub struct HirFunction {
    pub def_id: DefId,
    pub name: Symbol,
    pub name_span: Range<usize>,
    pub params: Vec<HirParam>,
    pub return_type: Type,
    pub body: Vec<HirStmt>,
}

pub struct HirParam {
    pub name: Symbol,
    pub span: Range<usize>,
    pub ty: Type,
    pub mutable: bool,
}

pub enum HirStmt {
    Let { pattern: LocalDef, mutable: bool, explicit_ty: Option<Type>,
          value: HirExpr, span: Range<usize> },
    Expr(HirExpr, Range<usize>),
    Return(Option<HirExpr>, Range<usize>),
    Println(HirExpr, Range<usize>),
}

pub struct LocalDef {
    pub def_id: DefId,
    pub name: Symbol,
    pub ty: Type,
    pub mutable: bool,
    pub span: Range<usize>,
}

pub enum HirExpr {
    Literal { value: LiteralValue, ty: Type, span: Range<usize> },
    Var { symbol: Symbol, def_id: DefId, ty: Type, span: Range<usize> },
    Assign { target: DefId, value: Box<HirExpr>, span: Range<usize> },
    AugAssign { target: DefId, op: AugOp, value: Box<HirExpr>,
                span: Range<usize> },
    Binary { op: BinOp, lhs: Box<HirExpr>, rhs: Box<HirExpr>,
             ty: Type, span: Range<usize> },
    Unary { op: UnOp, expr: Box<HirExpr>, ty: Type, span: Range<usize> },
    Call { func: Symbol, def_id: DefId, args: Vec<HirExpr>,
           ty: Type, span: Range<usize> },
    If { condition: Box<HirExpr>, then_branch: Vec<HirStmt>,
         elif_branches: Vec<(HirExpr, Vec<HirStmt>)>,
         else_branch: Option<Vec<HirStmt>>, span: Range<usize> },
    For { var: LocalDef, iter: Box<HirExpr>, body: Vec<HirStmt>,
          span: Range<usize> },
    While { condition: Box<HirExpr>, body: Vec<HirStmt>,
            span: Range<usize> },
    Range { start: Box<HirExpr>, end: Box<HirExpr>,
            is_inclusive: bool, span: Range<usize> },
}

pub enum LiteralValue { I64(i64), F64(f64), Str(String), Bool(bool), Unit }
```

Key differences from AST:
- **String → Symbol:** all name fields use `Symbol` (interned), not `String`.
- **DefId on references:** `Var` and `Call` carry `DefId` — no name lookup
  needed during codegen.
- **Type on every expression:** every `HirExpr` has a `ty: Type` field —
  populated during lowering, read directly by codegen.
- **LocalDef for bindings:** `Let` carries a `LocalDef` with span, type,
  and mutability — replacing bare `name: String` + `mutable: bool`.
- **Params carry spans:** `HirParam` has a span, replacing `Vec<(String,Type)>`.

### H.5 HIR construction (lowering from AST)

The `hir::lower(program: &ast::Program)` function walks the AST once,
performing:

1. **Name resolution** — all `Var(String)` → `Var { symbol, def_id, ty }`,
   all `Call { func: String }` → `Call { func: Symbol, def_id, ty }`.
   This is the **only** place name resolution happens.  The codegen
   will not re-resolve names.

2. **Type inference** — every `HirExpr` gets a `ty: Type` field.  The
   type-checking logic is moved here (from `semantic.rs`).  The codegen
   will read `expr.ty` instead of calling `type_to_llvm` on a bare node.

3. **Scope management** — child scopes are created for blocks, loop bodies,
   and if branches.  Definitions are registered with `DefId`s in a
   `HashMap<Symbol, LocalDef>` (arena-allocated, no cloning).

4. **Error collection** — unlike 0.2's `analyze()` which returns on the
   first error, HIR construction collects all errors and reports them
   together (as suggested in §3.6 of the architecture review).  Errors
   carry spans (fixing §2.1 of the architecture review).

### H.6 What HIR eliminates

| 0.2 problem | HIR solution |
|---|---|
| Codegen re-resolves variable names | HIR stores `DefId` on every `Var` — no lookup needed |
| Codegen re-resolves function names | HIR stores `DefId` on every `Call` |
| Codegen re-derives `Type` → LLVM via `type_to_llvm` | HIR stores `ty: Type` on every expression |
| `semantic.rs` `Scope` and codegen `FunctionScope` are separate tables | HIR is the single table |
| `Stmt::Let { name: String, mutable: bool }` no span | `Let { pattern: LocalDef }` with span, type, mutability |
| `Function.params: Vec<(String, Type)>` no span | `HirParam { name, span, ty, mutable }` |
| `println` hardcoded as string in codegen | HIR registers `println` as an intrinsic with a `DefId` |
| `Scope::with_parent(global_scope.clone())` clones entire scope | HIR uses `DefId` and arena storage — no cloning |

### H.7 What codegen must change

The `codegen/context.rs` will be modified to consume HIR:

- `generate_ir(hir_program: &HirProgram)` instead of `&Program`
- `gen_expr(expr: &HirExpr)` — reads `expr.ty` and `expr.def_id` directly
- `gen_stmt(stmt: &HirStmt)` — reads resolved types and names from HIR
- `type_to_llvm` becomes a simple cached lookup (HIR already has the
  resolved `Type` on every expression)

### H.8 Backward compatibility strategy

- `lib.rs` should export the new `hir` module alongside `ast`.
- `semantic::analyze` becomes a thin wrapper: it calls `hir::lower(&program)`
  and returns `Ok(())` if no errors, preserving the existing
  `CompilerResult<()>` signature for callers that don't need the HIR tree.
- The codegen functions that accept `&Program` will be updated to accept
  `&HirProgram`.  The test helper `compile_src` in `tests/common/mod.rs`
  will insert the HIR-lowering step between `analyze` and `codegen`.
- AST types (`ast::Type`, `ast::BinOp`, etc.) remain unchanged — they are
  parsed and then lowered; no callers outside the parser need them after
  HIR construction.

### H.9 Phase ordering for 0.3

1. **Phase 1:** Add `hir.rs` with types and `lower()` function.  Keep
   `semantic.rs` as a wrapper.  Tests still pass with AST→semantic→codegen
   path (HIR not yet wired to codegen).
2. **Phase 2:** Update `codegen` to accept HIR.  Add `hir::lower` call to
   the pipeline in `main.rs` and test helpers.  Verify all existing tests
   pass.
3. **Phase 3 (future):** Add MIR between HIR and LLVM codegen.

---

## Audit Summary Table

| Q# | Question | Answer (one sentence) |
|---|---|---|
| 1 | AST info stored | `Type` enum, `Program` (Vec\<Function\>), `Function` (name:String, params:Vec\<(String,Type)>, return_type, body, span), `Stmt` (Let/Expr/Return/Println), `Expr` (16 variants with String identifiers and Range\<usize\> spans), `BinOp`/`UnOp`/`AugOp` — no resolved types or DefIds on any node |
| 2 | ScopeTable info | `Scope` with `variables: HashMap<String,(Type,bool)>`, `functions: HashMap<String,(Vec<Type>,Type)>`, `parent: Option<Box<Scope>>` — no spans, no param names, full clone per function |
| 3 | Identifier resolution | Semantic: `check_expr`/`check_stmt` via `lookup_variable`/`lookup_function`; Codegen: re-resolves in `gen_expr`/`gen_stmt` via `FunctionScope.get` and `module.get_function` |
| 4 | Type checking | Entirely in `semantic.rs` `check_expr` (returns `Type`) and `check_stmt`; codegen does zero type checking, re-derives via `type_to_llvm` |
| 5 | Mutability checking | Semantic: `lookup_variable_mutability` in Assign/AugAssign; Codegen: re-reads `mutable: bool` from `Stmt::Let` to decide alloca vs SSA |
| 6 | Function signature resolution | Registration in `analyze()` via `define_function`; call-site checking in `check_expr(Call)`; re-derived in `declare_function`/`generate_function` via `type_to_llvm` |
| 7 | Control-flow validation | Semantic: condition must be Bool in If/While; Codegen: manually creates basic blocks with int_cast to i1; no reachability analysis |
| 8 | Codegen recomputes | Function signatures (`type_to_llvm`), variable resolution (`FunctionScope.get`), mutability (alloca vs SSA), function call resolution (`module.get_function`), type→LLVM mapping (no cache), default return values |
| 9 | AST nodes consumed by LLVM | Every AST node: `Program`, `Function`, all 4 `Stmt` variants, all 16 `Expr` variants, plus `BinOp`/`UnOp`/`AugOp` — codegen is a direct AST-to-LLVM visitor |
| 10 | Reusable types | `Type`, `BinOp`, `UnOp`, `AugOp` from `ast.rs`; `CompilerError`/`CompilerResult` from `error.rs`; `Scope` shape (adapt String→Symbol); `Variable` concept (adapt to target-independent form) |
