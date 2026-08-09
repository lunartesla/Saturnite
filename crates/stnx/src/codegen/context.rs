use crate::error::{CompilerError, CompilerResult};
use crate::hir::expr::{HirExpr, HirExprKind};
use crate::hir::function::{HirFunction, HirProgram};
use crate::hir::stmt::{HirStmt, HirStmtKind};
use crate::hir::symbol::{DefId, SymbolId, SymbolInterner};
use crate::hir::types::HirType;
use inkwell::builder::Builder as IRBuilder;
use inkwell::context::Context as LLVMContext;
use inkwell::module::Module;
use inkwell::types::BasicType;
use inkwell::types::BasicTypeEnum;
use inkwell::values::{BasicMetadataValueEnum, BasicValue, BasicValueEnum, PointerValue};
use inkwell::IntPredicate;
use std::collections::HashMap;

/// A variable in a function scope.
/// For mutable variables, the value is also stored in an alloca'd slot
/// so that assignments (which may occur across basic-block boundaries)
/// are visible to subsequent reads.
#[derive(Clone, Copy)]
pub struct Variable<'ctx> {
    pub ssa_value: BasicValueEnum<'ctx>,
    pub alloca: Option<PointerValue<'ctx>>,
}

pub struct CodeGenContext<'ctx> {
    pub context: &'ctx LLVMContext,
    pub module: Module<'ctx>,
    pub builder: IRBuilder<'ctx>,
    /// Function name lookup: DefId → resolved name string.
    pub func_names: HashMap<DefId, String>,
}

impl<'ctx> CodeGenContext<'ctx> {
    pub fn new(context: &'ctx LLVMContext) -> Self {
        let module = context.create_module("saturnite");
        let builder = context.create_builder();
        Self {
            context,
            module,
            builder,
            func_names: HashMap::new(),
        }
    }

    pub fn declare_builtin_functions(&mut self) {
        let i64_type = self.context.i64_type();
        let fn_type = i64_type.fn_type(&[i64_type.into()], false);
        self.module.add_function("println_i64", fn_type, None);
    }

    pub fn declare_function(
        &mut self,
        func: &HirFunction,
        symbols: &SymbolInterner,
    ) -> CompilerResult<()> {
        let name = symbols.lookup(func.name).unwrap_or("unknown");
        self.func_names.insert(func.def_id, name.to_string());

        let ret_basic = type_to_llvm(self.context, &func.return_type);
        let param_types: Vec<_> = func
            .params
            .iter()
            .map(|(_, t)| type_to_llvm(self.context, t).as_basic_type_enum().into())
            .collect();
        let fn_type = ret_basic.as_basic_type_enum().fn_type(&param_types, false);
        self.module.add_function(name, fn_type, None);
        Ok(())
    }

    pub fn generate_function(
        &mut self,
        func: &HirFunction,
        program: &HirProgram,
    ) -> CompilerResult<()> {
        let name = program.symbols.lookup(func.name).unwrap_or("unknown");
        let function_value = self
            .module
            .get_function(name)
            .ok_or_else(|| CompilerError::codegen(format!("function not found: {}", name)))?;

        let entry = self.context.append_basic_block(function_value, "entry");
        self.builder.position_at_end(entry);

        let mut scope = FunctionScope::new();
        for (i, (param_sym, _)) in func.params.iter().enumerate() {
            let param = function_value.get_nth_param(i as u32).ok_or_else(|| {
                CompilerError::codegen(format!("parameter {} not found for function {}", i, name))
            })?;
            scope.insert_immutable(*param_sym, param.as_basic_value_enum());
        }

        // Process all statements except the last one.
        // The last statement may be an implicit return value (Expr kind)
        // which we handle separately to emit a proper return instruction.
        let (last_stmt, rest) = if func.body.is_empty() {
            (None, &[][..])
        } else {
            (
                Some(&func.body[func.body.len() - 1]),
                &func.body[..func.body.len() - 1],
            )
        };
        for stmt in rest {
            self.gen_stmt(stmt, &mut scope, program)?;
        }

        // Handle the last statement: if it's an Expr, use its value as return value
        let last_expr_val = if let Some(stmt) = last_stmt {
            match &stmt.kind {
                HirStmtKind::Expr(e) => Some(self.gen_expr(e, &mut scope, program)?),
                _ => {
                    self.gen_stmt(stmt, &mut scope, program)?;
                    None
                }
            }
        } else {
            None
        };

        let block = self.builder.get_insert_block().unwrap();
        if block.get_terminator().is_none() {
            if let Some(val) = last_expr_val {
                self.builder.build_return(Some(&val)).unwrap();
            } else {
                match &func.return_type {
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
                    HirType::Unit | HirType::Str | HirType::Struct(_) | HirType::Enum(_) => {
                        self.builder.build_return(None).unwrap();
                    }
                }
            }
        }

        Ok(())
    }

    fn resolve_func_name(&self, def_id: DefId) -> Option<&str> {
        self.func_names.get(&def_id).map(|s| s.as_str())
    }

    pub fn gen_stmt(
        &mut self,
        stmt: &HirStmt,
        scope: &mut FunctionScope<'ctx>,
        program: &HirProgram,
    ) -> CompilerResult<()> {
        match &stmt.kind {
            HirStmtKind::Let {
                name,
                mutable,
                value,
                ..
            } => {
                let val = self.gen_expr(value, scope, program)?;
                if *mutable {
                    let alloca = self.builder.build_alloca(val.get_type(), "var").unwrap();
                    self.builder.build_store(alloca, val).unwrap();
                    scope.insert_mutable(*name, val, alloca);
                } else {
                    scope.insert_immutable(*name, val);
                }
            }
            HirStmtKind::Expr(e) => {
                self.gen_expr(e, scope, program)?;
            }
            HirStmtKind::Return(opt_expr) => {
                if let Some(e) = opt_expr {
                    let val = self.gen_expr(e, scope, program)?;
                    self.builder.build_return(Some(&val)).unwrap();
                } else {
                    self.builder.build_return(None).unwrap();
                }
            }
            HirStmtKind::Println(e) => {
                let val = self.gen_expr(e, scope, program)?;
                let fn_val = self.module.get_function("println_i64").unwrap();
                let args: Vec<BasicMetadataValueEnum> = vec![val.into()];
                self.builder.build_call(fn_val, &args, "println").unwrap();
            }
            HirStmtKind::StructDef { .. } => {
                // Struct definitions are used during type resolution;
                // no LLVM IR is generated at this point.
            }
            HirStmtKind::EnumDef { .. } => {
                // Enum definitions are used during type resolution;
                // no LLVM IR is generated at this point.
            }
        }
        Ok(())
    }

    pub fn gen_expr(
        &mut self,
        expr: &HirExpr,
        scope: &mut FunctionScope<'ctx>,
        program: &HirProgram,
    ) -> CompilerResult<BasicValueEnum<'ctx>> {
        match &expr.kind {
            HirExprKind::Integer(n) => Ok(self
                .context
                .i64_type()
                .const_int(*n as u64, true)
                .as_basic_value_enum()),

            HirExprKind::Float(f) => Ok(self
                .context
                .f64_type()
                .const_float(*f)
                .as_basic_value_enum()),

            HirExprKind::Bool(b) => Ok(self
                .context
                .bool_type()
                .const_int(*b as u64, false)
                .as_basic_value_enum()),

            HirExprKind::Unit => Ok(self.context.i64_type().const_zero().as_basic_value_enum()),

            HirExprKind::StrLit(str_id) => {
                let str_val = program.symbols.lookup(*str_id).unwrap_or("");
                let ptr = self
                    .builder
                    .build_global_string_ptr(str_val, "str")
                    .unwrap();
                let int_val = self
                    .builder
                    .build_ptr_to_int(
                        ptr.as_pointer_value(),
                        self.context.i64_type(),
                        "str_as_int",
                    )
                    .unwrap();
                Ok(int_val.as_basic_value_enum())
            }

            HirExprKind::Variable { symbol } => {
                let var = scope.get_value(symbol).ok_or_else(|| {
                    let name = program.symbols.lookup(*symbol).unwrap_or("?");
                    CompilerError::codegen(format!("undefined variable: {}", name))
                })?;
                if let Some(alloca) = var.alloca {
                    let loaded = self
                        .builder
                        .build_load(var.ssa_value.get_type(), alloca, "load")
                        .unwrap();
                    let loaded_val = loaded.as_basic_value_enum();
                    scope.variables.get_mut(symbol).unwrap().ssa_value = loaded_val;
                    Ok(loaded_val)
                } else {
                    Ok(var.ssa_value)
                }
            }

            HirExprKind::Assign { symbol, value } => {
                let val = self.gen_expr(value, scope, program)?;
                if let Some(var) = scope.variables.get_mut(symbol) {
                    if let Some(alloca) = var.alloca {
                        self.builder.build_store(alloca, val).unwrap();
                        var.ssa_value = val;
                    } else {
                        var.ssa_value = val;
                    }
                } else {
                    let name = program.symbols.lookup(*symbol).unwrap_or("?");
                    return Err(CompilerError::codegen(format!(
                        "undefined variable: {}",
                        name
                    )));
                }
                Ok(val)
            }

            HirExprKind::AugAssign { symbol, op, value } => {
                let var = scope.variables.get(symbol).copied().ok_or_else(|| {
                    let name = program.symbols.lookup(*symbol).unwrap_or("?");
                    CompilerError::codegen(format!("undefined variable: {}", name))
                })?;
                let old_val = if let Some(alloca) = var.alloca {
                    self.builder
                        .build_load(var.ssa_value.get_type(), alloca, "aug_load")
                        .unwrap()
                        .as_basic_value_enum()
                } else {
                    var.ssa_value
                };
                let val = self.gen_expr(value, scope, program)?;
                let result = match op {
                    crate::ast::AugOp::Add => self.builder.build_int_add(
                        old_val.into_int_value(),
                        val.into_int_value(),
                        "aug_add",
                    ),
                    crate::ast::AugOp::Sub => self.builder.build_int_sub(
                        old_val.into_int_value(),
                        val.into_int_value(),
                        "aug_sub",
                    ),
                    crate::ast::AugOp::Mul => self.builder.build_int_mul(
                        old_val.into_int_value(),
                        val.into_int_value(),
                        "aug_mul",
                    ),
                    crate::ast::AugOp::Div => self.builder.build_int_unsigned_div(
                        old_val.into_int_value(),
                        val.into_int_value(),
                        "aug_div",
                    ),
                };
                let result_val = result.unwrap().as_basic_value_enum();
                if let Some(var) = scope.variables.get_mut(symbol) {
                    if let Some(alloca) = var.alloca {
                        self.builder.build_store(alloca, result_val).unwrap();
                    }
                    var.ssa_value = result_val;
                }
                Ok(result_val)
            }

            HirExprKind::Binary { op, lhs, rhs } => {
                let lhs_val = self.gen_expr(lhs, scope, program)?;
                let rhs_val = self.gen_expr(rhs, scope, program)?;
                match op {
                    crate::ast::BinOp::Add => Ok(self
                        .builder
                        .build_int_add(lhs_val.into_int_value(), rhs_val.into_int_value(), "add")
                        .unwrap()
                        .as_basic_value_enum()),
                    crate::ast::BinOp::Sub => Ok(self
                        .builder
                        .build_int_sub(lhs_val.into_int_value(), rhs_val.into_int_value(), "sub")
                        .unwrap()
                        .as_basic_value_enum()),
                    crate::ast::BinOp::Mul => Ok(self
                        .builder
                        .build_int_mul(lhs_val.into_int_value(), rhs_val.into_int_value(), "mul")
                        .unwrap()
                        .as_basic_value_enum()),
                    crate::ast::BinOp::Div => Ok(self
                        .builder
                        .build_int_unsigned_div(
                            lhs_val.into_int_value(),
                            rhs_val.into_int_value(),
                            "div",
                        )
                        .unwrap()
                        .as_basic_value_enum()),
                    crate::ast::BinOp::Mod => Ok(self
                        .builder
                        .build_int_unsigned_rem(
                            lhs_val.into_int_value(),
                            rhs_val.into_int_value(),
                            "rem",
                        )
                        .unwrap()
                        .as_basic_value_enum()),
                    crate::ast::BinOp::Eq => Ok(self
                        .builder
                        .build_int_compare(
                            IntPredicate::EQ,
                            lhs_val.into_int_value(),
                            rhs_val.into_int_value(),
                            "eq",
                        )
                        .unwrap()
                        .as_basic_value_enum()),
                    crate::ast::BinOp::Ne => Ok(self
                        .builder
                        .build_int_compare(
                            IntPredicate::NE,
                            lhs_val.into_int_value(),
                            rhs_val.into_int_value(),
                            "ne",
                        )
                        .unwrap()
                        .as_basic_value_enum()),
                    crate::ast::BinOp::Lt => Ok(self
                        .builder
                        .build_int_compare(
                            IntPredicate::ULT,
                            lhs_val.into_int_value(),
                            rhs_val.into_int_value(),
                            "lt",
                        )
                        .unwrap()
                        .as_basic_value_enum()),
                    crate::ast::BinOp::Gt => Ok(self
                        .builder
                        .build_int_compare(
                            IntPredicate::UGT,
                            lhs_val.into_int_value(),
                            rhs_val.into_int_value(),
                            "gt",
                        )
                        .unwrap()
                        .as_basic_value_enum()),
                    crate::ast::BinOp::Le => Ok(self
                        .builder
                        .build_int_compare(
                            IntPredicate::ULE,
                            lhs_val.into_int_value(),
                            rhs_val.into_int_value(),
                            "le",
                        )
                        .unwrap()
                        .as_basic_value_enum()),
                    crate::ast::BinOp::Ge => Ok(self
                        .builder
                        .build_int_compare(
                            IntPredicate::UGE,
                            lhs_val.into_int_value(),
                            rhs_val.into_int_value(),
                            "ge",
                        )
                        .unwrap()
                        .as_basic_value_enum()),
                    crate::ast::BinOp::And => Ok(self
                        .builder
                        .build_and(lhs_val.into_int_value(), rhs_val.into_int_value(), "and")
                        .unwrap()
                        .as_basic_value_enum()),
                    crate::ast::BinOp::Or => Ok(self
                        .builder
                        .build_or(lhs_val.into_int_value(), rhs_val.into_int_value(), "or")
                        .unwrap()
                        .as_basic_value_enum()),
                }
            }

            HirExprKind::Unary { op, expr: inner } => {
                let val = self.gen_expr(inner, scope, program)?;
                match op {
                    crate::ast::UnOp::Neg => Ok(self
                        .builder
                        .build_int_neg(val.into_int_value(), "neg")
                        .unwrap()
                        .as_basic_value_enum()),
                    crate::ast::UnOp::Not => Ok(self
                        .builder
                        .build_not(val.into_int_value(), "not")
                        .unwrap()
                        .as_basic_value_enum()),
                }
            }

            HirExprKind::Call { func: def_id, args } => {
                let fname = self.resolve_func_name(*def_id).unwrap_or("?");
                let llvm_name = if fname == "println" {
                    "println_i64"
                } else {
                    fname
                };
                let function = self.module.get_function(llvm_name).ok_or_else(|| {
                    CompilerError::codegen(format!("undefined function: {}", fname))
                })?;
                let mut arg_vals: Vec<BasicMetadataValueEnum> = Vec::new();
                for arg in args {
                    arg_vals.push(self.gen_expr(arg, scope, program)?.into());
                }
                let call = self
                    .builder
                    .build_call(function, &arg_vals, "call")
                    .unwrap();
                let result = call.try_as_basic_value();
                if result.is_basic() {
                    Ok(result.basic().unwrap_or_else(|| {
                        self.context.i64_type().const_zero().as_basic_value_enum()
                    }))
                } else {
                    Ok(self.context.i64_type().const_zero().as_basic_value_enum())
                }
            }

            HirExprKind::If {
                condition,
                then_branch,
                elif_branches,
                else_branch,
            } => {
                let current_func = self
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_parent()
                    .unwrap();
                let cond_val = self.gen_expr(condition, scope, program)?;
                let cond_bool = self
                    .builder
                    .build_int_cast(
                        cond_val.into_int_value(),
                        self.context.bool_type(),
                        "if_cond",
                    )
                    .unwrap();
                let then_bb = self.context.append_basic_block(current_func, "if_then");
                let end_bb = self.context.append_basic_block(current_func, "if_end");
                if elif_branches.is_empty() {
                    let else_bb = self.context.append_basic_block(current_func, "if_else");
                    self.builder
                        .build_conditional_branch(cond_bool, then_bb, else_bb)
                        .unwrap();
                    self.builder.position_at_end(then_bb);
                    for s in then_branch {
                        self.gen_stmt(s, scope, program)?;
                    }
                    self.builder.build_unconditional_branch(end_bb).unwrap();
                    self.builder.position_at_end(else_bb);
                    if let Some(else_body) = else_branch {
                        for s in else_body {
                            self.gen_stmt(s, scope, program)?;
                        }
                    }
                    self.builder.build_unconditional_branch(end_bb).unwrap();
                } else {
                    let mut elif_cond_bbs = Vec::with_capacity(elif_branches.len());
                    let mut elif_body_bbs = Vec::with_capacity(elif_branches.len());
                    for (i, _) in elif_branches.iter().enumerate() {
                        elif_cond_bbs.push(
                            self.context
                                .append_basic_block(current_func, &format!("elif{}_cond", i)),
                        );
                        elif_body_bbs.push(
                            self.context
                                .append_basic_block(current_func, &format!("elif{}_body", i)),
                        );
                    }
                    let else_bb = self.context.append_basic_block(current_func, "if_else");
                    self.builder
                        .build_conditional_branch(cond_bool, then_bb, elif_cond_bbs[0])
                        .unwrap();
                    self.builder.position_at_end(then_bb);
                    for s in then_branch {
                        self.gen_stmt(s, scope, program)?;
                    }
                    self.builder.build_unconditional_branch(end_bb).unwrap();
                    for (elif_idx, (cond_expr, body)) in elif_branches.iter().enumerate() {
                        let elif_cond_bb = elif_cond_bbs[elif_idx];
                        let elif_body_bb = elif_body_bbs[elif_idx];
                        self.builder.position_at_end(elif_cond_bb);
                        let elif_val = self.gen_expr(cond_expr, scope, program)?;
                        let elif_bool = self
                            .builder
                            .build_int_cast(
                                elif_val.into_int_value(),
                                self.context.bool_type(),
                                &format!("elif{}_cond", elif_idx),
                            )
                            .unwrap();
                        let next_bb = if elif_idx + 1 < elif_cond_bbs.len() {
                            elif_cond_bbs[elif_idx + 1]
                        } else {
                            else_bb
                        };
                        self.builder
                            .build_conditional_branch(elif_bool, elif_body_bb, next_bb)
                            .unwrap();
                        self.builder.position_at_end(elif_body_bb);
                        for s in body {
                            self.gen_stmt(s, scope, program)?;
                        }
                        self.builder.build_unconditional_branch(end_bb).unwrap();
                    }
                    self.builder.position_at_end(else_bb);
                    if let Some(else_body) = else_branch {
                        for s in else_body {
                            self.gen_stmt(s, scope, program)?;
                        }
                    }
                    self.builder.build_unconditional_branch(end_bb).unwrap();
                }
                self.builder.position_at_end(end_bb);
                Ok(self.context.i64_type().const_zero().as_basic_value_enum())
            }

            HirExprKind::For { var, iter, body } => {
                let current_func = self
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_parent()
                    .unwrap();
                let cond_bb = self.context.append_basic_block(current_func, "for_cond");
                let body_bb = self.context.append_basic_block(current_func, "for_body");
                let end_bb = self.context.append_basic_block(current_func, "for_end");
                let i64_type = self.context.i64_type();
                let loop_var_ptr = self.builder.build_alloca(i64_type, "for_var_ptr").unwrap();
                // Extract start/end from the Range HIR expression
                let (start_val, end_val, is_inclusive) = match &iter.kind {
                    HirExprKind::Range {
                        start,
                        end,
                        is_inclusive,
                        ..
                    } => {
                        let s = self.gen_expr(start, scope, program)?.into_int_value();
                        let e = self.gen_expr(end, scope, program)?.into_int_value();
                        (s, e, *is_inclusive)
                    }
                    _ => {
                        return Err(CompilerError::codegen(
                            "for loop requires a range expression",
                        ))
                    }
                };
                self.builder.build_store(loop_var_ptr, start_val).unwrap();
                self.builder.build_unconditional_branch(cond_bb).unwrap();
                // Condition block
                self.builder.position_at_end(cond_bb);
                let current_val = self
                    .builder
                    .build_load(i64_type, loop_var_ptr, "for_cond_val")
                    .unwrap()
                    .as_basic_value_enum();
                let predicate = if is_inclusive {
                    IntPredicate::ULE
                } else {
                    IntPredicate::ULT
                };
                let cmp = self
                    .builder
                    .build_int_compare(predicate, current_val.into_int_value(), end_val, "for_cond")
                    .unwrap();
                self.builder
                    .build_conditional_branch(cmp, body_bb, end_bb)
                    .unwrap();
                // Body block
                self.builder.position_at_end(body_bb);
                let current_for_scope = self
                    .builder
                    .build_load(i64_type, loop_var_ptr, "for_var")
                    .unwrap()
                    .as_basic_value_enum();
                scope.insert_mutable(*var, current_for_scope, loop_var_ptr);
                for s in body {
                    self.gen_stmt(s, scope, program)?;
                }
                // Increment loop variable
                let loaded = self
                    .builder
                    .build_load(i64_type, loop_var_ptr, "for_next_load")
                    .unwrap()
                    .as_basic_value_enum();
                let next_val = self
                    .builder
                    .build_int_add(
                        loaded.into_int_value(),
                        i64_type.const_int(1, false),
                        "for_next",
                    )
                    .unwrap();
                self.builder.build_store(loop_var_ptr, next_val).unwrap();
                self.builder.build_unconditional_branch(cond_bb).unwrap();
                self.builder.position_at_end(end_bb);
                Ok(self.context.i64_type().const_zero().as_basic_value_enum())
            }

            HirExprKind::While { condition, body } => {
                let current_func = self
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_parent()
                    .unwrap();
                let cond_bb = self.context.append_basic_block(current_func, "while_cond");
                let body_bb = self.context.append_basic_block(current_func, "while_body");
                let end_bb = self.context.append_basic_block(current_func, "while_end");
                self.builder.build_unconditional_branch(cond_bb).unwrap();
                self.builder.position_at_end(cond_bb);
                let cond_val = self.gen_expr(condition, scope, program)?;
                let cond_bool = self
                    .builder
                    .build_int_cast(
                        cond_val.into_int_value(),
                        self.context.bool_type(),
                        "while_cond",
                    )
                    .unwrap();
                self.builder
                    .build_conditional_branch(cond_bool, body_bb, end_bb)
                    .unwrap();
                self.builder.position_at_end(body_bb);
                for s in body {
                    self.gen_stmt(s, scope, program)?;
                }
                self.builder.build_unconditional_branch(cond_bb).unwrap();
                self.builder.position_at_end(end_bb);
                Ok(self.context.i64_type().const_zero().as_basic_value_enum())
            }

            HirExprKind::Range { start, end, .. } => {
                let _ = end; // Range in expression position returns start value
                let start_val = self.gen_expr(start, scope, program)?;
                Ok(start_val)
            }

            HirExprKind::StructLiteral { name, fields } => {
                let struct_def = program.struct_def(*name).ok_or_else(|| {
                    let name_str = program.symbols.lookup(*name).unwrap_or("?");
                    CompilerError::codegen(format!("undefined struct: {}", name_str))
                })?;
                let llvm_tys: Vec<_> = struct_def
                    .fields
                    .iter()
                    .map(|(_, ty)| hir_type_to_llvm(self.context, program, ty).as_basic_type_enum())
                    .collect();
                let llvm_struct = self.context.struct_type(&llvm_tys, false);
                let undef = llvm_struct.get_undef();
                // Build a map from field SymbolId -> value expression for lookup
                let field_map: HashMap<SymbolId, &Box<HirExpr>> =
                    fields.iter().map(|(k, v)| (*k, v)).collect();
                // Insert values in struct definition field order
                let mut result = undef;
                for (i, (field_sym, field_ty)) in struct_def.fields.iter().enumerate() {
                    let val = if let Some(expr) = field_map.get(field_sym) {
                        self.gen_expr(expr, scope, program)?
                    } else {
                        match hir_type_to_llvm(self.context, program, field_ty) {
                            BasicTypeEnum::IntType(t) => t.get_undef().as_basic_value_enum(),
                            BasicTypeEnum::FloatType(t) => t.get_undef().as_basic_value_enum(),
                            BasicTypeEnum::PointerType(t) => t.const_zero().as_basic_value_enum(),
                            _ => unreachable!("unsupported field type for struct default"),
                        }
                    };
                    let inserted = self
                        .builder
                        .build_insert_value(result, val, i as u32, "struct_field")
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

            HirExprKind::FieldAccess { expr, field } => {
                let struct_val = self.gen_expr(expr, scope, program)?;
                // Determine struct type from the expression's HIR type
                let struct_ty = match &expr.ty {
                    HirType::Struct(sym) => *sym,
                    _ => return Err(CompilerError::codegen("field access requires a struct")),
                };
                let struct_def = program
                    .struct_def(struct_ty)
                    .ok_or_else(|| CompilerError::codegen("undefined struct for field access"))?;
                let field_idx = struct_def
                    .fields
                    .iter()
                    .position(|(f, _)| *f == *field)
                    .ok_or_else(|| {
                        let field_str = program.symbols.lookup(*field).unwrap_or("?");
                        CompilerError::codegen(format!("undefined field: {}", field_str))
                    })?;
                let _field_ty = struct_def.fields[field_idx].1;
                let ptr_ty = struct_def
                    .fields
                    .iter()
                    .map(|(_, ty)| hir_type_to_llvm(self.context, program, ty).as_basic_type_enum())
                    .collect::<Vec<_>>();
                let llvm_struct = self.context.struct_type(&ptr_ty, false);
                let loaded = self
                    .builder
                    .build_load(
                        llvm_struct.as_basic_type_enum(),
                        struct_val.into_pointer_value(),
                        "load_struct",
                    )
                    .unwrap();
                let struct_val_loaded = match loaded {
                    BasicValueEnum::StructValue(sv) => sv,
                    _ => {
                        return Err(CompilerError::codegen(
                            "expected struct value for field access",
                        ))
                    }
                };
                let field_val = self
                    .builder
                    .build_extract_value(struct_val_loaded, field_idx as u32, "field_access")
                    .unwrap();
                Ok(field_val)
            }

            HirExprKind::EnumConstructor { name, variant } => {
                let enum_def = program.enum_def(*name).ok_or_else(|| {
                    let name_str = program.symbols.lookup(*name).unwrap_or("?");
                    CompilerError::codegen(format!("undefined enum: {}", name_str))
                })?;
                let variant_idx = enum_def
                    .variants
                    .iter()
                    .position(|v| *v == *variant)
                    .ok_or_else(|| {
                        let variant_str = program.symbols.lookup(*variant).unwrap_or("?");
                        CompilerError::codegen(format!("undefined enum variant: {}", variant_str))
                    })?;
                Ok(self
                    .context
                    .i64_type()
                    .const_int(variant_idx as u64, false)
                    .as_basic_value_enum())
            }
        }
    }
}

pub struct FunctionScope<'ctx> {
    pub variables: HashMap<SymbolId, Variable<'ctx>>,
}

impl<'ctx> FunctionScope<'ctx> {
    pub fn new() -> Self {
        Self {
            variables: HashMap::new(),
        }
    }
    pub fn insert_immutable(&mut self, sym: SymbolId, value: BasicValueEnum<'ctx>) {
        self.variables.insert(
            sym,
            Variable {
                ssa_value: value,
                alloca: None,
            },
        );
    }
    pub fn insert_mutable(
        &mut self,
        sym: SymbolId,
        value: BasicValueEnum<'ctx>,
        alloca: PointerValue<'ctx>,
    ) {
        self.variables.insert(
            sym,
            Variable {
                ssa_value: value,
                alloca: Some(alloca),
            },
        );
    }
    pub fn get_value(&self, sym: &SymbolId) -> Option<Variable<'ctx>> {
        self.variables.get(sym).copied()
    }
}

impl<'ctx> Default for FunctionScope<'ctx> {
    fn default() -> Self {
        Self::new()
    }
}

fn type_to_llvm<'ctx>(ctx: &'ctx LLVMContext, ty: &HirType) -> BasicTypeEnum<'ctx> {
    match ty {
        HirType::I64 => ctx.i64_type().as_basic_type_enum(),
        HirType::F64 => ctx.f64_type().as_basic_type_enum(),
        HirType::Bool => ctx.bool_type().as_basic_type_enum(),
        HirType::Str | HirType::Unit => ctx.i64_type().as_basic_type_enum(),
        HirType::Struct(_sym) => {
            // Struct types are resolved via the program's struct definitions
            // at codegen time. For standalone type queries, fall back to i64.
            ctx.i64_type().as_basic_type_enum()
        }
        HirType::Enum(_) => ctx.i64_type().as_basic_type_enum(),
    }
}

/// Resolve an HIR type to its LLVM representation, using the program's
/// struct definitions to build proper struct types (as pointers).
///
/// Struct types in the LLVM IR are represented as pointers to the
/// allocated struct on the stack. This function recursively resolves
/// nested struct field types.
fn hir_type_to_llvm<'ctx>(
    ctx: &'ctx LLVMContext,
    program: &HirProgram,
    ty: &HirType,
) -> BasicTypeEnum<'ctx> {
    match ty {
        HirType::I64 => ctx.i64_type().as_basic_type_enum(),
        HirType::F64 => ctx.f64_type().as_basic_type_enum(),
        HirType::Bool => ctx.bool_type().as_basic_type_enum(),
        HirType::Str | HirType::Unit => ctx.i64_type().as_basic_type_enum(),
        HirType::Enum(_) => ctx.i64_type().as_basic_type_enum(),
        HirType::Struct(sym) => {
            let struct_def = match program.struct_def(*sym) {
                Some(def) => def,
                None => return ctx.i64_type().as_basic_type_enum(),
            };
            let field_types: Vec<BasicTypeEnum<'ctx>> = struct_def
                .fields
                .iter()
                .map(|(_, ty)| hir_type_to_llvm(ctx, program, ty))
                .collect();
            // In LLVM 21 (opaque pointers), all pointer types are the same.
            // We return a pointer-to-struct type so that struct-typed fields
            // in other structs are correctly represented.
            let _llvm_struct = ctx.struct_type(&field_types, false);
            ctx.ptr_type(inkwell::AddressSpace::default())
                .as_basic_type_enum()
        }
    }
}
