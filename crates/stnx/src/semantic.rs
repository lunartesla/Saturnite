use crate::ast::{Program, Type};
use crate::error::{CompilerError, CompilerResult};
use std::collections::HashMap;

#[derive(Clone)]
pub struct Scope {
    variables: HashMap<String, (Type, bool)>,
    functions: HashMap<String, (Vec<Type>, Type)>,
    parent: Option<Box<Scope>>,
}

impl Scope {
    pub fn new() -> Self {
        Self {
            variables: HashMap::new(),
            functions: HashMap::new(),
            parent: None,
        }
    }

    pub fn with_parent(parent: Scope) -> Self {
        Self {
            variables: HashMap::new(),
            functions: HashMap::new(),
            parent: Some(Box::new(parent)),
        }
    }

    pub fn define_variable(&mut self, name: &str, ty: Type, mutable: bool) {
        self.variables.insert(name.to_string(), (ty, mutable));
    }

    pub fn lookup_variable(&self, name: &str) -> Option<Type> {
        if let Some((t, _)) = self.variables.get(name) {
            Some(t.clone())
        } else {
            self.parent.as_ref().and_then(|p| p.lookup_variable(name))
        }
    }

    pub fn lookup_variable_mutability(&self, name: &str) -> Option<bool> {
        if let Some((_, mutable)) = self.variables.get(name) {
            Some(*mutable)
        } else {
            self.parent.as_ref().and_then(|p| p.lookup_variable_mutability(name))
        }
    }

    pub fn define_function(&mut self, name: &str, params: Vec<Type>, ret: Type) {
        self.functions.insert(name.to_string(), (params, ret));
    }

    pub fn lookup_function(&self, name: &str) -> Option<(Vec<Type>, Type)> {
        if let Some(f) = self.functions.get(name) {
            Some(f.clone())
        } else {
            self.parent.as_ref().and_then(|p| p.lookup_function(name))
        }
    }
}

pub fn analyze(program: &Program) -> CompilerResult<()> {
    let mut global_scope = Scope::new();

    // Register builtin println once, not per-function
    global_scope.define_function("println", vec![Type::I64], Type::Unit);

    for func in &program.functions {
        let param_types: Vec<Type> = func.params.iter().map(|(_, t)| t.clone()).collect();
        global_scope.define_function(&func.name, param_types, func.return_type.clone());
    }

    // Check for main function
    if !global_scope.functions.contains_key("main") {
        return Err(CompilerError::semantic("no `main` function defined".to_string()));
    }

    for func in &program.functions {
        let mut func_scope = Scope::with_parent(global_scope.clone());
        for (param_name, param_ty) in &func.params {
            func_scope.define_variable(param_name, param_ty.clone(), false);
        }
        check_body(&func.body, &mut func_scope, &func.return_type)?;
    }

    Ok(())
}

fn check_body(
    stmts: &[crate::ast::Stmt],
    scope: &mut Scope,
    return_type: &crate::ast::Type,
) -> CompilerResult<()> {
    for stmt in stmts {
        check_stmt(stmt, scope, return_type)?;
    }
    Ok(())
}

fn check_expr(
    expr: &crate::ast::Expr,
    scope: &mut Scope,
    return_type: &crate::ast::Type,
) -> CompilerResult<Type> {
    use crate::ast::{BinOp, Expr, UnOp};

    match expr {
        Expr::Integer(_, _) => Ok(Type::I64),
        Expr::Float(_, _) => Ok(Type::F64),
        Expr::StrLit(_, _) => Ok(Type::Str),
        Expr::Bool(_, _) => Ok(Type::Bool),
        Expr::Unit(_) => Ok(Type::Unit),
        Expr::Var(name, _) => scope
            .lookup_variable(name)
            .ok_or_else(|| CompilerError::semantic(format!("undefined variable: {}", name))),
        Expr::Binary { op, lhs, rhs, .. } => {
            let lt = check_expr(lhs, scope, return_type)?;
            let rt = check_expr(rhs, scope, return_type)?;
            match op {
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                    if lt != rt {
                        return Err(CompilerError::semantic(format!(
                                "binary op {:?}: type mismatch {:?} vs {:?}",
                                op, lt, rt
                            )));
                    }
                    Ok(lt)
                }
                BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
                    Ok(Type::Bool)
                }
                BinOp::And | BinOp::Or => Ok(Type::Bool),
            }
        }
        Expr::Unary { op, expr, .. } => {
            let ty = check_expr(expr, scope, return_type)?;
            match op {
                UnOp::Neg => {
                    if ty != Type::I64 && ty != Type::F64 {
                        return Err(CompilerError::semantic(format!("cannot negate {:?}", ty)));
                    }
                    Ok(ty)
                }
                UnOp::Not => Ok(Type::Bool),
            }
        }
        Expr::Call { func, args, .. } => {
            if func == "println" {
                for arg in args {
                    let arg_ty = check_expr(arg, scope, return_type)?;
                    if arg_ty != Type::I64 {
                        return Err(CompilerError::semantic(format!(
                                "println expects i64 argument, got {:?}",
                                arg_ty
                            )));
                    }
                }
                return Ok(Type::Unit);
            }
            let (param_types, ret_type) = scope.lookup_function(func).ok_or_else(|| {
                CompilerError::semantic(format!("undefined function: {}", func))
            })?;
            if args.len() != param_types.len() {
                return Err(CompilerError::semantic(format!(
                    "function {} expects {} args, got {}",
                    func,
                    param_types.len(),
                    args.len()
                )));
            }
            for (arg, expected) in args.iter().zip(param_types.iter()) {
                let actual = check_expr(arg, scope, return_type)?;
                if actual != *expected {
                    return Err(CompilerError::semantic(format!(
                            "function {} arg type mismatch: expected {:?}, got {:?}",
                            func, expected, actual
                        )));
                }
            }
            Ok(ret_type)
        }
        Expr::If {
            condition,
            then_branch,
            elif_branches,
            else_branch,
            ..
        } => {
            let cond_ty = check_expr(condition, scope, return_type)?;
            if cond_ty != Type::Bool {
                return Err(CompilerError::semantic("if condition must be bool"));
            }
            for stmt in then_branch {
                check_stmt(stmt, scope, return_type)?;
            }
            for (cond, body) in elif_branches {
                let ct = check_expr(cond, scope, return_type)?;
                if ct != Type::Bool {
                    return Err(CompilerError::semantic("elif condition must be bool"));
                }
                for stmt in body {
                    check_stmt(stmt, scope, return_type)?;
                }
            }
            if let Some(else_body) = else_branch {
                for stmt in else_body {
                    check_stmt(stmt, scope, return_type)?;
                }
            }
            Ok(Type::Unit)
        }
        Expr::For { var, iter, body, .. } => {
            check_expr(iter, scope, return_type)?;
            let loop_scope = Scope::with_parent(scope.clone());
            let mut loop_scope = loop_scope;
            loop_scope.define_variable(var, Type::I64, false);
            for stmt in body {
                check_stmt(stmt, &mut loop_scope, return_type)?;
            }
            Ok(Type::Unit)
        }
        Expr::While { condition, body, .. } => {
            let cond_ty = check_expr(condition, scope, return_type)?;
            if cond_ty != Type::Bool {
                return Err(CompilerError::semantic("while condition must be bool"));
            }
            let loop_scope = Scope::with_parent(scope.clone());
            let mut loop_scope = loop_scope;
            for stmt in body {
                check_stmt(stmt, &mut loop_scope, return_type)?;
            }
            Ok(Type::Unit)
        }
        Expr::Assign { target, value, .. } => {
            let var_ty = scope.lookup_variable(target).ok_or_else(|| {
                CompilerError::semantic(format!("cannot assign to undefined variable: {}", target))
            })?;
            let val_ty = check_expr(value, scope, return_type)?;
            if var_ty != val_ty {
                return Err(CompilerError::semantic(format!(
                        "assign type mismatch: variable is {:?}, value is {:?}",
                        var_ty, val_ty
                    )));
            }
            // Check that the variable is mutable
            if !scope.lookup_variable_mutability(target).unwrap_or(false) {
                return Err(CompilerError::semantic(format!(
                    "cannot assign to immutable variable: {}", target
                )));
            }
            Ok(var_ty)
        }
        Expr::AugAssign { target, op, value, .. } => {
            let var_ty = scope.lookup_variable(target).ok_or_else(|| {
                CompilerError::semantic(format!("undefined variable: {}", target))
            })?;
            let val_ty = check_expr(value, scope, return_type)?;
            if var_ty != val_ty {
                return Err(CompilerError::semantic(format!(
                        "aug-assign type mismatch: {:?} vs {:?}",
                        var_ty, val_ty
                    )));
            }
            // Check that the variable is mutable
            if !scope.lookup_variable_mutability(target).unwrap_or(false) {
                return Err(CompilerError::semantic(format!(
                    "cannot assign to immutable variable: {}", target
                )));
            }
            let _ = op;
            Ok(var_ty)
        }
        Expr::Range { start, end, .. } => {
            let start_ty = check_expr(start, scope, return_type)?;
            let end_ty = check_expr(end, scope, return_type)?;
            if start_ty != Type::I64 {
                return Err(CompilerError::semantic(format!(
                        "range start type mismatch: expected I64, got {:?}",
                        start_ty
                    )));
            }
            if end_ty != Type::I64 {
                return Err(CompilerError::semantic(format!(
                        "range end type mismatch: expected I64, got {:?}",
                        end_ty
                    )));
            }
            Ok(Type::I64)
        }
    }
}

fn check_stmt(
    stmt: &crate::ast::Stmt,
    scope: &mut Scope,
    return_type: &crate::ast::Type,
) -> CompilerResult<()> {
    use crate::ast::Stmt;
    match stmt {
        Stmt::Let { name, mutable, ty, value, .. } => {
            let inferred = check_expr(value, scope, return_type)?;
            if let Some(t) = ty {
                if *t != inferred {
                    return Err(CompilerError::semantic(format!("type mismatch: expected {:?}, got {:?}", t, inferred)));
                }
            }
            let resolved = ty.clone().unwrap_or(inferred);
            scope.define_variable(name, resolved, *mutable);
        }
        Stmt::Expr(e, _) => {
            check_expr(e, scope, return_type)?;
        }
        Stmt::Return(opt, _) => {
            match opt {
                Some(e) => {
                    let inferred = check_expr(e, scope, return_type)?;
                    if inferred != *return_type {
                        return Err(CompilerError::semantic(format!(
                                "return type mismatch: expected {:?}, got {:?}",
                                return_type, inferred
                            )));
                    }
                }
                None => {
                    if *return_type != crate::ast::Type::Unit {
                        return Err(CompilerError::semantic(format!(
                                "expected return value of type {:?}, got none",
                                return_type
                            )));
                    }
                }
            }
        }
        Stmt::Println(e, _) => {
            let arg_ty = check_expr(e, scope, return_type)?;
            if arg_ty != Type::I64 {
                return Err(CompilerError::semantic(format!(
                        "println expects i64 argument, got {:?}",
                        arg_ty
                    )));
            }
        }
    }
    Ok(())
}
