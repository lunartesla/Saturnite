//! MIR CFG verifier.
//!
//! Validates structural invariants of a [`MirProgram`] *before* LLVM codegen.
//! The verifier returns structured errors rather than panicking so that
//! diagnostic infrastructure can present them to the user.

use crate::mir::{
    BlockId, LocalId, MirBasicBlock, MirFunction, MirOperand, MirProgram, MirRvalue, MirStmt,
    MirStmtKind, MirTerminator,
};
use std::collections::HashSet;

/// A single verification failure.
#[derive(Debug, Clone)]
pub struct MirVerifyError {
    pub message: String,
    pub location: Option<(String, String)>,
}

impl std::fmt::Display for MirVerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.location {
            Some((func, block)) => {
                write!(f, "{} (in fn `{}`, block `{}`)", self.message, func, block)
            }
            None => write!(f, "{}", self.message),
        }
    }
}

/// Verification result: `Ok(())` or a list of errors.
pub type VerifyResult = Result<(), Vec<MirVerifyError>>;

impl MirVerifyError {
    pub fn to_compiler_error(&self) -> crate::error::CompilerError {
        crate::error::CompilerError::codegen(format!("{}", self))
    }
}

impl MirProgram {
    /// Run all verification checks on the entire program.
    pub fn verify(&self) -> VerifyResult {
        let mut errors = Vec::new();
        for func in &self.functions {
            Self::verify_function(func, self, &mut errors);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    fn verify_function(func: &MirFunction, prog: &MirProgram, errors: &mut Vec<MirVerifyError>) {
        let name = prog.symbols.lookup(func.name).unwrap_or("?").to_string();
        let valid_blocks: HashSet<BlockId> = func.blocks.iter().map(|b| b.id).collect();
        let valid_locals: HashSet<LocalId> = func.locals.iter().map(|l| l.id).collect();

        // Check 1: every block has a real terminator (not Unreachable placeholder)
        for block in &func.blocks {
            if matches!(block.terminator, MirTerminator::Unreachable) {
                errors.push(MirVerifyError {
                    message: format!(
                        "block `{}` has no terminator (Unreachable placeholder remains)",
                        block.name
                    ),
                    location: Some((name.clone(), block.name.clone())),
                });
            }
        }

        // Check 2: all terminator target blocks exist
        for block in &func.blocks {
            check_terminator_blocks(block, &valid_blocks, &name, errors);
        }

        // Check 3: LocalId references in operands are valid
        for block in &func.blocks {
            check_local_refs(block, &valid_locals, &name, &block.name, errors);
        }

        // Check 4: parameters exist as locals
        for (i, param_lid) in func.param_locals.iter().enumerate() {
            if !valid_locals.contains(param_lid) {
                errors.push(MirVerifyError {
                    message: format!("parameter {} local {:?} not found in locals", i, param_lid),
                    location: Some((name.clone(), "entry".to_string())),
                });
            }
        }

        // Check 5: start_block is valid
        if !valid_blocks.contains(&func.start_block) {
            errors.push(MirVerifyError {
                message: format!(
                    "start_block {:?} does not exist in function body",
                    func.start_block
                ),
                location: Some((name.clone(), "entry".to_string())),
            });
        }

        // Reachability (formerly Check 6) is intentionally NOT enforced here.
        // Unreachable blocks are valid MIR — they arise naturally from
        // exhaustive if/elif/else chains where every branch returns.  Such
        // blocks carry a real terminator (e.g. `Return`) and are accepted by
        // LLVM IR; dead-code elimination is a MIR-opt pass (Phase 0.5+).
        // Only blocks that lack *any* terminator are flagged (Check 1 above).
    }
}

/// Check that all block references in a terminator point to valid blocks.
fn check_terminator_blocks(
    block: &MirBasicBlock,
    valid: &HashSet<BlockId>,
    func_name: &str,
    errors: &mut Vec<MirVerifyError>,
) {
    let loc = |msg: String| MirVerifyError {
        message: msg,
        location: Some((func_name.to_string(), block.name.clone())),
    };
    match &block.terminator {
        MirTerminator::Goto { target } => {
            if !valid.contains(target) {
                errors.push(loc(format!(
                    "Goto references non-existent block {:?}",
                    target
                )));
            }
        }
        MirTerminator::SwitchInt {
            branches,
            else_target,
            ..
        } => {
            if !valid.contains(else_target) {
                errors.push(loc(format!(
                    "SwitchInt else_target references non-existent block {:?}",
                    else_target
                )));
            }
            for (_, target) in branches {
                if !valid.contains(target) {
                    errors.push(loc(format!(
                        "SwitchInt branch references non-existent block {:?}",
                        target
                    )));
                }
            }
        }
        MirTerminator::Call { next, .. } => {
            if !valid.contains(next) {
                errors.push(loc(format!(
                    "Call next references non-existent block {:?}",
                    next
                )));
            }
        }
        MirTerminator::Return(_) | MirTerminator::Unreachable => {}
    }
}

/// Collect all `MirOperand` references from a `MirStmt`.
fn stmt_operands(stmt: &MirStmt) -> Vec<MirOperand> {
    match &stmt.kind {
        MirStmtKind::LocalDecl { .. } => vec![],
        MirStmtKind::Assign { rvalue, .. } => rvalue_operands(rvalue),
    }
}

fn rvalue_operands(rvalue: &MirRvalue) -> Vec<MirOperand> {
    match rvalue {
        MirRvalue::Use(op) => vec![op.clone()],
        MirRvalue::Binary { lhs, rhs, .. } => vec![lhs.clone(), rhs.clone()],
        MirRvalue::Unary { operand, .. } => vec![operand.clone()],
        MirRvalue::FieldAccess { local, .. } => vec![MirOperand::Local(*local)],
        MirRvalue::StructLit { fields, .. } => fields.iter().map(|(_, v)| v.clone()).collect(),
        MirRvalue::EnumCtor { .. } => vec![],
        MirRvalue::ListLiteral { elements } => elements.clone(),
        MirRvalue::Index { list_local, index } => {
            vec![MirOperand::Local(*list_local), index.clone()]
        }
        MirRvalue::Length { list_local } => vec![MirOperand::Local(*list_local)],
        MirRvalue::StrLit(_) => vec![],
    }
}

fn check_local_refs(
    block: &MirBasicBlock,
    valid: &HashSet<LocalId>,
    func_name: &str,
    block_name: &str,
    errors: &mut Vec<MirVerifyError>,
) {
    for stmt in &block.stmts {
        for op in stmt_operands(stmt) {
            if let MirOperand::Local(lid) = op {
                if !valid.contains(&lid) {
                    errors.push(MirVerifyError {
                        message: format!("operand references undefined local {:?}", lid),
                        location: Some((func_name.to_string(), block_name.to_string())),
                    });
                }
            }
        }
    }
}
