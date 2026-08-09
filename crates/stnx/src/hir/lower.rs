//! HIR lowering — transforms the AST into a typed, resolved HIR.
//!
//! Pipeline: `AST → HirLower::lower_program → HirProgram (typed HIR)`
//!
//! All identifiers are interned to `SymbolId` / `DefId` so later
//! stages (MIR, LLVM codegen) never perform string lookups. Every HIR
//! node carries a resolved `HirType` and a preserved source `SourceSpan`.

use crate::ast::{BinOp, Expr, Function, Program, Stmt, Type, UnOp};
use crate::error::{CompilerError, CompilerResult};
use crate::hir::expr::{HirExpr, HirExprKind};
use crate::hir::function::{EnumDef, HirFunction, HirProgram, StructDef};
use crate::hir::stmt::{HirStmt, HirStmtKind};
use crate::hir::symbol::{DefId, SymbolId, SymbolInterner};
use crate::hir::types::HirType;
use miette::SourceSpan;
use std::collections::HashMap;

/// Convert a byte-offset `Range<usize>` from the AST to a `SourceSpan`.
fn span_to_source_span(r: &std::ops::Range<usize>) -> SourceSpan {
    SourceSpan::new(r.start.into(), r.end.saturating_sub(r.start))
}

/// A lightweight function signature for call-site checking.
struct FunctionSig {
    def_id: DefId,
    param_types: Vec<HirType>,
    return_type: HirType,
}

/// DefId sentinel for the builtin `println` function.
const PRINTLN_DEF_ID: DefId = DefId(u32::MAX - 1);

/// Context passed to lowering functions, bundling immutable references to
/// the function signature table and the struct/enum registries.  This allows
/// `lower_stmt` / `lower_expr` to resolve type names and look up struct/enum
/// definitions without conflicting with the `&mut self` borrow on `HirLower`.
struct LowerContext<'a> {
    function_sigs: &'a HashMap<SymbolId, FunctionSig>,
    struct_defs: &'a [StructDef],
    enum_defs: &'a [EnumDef],
    /// Set of enum name strings, used to resolve Type::Struct references
    /// that are actually enum types (since the parser produces Type::Struct
    /// for all user-defined type names).
    enum_names: &'a HashMap<&'a str, ()>,
}

/// A variable entry tracked during lowering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VarInfo {
    ty: HirType,
    mutable: bool,
}

/// A lexical scope stack for name resolution during lowering.
#[derive(Clone)]
struct LowerScope {
    variables: HashMap<SymbolId, VarInfo>,
    parent: Option<Box<LowerScope>>,
}

impl LowerScope {
    fn new() -> Self {
        Self {
            variables: HashMap::new(),
            parent: None,
        }
    }
    fn with_parent(parent: LowerScope) -> Self {
        Self {
            variables: HashMap::new(),
            parent: Some(Box::new(parent)),
        }
    }
    fn define_variable(&mut self, sym: SymbolId, ty: HirType, mutable: bool) {
        self.variables.insert(sym, VarInfo { ty, mutable });
    }
    fn lookup_variable(&self, sym: &SymbolId) -> Option<VarInfo> {
        if let Some(v) = self.variables.get(sym) {
            Some(*v)
        } else {
            self.parent.as_ref().and_then(|p| p.lookup_variable(sym))
        }
    }
}

/// Convert an `ast::Type` to a `HirType`, interning names.
/// Used during Pass 1 (before struct/enum definitions are fully collected).
fn ast_type_to_hir(
    ty: &Type,
    symbols: &mut SymbolInterner,
    enum_names: &HashMap<&str, ()>,
) -> HirType {
    match ty {
        Type::I64 => HirType::I64,
        Type::F64 => HirType::F64,
        Type::Bool => HirType::Bool,
        Type::Str => HirType::Str,
        Type::Unit => HirType::Unit,
        Type::Struct(name) => {
            let sym = symbols.intern(name);
            // The parser produces Type::Struct for all user-defined type
            // references. If the name is actually an enum, resolve it as
            // HirType::Enum instead.
            if enum_names.contains_key(name.as_str()) {
                HirType::Enum(sym)
            } else {
                HirType::Struct(sym)
            }
        }
        Type::Enum(name) => {
            let sym = symbols.intern(name);
            HirType::Enum(sym)
        }
    }
}

/// The HIR lowering driver.
pub struct HirLower {
    pub symbols: SymbolInterner,
}

impl Default for HirLower {
    fn default() -> Self {
        Self::new()
    }
}

impl HirLower {
    pub fn new() -> Self {
        Self {
            symbols: SymbolInterner::default(),
        }
    }

    pub fn lower_program(&mut self, program: &Program) -> CompilerResult<HirProgram> {
        // Phase 0: collect all enum names up front so that type annotations
        // (in function signatures, struct fields, and variable declarations)
        // can resolve user-defined types that are actually enums. The parser
        // produces Type::Struct for all user-defined type names.
        let mut enum_names: HashMap<&str, ()> = HashMap::new();
        for func in &program.functions {
            for stmt in &func.body {
                if let Stmt::EnumDef { name, .. } = stmt {
                    enum_names.insert(name.as_str(), ());
                }
            }
        }

        // Pass 1: intern all function names and build the signature table.
        let mut function_sigs: HashMap<SymbolId, FunctionSig> = HashMap::new();
        for (i, func) in program.functions.iter().enumerate() {
            let name_id = self.symbols.intern(&func.name);
            let def_id = DefId(i as u32);
            let param_types: Vec<HirType> = func
                .params
                .iter()
                .map(|(_, t)| ast_type_to_hir(t, &mut self.symbols, &enum_names))
                .collect();
            let return_type = ast_type_to_hir(&func.return_type, &mut self.symbols, &enum_names);
            function_sigs.insert(
                name_id,
                FunctionSig {
                    def_id,
                    param_types,
                    return_type,
                },
            );
        }
        // Register builtin println
        let println_sym = self.symbols.intern("println");
        function_sigs.insert(
            println_sym,
            FunctionSig {
                def_id: PRINTLN_DEF_ID,
                param_types: vec![HirType::I64],
                return_type: HirType::Unit,
            },
        );
        // Check for main
        let main_sym = self.symbols.intern("main");
        if !function_sigs.contains_key(&main_sym) {
            return Err(CompilerError::semantic("no `main` function defined"));
        }

        // Pre-pass: scan all function bodies for struct/enum definitions and
        // intern their names + field/variant names into the symbol table.
        let mut structs: Vec<StructDef> = Vec::new();
        let mut enums: Vec<EnumDef> = Vec::new();

        // Phase 2: collect struct and enum definitions, resolving type
        // annotations using the name sets collected in Phase 0.
        for func in &program.functions {
            for stmt in &func.body {
                match stmt {
                    Stmt::StructDef { name, fields, span } => {
                        let name_id = self.symbols.intern(name);
                        let field_syms: Vec<(SymbolId, HirType)> = fields
                            .iter()
                            .map(|(fname, fty)| {
                                let fid = self.symbols.intern(fname);
                                (fid, ast_type_to_hir(fty, &mut self.symbols, &enum_names))
                            })
                            .collect();
                        structs.push(StructDef {
                            def_id: DefId(structs.len() as u32),
                            name: name_id,
                            fields: field_syms,
                            span: span_to_source_span(span),
                        });
                    }
                    Stmt::EnumDef {
                        name,
                        variants,
                        span,
                    } => {
                        let name_id = self.symbols.intern(name);
                        let variant_syms: Vec<SymbolId> =
                            variants.iter().map(|v| self.symbols.intern(v)).collect();
                        enums.push(EnumDef {
                            def_id: DefId(enums.len() as u32),
                            name: name_id,
                            variants: variant_syms,
                            span: span_to_source_span(span),
                        });
                    }
                    _ => {}
                }
            }
        }

        // Build the lowering context — borrows from local variables (not from self)
        let ctx = LowerContext {
            function_sigs: &function_sigs,
            struct_defs: &structs,
            enum_defs: &enums,
            enum_names: &enum_names,
        };

        // Pass 2: lower each function body
        let mut functions: Vec<HirFunction> = Vec::new();
        for (i, func) in program.functions.iter().enumerate() {
            functions.push(self.lower_function(func, DefId(i as u32), &ctx)?);
        }

        Ok(HirProgram {
            functions,
            structs,
            enums,
            symbols: std::mem::take(&mut self.symbols),
        })
    }

    fn lower_function(
        &mut self,
        func: &Function,
        def_id: DefId,
        ctx: &LowerContext,
    ) -> CompilerResult<HirFunction> {
        let name = self.symbols.intern(&func.name);
        let return_type = ast_type_to_hir(&func.return_type, &mut self.symbols, ctx.enum_names);
        let mut scope = LowerScope::new();
        let mut params: Vec<(SymbolId, HirType)> = Vec::new();
        for (param_name, param_ty) in &func.params {
            let param_id = self.symbols.intern(param_name);
            let hir_ty = ast_type_to_hir(param_ty, &mut self.symbols, ctx.enum_names);
            scope.define_variable(param_id, hir_ty, false);
            params.push((param_id, hir_ty));
        }
        let mut body: Vec<HirStmt> = Vec::new();
        for stmt in &func.body {
            body.push(self.lower_stmt(stmt, &mut scope, &return_type, ctx)?);
        }
        Ok(HirFunction {
            def_id,
            name,
            params,
            return_type,
            body,
            span: span_to_source_span(&func.span),
        })
    }

    fn lower_stmt(
        &mut self,
        stmt: &Stmt,
        scope: &mut LowerScope,
        return_type: &HirType,
        ctx: &LowerContext,
    ) -> CompilerResult<HirStmt> {
        match stmt {
            Stmt::Let {
                name,
                mutable,
                ty,
                value,
                span,
            } => {
                let name_id = self.symbols.intern(name);
                let inferred = self.lower_expr(value, scope, return_type, ctx)?;
                let resolved_ty = if let Some(t) = ty {
                    let ann = ast_type_to_hir(t, &mut self.symbols, ctx.enum_names);
                    // If the annotation is a Struct type, check if it's actually an enum
                    let resolved = if let HirType::Struct(sym) = ann {
                        // Check if there's an enum with this name
                        if ctx.enum_defs.iter().any(|e| e.name == sym) {
                            HirType::Enum(sym)
                        } else {
                            HirType::Struct(sym)
                        }
                    } else {
                        ann
                    };
                    if resolved != inferred.ty {
                        return Err(CompilerError::semantic(format!(
                            "type mismatch: expected {:?}, got {:?}",
                            resolved, inferred.ty
                        )));
                    }
                    resolved
                } else {
                    inferred.ty
                };
                scope.define_variable(name_id, resolved_ty, *mutable);
                Ok(HirStmt {
                    kind: HirStmtKind::Let {
                        name: name_id,
                        mutable: *mutable,
                        ty: if ty.is_some() {
                            Some(resolved_ty)
                        } else {
                            None
                        },
                        value: inferred,
                    },
                    span: span_to_source_span(span),
                })
            }
            Stmt::Expr(e, span) => Ok(HirStmt {
                kind: HirStmtKind::Expr(self.lower_expr(e, scope, return_type, ctx)?),
                span: span_to_source_span(span),
            }),
            Stmt::Return(opt_expr, span) => {
                let hir_opt = if let Some(e) = opt_expr {
                    let hir_e = self.lower_expr(e, scope, return_type, ctx)?;
                    if hir_e.ty != *return_type {
                        return Err(CompilerError::semantic(format!(
                            "return type mismatch: expected {:?}, got {:?}",
                            return_type, hir_e.ty
                        )));
                    }
                    Some(hir_e)
                } else {
                    if *return_type != HirType::Unit {
                        return Err(CompilerError::semantic(format!(
                            "expected return value of type {:?}, got none",
                            return_type
                        )));
                    }
                    None
                };
                Ok(HirStmt {
                    kind: HirStmtKind::Return(hir_opt),
                    span: span_to_source_span(span),
                })
            }
            Stmt::Println(e, span) => {
                let hir_expr = self.lower_expr(e, scope, return_type, ctx)?;
                // Enums are represented as i64 tags at the LLVM level, so
                // println_i64 accepts them just like raw i64 values.
                if hir_expr.ty != HirType::I64 && !matches!(hir_expr.ty, HirType::Enum(_)) {
                    return Err(CompilerError::semantic(format!(
                        "println expects i64 argument, got {:?}",
                        hir_expr.ty
                    )));
                }
                Ok(HirStmt {
                    kind: HirStmtKind::Println(hir_expr),
                    span: span_to_source_span(span),
                })
            }
            Stmt::StructDef { span, .. } | Stmt::EnumDef { span, .. } => {
                // Definitions are collected during the pre-pass; emit a no-op unit expr.
                Ok(HirStmt {
                    kind: HirStmtKind::Expr(HirExpr {
                        kind: HirExprKind::Unit,
                        ty: HirType::Unit,
                        span: span_to_source_span(span),
                    }),
                    span: span_to_source_span(span),
                })
            }
        }
    }

    fn lower_expr(
        &mut self,
        expr: &Expr,
        scope: &mut LowerScope,
        return_type: &HirType,
        ctx: &LowerContext,
    ) -> CompilerResult<HirExpr> {
        match expr {
            Expr::Integer(n, span) => Ok(HirExpr {
                kind: HirExprKind::Integer(*n),
                ty: HirType::I64,
                span: span_to_source_span(span),
            }),
            Expr::Float(f, span) => Ok(HirExpr {
                kind: HirExprKind::Float(*f),
                ty: HirType::F64,
                span: span_to_source_span(span),
            }),
            Expr::Bool(b, span) => Ok(HirExpr {
                kind: HirExprKind::Bool(*b),
                ty: HirType::Bool,
                span: span_to_source_span(span),
            }),
            Expr::Unit(span) => Ok(HirExpr {
                kind: HirExprKind::Unit,
                ty: HirType::Unit,
                span: span_to_source_span(span),
            }),
            Expr::StrLit(s, span) => {
                let str_id = self.symbols.intern(s);
                Ok(HirExpr {
                    kind: HirExprKind::StrLit(str_id),
                    ty: HirType::Str,
                    span: span_to_source_span(span),
                })
            }
            Expr::Var(name, span) => {
                let sym = self.symbols.intern(name);
                let var_info = scope.lookup_variable(&sym).ok_or_else(|| {
                    CompilerError::semantic(format!("undefined variable: {}", name))
                })?;
                Ok(HirExpr {
                    kind: HirExprKind::Variable { symbol: sym },
                    ty: var_info.ty,
                    span: span_to_source_span(span),
                })
            }
            Expr::Assign {
                target,
                value,
                span,
            } => {
                let sym = self.symbols.intern(target);
                let var_info = scope.lookup_variable(&sym).ok_or_else(|| {
                    CompilerError::semantic(format!(
                        "cannot assign to undefined variable: {}",
                        target
                    ))
                })?;
                let val_expr = self.lower_expr(value, scope, return_type, ctx)?;
                if var_info.ty != val_expr.ty {
                    return Err(CompilerError::semantic(format!(
                        "assign type mismatch: variable is {:?}, value is {:?}",
                        var_info.ty, val_expr.ty
                    )));
                }
                if !var_info.mutable {
                    return Err(CompilerError::semantic(format!(
                        "cannot assign to immutable variable: {}",
                        target
                    )));
                }
                Ok(HirExpr {
                    kind: HirExprKind::Assign {
                        symbol: sym,
                        value: Box::new(val_expr),
                    },
                    ty: var_info.ty,
                    span: span_to_source_span(span),
                })
            }
            Expr::AugAssign {
                target,
                op,
                value,
                span,
            } => {
                let sym = self.symbols.intern(target);
                let var_info = scope.lookup_variable(&sym).ok_or_else(|| {
                    CompilerError::semantic(format!("undefined variable: {}", target))
                })?;
                if !var_info.mutable {
                    return Err(CompilerError::semantic(format!(
                        "cannot assign to immutable variable: {}",
                        target
                    )));
                }
                let val_expr = self.lower_expr(value, scope, return_type, ctx)?;
                if var_info.ty != val_expr.ty {
                    return Err(CompilerError::semantic(format!(
                        "aug-assign type mismatch: {:?} vs {:?}",
                        var_info.ty, val_expr.ty
                    )));
                }
                Ok(HirExpr {
                    kind: HirExprKind::AugAssign {
                        symbol: sym,
                        op: *op,
                        value: Box::new(val_expr),
                    },
                    ty: var_info.ty,
                    span: span_to_source_span(span),
                })
            }
            Expr::Binary { op, lhs, rhs, span } => {
                let l = self.lower_expr(lhs, scope, return_type, ctx)?;
                let r = self.lower_expr(rhs, scope, return_type, ctx)?;
                match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                        if l.ty != r.ty {
                            return Err(CompilerError::semantic(format!(
                                "binary op {:?}: type mismatch {:?} vs {:?}",
                                op, l.ty, r.ty
                            )));
                        }
                        Ok(HirExpr {
                            kind: HirExprKind::Binary {
                                op: *op,
                                lhs: Box::new(l.clone()),
                                rhs: Box::new(r),
                            },
                            ty: l.ty,
                            span: span_to_source_span(span),
                        })
                    }
                    BinOp::Eq
                    | BinOp::Ne
                    | BinOp::Lt
                    | BinOp::Gt
                    | BinOp::Le
                    | BinOp::Ge
                    | BinOp::And
                    | BinOp::Or => Ok(HirExpr {
                        kind: HirExprKind::Binary {
                            op: *op,
                            lhs: Box::new(l),
                            rhs: Box::new(r),
                        },
                        ty: HirType::Bool,
                        span: span_to_source_span(span),
                    }),
                }
            }
            Expr::Unary {
                op,
                expr: inner,
                span,
            } => {
                let e = self.lower_expr(inner, scope, return_type, ctx)?;
                match op {
                    UnOp::Neg => {
                        if e.ty != HirType::I64 && e.ty != HirType::F64 {
                            return Err(CompilerError::semantic(format!(
                                "cannot negate {:?}",
                                e.ty
                            )));
                        }
                        Ok(HirExpr {
                            kind: HirExprKind::Unary {
                                op: *op,
                                expr: Box::new(e.clone()),
                            },
                            ty: e.ty,
                            span: span_to_source_span(span),
                        })
                    }
                    UnOp::Not => Ok(HirExpr {
                        kind: HirExprKind::Unary {
                            op: *op,
                            expr: Box::new(e),
                        },
                        ty: HirType::Bool,
                        span: span_to_source_span(span),
                    }),
                }
            }
            Expr::Call { func, args, span } => {
                let func_sym = self.symbols.intern(func);
                let sig = ctx.function_sigs.get(&func_sym).ok_or_else(|| {
                    CompilerError::semantic(format!("undefined function: {}", func))
                })?;
                if args.len() != sig.param_types.len() {
                    return Err(CompilerError::semantic(format!(
                        "function {} expects {} args, got {}",
                        func,
                        sig.param_types.len(),
                        args.len()
                    )));
                }
                let mut arg_exprs: Vec<HirExpr> = Vec::new();
                for (arg, expected_ty) in args.iter().zip(sig.param_types.iter()) {
                    let arg_expr = self.lower_expr(arg, scope, return_type, ctx)?;
                    if arg_expr.ty != *expected_ty {
                        return Err(CompilerError::semantic(format!(
                            "function {} arg type mismatch: expected {:?}, got {:?}",
                            func, expected_ty, arg_expr.ty
                        )));
                    }
                    arg_exprs.push(arg_expr);
                }
                Ok(HirExpr {
                    kind: HirExprKind::Call {
                        func: sig.def_id,
                        args: arg_exprs,
                    },
                    ty: sig.return_type,
                    span: span_to_source_span(span),
                })
            }
            Expr::If {
                condition,
                then_branch,
                elif_branches,
                else_branch,
                span,
            } => {
                let cond = self.lower_expr(condition, scope, return_type, ctx)?;
                if cond.ty != HirType::Bool {
                    return Err(CompilerError::semantic("if condition must be bool"));
                }
                let mut then_hir: Vec<HirStmt> = Vec::new();
                for s in then_branch {
                    then_hir.push(self.lower_stmt(s, scope, return_type, ctx)?);
                }
                let mut elif_hir: Vec<(HirExpr, Vec<HirStmt>)> = Vec::new();
                for (cond_expr, body) in elif_branches {
                    let c = self.lower_expr(cond_expr, scope, return_type, ctx)?;
                    if c.ty != HirType::Bool {
                        return Err(CompilerError::semantic("elif condition must be bool"));
                    }
                    let mut body_hir: Vec<HirStmt> = Vec::new();
                    for s in body {
                        body_hir.push(self.lower_stmt(s, scope, return_type, ctx)?);
                    }
                    elif_hir.push((c, body_hir));
                }
                let mut else_hir: Option<Vec<HirStmt>> = None;
                if let Some(else_body) = else_branch {
                    let mut body_hir: Vec<HirStmt> = Vec::new();
                    for s in else_body {
                        body_hir.push(self.lower_stmt(s, scope, return_type, ctx)?);
                    }
                    else_hir = Some(body_hir);
                }
                Ok(HirExpr {
                    kind: HirExprKind::If {
                        condition: Box::new(cond),
                        then_branch: then_hir,
                        elif_branches: elif_hir,
                        else_branch: else_hir,
                    },
                    ty: HirType::Unit,
                    span: span_to_source_span(span),
                })
            }
            Expr::For {
                var,
                iter,
                body,
                span,
            } => {
                let iter_expr = self.lower_expr(iter, scope, return_type, ctx)?;
                match &iter_expr.kind {
                    HirExprKind::Range { .. } => {}
                    _ => {
                        return Err(CompilerError::codegen(
                            "for loop requires a range expression",
                        ))
                    }
                }
                let var_sym = self.symbols.intern(var);
                let mut loop_scope = LowerScope::with_parent(scope.clone());
                loop_scope.define_variable(var_sym, HirType::I64, false);
                let mut body_hir: Vec<HirStmt> = Vec::new();
                for s in body {
                    body_hir.push(self.lower_stmt(s, &mut loop_scope, return_type, ctx)?);
                }
                Ok(HirExpr {
                    kind: HirExprKind::For {
                        var: var_sym,
                        iter: Box::new(iter_expr),
                        body: body_hir,
                    },
                    ty: HirType::Unit,
                    span: span_to_source_span(span),
                })
            }
            Expr::While {
                condition,
                body,
                span,
            } => {
                let cond = self.lower_expr(condition, scope, return_type, ctx)?;
                if cond.ty != HirType::Bool {
                    return Err(CompilerError::semantic("while condition must be bool"));
                }
                let mut loop_scope = LowerScope::with_parent(scope.clone());
                let mut body_hir: Vec<HirStmt> = Vec::new();
                for s in body {
                    body_hir.push(self.lower_stmt(s, &mut loop_scope, return_type, ctx)?);
                }
                Ok(HirExpr {
                    kind: HirExprKind::While {
                        condition: Box::new(cond),
                        body: body_hir,
                    },
                    ty: HirType::Unit,
                    span: span_to_source_span(span),
                })
            }
            Expr::Range {
                start,
                end,
                is_inclusive,
                span,
            } => {
                let s = self.lower_expr(start, scope, return_type, ctx)?;
                let e = self.lower_expr(end, scope, return_type, ctx)?;
                if s.ty != HirType::I64 {
                    return Err(CompilerError::semantic(format!(
                        "range start type mismatch: expected I64, got {:?}",
                        s.ty
                    )));
                }
                if e.ty != HirType::I64 {
                    return Err(CompilerError::semantic(format!(
                        "range end type mismatch: expected I64, got {:?}",
                        e.ty
                    )));
                }
                Ok(HirExpr {
                    kind: HirExprKind::Range {
                        start: Box::new(s),
                        end: Box::new(e),
                        is_inclusive: *is_inclusive,
                    },
                    ty: HirType::I64,
                    span: span_to_source_span(span),
                })
            }
            Expr::StructLiteral { name, fields, span } => {
                let name_id = self.symbols.intern(name);
                let struct_def = ctx
                    .struct_defs
                    .iter()
                    .find(|s| s.name == name_id)
                    .ok_or_else(|| {
                        CompilerError::semantic(format!("undefined struct: {}", name))
                    })?;
                // Build a field type lookup from the struct definition
                let field_type_map: HashMap<SymbolId, HirType> =
                    struct_def.fields.iter().cloned().collect();
                let mut lowered_fields: Vec<(SymbolId, Box<HirExpr>)> = Vec::new();
                for (field_name, field_expr) in fields {
                    let fid = self.symbols.intern(field_name);
                    let expected_ty = field_type_map.get(&fid).copied().ok_or_else(|| {
                        CompilerError::semantic(format!(
                            "struct {} has no field {}",
                            name, field_name
                        ))
                    })?;
                    let expr = self.lower_expr(field_expr, scope, return_type, ctx)?;
                    if expr.ty != expected_ty {
                        return Err(CompilerError::semantic(format!(
                            "field {} expects {:?}, got {:?}",
                            field_name, expected_ty, expr.ty
                        )));
                    }
                    lowered_fields.push((fid, Box::new(expr)));
                }
                Ok(HirExpr {
                    kind: HirExprKind::StructLiteral {
                        name: name_id,
                        fields: lowered_fields,
                    },
                    ty: HirType::Struct(name_id),
                    span: span_to_source_span(span),
                })
            }
            Expr::FieldAccess {
                expr: inner_expr,
                field,
                span,
            } => {
                let inner = self.lower_expr(inner_expr, scope, return_type, ctx)?;
                let struct_sym = match inner.ty {
                    HirType::Struct(s) => s,
                    _ => {
                        return Err(CompilerError::semantic(format!(
                            "field access on non-struct type: {:?}",
                            inner.ty
                        )))
                    }
                };
                let struct_def = ctx
                    .struct_defs
                    .iter()
                    .find(|s| s.name == struct_sym)
                    .ok_or_else(|| {
                        CompilerError::semantic(format!(
                            "undefined struct for field access: {:?}",
                            inner.ty
                        ))
                    })?;
                let field_id = self.symbols.intern(field);
                let field_ty = struct_def
                    .fields
                    .iter()
                    .find(|(f, _)| *f == field_id)
                    .map(|(_, ty)| *ty)
                    .ok_or_else(|| {
                        CompilerError::semantic(format!("struct has no field: {}", field))
                    })?;
                Ok(HirExpr {
                    kind: HirExprKind::FieldAccess {
                        expr: Box::new(inner),
                        field: field_id,
                    },
                    ty: field_ty,
                    span: span_to_source_span(span),
                })
            }
            Expr::EnumConstructor {
                name,
                variant,
                span,
            } => {
                let name_id = self.symbols.intern(name);
                let enum_def = ctx
                    .enum_defs
                    .iter()
                    .find(|e| e.name == name_id)
                    .ok_or_else(|| CompilerError::semantic(format!("undefined enum: {}", name)))?;
                let variant_id = self.symbols.intern(variant);
                let _ = enum_def
                    .variants
                    .iter()
                    .position(|v| *v == variant_id)
                    .ok_or_else(|| {
                        CompilerError::semantic(format!("enum {} has no variant {}", name, variant))
                    })?;
                Ok(HirExpr {
                    kind: HirExprKind::EnumConstructor {
                        name: name_id,
                        variant: variant_id,
                    },
                    ty: HirType::Enum(name_id),
                    span: span_to_source_span(span),
                })
            }
        }
    }
}

/// Lower an `ast::Program` into a `HirProgram`, performing full
/// semantic analysis (type checking, name resolution, mutability).
pub fn lower(program: &Program) -> CompilerResult<HirProgram> {
    let mut hir_lower = HirLower::new();
    hir_lower.lower_program(program)
}

/// Convenience: lower a program and return `Ok(())` or the first error.
/// Preserves the `CompilerResult<()>` signature used by `semantic::analyze`.
pub fn lower_unit(program: &Program) -> CompilerResult<()> {
    lower(program).map(|_| ())
}

/// Convert a `Range<usize>` to a `SourceSpan`.
pub fn range_to_span(range: &std::ops::Range<usize>) -> SourceSpan {
    span_to_source_span(range)
}
