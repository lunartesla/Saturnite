# MIR (Mid-level IR) Design — Phase 15

## Status: Design Proposal

## Overview

This document describes the Machine-Independent Representation (MIR) layer for
Saturnite 0.3. MIR sits between HIR and LLVM IR, providing a stable, typed,
control-flow-graph based intermediate representation that enables mid-level
optimizations, language-level analyses, backend flexibility, and debugging
support.

## 1. Motivation

The current pipeline is:
```
Source -> Lexer -> Parser -> AST -> HIR -> LLVM IR -> Machine Code
```

MIR provides a middle ground: a well-defined, type-checked CFG that is:
- **Language-agnostic** in structure (no Saturnite-specific syntax).
- **Backend-agnostic** (can target LLVM, Cranelift, or a custom backend).
- **Debuggable** (preserves enough information for meaningful error messages).

## 3. MIR IR Structure

### 3.1 Basic Blocks

```rust
struct MirBasicBlock {
    id: BlockId,
    stmts: Vec<MirStmt>,
    terminator: MirTerminator,
}
```

### 3.2 Statements

```rust
enum MirStmt {
    LocalDecl { local: LocalId, ty: MirType, init: Option<MirExpr> },
    Assign { place: MirPlace, rvalue: MirRvalue },
    Call { callee: MirOperand, args: Vec<MirOperand>, destination: Option<MirPlace>, target: BlockId, unwind: BlockId },
    StorageLive(LocalId),
    StorageDead(LocalId),
    DebugInfo { local: LocalId, file: &str, line: u32, col: u32 },
}
```

### 3.3 Terminators

```rust
enum MirTerminator {
    Goto { target: BlockId },
    Switch { cond: MirOperand, target: BlockId, else_target: BlockId },
    Return(Option<MirOperand>),
    ReturnVoid,
    Unwind,
    Unreachable,
}
```

### 3.4 Places and Operands

```rust
enum MirPlace {
    Local(LocalId),
    Field(Box<MirPlace>, SymbolId),
    Deref(Box<MirPlace>),
    Index(Box<MirPlace>, MirOperand),
}

enum MirOperand {
    Constant(MirConst),
    Local(LocalId),
    Place(MirPlace),
}

enum MirConst {
    I64(i64), F64(f64), Bool(bool), Str(String), EnumTag(i64),
}
```

### 3.5 Rvalues

```rust
enum MirRvalue {
    Use(MirOperand),
    Binary { op: BinOp, lhs: MirOperand, rhs: MirOperand },
    Unary { op: UnOp, operand: MirOperand },
    StructLit { struct_def: DefId, fields: Vec<(SymbolId, MirOperand)> },
    FieldAccess { place: MirPlace, field: SymbolId },
    EnumCtor { enum_def: DefId, variant: SymbolId, arg: Option<MirOperand> },
    Ref { mode: BorrowMode, place: MirPlace },
}
```

### 3.6 Functions

```rust
struct MirFunction {
    def_id: DefId,
    name: SymbolId,
    params: Vec<(SymbolId, MirType)>,
    ret_ty: MirType,
    basic_blocks: Vec<MirBasicBlock>,
    locals: Vec<MirLocal>,
}

struct MirProgram {
    functions: Vec<MirFunction>,
    struct_defs: Vec<MirStructDef>,
    enum_defs: Vec<MirEnumDef>,
}
```

## 4. MIR Generation from HIR

Each `HirExpr` is lowered to a sequence of MIR statements:

| HIR Expression | MIR Generation |
|---|---|
| `Literal(I64(n))` | `LocalDecl { init: Some(Constant(I64(n))) }` |
| `BinaryOp(op, lhs, rhs)` | Lower lhs -> temp1, lower rhs -> temp2, `Assign(Binary{op,temp1,temp2})` |
| `VarRef(name)` | `Local(name)` |
| `StructLit { fields }` | Lower each field, emit `StructLit` rvalue |
| `FieldAccess { expr, field }` | Lower expr to temp, emit `FieldAccess` rvalue |
| `Block { stmts, tail }` | Lower each stmt; tail becomes return value |

## 5. MIR Optimizations (Planned)

1. **Constant folding:** `1 + 2` -> `3`
2. **CSE:** Avoid recomputing identical subexpressions.
3. **DCE:** Remove unreachable blocks and unused locals.
4. **Copy propagation:** Replace `a = b; c = a` with `c = b`.
5. **Loop invariant code motion:** Hoist invariants out of loops.
6. **Tail call optimization:** Convert tail recursion to jumps.

```rust
trait MirPass {
    fn run(&self, mir: &mut MirProgram);
}
```

## 6. Codegen from MIR to LLVM

The MIR -> LLVM IR pass walks each `MirFunction`:
- Maps `LocalId` -> LLVM `AllocaInst` / `ValueRef`.
- Maps `MirBasicBlock` -> LLVM `BasicBlock`.
- Maps `MirTerminator` -> LLVM terminator instructions.
- Maps `MirRvalue` -> LLVM IR instructions.

## 7. Debugging and Diagnostics

- Each `MirStmt` carries an optional `SourceSpan`.
- `DebugInfo` markers link MIR locals to source positions.
- MIR can be dumped via `--emit-mir` flag for debugging.

## 8. Open Questions

- Should MIR be serialized to disk (like LLVM bitcode)?
- How to handle generics / monomorphization at the MIR level?
- Should MIR support inline assembly?


## 2. MIR Type System

| MIR Type | HIR Type | Description |
|----------|----------|-------------|
| `MIrI64` | `HirType::I64` | 64-bit signed integer |
| `MIrF64` | `HirType::F64` | 64-bit floating point |
| `MIrBool` | `HirType::Bool` | Boolean |
| `MIrUnit` | `HirType::Unit` | Void/unit type |
| `MIrStr` | `HirType::Str` | String literal reference |
| `MIrPtr<T>` | N/A (new) | Pointer to MIR type T |
| `MIrStruct` | `HirType::Struct` | Struct type |
| `MIrEnum` | `HirType::Enum` | Enum type |
| `MIrFn` | `HirType::Fn` | Function pointer type |
