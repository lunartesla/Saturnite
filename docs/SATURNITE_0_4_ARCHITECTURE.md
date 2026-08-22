# Saturnite 0.4 Architecture

> **Status:** Current architecture (post MIR-migration)
>
> This document describes the MIR-based compiler architecture introduced in
> Saturnite 0.4, where MIR is the sole production codegen path.

## 1. Compilation pipeline

```
Saturnite source
   │
   ▼
┌──────────────────────────────────────────────────────────┐
│  Phase 1  │  Lexer             (src/lexer.rs)             │
│           │  logos-based tokenization                    │
│           │  tokens carry byte spans                     │
├──────────────────────────────────────────────────────────┤
│  Phase 2  │  Parser             (src/parser.rs)          │
│           │  chumsky 0.13                              │
│           │  produces spanned AST                       │
├──────────────────────────────────────────────────────────┤
│  Phase 3  │  Semantic analysis   (src/semantic.rs)     │
│           │  AST → HIR: type checking,                  │
│           │  mutability enforcement, scope resolution   │
├──────────────────────────────────────────────────────────┤
│  Phase 4  │  MIR lowering       (src/mir/lower.rs)      │
│           │  HIR → MIR: builds typed CFG                │
│           │  with LocalId, BlockId, BasicBlock,         │
│           │  Rvalue, Terminator                         │
├──────────────────────────────────────────────────────────┤
│  Phase 5  │  MIR verification    (src/mir/verify.rs)    │
│           │  checks CFG integrity:                      │
│           │  unreachable blocks, type consistency       │
├──────────────────────────────────────────────────────────┤
│  Phase 6  │  MIR optimization    (src/mir/lower.rs)     │
│           │  future: dead-store elimination,            │
│           │  constant folding                           │
├──────────────────────────────────────────────────────────┤
│  Phase 7  │  MIR → LLVM IR    (src/mir/codegen.rs)      │
│           │  the sole production codegen path           │
│           │  translates each MIR construct to           │
│           │  LLVM IR via inkwell                        │
├──────────────────────────────────────────────────────────┤
│  Phase 8  │  Object emission  (src/codegen/emitter.rs)  │
│           │  TargetMachine writes .o / .ll              │
├──────────────────────────────────────────────────────────┤
│  Phase 9  │  Linking         (src/codegen/linker.rs)    │
│           │  system linker (cc / clang / link.exe)      │
└──────────────────────────────────────────────────────────┘
   │
   ▼
Executable
```

## 2. Module layout

| Layer              | Module                         | Description                              |
|--------------------|--------------------------------|------------------------------------------|
| Lexing             | `src/lexer.rs`                 | logos tokenizer with byte spans          |
| Parsing            | `src/parser.rs`                | chumsky 0.13 parser → AST                |
| AST                | `src/ast.rs`                   | spanned AST node definitions             |
| Semantic analysis  | `src/semantic.rs`              | AST → HIR lowering                       |
| HIR                | `src/hir/`                     | typed, span-bearing IR                   |
| MIR                | `src/mir/`                     | typed CFG (lower, verify, optimize)      |
| Codegen (MIR→LLVM) | `src/mir/codegen.rs`           | **sole** codegen path                    |
| Object emission    | `src/codegen/emitter.rs`       | writes .o / .ll from an LLVM module      |
| Linking            | `src/codegen/linker.rs`        | invokes the system linker                |
| Target config      | `src/target.rs`                | triple validation, opt levels, debug info|
| Errors             | `src/error.rs`                 | thiserror + miette Diagnostic            |
| CLI                | `src/main.rs`                  | build / check / run / doctor             |
| Runtime            | `runtime/println_i64.c`        | C runtime compiled via `build.rs` + `cc` |

## 3. MIR overview

The MIR (Mid-level IR) is the compiler's single codegen seam.  It owns a typed
control-flow graph:

```
MirProgram
  ├─ symbols: SymbolInterner
  ├─ functions: Vec<MirFunction>
  │     ├─ def_id: DefId
  │     ├─ params: Vec<(DefId, MirType)>
  │     ├─ locals: Vec<MirLocal>          // typed stack slots
  │     ├─ blocks: Vec<MirBasicBlock>
  │     └─ start_block: BlockId
  │
  ├─ MirBasicBlock
  │     ├─ id: BlockId
  │     ├─ name: String
  │     ├─ stmts: Vec<MirStmt>
  │     └─ terminator: MirTerminator
  │
  ├─ MirStmt
  │     └─ kind: MirStmtKind
  │           ├─ LocalDecl { local, ty }
  │           └─ Assign { local, rvalue: MirRvalue }
  │
  ├─ MirTerminator
  │     ├─ Return(MirOperand)
  │     ├─ Goto { target: BlockId }
  │     ├─ SwitchInt { scrutinee, arms: Vec<(i128, BlockId)>, default: BlockId }
  │     └─ Call { callee, args, destination, unwind }
  │
  ├─ MirRvalue
  │     ├─ Use(MirOperand)
  │     ├─ Const(MirConst)
  │     ├─ BinOp(MirBinOp, MirOperand, MirOperand)
  │     ├─ UnOp(MirUnOp, MirOperand)
  │     ├─ Struct { fields: Vec<(DefId, MirOperand)> }
  │     └─ EnumVariant { variant: DefId, fields: Vec<MirOperand> }
  │
  ├─ MirOperand
  │     ├─ Local(LocalId)        // load from a stack slot
  │     ├─ Const(MirConst)
  │     └─ ...
  │
  ├─ MirType
  │     ├─ I64, F64, Bool, Str, Unit, Struct(DefId), Enum(DefId, variant)
  │
  └─ MirConst
        ├─ I64(i64), F64(f64), Bool(bool), Str(String), Unit, Zero
```

### Key design decisions

- **Stack-alloced locals:** Every `MirLocal` is lowered to an `alloca` in the
  entry block, preserving mutable-variable semantics across basic-block
  boundaries (including loops).
- **SwitchInt type selection:** The `SwitchInt` terminator selects the correct
  LLVM integer width based on the scrutinee's `MirType` (e.g. `i1` for `Bool`),
  avoiding type-mismatch segfaults.
- **Shadowing safety:** `lower_stmt` inserts `LocalDecl` before evaluating the
  initializer rvalue, so `let x = x + 1` reads the *previous* value correctly.
- **Builtins:** `println` is a builtin (`println_i64`) declared at module level
  with a sentinel `DefId` (`u32::MAX - 1`); user functions with that sentinel
  are skipped during declaration.

## 4. Codegen seam

```
main.rs (Build / Run command)
  │
  ├─ stnx::lexer::Lexer::new(src) → tokens
  ├─ stnx::parser::parse(src, tokens) → AST (ast::Program)
  ├─ stnx::semantic::analyze_and_lower(&program) → HIR (hir::HirProgram)
  │
  ├─ stnx::mir::lower::lower_program(&hir) → MIR (mir::MirProgram)
  ├─ mir.verify() → Result<(), Vec<MirVerifyError>>
  ├─ stnx::mir::optimize(&mut mir)  (future optimizations)
  │
  ├─ match output_kind:
  │     OutputKind::Ir  → generate_ir_from_mir(&mir) → write .ll text file
  │     OutputKind::Obj → compile_from_mir_ext(&mir, path, config, save_temps)
  │     OutputKind::Exe → compile_from_mir_ext(&mir, path, config, save_temps)
  │
  └─ compile_from_mir_ext dispatches to ObjectEmitter + Linker
```

Entry points (all in `src/mir/codegen.rs`):

| Function               | Purpose                              |
|------------------------|--------------------------------------|
| `generate_ir_from_mir` | Emit LLVM IR text from a `MirProgram`|
| `compile_from_mir`     | Compile a `MirProgram` to an artifact|
| `compile_from_mir_ext` | Same, with `save_temps` flag          |

These functions delegate object emission and linking to the shared
`codegen::ObjectEmitter` and `codegen::Linker` infrastructure.

## 5. Code generation infrastructure (shared)

The `codegen` module provides the object-emission and linking seams that the
MIR backend delegates to:

- **`ObjectEmitter`** (`src/codegen/emitter.rs`): Wraps an LLVM module and a
  `TargetMachine` to emit `.o` object files or `.ll` IR text files.
- **`Linker`** (`src/codegen/linker.rs`): Invokes the system linker (`cc` on
  Linux, `clang` on macOS, `link.exe`/`gcc` on Windows) to produce a final
  executable from an object file.
- **`check_linker`** / **`host_triple`** / **`run_diagnostics`**: Utility
  functions used by `main.rs` for cross-compilation guards and diagnostics.

These are **not** tied to any particular IR (HIR or MIR) — they operate on
generic LLVM modules.

## 6. Runtime

The Saturnite runtime is a minimal C library providing `println_i64`:

```
runtime/println_i64.c   →  compiled at build time via build.rs + cc crate
                         →  linked into every Saturnite executable
```

The runtime is host-only: cross-compilation to a non-host target is rejected
at the `Build` command level with a clear error message.

## 7. CLI

```
saturnite build <FILE> [OPTIONS]    # Build to executable / object / IR
saturnite check <FILE>              # Type & semantic check (no codegen)
saturnite run <FILE>                # Build then execute
saturnite doctor                    # Print environment diagnostics
```

All `build`/`run` paths go through the full MIR pipeline:
`parse → semantic → lower → verify → optimize → codegen → emit → link`.

## 8. Testing

| Test binary           | Tests                     | What it covers                           |
|-----------------------|---------------------------|------------------------------------------|
| `codegen.rs`          | 14                        | MIR codegen: IR output, exe, object, etc. |
| `native_compilation`| 47                        | Full build+run of native executables      |
| `semantic.rs`         | 28                        | Type checking, mutability, scope          |
| `lexer.rs`            | 17                        | Tokenization accuracy                    |
| `mir_lower.rs`        | 10                        | HIR → MIR lowering                       |
| `diagnostics.rs`      | 6                         | Error span reporting                     |
| `test_full_compile`   | 1                         | End-to-end build                         |
| `test_ir_only`        | 1                         | IR-only generation                       |
| `test_native_only`    | 1                         | Native compile+run                       |
| `test_target_machine` | 1                         | Raw inkwell TargetMachine                 |
| Library unit tests    | 7                         | Internal module tests                    |

**Total: 133 tests, all passing.**
