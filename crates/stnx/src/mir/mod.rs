//! MIR (Mid-level IR) — the control-flow-aware intermediate representation.
//!
//! MIR sits between HIR and LLVM IR.  It owns the compiler's control-flow graph
//! so that language-level optimizations and analyses (constant folding, dead
//! block elimination, copy propagation) happen on a target-independent
//! representation before LLVM sees the code.
//!
//! ## Pipeline (0.4)
//!
//! ```text
//! Source → Lexer → Parser → AST → HIR → MIR → LLVM IR → Object → Linker → Executable
//! ```
//!
//! ## Design principles
//!
//! * **Explicit CFG.**  Every function is a graph of `MirBasicBlock`s.  Control-flow
//!   expressions (`if`, `for`, `while`) are lowered into `Goto` / `SwitchInt`
//!   terminators by the HIR→MIR pass, not rediscovered by LLVM codegen.
//! * **Flat locals.**  Variables and temporaries are `LocalId`s stored in `MirLocal`s.
//!   No place projection or `MirPlace` — a local is just a typed slot.
//! * **Rvalues, not nested expressions.**  Every compound expression is lowered to
//!   an `Assign` that writes its result into a local.
//! * **Calls are terminators.**  Function calls end their block; the return value
//!   is stored in a destination local and the next block continues.
//! * **Rust interoperability.**  MIR types reuse `HirType` — no parallel type system.

use crate::ast::{AugOp, BinOp, UnOp};
use crate::hir::symbol::{DefId, SymbolId, SymbolInterner};
use crate::hir::{EnumDef, HirType, StructDef};
use serde::{Deserialize, Serialize};

pub mod codegen;
pub mod lower;
pub mod monomorphize;
pub mod opt;
pub mod verify;

/// A stable identifier for a MIR basic block.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BlockId(pub u32);

/// A stable identifier for a MIR local (parameter, variable, or temporary).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LocalId(pub u32);

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// MIR type — reuses `HirType` so there is no parallel type system.
pub type MirType = HirType;

/// A local variable descriptor.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MirLocal {
    pub id: LocalId,
    pub ty: MirType,
    /// Symbol name for diagnostics (empty string = compiler temp).
    pub name: SymbolId,
    /// Whether this local is mutated (informs LLVM alloc vs. SSA).
    pub mutable: bool,
}

// ---------------------------------------------------------------------------
// Operands
// ---------------------------------------------------------------------------

/// A compile-time constant value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MirConst {
    I64(i64),
    F64(f64),
    Bool(bool),
}

/// A value that can be consumed without further computation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MirOperand {
    /// A compile-time constant.
    Const(MirConst),
    /// A reference to a local variable or temporary.
    Local(LocalId),
}

impl MirOperand {
    /// Return the operand's `MirType`, using the program's local table.
    pub fn ty(&self, locals: &[MirLocal]) -> MirType {
        match self {
            MirOperand::Const(c) => c.ty(),
            MirOperand::Local(lid) => locals
                .iter()
                .find(|l| l.id == *lid)
                .map(|l| l.ty.clone())
                .unwrap_or(MirType::I64),
        }
    }
}

impl MirConst {
    pub fn ty(&self) -> MirType {
        match self {
            MirConst::I64(_) => MirType::I64,
            MirConst::F64(_) => MirType::F64,
            MirConst::Bool(_) => MirType::Bool,
        }
    }
}

// ---------------------------------------------------------------------------
// Operators
// ---------------------------------------------------------------------------

/// MIR binary operator — mirrors `ast::BinOp`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MirBinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
}

impl From<BinOp> for MirBinOp {
    fn from(op: BinOp) -> Self {
        match op {
            BinOp::Add => MirBinOp::Add,
            BinOp::Sub => MirBinOp::Sub,
            BinOp::Mul => MirBinOp::Mul,
            BinOp::Div => MirBinOp::Div,
            BinOp::Mod => MirBinOp::Mod,
            BinOp::Eq => MirBinOp::Eq,
            BinOp::Ne => MirBinOp::Ne,
            BinOp::Lt => MirBinOp::Lt,
            BinOp::Gt => MirBinOp::Gt,
            BinOp::Le => MirBinOp::Le,
            BinOp::Ge => MirBinOp::Ge,
            BinOp::And => MirBinOp::And,
            BinOp::Or => MirBinOp::Or,
        }
    }
}

/// MIR unary operator — mirrors `ast::UnOp`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MirUnOp {
    Neg,
    Not,
}

impl From<UnOp> for MirUnOp {
    fn from(op: UnOp) -> Self {
        match op {
            UnOp::Neg => MirUnOp::Neg,
            UnOp::Not => MirUnOp::Not,
        }
    }
}

/// AugOp maps to the corresponding `MirBinOp` (+= → Add, etc.)
impl From<AugOp> for MirBinOp {
    fn from(op: AugOp) -> Self {
        match op {
            AugOp::Add => MirBinOp::Add,
            AugOp::Sub => MirBinOp::Sub,
            AugOp::Mul => MirBinOp::Mul,
            AugOp::Div => MirBinOp::Div,
        }
    }
}

// ---------------------------------------------------------------------------
// Rvalues
// ---------------------------------------------------------------------------

/// A value-producing computation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum MirRvalue {
    /// Copy/clone an operand into a local.
    Use(MirOperand),
    /// Binary operation on two operands.
    Binary {
        op: MirBinOp,
        lhs: MirOperand,
        rhs: MirOperand,
    },
    /// Unary operation.
    Unary { op: MirUnOp, operand: MirOperand },
    /// Struct construction: `Point { x: 10, y: 20 }`.
    ///
    /// `fields` are ordered to match the struct definition's field order.
    StructLit {
        struct_def: SymbolId,
        fields: Vec<(SymbolId, MirOperand)>,
    },
    /// Field access on a struct local: `p.x`.
    FieldAccess { local: LocalId, field: SymbolId },
    /// Enum variant constructor as an i64 tag: `Color::Red` → `0`.
    EnumCtor {
        enum_def: SymbolId,
        variant: SymbolId,
    },
    /// String literal → global string pointer cast to i64.
    StrLit(SymbolId),
}

// ---------------------------------------------------------------------------
// Statements
// ---------------------------------------------------------------------------

/// A MIR statement.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MirStmt {
    pub kind: MirStmtKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum MirStmtKind {
    /// Declare a local with type and mutability.
    LocalDecl {
        local: LocalId,
        ty: MirType,
        mutable: bool,
    },
    /// Assign an rvalue to a local.
    Assign { local: LocalId, rvalue: MirRvalue },
}

// ---------------------------------------------------------------------------
// Terminators
// ---------------------------------------------------------------------------

/// A block terminator (every block has exactly one).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum MirTerminator {
    /// Unconditional branch.
    Goto { target: BlockId },

    /// Switch on an integer/bool value.  `branches` maps concrete values to
    /// target blocks; anything not listed falls through to `else_target`.
    /// For boolean conditions, `branches = [(1, then_bb)]` and `else_target`
    /// handles the false case.
    SwitchInt {
        scrutinee: MirOperand,
        ty: MirType,
        branches: Vec<(u64, BlockId)>,
        else_target: BlockId,
    },

    /// Function call — a terminator because calls may have side-effects.
    /// The return value is stored in `destination`; control continues at `next`.
    Call {
        func: DefId,
        args: Vec<MirOperand>,
        destination: LocalId,
        next: BlockId,
    },

    /// Return from the function.
    Return(Option<MirOperand>),

    /// Block is unreachable — used as a placeholder during construction
    /// and to mark unreachable code.
    Unreachable,
}

// ---------------------------------------------------------------------------
// Basic blocks
// ---------------------------------------------------------------------------

/// A MIR basic block: a list of statements terminated by a single terminator.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MirBasicBlock {
    pub id: BlockId,
    /// Optional name for debugging and IR diagnostics.
    pub name: String,
    pub stmts: Vec<MirStmt>,
    pub terminator: MirTerminator,
}

// ---------------------------------------------------------------------------
// Functions
// ---------------------------------------------------------------------------

/// A MIR function: locals + basic blocks forming an explicit CFG.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MirFunction {
    pub def_id: DefId,
    pub name: SymbolId,
    /// Parameter types (for signature matching).
    pub params: Vec<(SymbolId, MirType)>,
    pub return_type: MirType,
    /// All locals, including parameters (index 0..n are params).
    pub locals: Vec<MirLocal>,
    /// Local IDs for each parameter (parallel to `params`).
    pub param_locals: Vec<LocalId>,
    /// The function body as a list of basic blocks.
    pub blocks: Vec<MirBasicBlock>,
    /// Entry point block.
    pub start_block: BlockId,
}

// ---------------------------------------------------------------------------
// Program
// ---------------------------------------------------------------------------

/// A compiled MIR program (collection of functions + shared metadata).
#[derive(Debug)]
pub struct MirProgram {
    pub functions: Vec<MirFunction>,
    /// Shared symbol table (cloned from HIR).
    pub symbols: SymbolInterner,
    /// Struct definitions (needed for struct layout in codegen).
    pub structs: Vec<StructDef>,
    /// Enum definitions (needed for enum tag resolution in codegen).
    pub enums: Vec<EnumDef>,
}

impl MirProgram {
    /// Look up a struct definition by name symbol.
    pub fn struct_def(&self, sym: SymbolId) -> Option<&StructDef> {
        self.structs.iter().find(|s| s.name == sym)
    }

    /// Look up an enum definition by name symbol.
    pub fn enum_def(&self, sym: SymbolId) -> Option<&EnumDef> {
        self.enums.iter().find(|e| e.name == sym)
    }

    /// Resolve a `DefId` to its function name string.
    pub fn function_name(&self, id: DefId) -> Option<&str> {
        self.functions
            .iter()
            .find(|f| f.def_id == id)
            .and_then(|f| self.symbols.lookup(f.name))
    }
}
