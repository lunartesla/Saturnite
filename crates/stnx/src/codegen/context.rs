use crate::ast::{BinOp, Expr, Function, Stmt, Type, UnOp};
use crate::error::{CompilerError, CompilerResult};
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
    /// The current SSA value (for immutable vars) or the last-loaded value (for mutable).
    pub ssa_value: BasicValueEnum<'ctx>,
    /// Stack slot for mutable variables. None for immutable variables.
    pub alloca: Option<PointerValue<'ctx>>,
}

pub struct CodeGenContext<'ctx> {
    pub context: &'ctx LLVMContext,
    pub module: Module<'ctx>,
    pub builder: IRBuilder<'ctx>,
}

impl<'ctx> CodeGenContext<'ctx> {
    pub fn new(context: &'ctx LLVMContext) -> Self {
        let module = context.create_module("saturnite");
        let builder = context.create_builder();
        Self {
            context,
            module,
            builder,
        }
    }

    pub fn declare_builtin_functions(&mut self) {
        let i64_type = self.context.i64_type();
        let fn_type = i64_type.fn_type(&[i64_type.into()], false);
        self.module.add_function("println_i64", fn_type, None);
    }

    pub fn declare_function(&mut self, func: &Function) -> CompilerResult<()> {
        let ret_basic = type_to_llvm(self.context, &func.return_type);
        let param_types: Vec<_> = func
            .params
            .iter()
            .map(|(_, t)| type_to_llvm(self.context, t).as_basic_type_enum().into())
            .collect();
        let fn_type = ret_basic.as_basic_type_enum().fn_type(&param_types, false);
        self.module.add_function(&func.name, fn_type, None);
        Ok(())
    }

    pub fn generate_function(&mut self, func: &Function) -> CompilerResult<()> {
        let function_value = self
            .module
            .get_function(&func.name)
            .ok_or_else(|| CompilerError::codegen(format!("function not found: {}", func.name)))?;

        let entry = self.context.append_basic_block(function_value, "entry");
        self.builder.position_at_end(entry);

        let mut scope = FunctionScope::new();

        for (i, (name, _)) in func.params.iter().enumerate() {
            let param = function_value.get_nth_param(i as u32).ok_or_else(|| {
                CompilerError::codegen(format!(
                    "parameter {} not found for function {}",
                    i, func.name
                ))
            })?;
            scope.insert_immutable(name.clone(), param);
        }

        for stmt in &func.body {
            self.gen_stmt(stmt, &mut scope)?;
        }

        let block = self.builder.get_insert_block().unwrap();
        if block.get_terminator().is_none() {
            match &func.return_type {
                Type::I64 => {
                    self.builder
                        .build_return(Some(&self.context.i64_type().const_int(0, true)))
                        .unwrap();
                }
                Type::F64 => {
                    self.builder
                        .build_return(Some(&self.context.f64_type().const_float(0.0)))
                        .unwrap();
                }
                Type::Bool => {
                    self.builder
                        .build_return(Some(&self.context.bool_type().const_int(0, false)))
                        .unwrap();
                }
                Type::Unit => {
                    self.builder.build_return(None).unwrap();
                }
                Type::Str => {
                    self.builder
                        .build_return(Some(&self.context.i64_type().const_int(0, true)))
                        .unwrap();
                }
            }
        }

        Ok(())
    }

    pub fn gen_stmt(&mut self, stmt: &Stmt, scope: &mut FunctionScope<'ctx>) -> CompilerResult<()> {
        match stmt {
            Stmt::Let {
                name,
                mutable,
                value,
                ..
            } => {
                let val = self.gen_expr(value, scope)?;
                if *mutable {
                    // Allocate stack slot for mutable variable
                    let alloca = self
                        .builder
                        .build_alloca(val.get_type(), name.as_str())
                        .unwrap();
                    self.builder.build_store(alloca, val).unwrap();
                    scope.insert_mutable(name.clone(), val, alloca);
                } else {
                    scope.insert_immutable(name.clone(), val);
                }
                Ok(())
            }
            Stmt::Expr(e, _) => {
                self.gen_expr(e, scope)?;
                Ok(())
            }
            Stmt::Return(opt_expr, _) => {
                if let Some(e) = opt_expr {
                    let val = self.gen_expr(e, scope)?;
                    self.builder.build_return(Some(&val)).unwrap();
                } else {
                    self.builder.build_return(None).unwrap();
                }
                Ok(())
            }
            Stmt::Println(e, _) => {
                let val = self.gen_expr(e, scope)?;
                let fn_val = self.module.get_function("println_i64").unwrap();
                let args: Vec<BasicMetadataValueEnum> = vec![val.into()];
                self.builder.build_call(fn_val, &args, "println").unwrap();
                Ok(())
            }
        }
    }

    pub fn gen_expr(
        &mut self,
        expr: &Expr,
        scope: &mut FunctionScope<'ctx>,
    ) -> CompilerResult<BasicValueEnum<'ctx>> {
        match expr {
            Expr::Integer(n, _) => Ok(self
                .context
                .i64_type()
                .const_int(*n as u64, true)
                .as_basic_value_enum()),

            Expr::Float(f, _) => Ok(self
                .context
                .f64_type()
                .const_float(*f)
                .as_basic_value_enum()),

            Expr::StrLit(s, _) => {
                let ptr = self.builder.build_global_string_ptr(s, "str").unwrap();
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

            Expr::Bool(b, _) => Ok(self
                .context
                .bool_type()
                .const_int(*b as u64, false)
                .as_basic_value_enum()),

            Expr::Unit(_) => Ok(self.context.i64_type().const_zero().as_basic_value_enum()),

            Expr::Var(name, _) => {
                let var = scope.variables.get(name).ok_or_else(|| {
                    CompilerError::codegen(format!("undefined variable: {}", name))
                })?;

                if let Some(alloca) = var.alloca {
                    // Load the current value from the stack slot
                    let loaded = self
                        .builder
                        .build_load(var.ssa_value.get_type(), alloca, &format!("load_{}", name))
                        .unwrap();
                    let loaded_val = loaded.as_basic_value_enum();
                    // Update the cached SSA value in the variable
                    // (needed so subsequent reads in the same block get the right value)
                    scope.variables.get_mut(name).unwrap().ssa_value = loaded_val;
                    Ok(loaded_val)
                } else {
                    Ok(var.ssa_value)
                }
            }

            Expr::Binary { op, lhs, rhs, .. } => {
                let lhs_val = self.gen_expr(lhs, scope)?;
                let rhs_val = self.gen_expr(rhs, scope)?;

                match op {
                    BinOp::Add => Ok(self
                        .builder
                        .build_int_add(lhs_val.into_int_value(), rhs_val.into_int_value(), "add")
                        .unwrap()
                        .as_basic_value_enum()),
                    BinOp::Sub => Ok(self
                        .builder
                        .build_int_sub(lhs_val.into_int_value(), rhs_val.into_int_value(), "sub")
                        .unwrap()
                        .as_basic_value_enum()),
                    BinOp::Mul => Ok(self
                        .builder
                        .build_int_mul(lhs_val.into_int_value(), rhs_val.into_int_value(), "mul")
                        .unwrap()
                        .as_basic_value_enum()),
                    BinOp::Div => Ok(self
                        .builder
                        .build_int_unsigned_div(
                            lhs_val.into_int_value(),
                            rhs_val.into_int_value(),
                            "div",
                        )
                        .unwrap()
                        .as_basic_value_enum()),
                    BinOp::Mod => Ok(self
                        .builder
                        .build_int_unsigned_rem(
                            lhs_val.into_int_value(),
                            rhs_val.into_int_value(),
                            "rem",
                        )
                        .unwrap()
                        .as_basic_value_enum()),
                    BinOp::Eq => Ok(self
                        .builder
                        .build_int_compare(
                            IntPredicate::EQ,
                            lhs_val.into_int_value(),
                            rhs_val.into_int_value(),
                            "eq",
                        )
                        .unwrap()
                        .as_basic_value_enum()),
                    BinOp::Ne => Ok(self
                        .builder
                        .build_int_compare(
                            IntPredicate::NE,
                            lhs_val.into_int_value(),
                            rhs_val.into_int_value(),
                            "ne",
                        )
                        .unwrap()
                        .as_basic_value_enum()),
                    BinOp::Lt => Ok(self
                        .builder
                        .build_int_compare(
                            IntPredicate::SLT,
                            lhs_val.into_int_value(),
                            rhs_val.into_int_value(),
                            "lt",
                        )
                        .unwrap()
                        .as_basic_value_enum()),
                    BinOp::Gt => Ok(self
                        .builder
                        .build_int_compare(
                            IntPredicate::SGT,
                            lhs_val.into_int_value(),
                            rhs_val.into_int_value(),
                            "gt",
                        )
                        .unwrap()
                        .as_basic_value_enum()),
                    BinOp::Le => Ok(self
                        .builder
                        .build_int_compare(
                            IntPredicate::SLE,
                            lhs_val.into_int_value(),
                            rhs_val.into_int_value(),
                            "le",
                        )
                        .unwrap()
                        .as_basic_value_enum()),
                    BinOp::Ge => Ok(self
                        .builder
                        .build_int_compare(
                            IntPredicate::SGE,
                            lhs_val.into_int_value(),
                            rhs_val.into_int_value(),
                            "ge",
                        )
                        .unwrap()
                        .as_basic_value_enum()),
                    BinOp::And => Ok(self
                        .builder
                        .build_and(lhs_val.into_int_value(), rhs_val.into_int_value(), "and")
                        .unwrap()
                        .as_basic_value_enum()),
                    BinOp::Or => Ok(self
                        .builder
                        .build_or(lhs_val.into_int_value(), rhs_val.into_int_value(), "or")
                        .unwrap()
                        .as_basic_value_enum()),
                }
            }

            Expr::Unary { op, expr, .. } => {
                let val = self.gen_expr(expr, scope)?;
                match op {
                    UnOp::Neg => Ok(self
                        .builder
                        .build_int_neg(val.into_int_value(), "neg")
                        .unwrap()
                        .as_basic_value_enum()),
                    UnOp::Not => Ok(self
                        .builder
                        .build_not(val.into_int_value(), "not")
                        .unwrap()
                        .as_basic_value_enum()),
                }
            }

            Expr::Assign { target, value, .. } => {
                let val = self.gen_expr(value, scope)?;

                if let Some(var) = scope.variables.get_mut(target) {
                    if let Some(alloca) = var.alloca {
                        // Store to the mutable variable's stack slot
                        self.builder.build_store(alloca, val).unwrap();
                        var.ssa_value = val;
                    } else {
                        // Immutable variable -- just update the cached SSA value
                        var.ssa_value = val;
                    }
                } else {
                    return Err(CompilerError::codegen(format!(
                        "undefined variable: {}",
                        target
                    )));
                }

                Ok(val)
            }

            Expr::AugAssign {
                target, op, value, ..
            } => {
                let var = scope.variables.get(target).cloned().ok_or_else(|| {
                    CompilerError::codegen(format!("undefined variable: {}", target))
                })?;

                let old_val = if let Some(alloca) = var.alloca {
                    // Load current value from the mutable slot
                    self.builder
                        .build_load(
                            var.ssa_value.get_type(),
                            alloca,
                            &format!("aug_load_{}", target),
                        )
                        .unwrap()
                        .as_basic_value_enum()
                } else {
                    var.ssa_value
                };

                let val = self.gen_expr(value, scope)?;
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

                if let Some(var) = scope.variables.get_mut(target) {
                    if let Some(alloca) = var.alloca {
                        self.builder.build_store(alloca, result_val).unwrap();
                    }
                    var.ssa_value = result_val;
                }

                Ok(result_val)
            }

            Expr::Call { func, args, .. } => {
                let function = self.module.get_function(func).ok_or_else(|| {
                    CompilerError::codegen(format!("undefined function: {}", func))
                })?;

                let mut arg_vals: Vec<BasicMetadataValueEnum> = Vec::new();
                for arg in args {
                    arg_vals.push(self.gen_expr(arg, scope)?.into());
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

            Expr::If {
                condition,
                then_branch,
                elif_branches,
                else_branch,
                ..
            } => {
                let current_func = self
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_parent()
                    .unwrap();

                // Build the chain of condition blocks
                // Structure: if_cond -> [then, elif1_cond] -> elif1_body -> [elif2_cond, ...] -> else -> end
                let cond_val = self.gen_expr(condition, scope)?;
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

                // If there are elif branches, create blocks for them
                // and chain the conditional branches
                if elif_branches.is_empty() {
                    // No elifs: simple if-else
                    let else_bb = self.context.append_basic_block(current_func, "if_else");

                    self.builder
                        .build_conditional_branch(cond_bool, then_bb, else_bb)
                        .unwrap();

                    self.builder.position_at_end(then_bb);
                    for s in then_branch {
                        self.gen_stmt(s, scope)?;
                    }
                    self.builder.build_unconditional_branch(end_bb).unwrap();

                    self.builder.position_at_end(else_bb);
                    if let Some(else_body) = else_branch {
                        for s in else_body {
                            self.gen_stmt(s, scope)?;
                        }
                    }
                    self.builder.build_unconditional_branch(end_bb).unwrap();
                } else {
                    // Has elif branches: create a chain of elif condition blocks
                    let mut elif_cond_bbs: Vec<_> = Vec::with_capacity(elif_branches.len());
                    let mut elif_body_bbs: Vec<_> = Vec::with_capacity(elif_branches.len());
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

                    // First condition: if cond -> then, else -> first elif_cond
                    self.builder
                        .build_conditional_branch(cond_bool, then_bb, elif_cond_bbs[0])
                        .unwrap();

                    self.builder.position_at_end(then_bb);
                    for s in then_branch {
                        self.gen_stmt(s, scope)?;
                    }
                    self.builder.build_unconditional_branch(end_bb).unwrap();

                    // Chain elif conditions
                    for (elif_idx, (cond_expr, body)) in elif_branches.iter().enumerate() {
                        let elif_cond_bb = elif_cond_bbs[elif_idx];
                        let elif_body_bb = elif_body_bbs[elif_idx];

                        self.builder.position_at_end(elif_cond_bb);
                        let elif_val = self.gen_expr(cond_expr, scope)?;
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
                            self.gen_stmt(s, scope)?;
                        }
                        self.builder.build_unconditional_branch(end_bb).unwrap();
                    }

                    // Else block
                    self.builder.position_at_end(else_bb);
                    if let Some(else_body) = else_branch {
                        for s in else_body {
                            self.gen_stmt(s, scope)?;
                        }
                    }
                    self.builder.build_unconditional_branch(end_bb).unwrap();
                }

                self.builder.position_at_end(end_bb);
                Ok(self.context.i64_type().const_zero().as_basic_value_enum())
            }

            Expr::For {
                var, iter, body, ..
            } => {
                let iter_expr = self.gen_expr(iter, scope)?;

                let current_func = self
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_parent()
                    .unwrap();

                let cond_bb = self.context.append_basic_block(current_func, "for_cond");
                let body_bb = self.context.append_basic_block(current_func, "for_body");
                let end_bb = self.context.append_basic_block(current_func, "for_end");

                // For loop iterates over a Range expression
                // We use a local variable for the loop counter, allocated on the stack
                let i64_type = self.context.i64_type();
                let loop_var_ptr = self.builder.build_alloca(i64_type, var.as_str()).unwrap();

                // Initialize loop counter to 0; will be updated per iteration
                // For ranges, we evaluate start and end from the Range expression
                // iter_expr is the start value (from Range codegen),
                // but we need both start and end. We re-evaluate the Range expr.
                let (start_val, end_val, is_inclusive) = match iter.as_ref() {
                    crate::ast::Expr::Range {
                        start,
                        end,
                        is_inclusive,
                        ..
                    } => {
                        let s = self.gen_expr(start, scope)?;
                        let e = self.gen_expr(end, scope)?;
                        (s.into_int_value(), e.into_int_value(), *is_inclusive)
                    }
                    _ => {
                        return Err(CompilerError::codegen(
                            "for loop requires a range expression",
                        ));
                    }
                };

                let _ = iter_expr; // suppress unused warning

                // Store start value into loop variable
                self.builder.build_store(loop_var_ptr, start_val).unwrap();

                // Branch to condition check
                self.builder.build_unconditional_branch(cond_bb).unwrap();

                // Condition block
                self.builder.position_at_end(cond_bb);
                let current_val = self
                    .builder
                    .build_load(i64_type, loop_var_ptr, "for_load")
                    .unwrap()
                    .into_int_value();

                // Compare current < end (SLT) for exclusive ranges,
                // or current <= end (SLE) for inclusive ranges (`...`).
                let predicate = if is_inclusive {
                    IntPredicate::SLE
                } else {
                    IntPredicate::SLT
                };
                let cmp = self
                    .builder
                    .build_int_compare(predicate, current_val, end_val, "for_cond")
                    .unwrap();

                self.builder
                    .build_conditional_branch(cmp, body_bb, end_bb)
                    .unwrap();

                // Body block
                self.builder.position_at_end(body_bb);
                // Load current value and store in scope for variable access
                // Store as a mutable variable so mutations inside the body persist
                let current_for_scope = self
                    .builder
                    .build_load(i64_type, loop_var_ptr, "for_var")
                    .unwrap()
                    .as_basic_value_enum();
                scope.insert_mutable(var.clone(), current_for_scope, loop_var_ptr);

                for s in body {
                    self.gen_stmt(s, scope)?;
                }

                // Increment loop variable
                let next_val = self
                    .builder
                    .build_int_add(current_val, i64_type.const_int(1, false), "for_next")
                    .unwrap();
                self.builder.build_store(loop_var_ptr, next_val).unwrap();

                self.builder.build_unconditional_branch(cond_bb).unwrap();

                self.builder.position_at_end(end_bb);

                Ok(self.context.i64_type().const_zero().as_basic_value_enum())
            }

            Expr::While {
                condition, body, ..
            } => {
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

                let cond_val = self.gen_expr(condition, scope)?;
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
                    self.gen_stmt(s, scope)?;
                }
                self.builder.build_unconditional_branch(cond_bb).unwrap();

                self.builder.position_at_end(end_bb);
                Ok(self.context.i64_type().const_zero().as_basic_value_enum())
            }

            Expr::Range {
                start,
                end,
                is_inclusive,
                ..
            } => {
                let start_val = self.gen_expr(start, scope)?;
                let end_val = self.gen_expr(end, scope)?;
                let _ = end_val;
                let _ = is_inclusive;
                Ok(start_val)
            }
        }
    }
}

pub struct FunctionScope<'ctx> {
    pub variables: HashMap<String, Variable<'ctx>>,
}

impl<'ctx> FunctionScope<'ctx> {
    pub fn new() -> Self {
        Self {
            variables: HashMap::new(),
        }
    }

    /// Insert an immutable variable (no alloca slot).
    pub fn insert_immutable(&mut self, name: String, value: BasicValueEnum<'ctx>) {
        self.variables.insert(
            name,
            Variable {
                ssa_value: value,
                alloca: None,
            },
        );
    }

    /// Insert a mutable variable with an alloca slot.
    pub fn insert_mutable(
        &mut self,
        name: String,
        value: BasicValueEnum<'ctx>,
        alloca: PointerValue<'ctx>,
    ) {
        self.variables.insert(
            name,
            Variable {
                ssa_value: value,
                alloca: Some(alloca),
            },
        );
    }

    /// Get a variable's current value. If it has an alloca, loads from memory first.
    pub fn get_value(&self, name: &str) -> Option<BasicValueEnum<'ctx>> {
        self.variables.get(name).map(|v| v.ssa_value)
    }
}

impl<'ctx> Default for FunctionScope<'ctx> {
    fn default() -> Self {
        Self::new()
    }
}

fn type_to_llvm<'ctx>(ctx: &'ctx LLVMContext, ty: &Type) -> BasicTypeEnum<'ctx> {
    match ty {
        Type::I64 => ctx.i64_type().as_basic_type_enum(),
        Type::F64 => ctx.f64_type().as_basic_type_enum(),
        Type::Bool => ctx.bool_type().as_basic_type_enum(),
        Type::Str | Type::Unit => ctx.i64_type().as_basic_type_enum(),
    }
}
