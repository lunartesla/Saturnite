//! MIR -> LLVM IR code generation backend.
//!
//! Consumes a [`MirProgram`] (produced by [`crate::mir::lower::lower_program`])
//! and emits LLVM IR.  MIR owns the CFG; this backend translates each
//! [`MirBasicBlock`], [`MirStmtKind`], and [`MirTerminator`] into the
//! corresponding LLVM IR construct.

use crate::error::{CompilerError, CompilerResult};
use crate::hir::types::HirType;
use crate::mir::{
    BlockId, LocalId, MirBinOp, MirConst, MirExternalKind, MirFunction, MirOperand, MirProgram,
    MirRvalue, MirStmtKind, MirTerminator, MirType, MirUnOp,
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

/// DefId sentinel for the builtin `println_str` function (0.5 native
/// `say "..."` / `raise "..."`).
const PRINTLN_STR_DEF_ID: crate::hir::symbol::DefId = crate::hir::symbol::DefId(u32::MAX - 2);

/// DefId sentinel for the runtime `concat_str` function (0.5.1 string
/// interpolation). Must match `hir::lower::CONCAT_STR_DEF_ID`.
const CONCAT_STR_DEF_ID: crate::hir::symbol::DefId = crate::hir::symbol::DefId(u32::MAX - 3);

/// DefId sentinel for the runtime `str_i64` function (0.5.1 numeric string
/// interpolation). Must match `hir::lower::STR_I64_DEF_ID`.
const STR_I64_DEF_ID: crate::hir::symbol::DefId = crate::hir::symbol::DefId(u32::MAX - 4);

/// `sat_py_value_kind` discriminators, matching `runtime/pyrt.h`.
/// Used when emitting the flat `sat_py_call_flat` ABI.
const SAT_PY_NONE: u64 = 0;
const SAT_PY_BOOL: u64 = 1;
const SAT_PY_I64: u64 = 2;
const SAT_PY_F64: u64 = 3;
const SAT_PY_STR: u64 = 4;

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
        let ptr_ty = self.context.ptr_type(inkwell::AddressSpace::default());
        self.module
            .add_function("println_str", i64_ty.fn_type(&[ptr_ty.into()], false), None);
        // 0.5.1 string interpolation: `concat_str(i8*, i8*) -> i8*` and
        // `str_i64(i64) -> i8*`.
        self.module.add_function(
            "concat_str",
            ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false),
            None,
        );
        self.module
            .add_function("str_i64", ptr_ty.fn_type(&[i64_ty.into()], false), None);
        // 0.5.3 List<i64> runtime ABI (see runtime/list.c):
        //   list_new_from(long long* elems, long long count) -> sat_list*
        //   list_get(sat_list*, long long index) -> long long
        //   list_len(sat_list*) -> long long
        // The list pointer is the List LLVM representation (ptr).
        self.module.add_function(
            "list_new_from",
            ptr_ty.fn_type(&[ptr_ty.into(), i64_ty.into()], false),
            None,
        );
        self.module.add_function(
            "list_get",
            i64_ty.fn_type(&[ptr_ty.into(), i64_ty.into()], false),
            None,
        );
        self.module.add_function(
            "list_len",
            i64_ty.fn_type(&[ptr_ty.into()], false),
            None,
        );
    }

    /// Declare all functions from the MIR program into the module.
    pub fn declare_functions(&mut self, prog: &MirProgram) -> CompilerResult<()> {
        for func in &prog.functions {
            if func.def_id == PRINTLN_DEF_ID || func.def_id == PRINTLN_STR_DEF_ID {
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
                // 0.5: strings are private NUL-terminated globals; the
                // rvalue evaluates to an `i8*` pointing at the bytes.
                Ok(self
                    .builder
                    .build_global_string_ptr(s, "str_lit")
                    .unwrap()
                    .as_basic_value_enum())
            }
            MirRvalue::ListLiteral { elements } => {
                // 0.5.3 List<i64> construction. Elements are materialized
                // left-to-right into a stack buffer, then handed to the
                // runtime constructor `list_new_from`, which allocates the
                // process-lifetime sat_list. The rvalue evaluates to the
                // sat_list pointer (the List LLVM representation).
                let i64_ty = self.context.i64_type();
                let count = elements.len() as u64;

                // Stack buffer for the evaluated elements. Zero-length lists
                // cannot reach codegen (HIR rejects empty literals), but the
                // buffer is still sized safely via max(1).
                let buf_alloca = self
                    .builder
                    .build_array_alloca(i64_ty, i64_ty.const_int(count.max(1), false), "list_elems")
                    .unwrap();

                // Left-to-right evaluation: materialize each element in order
                // and store it into the buffer slot.
                for (idx, elem) in elements.iter().enumerate() {
                    let val = self.materialize_operand(elem)?;
                    let val_i64 = match val {
                        BasicValueEnum::IntValue(v) => v,
                        other => {
                            return Err(CompilerError::codegen(format!(
                                "list literal element is not an i64 value: {:?}",
                                other.get_type()
                            )))
                        }
                    };
                    let slot = unsafe {
                        self.builder
                            .build_gep(
                                i64_ty,
                                buf_alloca,
                                &[i64_ty.const_int(idx as u64, false)],
                                &format!("list_elem_{}", idx),
                            )
                            .unwrap()
                    };
                    self.builder.build_store(slot, val_i64).unwrap();
                }

                let list_fn = self
                    .module
                    .get_function("list_new_from")
                    .ok_or_else(|| CompilerError::codegen("list_new_from not declared"))?;
                let call = self
                    .builder
                    .build_call(
                        list_fn,
                        &[
                            buf_alloca.as_basic_value_enum().into(),
                            i64_ty.const_int(count, false).as_basic_value_enum().into(),
                        ],
                        "list_new",
                    )
                    .unwrap();
                let ret = call.try_as_basic_value();
                Ok(ret
                    .basic()
                    .ok_or_else(|| CompilerError::codegen("list_new_from returned no value"))?)
            }
            MirRvalue::Index { list_local, index } => {
                // Lower the list expression first, then the index expression.
                let (list_alloca, list_llvm_ty) = self.local_allocas.get(list_local).ok_or_else(|| {
                    CompilerError::codegen(format!("list local {:?} not found", list_local))
                })?;
                let list_val = self.builder.build_load(*list_llvm_ty, *list_alloca, "list").unwrap();
                let idx = self.materialize_operand(index)?;
                let idx_i64 = match idx {
                    BasicValueEnum::IntValue(v) => v,
                    other => {
                        return Err(CompilerError::codegen(format!(
                            "list index is not an i64 value: {:?}",
                            other.get_type()
                        )))
                    }
                };
                let list_fn = self
                    .module
                    .get_function("list_get")
                    .ok_or_else(|| CompilerError::codegen("list_get not declared"))?;
                let call = self
                    .builder
                    .build_call(
                        list_fn,
                        &[list_val.into(), idx_i64.into()],
                        "list_get",
                    )
                    .unwrap();
                let ret = call.try_as_basic_value();
                Ok(ret
                    .basic()
                    .ok_or_else(|| CompilerError::codegen("list_get returned no value"))?)
            }
            MirRvalue::ExternalCall { kind, symbol, args, ret_ty } => {
                gen_external_call(self, kind, symbol, args, ret_ty, prog)
            }
            MirRvalue::ExternalCall { kind, symbol, args, ret_ty } => {
                gen_external_call(self, kind, symbol, args, ret_ty, prog)
            }
            MirRvalue::Length { list_local } => {
                let (list_alloca, list_llvm_ty) = self.local_allocas.get(list_local).ok_or_else(|| {
                    CompilerError::codegen(format!("list local {:?} not found", list_local))
                })?;
                let list_val = self.builder.build_load(*list_llvm_ty, *list_alloca, "list").unwrap();
                let list_fn = self
                    .module
                    .get_function("list_len")
                    .ok_or_else(|| CompilerError::codegen("list_len not declared"))?;
                let call = self
                    .builder
                    .build_call(list_fn, &[list_val.into()], "list_len")
                    .unwrap();
                let ret = call.try_as_basic_value();
                Ok(ret
                    .basic()
                    .ok_or_else(|| CompilerError::codegen("list_len returned no value"))?)
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
                } else if *def_id == PRINTLN_STR_DEF_ID {
                    "println_str"
                } else if *def_id == CONCAT_STR_DEF_ID {
                    "concat_str"
                } else if *def_id == STR_I64_DEF_ID {
                    "str_i64"
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
                    | HirType::List(_)
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

/// Generate LLVM IR for an external call across an interoperability
/// boundary.
///
/// Rust/Native calls are emitted as direct LLVM `call`s to the declared
/// external symbol. The symbol is resolved by the linker against the
/// wrapper static library (Rust) or shared library (Native).
///
/// Python calls are emitted as calls to the `sat_py_call_spec` runtime
/// bridge helper, which initializes the interpreter, imports the module,
/// resolves the callable, and invokes it. The result is then converted
/// back into a Saturnite value.
///
/// Argument evaluation is performed left-to-right by the caller (the MIR
/// lowering guarantees this), so side effects are not reordered.
fn gen_external_call<'ctx>(
    ctx: &mut MirCodeGenContext<'ctx>,
    kind: &MirExternalKind,
    symbol: &str,
    args: &[MirOperand],
    ret_ty: &MirType,
    prog: &MirProgram,
) -> CompilerResult<BasicValueEnum<'ctx>> {
    let i64_ty = ctx.context.i64_type();
    let ptr_ty = ctx.context.ptr_type(inkwell::AddressSpace::default());

    match kind {
        MirExternalKind::Rust | MirExternalKind::Native => {
            // Rust/Native: direct external symbol call.
            let mut arg_vals: Vec<BasicValueEnum<'ctx>> = Vec::with_capacity(args.len());
            for arg in args {
                let v = ctx.materialize_operand(arg)?;
                arg_vals.push(v.into());
            }
            let llvm_ret = mir_type_to_llvm(ctx.context, prog, ret_ty);
            let fn_ty = llvm_ret.fn_type(
                &arg_vals
                    .iter()
                    .map(|v| v.get_type().into())
                    .collect::<Vec<_>>(),
                false,
            );
            let extern_fn = ctx.module.add_function(symbol, fn_ty, None);
            let call = ctx
                .builder
                .build_call(
                    extern_fn,
                    &arg_vals
                        .iter()
                        .map(|v| (*v).into())
                        .collect::<Vec<_>>(),
                    "ext_call",
                )
                .map_err(|e| CompilerError::codegen(format!("failed to emit external call to '{}': {}", symbol, e)))?;
            let result = call.try_as_basic_value();
            if result.is_basic() {
                Ok(result.basic().unwrap_or_else(|| {
                    i64_ty.const_int(0, true).as_basic_value_enum()
                }))
            } else {
                // Unit return: emit a zero value.
                Ok(i64_ty.const_int(0, true).as_basic_value_enum())
            }
        }
        MirExternalKind::Python => {
            // Python: call the runtime bridge helper `sat_py_call_flat`
            // with parallel int32 (kinds) / int64 (values) arrays. The
            // result is read from the `out` struct.
            let mut arg_vals: Vec<BasicValueEnum<'ctx>> = Vec::with_capacity(args.len());
            for arg in args {
                let v = ctx.materialize_operand(arg)?;
                arg_vals.push(v.into());
            }

            // The Python spec is "module::func". The `symbol` field of the
            // external declaration is the module-qualified function name.
            let spec_ptr = ctx
                .builder
                .build_global_string_ptr(symbol, "py_spec")
                .map_err(|e| CompilerError::codegen(format!("failed to emit Python spec: {}", e)))?;
            let search_ptr = ctx
                .builder
                .build_global_string_ptr("", "py_search")
                .map_err(|e| CompilerError::codegen(format!("failed to emit Python search path: {}", e)))?;

            // Build parallel kinds/values arrays on the stack.
            let kinds_alloca = ctx
                .builder
                .build_alloca(ctx.context.i32_type(), "py_kinds")
                .map_err(|e| CompilerError::codegen(format!("failed to allocate py kinds: {}", e)))?;
            let values_alloca = ctx
                .builder
                .build_alloca(i64_ty, "py_values")
                .map_err(|e| CompilerError::codegen(format!("failed to allocate py values: {}", e)))?;
            for (i, v) in arg_vals.iter().enumerate() {
                let kind = match v {
                    BasicValueEnum::IntValue(iv) => {
                        if iv.get_type().get_bit_width() == 1 {
                            SAT_PY_BOOL
                        } else {
                            SAT_PY_I64
                        }
                    }
                    BasicValueEnum::FloatValue(_) => SAT_PY_F64,
                    _ => SAT_PY_NONE,
                };
                let kind_slot = unsafe {
                    ctx.builder
                        .build_gep(
                            ctx.context.i32_type(),
                            kinds_alloca,
                            &[i64_ty.const_int(i as u64, false)],
                            &format!("py_kind_{}", i),
                        )
                        .map_err(|e| CompilerError::codegen(format!("failed to gep py kind: {}", e)))?
                };
                ctx.builder
                    .build_store(kind_slot, ctx.context.i32_type().const_int(kind as u64, false))
                    .map_err(|e| CompilerError::codegen(format!("failed to store py kind: {}", e)))?;

                let v_i64 = match v {
                    BasicValueEnum::IntValue(iv) => *iv,
                    BasicValueEnum::FloatValue(fv) => ctx
                        .builder
                        .build_bit_cast(*fv, i64_ty, "py_arg_f2i")
                        .map_err(|e| CompilerError::codegen(format!("failed to bitcast py arg: {}", e)))?
                        .into_int_value(),
                    other => other.into_int_value(),
                };
                let val_slot = unsafe {
                    ctx.builder
                        .build_gep(
                            i64_ty,
                            values_alloca,
                            &[i64_ty.const_int(i as u64, false)],
                            &format!("py_val_{}", i),
                        )
                        .map_err(|e| CompilerError::codegen(format!("failed to gep py val: {}", e)))?
                };
                ctx.builder
                    .build_store(val_slot, v_i64)
                    .map_err(|e| CompilerError::codegen(format!("failed to store py val: {}", e)))?;
            }

            // The `sat_py_result` struct layout (must match pyrt.h):
            //   bool ok, i32 kind, i64 union, i64 str_len, ptr err_class,
            //   ptr err_message, ptr handle
            let sat_py_result_ty = ctx.context.struct_type(
                &[
                    ctx.context.bool_type().into(), // ok
                    ctx.context.i32_type().into(), // kind
                    ctx.context.i64_type().into(), // union
                    ctx.context.i64_type().into(), // str_len
                    ptr_ty.into(), // error_class
                    ptr_ty.into(), // error_message
                    ptr_ty.into(), // handle
                ],
                false,
            );

            let out_alloca = ctx
                .builder
                .build_alloca(sat_py_result_ty, "py_out")
                .map_err(|e| CompilerError::codegen(format!("failed to allocate py out: {}", e)))?;
            // Zero-initialize the result struct so all fields start clean.
            ctx.builder
                .build_store(out_alloca, sat_py_result_ty.const_zero())
                .map_err(|e| CompilerError::codegen(format!("failed to zero py out: {}", e)))?;

            let call_fn_ty = ptr_ty.fn_type(
                &[
                    ptr_ty.into(), // spec
                    ptr_ty.into(), // search_path
                    ptr_ty.into(), // kinds
                    ptr_ty.into(), // values
                    i64_ty.into(), // arg_count
                    ptr_ty.into(), // out
                ],
                false,
            );
            let call_fn = ctx.module.add_function("sat_py_call_flat", call_fn_ty, None);
            let call = ctx
                .builder
                .build_call(
                    call_fn,
                    &[
                        spec_ptr.as_basic_value_enum().into(),
                        search_ptr.as_basic_value_enum().into(),
                        kinds_alloca.as_basic_value_enum().into(),
                        values_alloca.as_basic_value_enum().into(),
                        i64_ty.const_int(arg_vals.len() as u64, false).as_basic_value_enum().into(),
                        out_alloca.as_basic_value_enum().into(),
                    ],
                    "py_call",
                )
                .map_err(|e| CompilerError::codegen(format!("failed to emit sat_py_call_flat: {}", e)))?;

            // Read the result union field (index 2) out of the struct.
            // inkwell requires an aggregate *value* (not the call site) for
            // `build_extract_value`, so load the struct back from the alloca.
            let out_loaded = ctx
                .builder
                .build_load(sat_py_result_ty, out_alloca, "py_out_val")
                .map_err(|e| CompilerError::codegen(format!("failed to load py out: {}", e)))?;
            let result_val = ctx
                .builder
                .build_extract_value(
                    out_loaded.into_struct_value(),
                    2,
                    "py_val",
                )
                .map_err(|e| CompilerError::codegen(format!("failed to extract py_val: {}", e)))?;

            // If the call failed, the runtime leaves the value field zero-
            // initialized (we zero the struct before the call), so the
            // result is well-defined even on failure.
            Ok(result_val)
        }
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
        // 0.5: strings are NUL-terminated byte pointers (globals produced
        // by `StrLit` rvalues).
        HirType::Str => ctx
            .ptr_type(inkwell::AddressSpace::default())
            .as_basic_type_enum(),
        HirType::Unit => ctx.i64_type().as_basic_type_enum(),
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
        HirType::List(_) => {
            // 0.5.3: a List is a pointer to the runtime `sat_list` struct
            // (data/len/cap), consistent with the runtime ABI in list.c.
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
            lk.link_with_externals(&obj_path, output_path, &mir.external_libraries)?;
            if !save_temps {
                let _ = std::fs::remove_file(&obj_path);
            }
        }
    }

    Ok(())
}
