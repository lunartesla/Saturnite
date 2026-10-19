//! MIR CFG verifier.
//!
//! Validates structural invariants of a [`MirProgram`] *before* LLVM codegen.
//! The verifier returns structured errors rather than panicking so that
//! diagnostic infrastructure can present them to the user.

use crate::hir::HirType;
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

        // Check 6: ExternalCall invariants
        // - arg_types length must match args length
        // - all arg_types must be ABI-safe (i64, f64, bool, str, Unit)
        // - ret_ty must be ABI-safe
        for block in &func.blocks {
            for stmt in &block.stmts {
                if let MirStmtKind::Assign {
                    rvalue:
                        MirRvalue::ExternalCall {
                            args,
                            arg_types,
                            ret_ty,
                            kind,
                            symbol,
                            ..
                        },
                    ..
                } = &stmt.kind
                {
                    if args.len() != arg_types.len() {
                        errors.push(MirVerifyError {
                            message: format!(
                                "ExternalCall `{}` ({:?}): arg count {} does not match arg_types count {}",
                                symbol, kind, args.len(), arg_types.len()
                            ),
                            location: Some((name.clone(), block.name.clone())),
                        });
                    }
                    for (i, ty) in arg_types.iter().enumerate() {
                        if !is_abi_safe_mir(ty) {
                            errors.push(MirVerifyError {
                                message: format!(
                                    "ExternalCall `{}` ({:?}) arg {}: type {:?} is not ABI-safe",
                                    symbol, kind, i, ty
                                ),
                                location: Some((name.clone(), block.name.clone())),
                            });
                        }
                    }
                    if !is_abi_safe_mir(ret_ty) {
                        errors.push(MirVerifyError {
                            message: format!(
                                "ExternalCall `{}` ({:?}): return type {:?} is not ABI-safe",
                                symbol, kind, ret_ty
                            ),
                            location: Some((name.clone(), block.name.clone())),
                        });
                    }
                }
            }
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

/// Check whether a MIR type is ABI-safe for external calls.
/// Only i64, f64, bool, str, and Unit cross the FFI boundary safely.
fn is_abi_safe_mir(ty: &HirType) -> bool {
    matches!(
        ty,
        HirType::I64 | HirType::F64 | HirType::Bool | HirType::Str | HirType::Unit
    )
}

#[cfg(test)]
mod tests {
    use crate::hir::symbol::{DefId, SymbolId, SymbolInterner};
    use crate::hir::types::HirType;
    use crate::mir::{
        BlockId, LocalId, MirBasicBlock, MirFunction, MirLocal, MirOperand, MirProgram, MirRvalue,
        MirStmt, MirStmtKind, MirTerminator, MirType,
    };

    /// Build a minimal `MirProgram` with one function containing a single block
    /// that assigns the given rvalue to a local.
    fn build_program_with_rvalue(rvalue: MirRvalue) -> MirProgram {
        let mut symbols = SymbolInterner::default();
        let main_name = symbols.intern("main");

        let mut locals = Vec::new();
        // param 0 (unused, but present for a well-formed function)
        locals.push(MirLocal {
            id: LocalId(0),
            ty: HirType::I64,
            name: symbols.intern("return_slot"),
            mutable: false,
        });
        // destination local for the external call
        let dest_local = LocalId(1);
        locals.push(MirLocal {
            id: dest_local,
            ty: HirType::I64,
            name: symbols.intern("tmp"),
            mutable: false,
        });

        let block = MirBasicBlock {
            id: BlockId(0),
            name: "entry".to_string(),
            stmts: vec![MirStmt {
                kind: MirStmtKind::Assign {
                    local: dest_local,
                    rvalue,
                },
            }],
            terminator: MirTerminator::Return(Some(MirOperand::Local(dest_local))),
        };

        let func = MirFunction {
            def_id: DefId(0),
            name: main_name,
            params: vec![(symbols.intern("return_slot"), HirType::I64)],
            return_type: HirType::I64,
            locals,
            param_locals: vec![LocalId(0)],
            blocks: vec![block],
            start_block: BlockId(0),
        };

        MirProgram {
            functions: vec![func],
            symbols,
            structs: vec![],
            enums: vec![],
            external_libraries: vec![],
            python_search_path: String::new(),
            selected_backend: None,
        }
    }

    #[test]
    fn test_verify_accepts_external_call_with_matching_arg_count() {
        // A Python external call with Str arg — arg_count == arg_types.len() == 1
        let prog = build_program_with_rvalue(MirRvalue::ExternalCall {
            kind: crate::mir::MirExternalKind::Python,
            symbol: "json::dumps".to_string(),
            args: vec![MirOperand::Const(crate::mir::MirConst::I64(42))],
            arg_types: vec![MirType::I64],
            ret_ty: MirType::I64,
        });
        assert!(prog.verify().is_ok(), "valid ExternalCall should verify");
    }

    #[test]
    fn test_verify_accepts_external_call_with_str_arg() {
        let prog = build_program_with_rvalue(MirRvalue::ExternalCall {
            kind: crate::mir::MirExternalKind::Python,
            symbol: "builtins::len".to_string(),
            args: vec![MirOperand::Const(crate::mir::MirConst::I64(0))],
            arg_types: vec![MirType::Str],
            ret_ty: MirType::I64,
        });
        assert!(prog.verify().is_ok());
    }

    #[test]
    fn test_verify_accepts_external_call_with_bool_arg() {
        let prog = build_program_with_rvalue(MirRvalue::ExternalCall {
            kind: crate::mir::MirExternalKind::Python,
            symbol: "builtins::validate".to_string(),
            args: vec![MirOperand::Const(crate::mir::MirConst::Bool(true))],
            arg_types: vec![MirType::Bool],
            ret_ty: MirType::Unit,
        });
        assert!(prog.verify().is_ok());
    }

    #[test]
    fn test_verify_accepts_external_call_with_f64_arg() {
        let prog = build_program_with_rvalue(MirRvalue::ExternalCall {
            kind: crate::mir::MirExternalKind::Python,
            symbol: "math::sqrt".to_string(),
            args: vec![MirOperand::Const(crate::mir::MirConst::F64(81.0))],
            arg_types: vec![MirType::F64],
            ret_ty: MirType::F64,
        });
        assert!(prog.verify().is_ok());
    }

    #[test]
    fn test_verify_accepts_external_call_with_unit_return() {
        let prog = build_program_with_rvalue(MirRvalue::ExternalCall {
            kind: crate::mir::MirExternalKind::Python,
            symbol: "builtins::print".to_string(),
            args: vec![MirOperand::Const(crate::mir::MirConst::I64(42))],
            arg_types: vec![MirType::I64],
            ret_ty: MirType::Unit,
        });
        assert!(prog.verify().is_ok());
    }

    #[test]
    fn test_verify_accepts_external_call_with_zero_args() {
        let prog = build_program_with_rvalue(MirRvalue::ExternalCall {
            kind: crate::mir::MirExternalKind::Python,
            symbol: "random::random".to_string(),
            args: vec![],
            arg_types: vec![],
            ret_ty: MirType::I64,
        });
        assert!(prog.verify().is_ok());
    }

    #[test]
    fn test_verify_rejects_arg_count_mismatch() {
        // 2 args but only 1 arg_type — should fail.
        let prog = build_program_with_rvalue(MirRvalue::ExternalCall {
            kind: crate::mir::MirExternalKind::Python,
            symbol: "test::mismatch".to_string(),
            args: vec![
                MirOperand::Const(crate::mir::MirConst::I64(1)),
                MirOperand::Const(crate::mir::MirConst::I64(2)),
            ],
            arg_types: vec![MirType::I64],
            ret_ty: MirType::I64,
        });
        let result = prog.verify();
        assert!(result.is_err());
        let errs = result.unwrap_err();
        assert!(
            errs.iter().any(|e| e.message.contains("arg count")),
            "expected arg count mismatch error, got: {:?}",
            errs
        );
    }

    #[test]
    fn test_verify_rejects_more_arg_types_than_args() {
        // 1 arg but 2 arg_types — should fail.
        let prog = build_program_with_rvalue(MirRvalue::ExternalCall {
            kind: crate::mir::MirExternalKind::Python,
            symbol: "test::overtyped".to_string(),
            args: vec![MirOperand::Const(crate::mir::MirConst::I64(1))],
            arg_types: vec![MirType::I64, MirType::I64],
            ret_ty: MirType::I64,
        });
        let result = prog.verify();
        assert!(result.is_err());
        let errs = result.unwrap_err();
        assert!(
            errs.iter().any(|e| e.message.contains("arg count")),
            "expected arg count mismatch error, got: {:?}",
            errs
        );
    }

    #[test]
    fn test_verify_rejects_non_abi_safe_arg_type() {
        // Struct is not ABI-safe.
        let prog = build_program_with_rvalue(MirRvalue::ExternalCall {
            kind: crate::mir::MirExternalKind::Python,
            symbol: "test::struct_arg".to_string(),
            args: vec![MirOperand::Const(crate::mir::MirConst::I64(0))],
            arg_types: vec![MirType::Struct(SymbolId(0))],
            ret_ty: MirType::I64,
        });
        let result = prog.verify();
        assert!(result.is_err());
        let errs = result.unwrap_err();
        assert!(
            errs.iter().any(|e| e.message.contains("not ABI-safe")),
            "expected ABI-safe error for arg, got: {:?}",
            errs
        );
    }

    #[test]
    fn test_verify_rejects_non_abi_safe_return_type() {
        // List is not ABI-safe as a return type.
        let prog = build_program_with_rvalue(MirRvalue::ExternalCall {
            kind: crate::mir::MirExternalKind::Python,
            symbol: "test::list_ret".to_string(),
            args: vec![],
            arg_types: vec![],
            ret_ty: MirType::List(Box::new(MirType::I64)),
        });
        let result = prog.verify();
        assert!(result.is_err());
        let errs = result.unwrap_err();
        assert!(
            errs.iter().any(|e| e.message.contains("not ABI-safe")),
            "expected ABI-safe error for return type, got: {:?}",
            errs
        );
    }

    #[test]
    fn test_verify_rejects_enum_arg_type() {
        let prog = build_program_with_rvalue(MirRvalue::ExternalCall {
            kind: crate::mir::MirExternalKind::Rust,
            symbol: "test::enum_arg".to_string(),
            args: vec![MirOperand::Const(crate::mir::MirConst::I64(0))],
            arg_types: vec![MirType::Enum(SymbolId(0))],
            ret_ty: MirType::I64,
        });
        let result = prog.verify();
        assert!(result.is_err());
        let errs = result.unwrap_err();
        assert!(
            errs.iter().any(|e| e.message.contains("not ABI-safe")),
            "expected ABI-safe error for Enum arg, got: {:?}",
            errs
        );
    }

    #[test]
    fn test_verify_rejects_generic_arg_type() {
        let prog = build_program_with_rvalue(MirRvalue::ExternalCall {
            kind: crate::mir::MirExternalKind::Rust,
            symbol: "test::generic_arg".to_string(),
            args: vec![MirOperand::Const(crate::mir::MirConst::I64(0))],
            arg_types: vec![MirType::Generic(SymbolId(0))],
            ret_ty: MirType::I64,
        });
        let result = prog.verify();
        assert!(result.is_err());
        let errs = result.unwrap_err();
        assert!(
            errs.iter().any(|e| e.message.contains("not ABI-safe")),
            "expected ABI-safe error for Generic arg, got: {:?}",
            errs
        );
    }

    #[test]
    fn test_verify_accepts_rust_external_call() {
        let prog = build_program_with_rvalue(MirRvalue::ExternalCall {
            kind: crate::mir::MirExternalKind::Rust,
            symbol: "sat_serde_add".to_string(),
            args: vec![
                MirOperand::Const(crate::mir::MirConst::I64(1)),
                MirOperand::Const(crate::mir::MirConst::I64(2)),
            ],
            arg_types: vec![MirType::I64, MirType::I64],
            ret_ty: MirType::I64,
        });
        assert!(prog.verify().is_ok());
    }

    #[test]
    fn test_verify_accepts_native_external_call() {
        let prog = build_program_with_rvalue(MirRvalue::ExternalCall {
            kind: crate::mir::MirExternalKind::Native,
            symbol: "sat_libm_pow".to_string(),
            args: vec![
                MirOperand::Const(crate::mir::MirConst::F64(2.0)),
                MirOperand::Const(crate::mir::MirConst::F64(3.0)),
            ],
            arg_types: vec![MirType::F64, MirType::F64],
            ret_ty: MirType::F64,
        });
        assert!(prog.verify().is_ok());
    }

    #[test]
    fn test_verify_all_abi_safe_types_accepted() {
        // Verify that all five ABI-safe types pass verification.
        for ty in [
            MirType::I64,
            MirType::F64,
            MirType::Bool,
            MirType::Str,
            MirType::Unit,
        ] {
            let prog = build_program_with_rvalue(MirRvalue::ExternalCall {
                kind: crate::mir::MirExternalKind::Python,
                symbol: "test::all_types".to_string(),
                args: vec![],
                arg_types: vec![],
                ret_ty: ty.clone(),
            });
            assert!(
                prog.verify().is_ok(),
                "return type {:?} should be ABI-safe",
                ty
            );
        }
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
        MirRvalue::ExternalCall { args, .. } => args.clone(),
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
