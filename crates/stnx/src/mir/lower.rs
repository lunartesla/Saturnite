//! HIR → MIR lowering.
//!
//! Transforms a [`HirProgram`] into a [`MirProgram`] with an explicit
//! control-flow graph.  Every `if`, `for`, and `while` expression is lowered
//! into `SwitchInt` / `Goto` terminators and dedicated basic blocks, so that
//! LLVM codegen never needs to "rediscover" the CFG.
//!
//! ## Builder model
//!
//! Blocks are stored in a `Vec` whose indices correspond to `BlockId`s.
//! `current` tracks the index of the block being built.  `create_block`
//! allocates a new (empty, unterminated) block.  `switch_to` changes which
//! block is being built.  `finish` sets a terminator, closing the block.

use std::collections::HashMap;

use crate::error::{CompilerError, CompilerResult};
use crate::hir::expr::{HirExpr, HirExprKind};
use crate::hir::function::HirProgram;
use crate::hir::stmt::{HirStmt, HirStmtKind};
use crate::hir::symbol::{DefId, SymbolId};
use crate::hir::types::HirType;
use crate::mir::{
    BlockId, LocalId, MirBasicBlock, MirBinOp, MirConst, MirFunction, MirLocal, MirOperand,
    MirProgram, MirRvalue, MirStmt, MirStmtKind, MirTerminator, MirType,
};

/// `DefId` sentinel for the builtin `println` function.
/// Must match `hir::lower::PRINTLN_DEF_ID`.
const PRINTLN_DEF_ID: DefId = DefId(u32::MAX - 1);

/// `DefId` sentinel for the builtin `println_str` function (0.5 native
/// `say "..."` / `raise "..."`). Codegen maps this to the runtime
/// `println_str` function.
const PRINTLN_STR_DEF_ID: DefId = DefId(u32::MAX - 2);

/// Entry point: lower a `HirProgram` into a `MirProgram`.
pub fn lower_program(hir: &HirProgram) -> CompilerResult<MirProgram> {
    // Build a function signature table for call return-type resolution.
    let mut sigs: HashMap<DefId, (Vec<HirType>, HirType)> =
        HashMap::with_capacity(hir.functions.len());
    for func in &hir.functions {
        let param_types: Vec<HirType> = func.params.iter().map(|(_, t)| t.clone()).collect();
        sigs.insert(func.def_id, (param_types, func.return_type.clone()));
    }

    let mut funcs = Vec::new();
    for func in &hir.functions {
        let mut lower = MirLower::new(hir, func, &sigs);
        funcs.push(lower.lower_function()?);
    }

    Ok(MirProgram {
        functions: funcs,
        symbols: hir.symbols.clone(),
        structs: hir.structs.clone(),
        enums: hir.enums.clone(),
    })
}

/// Per-function lowering state.
pub struct MirLower<'hir> {
    hir: &'hir HirProgram,
    func: &'hir crate::hir::function::HirFunction,
    sigs: &'hir HashMap<DefId, (Vec<HirType>, HirType)>,
    /// All blocks (index == BlockId.0).
    blocks: Vec<MirBasicBlock>,
    /// Index into `blocks` of the block currently being built.
    current: usize,
    /// All locals (index == LocalId.0).
    locals: Vec<MirLocal>,
    /// Symbol name → local ID (current function scope).
    var_map: std::collections::HashMap<SymbolId, LocalId>,
    /// Symbol for compiler-generated temporaries.
    temp_symbol: SymbolId,
}

impl<'hir> MirLower<'hir> {
    pub fn new(
        hir: &'hir HirProgram,
        func: &'hir crate::hir::function::HirFunction,
        sigs: &'hir HashMap<DefId, (Vec<HirType>, HirType)>,
    ) -> Self {
        let mut symbols = hir.symbols.clone();
        let temp_symbol = symbols.intern("");
        Self {
            hir,
            func,
            sigs,
            blocks: Vec::new(),
            current: 0,
            locals: Vec::new(),
            var_map: std::collections::HashMap::new(),
            temp_symbol,
        }
    }

    fn new_local(&mut self, ty: MirType, name: SymbolId, mutable: bool) -> LocalId {
        let id = LocalId(self.locals.len() as u32);
        self.locals.push(MirLocal {
            id,
            ty,
            name,
            mutable,
        });
        id
    }

    /// Allocate a new block but do NOT make it current.
    fn create_block(&mut self, name: impl Into<String>) -> BlockId {
        let id = BlockId(self.blocks.len() as u32);
        self.blocks.push(MirBasicBlock {
            id,
            name: name.into(),
            stmts: Vec::new(),
            terminator: MirTerminator::Unreachable,
        });
        id
    }

    /// Create a new block and immediately make it current.
    fn start_block(&mut self, name: impl Into<String>) -> BlockId {
        let id = self.create_block(name);
        self.current = self.blocks.len() - 1;
        id
    }

    /// Make an existing block the current one (must be unterminated).
    fn switch_to(&mut self, block_id: BlockId) {
        self.current = block_id.0 as usize;
        debug_assert!(
            matches!(
                self.blocks[self.current].terminator,
                MirTerminator::Unreachable
            ),
            "switch_to: block {:?} is already terminated",
            block_id
        );
    }

    fn emit(&mut self, kind: MirStmtKind) {
        self.blocks[self.current].stmts.push(MirStmt { kind });
    }

    /// Close the current block with a terminator.
    fn finish(&mut self, terminator: MirTerminator) {
        self.blocks[self.current].terminator = terminator;
    }

    /// True if the current block already has a real terminator.
    fn current_closed(&self) -> bool {
        !matches!(
            self.blocks[self.current].terminator,
            MirTerminator::Unreachable
        )
    }

    /// If the current block is not yet terminated, emit a `Goto` to `target`.
    fn ensure_terminated(&mut self, target: BlockId) {
        if !self.current_closed() {
            self.finish(MirTerminator::Goto { target });
        }
    }

    fn lookup_var(&self, sym: SymbolId) -> Option<LocalId> {
        self.var_map.get(&sym).copied()
    }

    // -- Function lowering -----------------------------------------------

    pub fn lower_function(&mut self) -> CompilerResult<MirFunction> {
        self.start_block("entry");

        let mut param_locals: Vec<LocalId> = Vec::with_capacity(self.func.params.len());
        let mut param_types: Vec<(SymbolId, MirType)> = Vec::with_capacity(self.func.params.len());
        for (sym, ty) in &self.func.params {
            let lid = self.new_local(ty.clone(), *sym, false);
            self.var_map.insert(*sym, lid);
            param_locals.push(lid);
            param_types.push((*sym, ty.clone()));
        }

        let body = &self.func.body;
        if body.is_empty() {
            self.finish(MirTerminator::Return(None));
        } else {
            let (last, rest) = body.split_last().unwrap();
            for stmt in rest {
                self.lower_stmt(stmt)?;
            }
            match &last.kind {
                HirStmtKind::Expr(e) => {
                    let ret_val = self.lower_expr(e)?;
                    if !self.current_closed() {
                        self.finish(MirTerminator::Return(Some(ret_val)));
                    }
                }
                HirStmtKind::Return(_) => {
                    self.lower_stmt(last)?;
                }
                _ => {
                    self.lower_stmt(last)?;
                    if !self.current_closed() {
                        self.finish(MirTerminator::Return(None));
                    }
                }
            }
        }

        for block in &self.blocks {
            assert!(
                !matches!(block.terminator, MirTerminator::Unreachable),
                "block {:?} has no terminator",
                block.name
            );
        }

        Ok(MirFunction {
            def_id: self.func.def_id,
            name: self.func.name,
            params: param_types,
            return_type: self.func.return_type.clone(),
            locals: self.locals.clone(),
            param_locals,
            blocks: self.blocks.clone(),
            start_block: BlockId(0),
        })
    }

    // -- Statement lowering

    fn lower_stmt(&mut self, stmt: &HirStmt) -> CompilerResult<()> {
        match &stmt.kind {
            HirStmtKind::Let {
                name,
                mutable,
                ty,
                value,
            } => {
                let local_ty: MirType = ty.clone().unwrap_or_else(|| value.ty.clone());
                let local = self.new_local(local_ty.clone(), *name, *mutable);
                // Evaluate the initializer BEFORE updating var_map so that a
                // shadowing `let x = x + 1` correctly reads the *previous* local.
                let val = self.lower_expr(value)?;
                self.var_map.insert(*name, local);
                self.emit(MirStmtKind::LocalDecl {
                    local,
                    ty: local_ty,
                    mutable: *mutable,
                });
                self.emit(MirStmtKind::Assign {
                    local,
                    rvalue: MirRvalue::Use(val),
                });
                Ok(())
            }
            HirStmtKind::Expr(e) => {
                self.lower_expr(e)?;
                Ok(())
            }
            HirStmtKind::Return(opt_expr) => {
                let operand = if let Some(e) = opt_expr {
                    Some(self.lower_expr(e)?)
                } else {
                    None
                };
                if !self.current_closed() {
                    self.finish(MirTerminator::Return(operand));
                }
                Ok(())
            }
            HirStmtKind::Println(e) => {
                let val = self.lower_expr(e)?;
                let dest = self.new_local(MirType::I64, self.temp_symbol, false);
                self.emit(MirStmtKind::LocalDecl {
                    local: dest,
                    ty: MirType::I64,
                    mutable: false,
                });
                let next = self.create_block(format!("println_cont_{}", self.blocks.len()));
                self.finish(MirTerminator::Call {
                    func: PRINTLN_DEF_ID,
                    args: vec![val],
                    destination: dest,
                    next,
                });
                self.switch_to(next);
                Ok(())
            }
            HirStmtKind::PrintlnStr(e) => {
                let val = self.lower_expr(e)?;
                let dest = self.new_local(MirType::I64, self.temp_symbol, false);
                self.emit(MirStmtKind::LocalDecl {
                    local: dest,
                    ty: MirType::I64,
                    mutable: false,
                });
                let next = self.create_block(format!("println_str_cont_{}", self.blocks.len()));
                self.finish(MirTerminator::Call {
                    func: PRINTLN_STR_DEF_ID,
                    args: vec![val],
                    destination: dest,
                    next,
                });
                self.switch_to(next);
                Ok(())
            }
            HirStmtKind::StructDef { .. } | HirStmtKind::EnumDef { .. } => Ok(()),
            // 0.5: `raise expr` lowers to a stub — print the expression
            // (which is expected to be a StrLit in 0.5) and then emit an
            // Unreachable terminator. The LLVM backend will map Unreachable
            // in a position with no Return to a trap. Real error semantics
            // are deferred.
            HirStmtKind::Raise(e) => {
                let val = self.lower_expr(e)?;
                let dest = self.new_local(MirType::I64, self.temp_symbol, false);
                self.emit(MirStmtKind::LocalDecl {
                    local: dest,
                    ty: MirType::I64,
                    mutable: false,
                });
                let next = self.create_block(format!("raise_cont_{}", self.blocks.len()));
                // String messages go to the string printer; numeric tags to
                // the i64 printer.
                let func = if e.ty == HirType::Str {
                    PRINTLN_STR_DEF_ID
                } else {
                    PRINTLN_DEF_ID
                };
                self.finish(MirTerminator::Call {
                    func,
                    args: vec![val],
                    destination: dest,
                    next,
                });
                self.switch_to(next);
                // After printing, mark the block as unreachable.
                self.finish(MirTerminator::Unreachable);
                Ok(())
            }
        }
    }

    // -- Expression lowering

    fn lower_expr(&mut self, expr: &HirExpr) -> CompilerResult<MirOperand> {
        match &expr.kind {
            HirExprKind::Integer(n) => Ok(MirOperand::Const(MirConst::I64(*n))),
            HirExprKind::Float(f) => Ok(MirOperand::Const(MirConst::F64(*f))),
            HirExprKind::Bool(b) => Ok(MirOperand::Const(MirConst::Bool(*b))),

            HirExprKind::StrLit(sym) => {
                let local = self.new_local(MirType::Str, self.temp_symbol, false);
                self.emit(MirStmtKind::LocalDecl {
                    local,
                    ty: MirType::Str,
                    mutable: false,
                });
                self.emit(MirStmtKind::Assign {
                    local,
                    rvalue: MirRvalue::StrLit(*sym),
                });
                Ok(MirOperand::Local(local))
            }

            HirExprKind::Unit => Ok(MirOperand::Const(MirConst::I64(0))),

            HirExprKind::Variable { symbol } => {
                let lid = self.lookup_var(*symbol).ok_or_else(|| {
                    let name = self.hir.symbols.lookup(*symbol).unwrap_or("?");
                    CompilerError::codegen(format!("undefined variable: {}", name))
                })?;
                Ok(MirOperand::Local(lid))
            }

            HirExprKind::Assign { symbol, value } => {
                let target_local = self.lookup_var(*symbol).ok_or_else(|| {
                    let name = self.hir.symbols.lookup(*symbol).unwrap_or("?");
                    CompilerError::codegen(format!("undefined variable: {}", name))
                })?;
                let val = self.lower_expr(value)?;
                self.emit(MirStmtKind::Assign {
                    local: target_local,
                    rvalue: MirRvalue::Use(val),
                });
                Ok(MirOperand::Local(target_local))
            }

            HirExprKind::AugAssign { symbol, op, value } => {
                let target_local = self.lookup_var(*symbol).ok_or_else(|| {
                    let name = self.hir.symbols.lookup(*symbol).unwrap_or("?");
                    CompilerError::codegen(format!("undefined variable: {}", name))
                })?;
                let val = self.lower_expr(value)?;
                self.emit(MirStmtKind::Assign {
                    local: target_local,
                    rvalue: MirRvalue::Binary {
                        op: (*op).into(),
                        lhs: MirOperand::Local(target_local),
                        rhs: val,
                    },
                });
                Ok(MirOperand::Local(target_local))
            }

            HirExprKind::Binary { op, lhs, rhs } => {
                let l = self.lower_expr(lhs)?;
                let r = self.lower_expr(rhs)?;
                let result_local = self.new_local(expr.ty.clone(), self.temp_symbol, false);
                self.emit(MirStmtKind::LocalDecl {
                    local: result_local,
                    ty: expr.ty.clone(),
                    mutable: false,
                });
                self.emit(MirStmtKind::Assign {
                    local: result_local,
                    rvalue: MirRvalue::Binary {
                        op: (*op).into(),
                        lhs: l,
                        rhs: r,
                    },
                });
                Ok(MirOperand::Local(result_local))
            }

            HirExprKind::Unary { op, expr: inner } => {
                let val = self.lower_expr(inner)?;
                let result_local = self.new_local(expr.ty.clone(), self.temp_symbol, false);
                self.emit(MirStmtKind::LocalDecl {
                    local: result_local,
                    ty: expr.ty.clone(),
                    mutable: false,
                });
                self.emit(MirStmtKind::Assign {
                    local: result_local,
                    rvalue: MirRvalue::Unary {
                        op: (*op).into(),
                        operand: val,
                    },
                });
                Ok(MirOperand::Local(result_local))
            }

            HirExprKind::Call {
                func: def_id, args, ..
            } => {
                // By this point monomorphization has already retargeted
                // generic call sites to their concrete instantiations,
                // and the explicit turbofish has been folded into the
                // substituted callee's signature. We only need the
                // resolved DefId here.
                self.lower_call(*def_id, args, expr.ty.clone())
            }

            HirExprKind::If {
                condition,
                then_branch,
                elif_branches,
                else_branch,
            } => {
                self.lower_if(condition, then_branch, elif_branches, else_branch)?;
                Ok(MirOperand::Const(MirConst::I64(0)))
            }

            HirExprKind::For { var, iter, body } => {
                self.lower_for(*var, iter, body)?;
                Ok(MirOperand::Const(MirConst::I64(0)))
            }

            HirExprKind::While { condition, body } => {
                self.lower_while(condition, body)?;
                Ok(MirOperand::Const(MirConst::I64(0)))
            }

            HirExprKind::Range { start, .. } => self.lower_expr(start),

            HirExprKind::StructLiteral {
                name,
                fields,
                type_args: _,
            } => {
                let mut field_ops: Vec<(SymbolId, MirOperand)> = Vec::new();
                for (fid, fexpr) in fields {
                    field_ops.push((*fid, self.lower_expr(fexpr)?));
                }
                let local = self.new_local(expr.ty.clone(), self.temp_symbol, false);
                self.emit(MirStmtKind::LocalDecl {
                    local,
                    ty: expr.ty.clone(),
                    mutable: false,
                });
                self.emit(MirStmtKind::Assign {
                    local,
                    rvalue: MirRvalue::StructLit {
                        struct_def: *name,
                        fields: field_ops,
                    },
                });
                Ok(MirOperand::Local(local))
            }

            HirExprKind::FieldAccess { expr: inner, field } => {
                let inner_val = self.lower_expr(inner)?;
                let inner_local = self.new_local(inner.ty.clone(), self.temp_symbol, false);
                self.emit(MirStmtKind::LocalDecl {
                    local: inner_local,
                    ty: inner.ty.clone(),
                    mutable: false,
                });
                self.emit(MirStmtKind::Assign {
                    local: inner_local,
                    rvalue: MirRvalue::Use(inner_val),
                });
                let result_local = self.new_local(expr.ty.clone(), self.temp_symbol, false);
                self.emit(MirStmtKind::LocalDecl {
                    local: result_local,
                    ty: expr.ty.clone(),
                    mutable: false,
                });
                self.emit(MirStmtKind::Assign {
                    local: result_local,
                    rvalue: MirRvalue::FieldAccess {
                        local: inner_local,
                        field: *field,
                    },
                });
                Ok(MirOperand::Local(result_local))
            }

            HirExprKind::EnumConstructor { name, variant } => {
                let enum_def = self.hir.enum_def(*name).ok_or_else(|| {
                    let name_str = self.hir.symbols.lookup(*name).unwrap_or("?");
                    CompilerError::semantic(format!("undefined enum: {}", name_str))
                })?;
                let variant_idx = enum_def
                    .variants
                    .iter()
                    .position(|v| *v == *variant)
                    .ok_or_else(|| {
                        let name_str = self.hir.symbols.lookup(*name).unwrap_or("?");
                        let var_str = self.hir.symbols.lookup(*variant).unwrap_or("?");
                        CompilerError::semantic(format!(
                            "enum {} has no variant {}",
                            name_str, var_str
                        ))
                    })?;
                Ok(MirOperand::Const(MirConst::I64(variant_idx as i64)))
            }

            HirExprKind::Index { list, index } => {
                // Lower the list expression first, then the index expression.
                let list_val = self.lower_expr(list)?;
                let list_local = self.new_local(list.ty.clone(), self.temp_symbol, false);
                self.emit(MirStmtKind::LocalDecl {
                    local: list_local,
                    ty: list.ty.clone(),
                    mutable: false,
                });
                self.emit(MirStmtKind::Assign {
                    local: list_local,
                    rvalue: MirRvalue::Use(list_val),
                });
                let idx = self.lower_expr(index)?;
                let result_local = self.new_local(expr.ty.clone(), self.temp_symbol, false);
                self.emit(MirStmtKind::LocalDecl {
                    local: result_local,
                    ty: expr.ty.clone(),
                    mutable: false,
                });
                self.emit(MirStmtKind::Assign {
                    local: result_local,
                    rvalue: MirRvalue::Index {
                        list_local,
                        index: idx,
                    },
                });
                Ok(MirOperand::Local(result_local))
            }

            HirExprKind::Length { expr: inner } => {
                let inner_val = self.lower_expr(inner)?;
                let inner_local = self.new_local(inner.ty.clone(), self.temp_symbol, false);
                self.emit(MirStmtKind::LocalDecl {
                    local: inner_local,
                    ty: inner.ty.clone(),
                    mutable: false,
                });
                self.emit(MirStmtKind::Assign {
                    local: inner_local,
                    rvalue: MirRvalue::Use(inner_val),
                });
                let result_local = self.new_local(expr.ty.clone(), self.temp_symbol, false);
                self.emit(MirStmtKind::LocalDecl {
                    local: result_local,
                    ty: expr.ty.clone(),
                    mutable: false,
                });
                self.emit(MirStmtKind::Assign {
                    local: result_local,
                    rvalue: MirRvalue::Length {
                        list_local: inner_local,
                    },
                });
                Ok(MirOperand::Local(result_local))
            }

            HirExprKind::ListLiteral { elements } => {
                // 0.5.3: lower each element left-to-right, collecting operands.
                // The result local holds a `List<I64>`; codegen lowers the
                // rvalue to a runtime `list_new_from` call.
                let mut element_ops: Vec<MirOperand> = Vec::with_capacity(elements.len());
                for elem in elements {
                    let op = self.lower_expr(elem)?;
                    element_ops.push(op);
                }
                let list_ty = MirType::List(Box::new(MirType::I64));
                let list_local = self.new_local(list_ty.clone(), self.temp_symbol, false);
                self.emit(MirStmtKind::LocalDecl {
                    local: list_local,
                    ty: list_ty,
                    mutable: false,
                });
                self.emit(MirStmtKind::Assign {
                    local: list_local,
                    rvalue: MirRvalue::ListLiteral {
                        elements: element_ops,
                    },
                });
                Ok(MirOperand::Local(list_local))
            }
        }
    }

    // -- Call lowering ---------------------------------------------------

    /// Lower a function call.  The Call is a *terminator*: it ends the current
    /// block and starts a new continuation block.  Returns the destination
    /// local as a `Local` operand (valid in the continuation block).
    fn lower_call(
        &mut self,
        def_id: DefId,
        args: &[HirExpr],
        result_ty: MirType,
    ) -> CompilerResult<MirOperand> {
        let mut arg_ops: Vec<MirOperand> = Vec::with_capacity(args.len());
        for arg in args {
            arg_ops.push(self.lower_expr(arg)?);
        }

        let ret_ty = self
            .sigs
            .get(&def_id)
            .map(|(_, ret)| ret.clone())
            .unwrap_or(result_ty);

        let dest = self.new_local(ret_ty.clone(), self.temp_symbol, false);
        self.emit(MirStmtKind::LocalDecl {
            local: dest,
            ty: ret_ty,
            mutable: false,
        });

        let next = self.create_block(format!("call_cont_{}", self.blocks.len()));
        self.finish(MirTerminator::Call {
            func: def_id,
            args: arg_ops,
            destination: dest,
            next,
        });
        self.switch_to(next);

        Ok(MirOperand::Local(dest))
    }

    // -- Control-flow lowering

    /// Lower an `if` / `elif` / `else` expression into a CFG.
    fn lower_if(
        &mut self,
        condition: &HirExpr,
        then_branch: &[HirStmt],
        elif_branches: &[(HirExpr, Vec<HirStmt>)],
        else_branch: &Option<Vec<HirStmt>>,
    ) -> CompilerResult<()> {
        let then_bb = self.create_block("if_then");
        let end_bb = self.create_block("if_end");

        let mut elif_conds: Vec<BlockId> = Vec::new();
        let mut elif_bodies: Vec<BlockId> = Vec::new();
        for i in 0..elif_branches.len() {
            elif_conds.push(self.create_block(format!("elif{}_cond", i)));
            elif_bodies.push(self.create_block(format!("elif{}_body", i)));
        }
        let else_bb = self.create_block("if_else");

        let cond_val = self.lower_expr(condition)?;
        let next_cond = elif_conds.first().copied().unwrap_or(else_bb);
        self.finish(MirTerminator::SwitchInt {
            scrutinee: cond_val,
            ty: MirType::Bool,
            branches: vec![(1, then_bb)],
            else_target: next_cond,
        });

        // then body
        self.switch_to(then_bb);
        for s in then_branch {
            self.lower_stmt(s)?;
        }
        self.ensure_terminated(end_bb);

        // elif chain
        for i in 0..elif_branches.len() {
            let cond_expr = &elif_branches[i].0;
            let body = &elif_branches[i].1;
            self.switch_to(elif_conds[i]);

            let elif_cond_val = self.lower_expr(cond_expr)?;
            let elif_else = if i + 1 < elif_conds.len() {
                elif_conds[i + 1]
            } else {
                else_bb
            };
            self.finish(MirTerminator::SwitchInt {
                scrutinee: elif_cond_val,
                ty: MirType::Bool,
                branches: vec![(1, elif_bodies[i])],
                else_target: elif_else,
            });

            self.switch_to(elif_bodies[i]);
            for s in body {
                self.lower_stmt(s)?;
            }
            self.ensure_terminated(end_bb);
        }

        // else body
        self.switch_to(else_bb);
        if let Some(else_body) = else_branch {
            for s in else_body {
                self.lower_stmt(s)?;
            }
        }
        self.ensure_terminated(end_bb);

        self.switch_to(end_bb);
        Ok(())
    }

    /// Lower a `for var in start..end` (or `start...end` inclusive) loop.
    fn lower_for(&mut self, var: SymbolId, iter: &HirExpr, body: &[HirStmt]) -> CompilerResult<()> {
        let (start_expr, end_expr, is_inclusive) = match &iter.kind {
            HirExprKind::Range {
                start,
                end,
                is_inclusive,
            } => (start.as_ref(), end.as_ref(), *is_inclusive),
            _ => {
                return Err(CompilerError::semantic(
                    "for loop iterator must be a range expression".to_string(),
                ));
            }
        };

        // Lower range bounds into locals.
        let start_val = self.lower_expr(start_expr)?;
        let start_local = self.new_local(MirType::I64, self.temp_symbol, false);
        self.emit(MirStmtKind::LocalDecl {
            local: start_local,
            ty: MirType::I64,
            mutable: false,
        });
        self.emit(MirStmtKind::Assign {
            local: start_local,
            rvalue: MirRvalue::Use(start_val),
        });

        let end_val = self.lower_expr(end_expr)?;
        let end_local = self.new_local(MirType::I64, self.temp_symbol, false);
        self.emit(MirStmtKind::LocalDecl {
            local: end_local,
            ty: MirType::I64,
            mutable: false,
        });
        self.emit(MirStmtKind::Assign {
            local: end_local,
            rvalue: MirRvalue::Use(end_val),
        });

        // Loop variable (mutable).
        let loop_var = self.new_local(MirType::I64, var, true);

        let cond_bb = self.create_block("for_cond");
        let body_bb = self.create_block("for_body");
        let exit_bb = self.create_block("for_exit");

        // Init loop_var once, then goto cond.
        self.emit(MirStmtKind::LocalDecl {
            local: loop_var,
            ty: MirType::I64,
            mutable: true,
        });
        self.emit(MirStmtKind::Assign {
            local: loop_var,
            rvalue: MirRvalue::Use(MirOperand::Local(start_local)),
        });
        self.finish(MirTerminator::Goto { target: cond_bb });

        // cond_bb: check loop_var < end_local
        self.switch_to(cond_bb);
        let cmp_op = if is_inclusive {
            MirBinOp::Le
        } else {
            MirBinOp::Lt
        };
        let cond_local = self.new_local(MirType::Bool, self.temp_symbol, false);
        self.emit(MirStmtKind::LocalDecl {
            local: cond_local,
            ty: MirType::Bool,
            mutable: false,
        });
        self.emit(MirStmtKind::Assign {
            local: cond_local,
            rvalue: MirRvalue::Binary {
                op: cmp_op,
                lhs: MirOperand::Local(loop_var),
                rhs: MirOperand::Local(end_local),
            },
        });
        self.finish(MirTerminator::SwitchInt {
            scrutinee: MirOperand::Local(cond_local),
            ty: MirType::Bool,
            branches: vec![(1, body_bb)],
            else_target: exit_bb,
        });

        // body_bb: bind var, lower body, increment, goto cond
        self.switch_to(body_bb);
        self.var_map.insert(var, loop_var);
        for s in body {
            self.lower_stmt(s)?;
        }
        self.emit(MirStmtKind::Assign {
            local: loop_var,
            rvalue: MirRvalue::Binary {
                op: MirBinOp::Add,
                lhs: MirOperand::Local(loop_var),
                rhs: MirOperand::Const(MirConst::I64(1)),
            },
        });
        self.ensure_terminated(cond_bb);

        self.switch_to(exit_bb);
        Ok(())
    }

    /// Lower a `while condition` loop.
    fn lower_while(&mut self, condition: &HirExpr, body: &[HirStmt]) -> CompilerResult<()> {
        let cond_bb = self.create_block("while_cond");
        let body_bb = self.create_block("while_body");
        let exit_bb = self.create_block("while_exit");

        self.finish(MirTerminator::Goto { target: cond_bb });

        self.switch_to(cond_bb);
        let cond_val = self.lower_expr(condition)?;
        self.finish(MirTerminator::SwitchInt {
            scrutinee: cond_val,
            ty: MirType::Bool,
            branches: vec![(1, body_bb)],
            else_target: exit_bb,
        });

        self.switch_to(body_bb);
        for s in body {
            self.lower_stmt(s)?;
        }
        self.ensure_terminated(cond_bb);

        self.switch_to(exit_bb);
        Ok(())
    }
}
