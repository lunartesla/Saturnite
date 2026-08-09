# Saturnite Compiler Pipeline Audit — Evidence Fragment

Workspace crate under audit: `crates/stnx/` (single crate).
Verification method: full source inspection of `crates/stnx/src/**` via `sed`/`grep`.
All classifications: IMPLEMENTED / PARTIALLY IMPLEMENTED / DESIGN ONLY / STALE /
MISSING / BROKEN / DEAD/UNUSED.

---

## 1. ACTUAL PIPELINE (Section 3)

**CLASSIFICATION: IMPLEMENTED (with architectural gaps).**

The actual source-to-executable pipeline, traced through the real source files:

| Stage | Source File | Primary Types | Entry Function | Input | Output |
|-------|-------------|---------------|----------------|-------|--------|
| 1. Lexing | `lexer/mod.rs` | `LexicalToken`, `Token`, `TokenKind` | `Lexer::new` iterator | `&str` | `Vec<Token>` (each has `span: Range<usize>`) |
| 2. Parsing | `parser/mod.rs` | `Program`, `Function`, `Stmt`, `Expr` | `parse(src, tokens)` | `Vec<Token>` | `ast::Program` |
| 3. Semantic/Lowering | `semantic.rs` → `hir/lower.rs` | `HirProgram`, `SymbolInterner` | `analyze_and_lower` → `lower()` | `&ast::Program` | `HirProgram` |
| 4. Codegen | `codegen/mod.rs` + `codegen/context.rs` | `CodeGenContext`, `FunctionScope`, `Variable` | `CodeGenerator::emit` → `gen_expr`/`gen_stmt` | `&HirProgram` | `Module` (LLVM IR) |
| 5. Object emission | `codegen/emitter.rs` | `ObjectEmitter` | `emit_object(path)` | `Module` | `.o` file |
| 6. Linking | `codegen/linker.rs` | `Linker` | `Linker::link` | `.o` + runtime | Executable |

Pipeline invocation (main.rs:262-276):
```rust
let tokens: Vec<_> = stnx::lexer::Lexer::new(&src).by_ref()
    .collect::<Result<Vec<_>, _>>()?;
let program = stnx::parser::parse(&src, tokens)?;
let hir = stnx::semantic::analyze_and_lower(&program)?;
codegen::compile_with_target(&hir, &output_path, config)?;
```
The `Check` command (main.rs:532) stops at stage 3 (`semantic::analyze`), skipping codegen.

### IS MIR ACTUALLY IMPLEMENTED?

**CLASSIFICATION: MISSING (Design Only).**

There is NO MIR module, file, type, or function in the source. Confirmed by:
- `grep -rn "mir" crates/stnx/src/` → `NO MIR references in source`
- No file named `mir.rs` or directory `mir/` under `crates/stnx/src/`
- The only "MIR" mentions in source are two **doc comments** (not code):
  - `hir/lower.rs:6` — `//! stages (MIR, LLVM codegen) never perform string lookups.`
  - `hir/expr.rs:5` — `//! later stages (MIR, LLVM codegen) never need to perform string lookups.`
- A full MIR design doc exists at `docs/SATURNITE_MIR_DESIGN.md` but is NOT implemented.
- `docs/SATURNITE_FINAL_VERIFICATION.md:124` states: `**MIR layer:** Design documented but not yet implemented.`

### What's Between HIR and LLVM?

**CLASSIFICATION: NOTHING (direct HIR→LLVM).**

No intermediate representation between HIR and LLVM IR. `gen_stmt()` (context.rs:156) and `gen_expr()` (context.rs:207) walk HIR directly, emitting inkwell IR instructions immediately:

- `HirExprKind::If` (context.rs:500-567): constructs BBs inline, positions builder at each, calls `gen_stmt` on body. **The CFG is constructed on-the-fly from the HIR tree.**
- `HirExprKind::For` (context.rs:569-654): allocates `loop_var_ptr: i64`, creates `for_cond`/`for_body`/`for_end` BBs, uses `IntPredicate::ULE`/`ULT` (unsigned).
- `HirExprKind::While` (context.rs:661-708): creates `cond_bb`/`body_bb`/`end_bb`.
- `HirExprKind::Range` (context.rs:709-717): returns only `start`; `end` is discarded (`let _ = end;`).

### Optimization Pipeline

**CLASSIFICATION: IMPLEMENTED (LLVM IR only, not MIR-level).**

`codegen/mod.rs:112-129` runs LLVM IR optimization passes *after* codegen:
```rust
// codegen/mod.rs:112-129
let target_machine = self.target_config.create_target_machine()...?;
let opt_passes = match self.target_config.opt_level() {
    OptimizationLevel::Less => "default<O1>",      // mod.rs:119
    OptimizationLevel::Default => "default<O2>",  // mod.rs:120
    OptimizationLevel::Aggressive => "default<O3>", // mod.rs:121
    _ => "default<O0>",                            // mod.rs:122
};
let options = PassBuilderOptions::create();          // mod.rs:124
ctx.module.run_passes(opt_passes, &target_machine, options) // mod.rs:126
```
`PassBuilderOptions` imported at mod.rs:31. This is **LLVM IR-level**, not MIR-level.

---

## 2. AST AUDIT (Section 4)

**CLASSIFICATION: IMPLEMENTED (with semantic leakage).**

### `ast::Type` (ast.rs:6-17) — 7 variants:

| Variant | Field | Description |
|---------|-------|-------------|
| `I64` | — | 64-bit signed integer |
| `F64` | — | 64-bit floating point |
| `Bool` | — | Boolean |
| `Str` | — | String (unresolved) |
| `Unit` | — | Unit type |
| `Struct(String)` | name | User-defined struct type (unresolved name) |
| `Enum(String)` | name | User-defined enum type (unresolved name) |

### `ast::Expr` (ast.rs:63-139) — **18 variants** (not 16 as stated in prompt):

1. `Integer(i64, Range<usize>)` — ast.rs:64
2. `Float(f64, Range<usize>)` — ast.rs:65
3. `StrLit(String, Range<usize>)` — ast.rs:66
4. `Bool(bool, Range<usize>)` — ast.rs:67
5. `Unit(Range<usize>)` — ast.rs:68
6. `Var(String, Range<usize>)` — ast.rs:69
7. `Assign { target: String, value: Box<Expr>, span }` — ast.rs:70-74
8. `AugAssign { target: String, op: AugOp, value: Box<Expr>, span }` — ast.rs:75-80
9. `Binary { op: BinOp, lhs: Box<Expr>, rhs: Box<Expr>, span }` — ast.rs:81-86
10. `Unary { op: UnOp, expr: Box<Expr>, span }` — ast.rs:87-91
11. `Call { func: String, args: Vec<Expr>, span }` — ast.rs:92-96
12. `If { condition, then_branch, elif_branches, else_branch, span }` — ast.rs:97-103
13. `For { var: String, iter: Box<Expr>, body: Vec<Stmt>, span }` — ast.rs:104-109
14. `While { condition, body: Vec<Stmt>, span }` — ast.rs:110-114
15. `Range { start, end, is_inclusive, span }` — ast.rs:115-120
16. `StructLiteral { name: String, fields: Vec<(String, Expr)>, span }` — ast.rs:122-126
17. `FieldAccess { expr, field: String, span }` — ast.rs:128-132
18. `EnumConstructor { name: String, variant: String, span }` — ast.rs:134-138

### `ast::Stmt` (ast.rs:37-60) — **6 variants** (not 7 as stated in prompt):

1. `Let { name: String, mutable: bool, ty: Option<Type>, value: Expr, span }` — ast.rs:38-44
2. `Expr(Expr, Range<usize>)` — ast.rs:45
3. `Return(Option<Expr>, Range<usize>)` — ast.rs:46
4. `Println(Expr, Range<usize>)` — ast.rs:47
5. `StructDef { name: String, fields: Vec<(String, Type)>, span }` — ast.rs:49-53
6. `EnumDef { name: String, variants: Vec<String>, span }` — ast.rs:55-59

### `ast::Function` (ast.rs:27-34):
```rust
pub struct Function {
    pub name: String,                           // ast.rs:29
    pub params: Vec<(String, Type)>,            // ast.rs:30
    pub return_type: Type,                      // ast.rs:31
    pub body: Vec<Stmt>,                        // ast.rs:32
    pub span: Range<usize>,                     // ast.rs:33
}
```

### `ast::Program` (ast.rs:22-25):
```rust
pub struct Program {
    pub functions: Vec<Function>,               // ast.rs:24
}
```

### Operator enums:
- `BinOp` (ast.rs:141-156): 13 variants — Add, Sub, Mul, Div, Mod, Eq, Ne, Lt, Gt, Le, Ge, And, Or
- `UnOp` (ast.rs:158-162): 2 variants — Neg, Not
- `AugOp` (ast.rs:164-170): 4 variants — Add, Sub, Mul, Div

### Semantic leakage — AST carries info that belongs in HIR:

**CLASSIFICATION: PARTIALLY IMPLEMENTED (AST overlaps HIR responsibilities).**

The AST carries unresolved identifiers as `String` that should be resolved in HIR:

1. **`Stmt::Let { mutable: bool }`** (ast.rs:40) — mutability info on AST node. Also in HIR (hir/stmt.rs:25). Enforced in lowering (lower.rs:462, 485).
2. **`Expr::Var(String)`** (ast.rs:69) → HIR resolves to `SymbolId` (expr.rs:36)
3. **`Expr::Assign { target: String }`** (ast.rs:71) → HIR `SymbolId` (expr.rs:41)
4. **`Expr::Call { func: String }`** (ast.rs:93) → HIR `DefId` (expr.rs:64)
5. **`Expr::For { var: String }`** (ast.rs:105) → HIR `SymbolId` (expr.rs:78)
6. **`Expr::StructLiteral { name: String, fields: Vec<(String, Expr)> }`** (ast.rs:123-124) → HIR `SymbolId` for both (expr.rs:99-100)
7. **`Expr::FieldAccess { field: String }`** (ast.rs:130) → HIR `SymbolId` (expr.rs:108)
8. **`Expr::EnumConstructor { name: String, variant: String }`** (ast.rs:135-136) → HIR `SymbolId` both (expr.rs:115-116)
9. **`Function { name: String, params: Vec<(String, Type)> }`** (ast.rs:29-30) → HIR `SymbolId` both
10. **`Type::Struct(String)` and `Type::Enum(String)`** (ast.rs:15-17) → HIR `SymbolId` via `ast_type_to_hir` (lower.rs:99-107)
