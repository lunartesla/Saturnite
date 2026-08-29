//! MIR -> LLVM IR code generation backend.
//!
//! Consumes a [`MirProgram`] (produced by [`crate::mir::lower::lower_program`])
//! and emits LLVM IR.  MIR owns the CFG; this backend translates each
//! [`MirBasicBlock`], [`MirStmtKind`], and [`MirTerminator`] into the
//! corresponding LLVM IR construct.

use crate::error::{CompilerError, CompilerResult};
use crate::hir::types::HirType;
use crate::mir::{
    BlockId, LocalId, MirBinOp, MirConst, MirFunction, MirOperand, MirProgram, MirRvalue,
    MirStmtKind, MirTerminator, MirType, MirUnOp,
};
use crate::target::TargetConfig;
use inkwell::builder::Builder as IRBuilder;
use inkwell::context::Context as LLVMContext;
use inkwell::passes::PassBuilderOptions;
use inkwell::types::BasicType;
use inkwell::types::BasicTypeEnum;
use inkwell::values::{BasicValue, BasicValueEnum, PointerValue};
use inkwell::FloatPredicate;
use inkwell::IntPredicate;
use inkwell::OptimizationLevel as InkwellOptLevel;
use std::collections::HashMap;

/// DefId sentinel for the builtin `println` function.
const PRINTLN_DEF_ID: crate::hir::symbol::DefId = crate::hir::symbol::DefId(u32::MAX - 1);

/// A local alloca plus its LLVM type (needed for loading with the right type).
type AllocaInfo<'ctx> = (PointerValue<'ctx>, BasicTypeEnum<'ctx>);

/// A backend that translates a `MirProgram` into LLVM IR.
pub struct MirCodeGenContext<'ctx> {
    pub context: &'ctx LLVMContext,
    pub module: inkwell::module::Module<'ctx>,
    pub builder: IRBuilder<'ctx>,
    /// Per-function local -> (alloca pointer, llvm type).
    local_allocas: HashMap<LocalId, AllocaInfo<'ctx>>,
}

impl<'ctx> MirCodeGenContext<'ctx> {
    pub fn new(context: &'ctx LLVMContext) -> Self {
        let module = context.create_module("saturnite_mir");
        let builder = context.create_builder();
        Self {
            context,
            module,
            builder,
            local_allocas: HashMap::new(),
        }
    }

    /// Build the LLVM `FunctionType` for a MIR function, using the function's
    /// actual `return_type` (including `void` for `Unit`) rather than always
    /// defaulting to `i64`.
    ///
    /// Previously the return type was hardcoded as `i64`, which produced
    /// invalid LLVM IR for functions returning `bool` (`i1`) or `f64` — the
    /// function signature said `i64` but the `ret` instruction emitted the real
    /// type, causing undefined behaviour at call sites (stack corruption,
    /// segfaults, hangs).
    fn make_fn_type(
        &self,
        func: &MirFunction,
        prog: &MirProgram,
    ) -> inkwell::types::FunctionType<'ctx> {
        let param_types: Vec<BasicTypeEnum<'ctx>> = func
            .params
            .iter()
            .map(|(_, ty)| mir_type_to_llvm(self.context, prog, ty))
            .collect();
        if matches!(func.return_type, HirType::Unit) {
            self.context.void_type().fn_type(
                &param_types
                    .iter()
                    .map(|t| t.as_basic_type_enum().into())
                    .collect::<Vec<_>>(),
                false,
            )
        } else {
            let ret_ty = mir_type_to_llvm(self.context, prog, &func.return_type);
            ret_ty.fn_type(
                &param_types
                    .iter()
                    .map(|t| t.as_basic_type_enum().into())
                    .collect::<Vec<_>>(),
                false,
            )
        }
    }

    /// Declare builtin functions (e.g. `println_i64`) in the module.
    pub fn declare_builtin_functions(&mut self) {
        let i64_ty = self.context.i64_type();
        self.module
            .add_function("println_i64", i64_ty.fn_type(&[i64_ty.into()], false), None);
    }

    /// Declare all functions from the MIR program into the module.
    pub fn declare_functions(&mut self, prog: &MirProgram) -> CompilerResult<()> {
        for func in &prog.functions {
            if func.def_id == PRINTLN_DEF_ID {
                continue;
            }
            let name = prog
                .symbols
                .lookup(func.name)
                .ok_or_else(|| CompilerError::codegen("missing symbol for function name"))?;
            let fn_ty = self.make_fn_type(func, prog);
            self.module.add_function(name, fn_ty, None);
        }
        Ok(())
    }

    /// Generate LLVM IR for a single MIR function.
    pub fn generate_function(
        &mut self,
        func: &MirFunction,
        prog: &MirProgram,
    ) -> CompilerResult<()> {
        let name = prog
            .symbols
            .lookup(func.name)
            .ok_or_else(|| CompilerError::codegen("missing symbol for function name"))?;

        let function_value = self.module.get_function(name).unwrap_or_else(|| {
            let fn_ty = self.make_fn_type(func, prog);
            self.module.add_function(name, fn_ty, None)
        });

        // Reset local state for this function.
        self.local_allocas.clear();

        // Create LLVM basic blocks for each MIR block.
        let mut llvm_blocks: HashMap<BlockId, inkwell::basic_block::BasicBlock<'ctx>> =
            HashMap::new();
        for block in &func.blocks {
            let bb = self.context.append_basic_block(function_value, &block.name);
            llvm_blocks.insert(block.id, bb);
        }

        // Create allocas for all locals and store params in start block.
        let start_bb = llvm_blocks[&func.start_block];
        self.builder.position_at_end(start_bb);

        for local in &func.locals {
            let ty = mir_type_to_llvm(self.context, prog, &local.ty);
            let alloca = self
                .builder
                .build_alloca(ty, &format!("_{}", local.id.0))
                .unwrap();
            self.local_allocas.insert(local.id, (alloca, ty));
        }

        // Store parameters into allocas.
        for (param_idx, param_lid) in func.param_locals.iter().enumerate() {
            let llvm_param = function_value.get_nth_param(param_idx as u32).unwrap();
            if let Some((alloca, _)) = self.local_allocas.get(param_lid) {
                self.builder.build_store(*alloca, llvm_param).unwrap();
            }
        }

        // Walk each MIR block and generate instructions.
        for block in &func.blocks {
            let bb = llvm_blocks[&block.id];
            self.builder.position_at_end(bb);

            for stmt in &block.stmts {
                self.gen_stmt(stmt, prog)?;
            }

            self.gen_terminator(&block.terminator, func, prog, &llvm_blocks)?;
        }

        Ok(())
    }

    fn gen_stmt(&mut self, stmt: &crate::mir::MirStmt, prog: &MirProgram) -> CompilerResult<()> {
        match &stmt.kind {
            MirStmtKind::LocalDecl { .. } => {
                // Alloca created at function entry.
            }
            MirStmtKind::Assign { local, rvalue } => {
                let val = self.gen_rvalue(rvalue, prog)?;
                if let Some((alloca, _)) = self.local_allocas.get(local) {
                    self.builder.build_store(*alloca, val).unwrap();
                }
            }
        }
        Ok(())
    }

    fn gen_rvalue(
        &mut self,
        rvalue: &MirRvalue,
        prog: &MirProgram,
    ) -> CompilerResult<BasicValueEnum<'ctx>> {
        match rvalue {
            MirRvalue::Use(operand) => self.materialize_operand(operand),
            MirRvalue::Binary { op, lhs, rhs } => {
                let l = self.materialize_operand(lhs)?;
                let r = self.materialize_operand(rhs)?;
                Ok(self.gen_binop(*op, l, r)?)
            }
            MirRvalue::Unary { op, operand } => {
                let val = self.materialize_operand(operand)?;
                Ok(self.gen_unop(*op, val)?)
            }
            MirRvalue::StructLit { struct_def, fields } => {
                self.gen_struct_lit(*struct_def, fields, prog)
            }
            MirRvalue::FieldAccess { local, field } => self.gen_field_access(*local, *field, prog),
            MirRvalue::EnumCtor { enum_def, variant } => {
                let edef = prog
                    .enum_def(*enum_def)
                    .ok_or_else(|| CompilerError::codegen("undefined enum"))?;
                let idx = edef
                    .variants
                    .iter()
                    .position(|v| *v == *variant)
                    .ok_or_else(|| CompilerError::codegen("undefined enum variant"))?;
                Ok(self
                    .context
                    .i64_type()
                    .const_int(idx as u64, false)
                    .as_basic_value_enum())
            }
            MirRvalue::StrLit(sym) => {
                let s = prog
                    .symbols
                    .lookup(*sym)
                    .ok_or_else(|| CompilerError::codegen("undefined string symbol"))?;
                let gv = self.context.const_string(s.as_bytes(), false);
                Ok(gv.as_basic_value_enum())
            }
        }
    }

    fn materialize_operand(
        &mut self,
        operand: &MirOperand,
    ) -> CompilerResult<BasicValueEnum<'ctx>> {
        match operand {
            MirOperand::Const(MirConst::I64(n)) => Ok(self
                .context
                .i64_type()
                .const_int(*n as u64, true)
                .as_basic_value_enum()),
            MirOperand::Const(MirConst::F64(f)) => Ok(self
                .context
                .f64_type()
                .const_float(*f)
                .as_basic_value_enum()),
            MirOperand::Const(MirConst::Bool(b)) => {
                let v: u64 = if *b { 1 } else { 0 };
                Ok(self
                    .context
                    .bool_type()
                    .const_int(v, false)
                    .as_basic_value_enum())
            }
            MirOperand::Local(lid) => {
                let (alloca, llvm_ty) = self
                    .local_allocas
                    .get(lid)
                    .ok_or_else(|| CompilerError::codegen(format!("local {:?} not found", lid)))?;
                let loaded = self.builder.build_load(*llvm_ty, *alloca, "load").unwrap();
                Ok(loaded.as_basic_value_enum())
            }
        }
    }

    fn gen_binop(
        &self,
        op: MirBinOp,
        lhs: BasicValueEnum<'ctx>,
        rhs: BasicValueEnum<'ctx>,
    ) -> CompilerResult<BasicValueEnum<'ctx>> {
        let lhs_ty = lhs.get_type();
        if lhs_ty.is_int_type() {
            let lhs_int = lhs.into_int_value();
            let rhs_int = rhs.into_int_value();
            Ok(self.gen_integer_binop(op, lhs_int, rhs_int))
        } else if lhs_ty.is_float_type() {
            let lhs_float = lhs.into_float_value();
            let rhs_float = rhs.into_float_value();
            self.gen_float_binop(op, lhs_float, rhs_float)
        } else {
            Err(CompilerError::codegen(format!(
                "unsupported operand type for binop {:?}: {:?}",
                op, lhs_ty
            )))
        }
    }

    fn gen_integer_binop(
        &self,
        op: MirBinOp,
        lhs: inkwell::values::IntValue<'ctx>,
        rhs: inkwell::values::IntValue<'ctx>,
    ) -> BasicValueEnum<'ctx> {
        match op {
            MirBinOp::Add => self
                .builder
                .build_int_add(lhs, rhs, "add")
                .unwrap()
                .as_basic_value_enum(),
            MirBinOp::Sub => self
                .builder
                .build_int_sub(lhs, rhs, "sub")
                .unwrap()
                .as_basic_value_enum(),
            MirBinOp::Mul => self
                .builder
                .build_int_mul(lhs, rhs, "mul")
                .unwrap()
                .as_basic_value_enum(),
            MirBinOp::Div => self
                .builder
                .build_int_unsigned_div(lhs, rhs, "div")
                .unwrap()
                .as_basic_value_enum(),
            MirBinOp::Mod => self
                .builder
                .build_int_unsigned_rem(lhs, rhs, "rem")
                .unwrap()
                .as_basic_value_enum(),
            MirBinOp::Eq => self
                .builder
                .build_int_compare(IntPredicate::EQ, lhs, rhs, "eq")
                .unwrap()
                .as_basic_value_enum(),
            MirBinOp::Ne => self
                .builder
                .build_int_compare(IntPredicate::NE, lhs, rhs, "ne")
                .unwrap()
                .as_basic_value_enum(),
            MirBinOp::Lt => self
                .builder
                .build_int_compare(IntPredicate::ULT, lhs, rhs, "lt")
                .unwrap()
                .as_basic_value_enum(),
            MirBinOp::Gt => self
                .builder
                .build_int_compare(IntPredicate::UGT, lhs, rhs, "gt")
                .unwrap()
                .as_basic_value_enum(),
            MirBinOp::Le => self
                .builder
                .build_int_compare(IntPredicate::ULE, lhs, rhs, "le")
                .unwrap()
                .as_basic_value_enum(),
            MirBinOp::Ge => self
                .builder
                .build_int_compare(IntPredicate::UGE, lhs, rhs, "ge")
                .unwrap()
                .as_basic_value_enum(),
            MirBinOp::And => self
                .builder
                .build_and(lhs, rhs, "and")
                .unwrap()
                .as_basic_value_enum(),
            MirBinOp::Or => self
                .builder
                .build_or(lhs, rhs, "or")
                .unwrap()
                .as_basic_value_enum(),
        }
    }

    fn gen_float_binop(
        &self,
        op: MirBinOp,
        lhs: inkwell::values::FloatValue<'ctx>,
        rhs: inkwell::values::FloatValue<'ctx>,
    ) -> CompilerResult<BasicValueEnum<'ctx>> {
        match op {
            MirBinOp::Add => Ok(self
                .builder
                .build_float_add(lhs, rhs, "fadd")
                .unwrap()
                .as_basic_value_enum()),
            MirBinOp::Sub => Ok(self
                .builder
                .build_float_sub(lhs, rhs, "fsub")
                .unwrap()
                .as_basic_value_enum()),
            MirBinOp::Mul => Ok(self
                .builder
                .build_float_mul(lhs, rhs, "fmul")
                .unwrap()
                .as_basic_value_enum()),
            MirBinOp::Div => Ok(self
                .builder
                .build_float_div(lhs, rhs, "fdiv")
                .unwrap()
                .as_basic_value_enum()),
            // Mod is not currently supported for floating-point types.
            MirBinOp::Mod => Err(CompilerError::codegen(
                "floating-point modulo is not supported",
            )),
            // Comparisons use ordered floating-point predicates (O-series),
            // which return false for NaN comparisons, matching standard
            // IEEE 754 ordered comparison semantics.
            MirBinOp::Eq => Ok(self
                .builder
                .build_float_compare(FloatPredicate::OEQ, lhs, rhs, "feq")
                .unwrap()
                .as_basic_value_enum()),
            MirBinOp::Ne => Ok(self
                .builder
                .build_float_compare(FloatPredicate::ONE, lhs, rhs, "fne")
                .unwrap()
                .as_basic_value_enum()),
            MirBinOp::Lt => Ok(self
                .builder
                .build_float_compare(FloatPredicate::OLT, lhs, rhs, "flt")
                .unwrap()
                .as_basic_value_enum()),
            MirBinOp::Gt => Ok(self
                .builder
                .build_float_compare(FloatPredicate::OGT, lhs, rhs, "fgt")
                .unwrap()
                .as_basic_value_enum()),
            MirBinOp::Le => Ok(self
                .builder
                .build_float_compare(FloatPredicate::OLE, lhs, rhs, "fle")
                .unwrap()
                .as_basic_value_enum()),
            MirBinOp::Ge => Ok(self
                .builder
                .build_float_compare(FloatPredicate::OGE, lhs, rhs, "fge")
                .unwrap()
                .as_basic_value_enum()),
            // Logical and/or on floats are not valid.
            MirBinOp::And | MirBinOp::Or => Err(CompilerError::codegen(format!(
                "logical {:?} is not supported for floating-point operands",
                op
            ))),
        }
    }

    fn gen_unop(
        &self,
        op: MirUnOp,
        val: BasicValueEnum<'ctx>,
    ) -> CompilerResult<BasicValueEnum<'ctx>> {
        let val_ty = val.get_type();
        if val_ty.is_float_type() {
            let float_val = val.into_float_value();
            match op {
                MirUnOp::Neg => Ok(self
                    .builder
                    .build_float_neg(float_val, "fneg")
                    .unwrap()
                    .as_basic_value_enum()),
                MirUnOp::Not => Err(CompilerError::codegen(
                    "unary ! is not supported for floating-point values",
                )),
            }
        } else {
            let int_val = val.into_int_value();
            match op {
                MirUnOp::Neg => Ok(self
                    .builder
                    .build_int_neg(int_val, "neg")
                    .unwrap()
                    .as_basic_value_enum()),
                MirUnOp::Not => Ok(self
                    .builder
                    .build_not(int_val, "not")
                    .unwrap()
                    .as_basic_value_enum()),
            }
        }
    }

    fn gen_struct_lit(
        &mut self,
        struct_def: crate::hir::symbol::SymbolId,
        fields: &[(crate::hir::symbol::SymbolId, MirOperand)],
        prog: &MirProgram,
    ) -> CompilerResult<BasicValueEnum<'ctx>> {
        let sdef = prog
            .struct_def(struct_def)
            .ok_or_else(|| CompilerError::codegen("undefined struct"))?;
        let field_types: Vec<BasicTypeEnum<'ctx>> = sdef
            .fields
            .iter()
            .map(|(_, ty)| mir_type_to_llvm(self.context, prog, ty))
            .collect();
        let llvm_struct = self.context.struct_type(&field_types, false);
        let undef = llvm_struct.get_undef();
        let mut result = undef;
        let field_indices: HashMap<crate::hir::symbol::SymbolId, u32> = sdef
            .fields
            .iter()
            .enumerate()
            .map(|(i, (fid, _))| (*fid, i as u32))
            .collect();
        for (fid, val_operand) in fields {
            let idx = *field_indices.get(fid).ok_or_else(|| {
                let fname = prog.symbols.lookup(*fid).unwrap_or("?");
                CompilerError::codegen(format!("unknown struct field: {}", fname))
            })?;
            let val = self.materialize_operand(val_operand)?;
            let inserted = self
                .builder
                .build_insert_value(result, val, idx, "struct_field")
                .unwrap();
            result = match inserted.as_basic_value_enum() {
                BasicValueEnum::StructValue(sv) => sv,
                _ => return Err(CompilerError::codegen("struct construction failed")),
            };
        }
        let ptr = self
            .builder
            .build_alloca(llvm_struct.as_basic_type_enum(), "struct_val")
            .unwrap();
        self.builder.build_store(ptr, result).unwrap();
        Ok(ptr.as_basic_value_enum())
    }

    fn gen_field_access(
        &mut self,
        local: LocalId,
        field: crate::hir::symbol::SymbolId,
        prog: &MirProgram,
    ) -> CompilerResult<BasicValueEnum<'ctx>> {
        let (alloca, llvm_ty) = self.local_allocas.get(&local).ok_or_else(|| {
            CompilerError::codegen(format!("local {:?} not found for field access", local))
        })?;
        // Load the value stored in the local (for struct types, this is a pointer)
        let loaded = self
            .builder
            .build_load(*llvm_ty, *alloca, "load_struct")
            .unwrap();
        let struct_val = loaded.as_basic_value_enum();

        // Find the struct definition that contains this field
        for sdef in &prog.structs {
            if let Some(field_idx) = sdef.fields.iter().position(|(f, _)| *f == field) {
                let field_types: Vec<BasicTypeEnum<'ctx>> = sdef
                    .fields
                    .iter()
                    .map(|(_, ty)| mir_type_to_llvm(self.context, prog, ty))
                    .collect();
                let llvm_struct = self.context.struct_type(&field_types, false);

                // The struct value should be a pointer (from struct literal allocation)
                let ptr_val = match struct_val {
                    BasicValueEnum::PointerValue(pv) => pv,
                    _ => {
                        return Err(CompilerError::codegen(
                            "field access requires a struct pointer value",
                        ))
                    }
                };

                // Load the actual struct value from the pointer
                let struct_loaded = self
                    .builder
                    .build_load(llvm_struct.as_basic_type_enum(), ptr_val, "struct_load")
                    .unwrap();
                let sv = match struct_loaded {
                    BasicValueEnum::StructValue(sv) => sv,
                    _ => return Err(CompilerError::codegen("expected struct value after load")),
                };
                let field_val = self
                    .builder
                    .build_extract_value(sv, field_idx as u32, "field_access")
                    .unwrap();
                return Ok(field_val);
            }
        }
        Err(CompilerError::codegen(
            "undefined struct field for field access",
        ))
    }

    fn gen_terminator(
        &mut self,
        term: &MirTerminator,
        func: &MirFunction,
        prog: &MirProgram,
        llvm_blocks: &HashMap<BlockId, inkwell::basic_block::BasicBlock<'ctx>>,
    ) -> CompilerResult<()> {
        match term {
            MirTerminator::Goto { target } => {
                let target_bb = llvm_blocks[target];
                self.builder.build_unconditional_branch(target_bb).unwrap();
            }
            MirTerminator::SwitchInt {
                scrutinee,
                ty,
                branches,
                else_target,
                ..
            } => {
                let val = self.materialize_operand(scrutinee)?;
                let val_int = val.into_int_value();
                let else_bb = llvm_blocks[else_target];

                // Match the LLVM integer type to the scrutinee's MIR type so
                // that case constants are well-typed (e.g. i1 for bool conditions,
                // i64 for integer conditions).
                let int_ty = match ty {
                    MirType::Bool => self.context.bool_type(),
                    MirType::I64 => self.context.i64_type(),
                    _ => self.context.i64_type(),
                };

                if branches.len() <= 1 {
                    if let Some((val_const, target_id)) = branches.first() {
                        let case_val = int_ty.const_int(*val_const, false);
                        let cmp = self
                            .builder
                            .build_int_compare(IntPredicate::EQ, val_int, case_val, "switch")
                            .unwrap();
                        let target_bb = llvm_blocks[target_id];
                        self.builder
                            .build_conditional_branch(cmp, target_bb, else_bb)
                            .unwrap();
                    } else {
                        self.builder.build_unconditional_branch(else_bb).unwrap();
                    }
                } else {
                    let cases: Vec<_> = branches
                        .iter()
                        .map(|(val_const, target_id)| {
                            (int_ty.const_int(*val_const, false), llvm_blocks[target_id])
                        })
                        .collect();
                    self.builder.build_switch(val_int, else_bb, &cases).unwrap();
                }
            }
            MirTerminator::Call {
                func: def_id,
                args,
                destination,
                next,
            } => {
                let fname = if *def_id == PRINTLN_DEF_ID {
                    "println_i64"
                } else {
                    prog.function_name(*def_id).ok_or_else(|| {
                        CompilerError::codegen(format!(
                            "undefined function with DefId {:?}",
                            def_id
                        ))
                    })?
                };
                let callee = self.module.get_function(fname).ok_or_else(|| {
                    CompilerError::codegen(format!("undefined function: {}", fname))
                })?;

                let mut arg_vals: Vec<BasicValueEnum<'ctx>> = Vec::new();
                for arg in args {
                    arg_vals.push(self.materialize_operand(arg)?);
                }
                let call = self
                    .builder
                    .build_call(
                        callee,
                        &arg_vals
                            .iter()
                            .map(|v| v.as_basic_value_enum().into())
                            .collect::<Vec<_>>(),
                        "call",
                    )
                    .unwrap();

                let result = call.try_as_basic_value();
                if result.is_basic() {
                    let result_val = result.basic().unwrap_or_else(|| {
                        self.context
                            .i64_type()
                            .const_int(0, true)
                            .as_basic_value_enum()
                    });
                    if let Some((alloca, _)) = self.local_allocas.get(destination) {
                        self.builder.build_store(*alloca, result_val).unwrap();
                    }
                }
                let next_bb = llvm_blocks[next];
                self.builder.build_unconditional_branch(next_bb).unwrap();
            }
            MirTerminator::Return(opt_operand) => match opt_operand {
                Some(operand) => {
                    let val = self.materialize_operand(operand)?;
                    self.builder.build_return(Some(&val)).unwrap();
                }
                None => match &func.return_type {
                    HirType::I64 => {
                        self.builder
                            .build_return(Some(&self.context.i64_type().const_int(0, true)))
                            .unwrap();
                    }
                    HirType::F64 => {
                        self.builder
                            .build_return(Some(&self.context.f64_type().const_float(0.0)))
                            .unwrap();
                    }
                    HirType::Bool => {
                        self.builder
                            .build_return(Some(&self.context.bool_type().const_int(0, false)))
                            .unwrap();
                    }
                    HirType::Unit
                    | HirType::Str
                    | HirType::Struct(_)
                    | HirType::Enum(_)
                    | HirType::Generic(_)
                    | HirType::Apply { .. } => {
                        // For void/unit-like returns and for monomorphized return
                        // types, emit an empty `void` return. Generic and Apply
                        // are unreachable here because monomorphization runs
                        // before codegen, but we list them so the match is
                        // exhaustive.
                        self.builder.build_return(None).unwrap();
                    }
                },
            },
            MirTerminator::Unreachable => {
                self.builder.build_unreachable().unwrap();
            }
        }
        Ok(())
    }
}

/// Convert a `MirType` to an LLVM `BasicTypeEnum`.
pub fn mir_type_to_llvm<'ctx>(
    ctx: &'ctx LLVMContext,
    prog: &MirProgram,
    ty: &MirType,
) -> BasicTypeEnum<'ctx> {
    match ty {
        HirType::I64 => ctx.i64_type().as_basic_type_enum(),
        HirType::F64 => ctx.f64_type().as_basic_type_enum(),
        HirType::Bool => ctx.bool_type().as_basic_type_enum(),
        HirType::Str | HirType::Unit => ctx.i64_type().as_basic_type_enum(),
        HirType::Enum(_) => ctx.i64_type().as_basic_type_enum(),
        HirType::Struct(sym) => {
            let struct_def = match prog.struct_def(*sym) {
                Some(def) => def,
                None => return ctx.i64_type().as_basic_type_enum(),
            };
            let field_types: Vec<BasicTypeEnum<'ctx>> = struct_def
                .fields
                .iter()
                .map(|(_, ty)| mir_type_to_llvm(ctx, prog, ty))
                .collect();
            let _ = ctx.struct_type(&field_types, false);
            ctx.ptr_type(inkwell::AddressSpace::default())
                .as_basic_type_enum()
        }
        HirType::Apply { base, .. } => {
            // A monomorphized generic struct type (`Box<i64>`). The
            // monomorphizer must have produced a concrete StructDef for
            // `base`, so we look up that struct's LLVM shape.
            let struct_def = match prog.struct_def(*base) {
                Some(def) => def,
                None => return ctx.i64_type().as_basic_type_enum(),
            };
            let field_types: Vec<BasicTypeEnum<'ctx>> = struct_def
                .fields
                .iter()
                .map(|(_, ty)| mir_type_to_llvm(ctx, prog, ty))
                .collect();
            let _ = ctx.struct_type(&field_types, false);
            ctx.ptr_type(inkwell::AddressSpace::default())
                .as_basic_type_enum()
        }
        HirType::Generic(_) => {
            // Unreachable at codegen: monomorphization runs before this and
            // substitutes concrete types. We map these to `i64` defensively
            // so a future regression does not produce a confusing crash.
            ctx.i64_type().as_basic_type_enum()
        }
    }
}

/// Entry point: generate LLVM IR text from a `MirProgram`.
pub fn generate_ir_from_mir(mir: &MirProgram) -> CompilerResult<String> {
    let context = LLVMContext::create();
    let mut ctx = MirCodeGenContext::new(&context);
    ctx.declare_builtin_functions();
    ctx.declare_functions(mir)?;
    for func in &mir.functions {
        ctx.generate_function(func, mir)?;
    }
    let ir = ctx.module.print_to_string();
    Ok(ir.to_string())
}

/// Entry point: compile a `MirProgram` to a native executable.
pub fn compile_from_mir(
    mir: &MirProgram,
    output_path: &str,
    target_config: TargetConfig,
) -> CompilerResult<()> {
    compile_from_mir_ext(mir, output_path, target_config, false)
}

/// Entry point with save_temps flag.
pub fn compile_from_mir_ext(
    mir: &MirProgram,
    output_path: &str,
    target_config: TargetConfig,
    save_temps: bool,
) -> CompilerResult<()> {
    use crate::codegen::{Linker, ObjectEmitter};
    use std::path::Path;

    let context = LLVMContext::create();
    let mut ctx = MirCodeGenContext::new(&context);
    ctx.declare_builtin_functions();
    ctx.declare_functions(mir)?;
    for func in &mir.functions {
        ctx.generate_function(func, mir)?;
    }

    let triple = target_config.triple();
    ctx.module.set_triple(triple);

    // Run LLVM optimizations if in release mode.
    let opt_level = target_config.to_inkwell_opt_level();
    if opt_level != InkwellOptLevel::None {
        let target_machine = target_config
            .create_target_machine()
            .map_err(CompilerError::Target)?;
        let opt_passes = target_config.opt_pass_name();
        let opts = PassBuilderOptions::create();
        ctx.module
            .run_passes(opt_passes, &target_machine, opts)
            .map_err(|e| CompilerError::codegen(format!("LLVM optimization failed: {}", e)))?;
    }

    let output_path = Path::new(output_path);
    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| {
                CompilerError::codegen(format!("failed to create output dir: {}", e))
            })?;
        }
    }

    match target_config.output_kind() {
        crate::target::OutputKind::Ir => {
            ctx.module
                .print_to_file(output_path)
                .map_err(|e| CompilerError::codegen(format!("failed to write IR: {}", e)))?;
        }
        crate::target::OutputKind::Object => {
            let emitter = ObjectEmitter::new(ctx.module, &target_config)?;
            emitter.emit_object(output_path)?;
        }
        crate::target::OutputKind::Exe => {
            let obj_path = output_path.with_extension("o");
            {
                let emitter = ObjectEmitter::new(ctx.module, &target_config)?;
                emitter.emit_object(&obj_path)?;
            }
            let lk = Linker::new(&target_config);
            lk.link(&obj_path, output_path)?;
            if !save_temps {
                let _ = std::fs::remove_file(&obj_path);
            }
        }
    }

    Ok(())
}
