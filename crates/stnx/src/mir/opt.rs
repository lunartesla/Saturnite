//! MIR optimization passes.
//!
//! Constant folding evaluates operations whose operands are all compile-time
//! constants, replacing them with the resulting `MirConst`. This pass operates
//! on MIR (not LLVM IR) so that type-aware decisions can be made before the
//! backend sees the code.
//!
//! ## Current passes
//!
//! * **Constant folding** — folds arithmetic, comparison, and logical ops on
//!   `i64`, `f64`, and `bool` constants.

use crate::mir::{MirBinOp, MirConst, MirOperand, MirProgram, MirRvalue, MirStmtKind, MirUnOp};

/// Entry point for MIR optimization. Runs all passes on every function.
pub fn optimize(program: &mut MirProgram) {
    for func in &mut program.functions {
        ConstantFolder::run(func);
    }
}

/// Type-aware constant folding over a single function.
struct ConstantFolder;

impl ConstantFolder {
    fn run(func: &mut crate::mir::MirFunction) {
        for block in &mut func.blocks {
            for stmt in &mut block.stmts {
                if let MirStmtKind::Assign { rvalue, .. } = &mut stmt.kind {
                    let folded = Self::fold_rvalue(rvalue.clone());
                    *rvalue = folded;
                }
            }
        }
    }

    /// Fold a single `MirRvalue`, returning the (possibly folded) result.
    fn fold_rvalue(rvalue: MirRvalue) -> MirRvalue {
        match rvalue {
            MirRvalue::Binary { op, lhs, rhs } => {
                let (lhs_const, rhs_const) = match (operand_const(&lhs), operand_const(&rhs)) {
                    (Some(l), Some(r)) => (l, r),
                    _ => return MirRvalue::Binary { op, lhs, rhs },
                };

                match MirConst::fold_binop(op, &lhs_const, &rhs_const) {
                    Some(result) => MirRvalue::Use(MirOperand::Const(result)),
                    None => MirRvalue::Binary { op, lhs, rhs },
                }
            }
            MirRvalue::Unary { op, operand } => {
                if let Some(val) = operand_const(&operand) {
                    if let Some(result) = MirConst::fold_unop(op, &val) {
                        return MirRvalue::Use(MirOperand::Const(result));
                    }
                }
                MirRvalue::Unary { op, operand }
            }
            other => other,
        }
    }
}

/// Extract a `MirConst` from a `MirOperand` if it is a constant.
fn operand_const(operand: &MirOperand) -> Option<MirConst> {
    match operand {
        MirOperand::Const(c) => Some(c.clone()),
        MirOperand::Local(_) => None,
    }
}

impl MirConst {
    /// Fold a binary operation on two constants.
    ///
    /// Returns `None` if the operation is not supported for the given types
    /// (e.g. `%` on `F64`, `&&` on `I64`).
    pub fn fold_binop(op: MirBinOp, lhs: &MirConst, rhs: &MirConst) -> Option<MirConst> {
        match (lhs, rhs) {
            (MirConst::I64(a), MirConst::I64(b)) => fold_i64(op, *a, *b),
            (MirConst::F64(a), MirConst::F64(b)) => fold_f64(op, *a, *b),
            (MirConst::Bool(a), MirConst::Bool(b)) => fold_bool(op, *a, *b),
            _ => None, // type mismatch or unsupported type combination
        }
    }

    /// Fold a unary operation on a constant.
    pub fn fold_unop(op: MirUnOp, operand: &MirConst) -> Option<MirConst> {
        match (op, operand) {
            (MirUnOp::Neg, MirConst::I64(n)) => Some(MirConst::I64(-n)),
            (MirUnOp::Neg, MirConst::F64(f)) => Some(MirConst::F64(-f)),
            (MirUnOp::Not, MirConst::Bool(b)) => Some(MirConst::Bool(!b)),
            _ => None,
        }
    }
}

fn fold_i64(op: MirBinOp, a: i64, b: i64) -> Option<MirConst> {
    match op {
        MirBinOp::Add => Some(MirConst::I64(a.wrapping_add(b))),
        MirBinOp::Sub => Some(MirConst::I64(a.wrapping_sub(b))),
        MirBinOp::Mul => Some(MirConst::I64(a.wrapping_mul(b))),
        MirBinOp::Div => {
            // Division by zero is left for runtime (matching language semantics).
            if b == 0 {
                None
            } else {
                Some(MirConst::I64(a.wrapping_div(b)))
            }
        }
        MirBinOp::Mod => {
            if b == 0 {
                None
            } else {
                Some(MirConst::I64(a.wrapping_rem(b)))
            }
        }
        MirBinOp::Eq => Some(MirConst::Bool(a == b)),
        MirBinOp::Ne => Some(MirConst::Bool(a != b)),
        MirBinOp::Lt => Some(MirConst::Bool(a < b)),
        MirBinOp::Gt => Some(MirConst::Bool(a > b)),
        MirBinOp::Le => Some(MirConst::Bool(a <= b)),
        MirBinOp::Ge => Some(MirConst::Bool(a >= b)),
        MirBinOp::And => Some(MirConst::Bool(a != 0 && b != 0)),
        MirBinOp::Or => Some(MirConst::Bool(a != 0 || b != 0)),
    }
}

fn fold_f64(op: MirBinOp, a: f64, b: f64) -> Option<MirConst> {
    match op {
        MirBinOp::Add => Some(MirConst::F64(a + b)),
        MirBinOp::Sub => Some(MirConst::F64(a - b)),
        MirBinOp::Mul => Some(MirConst::F64(a * b)),
        MirBinOp::Div => Some(MirConst::F64(a / b)),
        MirBinOp::Mod => None, // floating-point modulo is not a language feature
        MirBinOp::Eq => Some(MirConst::Bool(a == b)),
        MirBinOp::Ne => Some(MirConst::Bool(a != b)),
        MirBinOp::Lt => Some(MirConst::Bool(a < b)),
        MirBinOp::Gt => Some(MirConst::Bool(a > b)),
        MirBinOp::Le => Some(MirConst::Bool(a <= b)),
        MirBinOp::Ge => Some(MirConst::Bool(a >= b)),
        // Logical and/or on floats are not valid Saturnite operations.
        MirBinOp::And | MirBinOp::Or => None,
    }
}

fn fold_bool(op: MirBinOp, a: bool, b: bool) -> Option<MirConst> {
    match op {
        MirBinOp::Eq => Some(MirConst::Bool(a == b)),
        MirBinOp::Ne => Some(MirConst::Bool(a != b)),
        MirBinOp::And => Some(MirConst::Bool(a && b)),
        MirBinOp::Or => Some(MirConst::Bool(a || b)),
        // Arithmetic and ordering on bools are not supported by the language.
        MirBinOp::Add
        | MirBinOp::Sub
        | MirBinOp::Mul
        | MirBinOp::Div
        | MirBinOp::Mod
        | MirBinOp::Lt
        | MirBinOp::Gt
        | MirBinOp::Le
        | MirBinOp::Ge => None,
    }
}
