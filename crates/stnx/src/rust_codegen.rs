//! Saturnite-to-Rust source code generation for the hybrid backend.
//!
//! When the hybrid backend is selected, the Saturnite compiler translates the
//! HIR program into Rust source code and compiles it with the real `rustc` /
//! `cargo` toolchain. This module is responsible for the HIR→Rust translation:
//!
//! - Saturnite functions → Rust `fn` items
//! - Saturnite variables → Rust locals (with proper mutability)
//! - Saturnite control flow (`if`, `for`, `while`) → Rust equivalents
//! - Saturnite types (`i64`, `f64`, `bool`, `str`, `unit`) → Rust equivalents
//! - Struct/enum literals → Rust struct/enum construction
//! - `use rust:crate as alias` → `use crate_name as alias;` (so calls like
//!   `alias.func(args)` resolve to `crate_name::func(args)` at link time)
//! - Native Saturnite builtins (`say`, `give`, `let` mutations) → Rust calls to
//!   the C runtime helpers (declared `extern "C"` and linked from the LLVM object)
//!
//! The generated crate is a `staticlib` so it can be linked into the LLVM-produced
//! object. Alternatively, when `--no-link` or a separate Rust entry point is needed,
//! the generated crate can be compiled as a `bin`.

use crate::error::{CompilerError, CompilerResult};
use crate::hir::types::HirType;
use crate::hir::{
    HirExpr, HirExprKind, HirFunction, HirProgram, HirStmt, HirStmtKind,
};
use crate::hir::symbol::SymbolId;
use crate::ast::ExternalKind;
use crate::interop::HirTypeRef;

// ---------------------------------------------------------------------------
// Type translation
// ---------------------------------------------------------------------------

/// Map a Saturnite `HirType` to the Rust `extern "C"` FFI type used in generated
/// shim signatures. Saturnite's `i64`/`f64`/`bool` map directly; `str` becomes
/// `*const i8` (NUL-terminated C string); `Unit` becomes `()` for Rust but `i64`
/// at the ABI boundary (we use `i64` as the carrier for simplicity in the
/// generated code).
pub fn sat_type_to_rust(ty: &HirType) -> &'static str {
    match ty {
        HirType::I64 => "i64",
        HirType::F64 => "f64",
        HirType::Bool => "bool",
        HirType::Str => "&'static str",
        HirType::Unit => "()",
        // Structs, enums, lists, generics, and Apply types are not yet
        // representable as direct Rust FFI types. They require runtime
        // support that is deferred to a future milestone.
        HirType::Struct(_)
        | HirType::Enum(_)
        | HirType::List(_)
        | HirType::Generic(_)
        | HirType::Apply { .. } => "i64",
    }
}

/// Map an ABI-safe `HirTypeRef` (from external declarations) to a Rust `extern "C"`
/// type string. This is used for generating `extern "C"` declarations for C-side
/// runtime functions that the generated Rust code may call.
fn _abi_type_to_rust(abi_ty: &HirTypeRef) -> &'static str {
    match abi_ty {
        HirTypeRef::I8 | HirTypeRef::I16 | HirTypeRef::I32 | HirTypeRef::I64 => "i64",
        HirTypeRef::U8 | HirTypeRef::U16 | HirTypeRef::U32 | HirTypeRef::U64 => "u64",
        HirTypeRef::F32 => "f32",
        HirTypeRef::F64 => "f64",
        HirTypeRef::Bool => "bool",
        HirTypeRef::CStr => "*const std::os::raw::c_char",
        HirTypeRef::Pointer => "*mut std::ffi::c_void",
    }
}

// ---------------------------------------------------------------------------
// Rust code generator
// ---------------------------------------------------------------------------

/// The output of [`RustCodeGenerator`]: Rust source code ready to be compiled
/// by `cargo build` in the generated crate.
#[derive(Debug, Clone)]
pub struct RustSource {
    /// The package (crate) name for the generated crate.
    pub crate_name: String,
    /// The crate's edition string (e.g. `"2021"`).
    pub edition: String,
    /// The dependency list: each entry is `(crate_name, version_or_path)`.
    pub dependencies: Vec<(String, String)>,
    /// The full Rust source for `src/lib.rs` (or `src/main.rs`).
    pub source: String,
}

/// Options controlling how Rust source code is generated.
#[derive(Debug, Clone)]
#[derive(Default)]
pub struct CodegenOptions {
    /// If true, generate a `bin` crate (with a `fn main()`) rather than a
    /// `staticlib` (with `fn run_main() -> i64`). This is used when compiling
    /// the Saturnite program as a standalone Rust executable.
    pub as_binary: bool,
}

/// Translate a `HirProgram` into Rust source code.
///
/// This is the entry point for the hybrid backend's code generation stage.
/// It converts each Saturnite function, struct, and enum into its Rust
/// equivalent, and emits `use` declarations for Rust ecosystem imports so that
/// qualified calls resolve to the real crate APIs.
pub fn generate_rust_source(
    program: &HirProgram,
    crate_name: &str,
    options: &CodegenOptions,
) -> CompilerResult<RustSource> {
    let mut gen = RustCodeGenerator::new(program, crate_name, options);
    gen.generate()?;
    Ok(RustSource {
        crate_name: gen.crate_name.clone(),
        edition: gen.edition.clone(),
        dependencies: gen.dependencies.clone(),
        source: gen.output,
    })
}

/// The HIR→Rust source translator. Builds up Rust source text in `output`.
pub struct RustCodeGenerator<'hir> {
    program: &'hir HirProgram,
    crate_name: String,
    edition: String,
    options: CodegenOptions,
    output: String,
    dependencies: Vec<(String, String)>,
    /// Counter for generating unique temporary variable names.
    _temp_counter: u64,
}

impl<'hir> RustCodeGenerator<'hir> {
    pub fn new(
        program: &'hir HirProgram,
        crate_name: &str,
        options: &CodegenOptions,
    ) -> Self {
        Self {
            program,
            crate_name: crate_name.to_string(),
            edition: "2021".to_string(),
            options: options.clone(),
            output: String::new(),
            dependencies: Vec::new(),
            _temp_counter: 0,
        }
    }

    /// Generate the complete Rust source file.
    pub fn generate(&mut self) -> CompilerResult<()> {
        self.emit_header();
        self.emit_uses();
        self.emit_structs();
        self.emit_enums();
        self.emit_functions();

        if self.options.as_binary {
            self.emit_main_wrapper();
        }

        Ok(())
    }

    /// Emit the file-level header: module doc, extern crate declarations,
    /// and `use` statements for Rust ecosystem imports.
    fn emit_header(&mut self) {
        self.output.push_str(
            r#"//! Auto-generated Saturnite Rust backend crate.
//!
//! This crate was produced by the Saturnite compiler's hybrid backend
//! (Milestone 2). It translates Saturnite HIR into Rust source and compiles
//! it with the real `rustc` toolchain.
//!
//! Do not edit by hand.

"#,
        );

        // Declare extern crates for Rust ecosystem imports.
        for ud in &self.program.use_decls {
            if ud.ecosystem != Some(ExternalKind::Rust) {
                continue;
            }
            let pkg = self
                .program
                .symbols
                .lookup(ud.path.first().copied().unwrap_or(SymbolId(0)))
                .unwrap_or("");
            if !pkg.is_empty() {
                // Sanitize the crate name for Rust (hyphens → underscores).
                let rust_name = pkg.replace('-', "_");
                self.output
                    .push_str(&format!("extern crate {};\n", rust_name));
                self.dependencies.push((pkg.to_string(), rust_name));
            }
        }

        self.output.push('\n');
    }

    /// Emit `use` declarations that alias Rust crate names to the Saturnite
    /// import alias, so qualified calls like `rust_lib.func(args)` resolve
    /// to `crate_name::func(args)`.
    fn emit_uses(&mut self) {
        for ud in &self.program.use_decls {
            if ud.ecosystem != Some(ExternalKind::Rust) {
                continue;
            }
            let pkg = self
                .program
                .symbols
                .lookup(ud.path.first().copied().unwrap_or(SymbolId(0)))
                .unwrap_or("");
            let alias = self
                .program
                .symbols
                .lookup(ud.alias)
                .unwrap_or("");
            if !pkg.is_empty() && !alias.is_empty() {
                let rust_name = pkg.replace('-', "_");
                self.output
                    .push_str(&format!("use {} as {};\n", rust_name, alias));
            }
        }

        if !self
            .program
            .use_decls
            .iter()
            .any(|ud| ud.ecosystem == Some(ExternalKind::Rust))
        {
            // No Rust imports — still emit a blank use section for consistency.
        }

        self.output.push('\n');
    }

    /// Emit struct definitions.
    fn emit_structs(&mut self) {
        for sdef in &self.program.structs {
            let name = self
                .program
                .symbols
                .lookup(sdef.name)
                .unwrap_or("Unknown");

            // Only emit structs that are visible or used by functions we
            // emit. For now, emit all (the linker handles dead code).
            let fields: Vec<String> = sdef
                .fields
                .iter()
                .map(|(fid, ty)| {
                    let fname = self
                        .program
                        .symbols
                        .lookup(*fid)
                        .unwrap_or("field");
                    let ftype = sat_type_to_rust(ty);
                    format!("    pub {}: {}", fname, ftype)
                })
                .collect();

            self.output.push_str(&format!(
                "struct {} {{\n{}\n}}\n\n",
                name,
                fields.join(",\n")
            ));
        }
    }

    /// Emit enum definitions.
    fn emit_enums(&mut self) {
        for edef in &self.program.enums {
            let name = self
                .program
                .symbols
                .lookup(edef.name)
                .unwrap_or("Unknown");

            let variants: Vec<String> = edef
                .variants
                .iter()
                .map(|v| {
                    self.program
                        .symbols
                        .lookup(*v)
                        .unwrap_or("Variant")
                        .to_string()
                })
                .collect();

            // Saturnite enums are represented as i64 tags. We emit them as
            // Rust enums with explicit discriminants so the layout matches.
            let discriminants = variants
                .iter()
                .enumerate()
                .map(|(i, v)| format!("    {} = {}", v, i));

            self.output.push_str(&format!(
                "#[repr(i64)]\nenum {} {{\n{}\n}}\n\n",
                name,
                discriminants.collect::<Vec<_>>().join(",\n")
            ));
        }
    }

    /// Emit all function definitions.
    fn emit_functions(&mut self) {
        for func in &self.program.functions {
            self.emit_function(func);
            self.output.push('\n');
        }
    }

    /// Emit a single function definition in Rust.
    fn emit_function(&mut self, func: &HirFunction) {
        let name = self
            .program
            .symbols
            .lookup(func.name)
            .unwrap_or("fn");

        let params = func
            .params
            .iter()
            .map(|(sym, ty)| {
                let pname = self.program.symbols.lookup(*sym).unwrap_or("arg");
                let ptype = sat_type_to_rust(ty);
                format!("{}: {}", pname, ptype)
            })
            .collect::<Vec<_>>()
            .join(", ");

        let ret_type = sat_type_to_rust(&func.return_type);

        if name == "main" {
            // `main` in Saturnite returns i64 (exit code). In a binary crate,
            // it must return `()`. We wrap it.
            if self.options.as_binary {
                self.output.push_str("fn main() {\n");
                self.output.push_str(&format!("    let exit_code = {}({});\n", name, ""));
                self.output.push_str("    std::process::exit(exit_code as i32);\n");
                self.output.push_str("}\n\n");
            } else {
                self.output
                    .push_str(&format!("pub fn {}({}) -> {} {{\n", name, params, ret_type));
            }
        } else {
            self.output
                .push_str(&format!("pub fn {}({}) -> {} {{\n", name, params, ret_type));
        }

        // Emit function body.
        self.emit_body(&func.body, func);

        self.output.push_str("}\n");
    }

    /// Emit a function body (list of statements).
    fn emit_body(&mut self, body: &[HirStmt], _func: &HirFunction) {
        for stmt in body {
            self.emit_stmt(stmt);
        }
    }

    /// Emit a single statement in Rust.
    fn emit_stmt(&mut self, stmt: &HirStmt) {
        match &stmt.kind {
            HirStmtKind::Let {
                name,
                mutable,
                ty: _,
                value,
            } => {
                let var_name = self
                    .program
                    .symbols
                    .lookup(*name)
                    .unwrap_or("var");
                let mut_str = if *mutable { "mut " } else { "" };
                let val = self.gen_expr(value);
                self.output
                    .push_str(&format!("    let {}{} = {};\n", mut_str, var_name, val));
            }
            HirStmtKind::Expr(expr) => {
                let val = self.gen_expr(expr);
                self.output.push_str(&format!("    {};\n", val));
            }
            HirStmtKind::Return(opt_expr) => {
                match opt_expr {
                    Some(e) => {
                        let val = self.gen_expr(e);
                        self.output.push_str(&format!("    return {};\n", val));
                    }
                    None => {
                        self.output.push_str("    return ();\n");
                    }
                }
            }
            HirStmtKind::Println(expr) => {
                let val = self.gen_expr(expr);
                self.output.push_str(&format!(
                    "    println_i64({});\n",
                    val
                ));
            }
            HirStmtKind::PrintlnStr(expr) => {
                let val = self.gen_expr(expr);
                self.output
                    .push_str(&format!("    println_str({});\n", val));
            }
            HirStmtKind::Raise(expr) => {
                let val = self.gen_expr(expr);
                self.output
                    .push_str(&format!("    eprintln!(\"raise: {{}}\", {});\n", val));
                self.output.push_str("    std::process::exit(1);\n");
            }
            HirStmtKind::StructDef { .. } => {
                // Struct definitions are emitted at the module level in emit_structs.
            }
            HirStmtKind::EnumDef { .. } => {
                // Enum definitions are emitted at the module level in emit_enums.
            }
        }
    }

    /// Generate a Rust expression string from a HIR expression.
    fn gen_expr(&mut self, expr: &HirExpr) -> String {
        match &expr.kind {
            HirExprKind::Integer(n) => n.to_string(),
            HirExprKind::Float(f) => f.to_string(),
            HirExprKind::Bool(b) => b.to_string(),
            HirExprKind::StrLit(sym) => {
                let s = self
                    .program
                    .symbols
                    .lookup(*sym)
                    .unwrap_or("");
                // Rust string literal with proper escaping.
                format!("\"{}\"", escape_rust_string(s))
            }
            HirExprKind::Unit => "()".to_string(),
            HirExprKind::Variable { symbol } => {
                let name = self
                    .program
                    .symbols
                    .lookup(*symbol)
                    .unwrap_or("var");
                name.to_string()
            }
            HirExprKind::Assign { symbol, value } => {
                let name = self
                    .program
                    .symbols
                    .lookup(*symbol)
                    .unwrap_or("var");
                let val = self.gen_expr(value);
                format!("({} = {})", name, val)
            }
            HirExprKind::AugAssign {
                symbol,
                op,
                value,
            } => {
                let name = self
                    .program
                    .symbols
                    .lookup(*symbol)
                    .unwrap_or("var");
                let val = self.gen_expr(value);
                let op_str = match op {
                    crate::ast::AugOp::Add => "+=",
                    crate::ast::AugOp::Sub => "-=",
                    crate::ast::AugOp::Mul => "*=",
                    crate::ast::AugOp::Div => "/=",
                };
                format!("({} {} {})", name, op_str, val)
            }
            HirExprKind::Binary { op, lhs, rhs } => {
                let l = self.gen_expr(lhs);
                let r = self.gen_expr(rhs);
                let op_str = match op {
                    crate::ast::BinOp::Add => "+",
                    crate::ast::BinOp::Sub => "-",
                    crate::ast::BinOp::Mul => "*",
                    crate::ast::BinOp::Div => "/",
                    crate::ast::BinOp::Mod => "%",
                    crate::ast::BinOp::Eq => "==",
                    crate::ast::BinOp::Ne => "!=",
                    crate::ast::BinOp::Lt => "<",
                    crate::ast::BinOp::Gt => ">",
                    crate::ast::BinOp::Le => "<=",
                    crate::ast::BinOp::Ge => ">=",
                    crate::ast::BinOp::And => "&&",
                    crate::ast::BinOp::Or => "||",
                };
                format!("({} {} {})", l, op_str, r)
            }
            HirExprKind::Unary { op, expr: inner } => {
                let v = self.gen_expr(inner);
                let op_str = match op {
                    crate::ast::UnOp::Neg => "-",
                    crate::ast::UnOp::Not => "!",
                };
                format!("({}{})", op_str, v)
            }
            HirExprKind::Call {
                func: def_id,
                args,
                ..
            } => {
                self.gen_call(*def_id, args)
            }
            HirExprKind::If {
                condition,
                then_branch,
                elif_branches,
                else_branch,
            } => {
                let cond = self.gen_expr(condition);
                let mut result = format!("if {} {{\n", cond);
                for stmt in then_branch {
                    self.emit_stmt_rust(stmt, &mut result);
                }
                result.push('}');

                for (elif_cond, elif_body) in elif_branches {
                    let elif_cond_str = self.gen_expr(elif_cond);
                    result.push_str(&format!(" else if {} {{\n", elif_cond_str));
                    for stmt in elif_body {
                        self.emit_stmt_rust(stmt, &mut result);
                    }
                    result.push('}');
                }

                if let Some(eb) = else_branch {
                    result.push_str(" else {\n");
                    for stmt in eb {
                        self.emit_stmt_rust(stmt, &mut result);
                    }
                    result.push('}');
                }

                result.push('\n');
                result
            }
            HirExprKind::For { var, iter, body } => {
                let var_name = self
                    .program
                    .symbols
                    .lookup(*var)
                    .unwrap_or("item");
                let iter_str = self.gen_expr(iter);
                // Saturnite for loops iterate over a range, which in Rust
                // is a `for` over the range values.
                let mut result = format!("for {} in {} {{\n", var_name, iter_str);
                for stmt in body {
                    self.emit_stmt_rust(stmt, &mut result);
                }
                result.push_str("}\n");
                result
            }
            HirExprKind::While { condition, body } => {
                let cond = self.gen_expr(condition);
                let mut result = format!("while {} {{\n", cond);
                for stmt in body {
                    self.emit_stmt_rust(stmt, &mut result);
                }
                result.push_str("}\n");
                result
            }
            HirExprKind::Range {
                start,
                end,
                is_inclusive,
            } => {
                let s = self.gen_expr(start);
                let e = self.gen_expr(end);
                if *is_inclusive {
                    format!("{}..={}", s, e)
                } else {
                    format!("{}..{}", s, e)
                }
            }
            HirExprKind::StructLiteral {
                name,
                fields,
                ..
            } => {
                let sname = self
                    .program
                    .symbols
                    .lookup(*name)
                    .unwrap_or("Unknown");
                let field_strs: Vec<String> = fields
                    .iter()
                    .map(|(fid, val)| {
                        let fname = self
                            .program
                            .symbols
                            .lookup(*fid)
                            .unwrap_or("field");
                        let v = self.gen_expr(val);
                        format!("{}: {}", fname, v)
                    })
                    .collect();
                format!("{} {{ {} }}", sname, field_strs.join(", "))
            }
            HirExprKind::FieldAccess { expr: inner, field } => {
                let base = self.gen_expr(inner);
                let fname = self
                    .program
                    .symbols
                    .lookup(*field)
                    .unwrap_or("field");
                format!("({}).{}", base, fname)
            }
            HirExprKind::ListLiteral { elements } => {
                let elems: Vec<String> = elements.iter().map(|e| self.gen_expr(e)).collect();
                format!("vec![{}]", elems.join(", "))
            }
            HirExprKind::Index { list, index } => {
                let l = self.gen_expr(list);
                let i = self.gen_expr(index);
                format!("({}).({})", l, i) // Saturnite lists are runtime objects; this is approximate
            }
            HirExprKind::Length { expr } => {
                let e = self.gen_expr(expr);
                format!("list_len({})", e) // Runtime call into Saturnite list API
            }
            HirExprKind::EnumConstructor { name, variant } => {
                let ename = self
                    .program
                    .symbols
                    .lookup(*name)
                    .unwrap_or("Unknown");
                let vname = self
                    .program
                    .symbols
                    .lookup(*variant)
                    .unwrap_or("Variant");
                format!("{}::{}", ename, vname)
            }
        }
    }

    /// Generate the call expression for a `DefId` callee. If the callee is an
    /// external (ecosystem) function, generate the appropriate Rust call.
    fn gen_call(&mut self, def_id: crate::hir::symbol::DefId, args: &[HirExpr]) -> String {
        // Check if this is an external function call.
        if let Some(ext) = self.program.external_functions.iter().find(|e| e.def_id == def_id) {
            return self.gen_external_call(ext, args);
        }

        // Otherwise, it's a regular Saturnite function call.
        let fname = self
            .program
            .functions
            .iter()
            .find(|f| f.def_id == def_id)
            .and_then(|f| self.program.symbols.lookup(f.name))
            .unwrap_or("unknown");
        let arg_strs: Vec<String> = args.iter().map(|a| self.gen_expr(a)).collect();
        format!("{}({})", fname, arg_strs.join(", "))
    }

    /// Generate a call to an external (ecosystem) function in Rust.
    fn gen_external_call(
        &mut self,
        ext: &crate::hir::function::HirExternalFunction,
        args: &[HirExpr],
    ) -> String {
        match ext.kind {
            ExternalKind::Rust => {
                // Rust external calls resolve through the `use` alias.
                // The external function's `name` is the interned qualified
                // name (e.g. "rust_lib.add"). We need to translate this back
                // to a Rust call expression.
                let ext_name = self
                    .program
                    .symbols
                    .lookup(ext.name)
                    .unwrap_or("");

                // The qualified name is "alias.member". In Rust, this is
                // `alias::member(args)`. But the alias is a Saturnite `use`
                // alias that maps to a crate name, so `alias.member` in Saturnite
                // becomes `alias::member` in Rust (since we emitted
                // `use crate_name as alias`).
                let parts: Vec<&str> = ext_name.splitn(2, '.').collect();
                let call = if parts.len() == 2 {
                    format!("{}::{}", parts[0], parts[1])
                } else {
                    ext_name.to_string()
                };

                let arg_strs: Vec<String> = args.iter().map(|a| self.gen_expr(a)).collect();
                format!("{}({})", call, arg_strs.join(", "))
            }
            ExternalKind::Python => {
                // Python calls go through the runtime bridge.
                // The symbol is "module::function".
                let arg_strs: Vec<String> = args.iter().map(|a| self.gen_expr(a)).collect();
                format!("sat_py_call(\"{}\", {})", ext.symbol, arg_strs.join(", "))
            }
            ExternalKind::Native => {
                // Native calls are direct C symbol calls.
                let arg_strs: Vec<String> = args.iter().map(|a| self.gen_expr(a)).collect();
                format!("{}({})", ext.symbol, arg_strs.join(", "))
            }
        }
    }

    /// Emit a statement into a Rust string (used for nested expressions like
    /// if/while/for bodies which are Rust expressions, not statements).
    fn emit_stmt_rust(&mut self, stmt: &HirStmt, out: &mut String) {
        match &stmt.kind {
            HirStmtKind::Let {
                name,
                mutable,
                ty: _,
                value,
            } => {
                let var_name = self
                    .program
                    .symbols
                    .lookup(*name)
                    .unwrap_or("var");
                let mut_str = if *mutable { "mut " } else { "" };
                let val = self.gen_expr(value);
                out.push_str(&format!("    let {}{} = {};\n", mut_str, var_name, val));
            }
            HirStmtKind::Expr(expr) => {
                let val = self.gen_expr(expr);
                out.push_str(&format!("    {};\n", val));
            }
            HirStmtKind::Return(opt_expr) => {
                match opt_expr {
                    Some(e) => {
                        let val = self.gen_expr(e);
                        out.push_str(&format!("    return {};\n", val));
                    }
                    None => {
                        out.push_str("    return ();\n");
                    }
                }
            }
            HirStmtKind::Println(expr) => {
                let val = self.gen_expr(expr);
                out.push_str(&format!("    println_i64({});\n", val));
            }
            HirStmtKind::PrintlnStr(expr) => {
                let val = self.gen_expr(expr);
                out.push_str(&format!("    println_str({});\n", val));
            }
            HirStmtKind::Raise(expr) => {
                let val = self.gen_expr(expr);
                out.push_str(&format!(
                    "    eprintln!(\"raise: {{}}\", {});\n",
                    val
                ));
                out.push_str("    std::process::exit(1);\n");
            }
            HirStmtKind::StructDef { .. } | HirStmtKind::EnumDef { .. } => {
                // Struct and enum definitions are emitted at module level.
            }
        }
    }

    /// Emit the `main` wrapper that calls the Saturnite `main` function and
    /// exits with its return value (when generating a binary crate).
    fn emit_main_wrapper(&mut self) {
        // When as_binary is true, we already emitted a `fn main()` wrapper
        // inside emit_function. No additional wrapper needed.
    }
}

/// Escape a string for inclusion in a Rust string literal.
pub fn escape_rust_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

// ---------------------------------------------------------------------------
// Cargo.toml generation
// ---------------------------------------------------------------------------

/// Generate a `Cargo.toml` content for the Rust backend crate.
pub fn generate_cargo_toml(
    crate_name: &str,
    edition: &str,
    dependencies: &[(String, String)],
    as_binary: bool,
) -> String {
    let crate_type = if as_binary {
        ""
    } else {
        r#"crate-type = ["staticlib"]

"#
    };

    let deps_section: String = if dependencies.is_empty() {
        "[dependencies]".to_string()
    } else {
        let mut lines = vec!["[dependencies]".to_string()];
        for (name, version) in dependencies {
            // If the dependency version contains a path, use path syntax.
            if version.starts_with('/') || version.contains("path =") {
                lines.push(format!("{} = {{ {} }}", name.replace('-', "_"), version));
            } else {
                lines.push(format!("{} = \"{}\"", name.replace('-', "_"), version));
            }
        }
        lines.join("\n")
    };

    format!(
        r#"[package]
name = "{crate_name}"
version = "0.1.0"
edition = "{edition}"

{crate_type}{deps_section}
"#,
        crate_name = crate_name,
        edition = edition,
        crate_type = crate_type,
        deps_section = deps_section
    )
}

// ---------------------------------------------------------------------------
// Build driver: compile generated Rust source with cargo
// ---------------------------------------------------------------------------

/// Build the generated Rust crate using `cargo build`.
///
/// Uses `std::process::Command` (process API, not shell strings) to invoke
/// `cargo`. Returns the path to the built artifact (`.a` static library or
/// executable).
pub fn build_rust_crate(
    build_dir: &std::path::Path,
    target_triple: Option<&str>,
    as_binary: bool,
) -> CompilerResult<std::path::PathBuf> {
    let mut cmd = std::process::Command::new("cargo");
    cmd.arg("build").arg("--release");

    if as_binary {
        cmd.arg("--bin").arg(build_dir.file_name().unwrap());
    }

    if let Some(triple) = target_triple {
        cmd.arg("--target").arg(triple);
    }

    cmd.current_dir(build_dir);

    let output = cmd.output().map_err(|e| {
        CompilerError::codegen(format!("failed to invoke cargo: {}", e))
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(CompilerError::codegen(format!(
            "cargo build failed:\n--- stdout ---\n{}\n--- stderr ---\n{}",
            stdout, stderr
        )));
    }

    // Locate the built artifact.
    let release_dir = build_dir.join("target").join("release");

    if as_binary {
        // For binary crates, the executable is in the release dir.
        let exe_name = build_dir
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let exe_path = release_dir.join(&exe_name);
        if exe_path.exists() {
            return Ok(exe_path);
        }
        // On Windows, check for .exe
        let exe_path_win = release_dir.join(format!("{}.exe", exe_name));
        if exe_path_win.exists() {
            return Ok(exe_path_win);
        }
    } else {
        // For staticlib crates, find the .a / .lib file.
        let lib_name = format!("lib{}.a", build_dir.file_name().unwrap().to_string_lossy());
        let lib_path = release_dir.join(&lib_name);
        if lib_path.exists() {
            return Ok(lib_path);
        }
        let lib_name_lib = format!("{}.lib", build_dir.file_name().unwrap().to_string_lossy());
        let lib_path_lib = release_dir.join(&lib_name_lib);
        if lib_path_lib.exists() {
            return Ok(lib_path_lib);
        }
    }

    Err(CompilerError::codegen(format!(
        "build artifact not found in {}",
        release_dir.display()
    )))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn simple_program() -> crate::hir::HirProgram {
        // Build a minimal HIR program with a `main` function
        // that returns 42.
        use crate::hir::symbol::{DefId, SymbolInterner};
        use crate::hir::types::HirType;
        use crate::hir::{HirFunction, HirStmt, HirStmtKind, HirExpr, HirExprKind};
        use crate::module::ModuleId;

        let mut symbols = SymbolInterner::default();
        let main_name = symbols.intern("main");
        symbols.intern("");

        let mut body = vec![HirStmt {
            kind: HirStmtKind::Let {
                name: symbols.intern("x"),
                mutable: false,
                ty: Some(HirType::I64),
                value: HirExpr {
                    kind: HirExprKind::Integer(42),
                    ty: HirType::I64,
                    span: miette::SourceSpan::new(0.into(), 0),
                },
            },
            span: miette::SourceSpan::new(0.into(), 0),
        }];
        body.push(HirStmt {
            kind: HirStmtKind::Return(Some(HirExpr {
                kind: HirExprKind::Variable { symbol: symbols.intern("x") },
                ty: HirType::I64,
                span: miette::SourceSpan::new(0.into(), 0),
            })),
            span: miette::SourceSpan::new(0.into(), 0),
        });

        let func = HirFunction {
            def_id: DefId(0),
            name: main_name,
            generic_params: Vec::new(),
            params: Vec::new(),
            return_type: HirType::I64,
            body,
            span: miette::SourceSpan::new(0.into(), 0),
            module: ModuleId::ROOT,
            visibility: crate::hir::symbol::Visibility::Private,
        };

        crate::hir::HirProgram {
            functions: vec![func],
            structs: Vec::new(),
            enums: Vec::new(),
            symbols,
            external_functions: Vec::new(),
            modules: Vec::new(),
            root_module: ModuleId::ROOT,
            module_paths: std::collections::HashMap::new(),
            def_table: crate::hir::symbol::DefTable::default(),
            module_scopes: Vec::new(),
            use_decls: Vec::new(),
            mod_decls: Vec::new(),
            python_search_path: String::new(),
        }
    }

    #[test]
    fn test_sat_type_to_rust() {
        assert_eq!(sat_type_to_rust(&crate::hir::HirType::I64), "i64");
        assert_eq!(sat_type_to_rust(&crate::hir::HirType::F64), "f64");
        assert_eq!(sat_type_to_rust(&crate::hir::HirType::Bool), "bool");
        assert_eq!(sat_type_to_rust(&crate::hir::HirType::Str), "&'static str");
        assert_eq!(sat_type_to_rust(&crate::hir::HirType::Unit), "()");
    }

    #[test]
    fn test_escape_rust_string() {
        assert_eq!(escape_rust_string("hello"), "hello");
        assert_eq!(escape_rust_string("say \"hi\""), "say \\\"hi\\\"");
        assert_eq!(escape_rust_string("a\\b"), "a\\\\b");
    }

    #[test]
    fn test_generate_rust_source_simple() {
        let program = simple_program();
        let result = generate_rust_source(&program, "test_crate", &CodegenOptions::default());
        assert!(result.is_ok(), "generate_rust_source should succeed");

        let src = result.unwrap();
        assert!(src.source.contains("pub fn main() -> i64"));
        assert!(src.source.contains("let x = 42"));
        assert!(src.source.contains("return x;"));
    }

    #[test]
    fn test_generate_cargo_toml_basic() {
        let toml = generate_cargo_toml("my_crate", "2021", &[], false);
        assert!(toml.contains("name = \"my_crate\""));
        assert!(toml.contains("edition = \"2021\""));
        assert!(toml.contains("crate-type = [\"staticlib\"]"));
    }

    #[test]
    fn test_generate_cargo_toml_with_deps() {
        let deps = vec![("serde".to_string(), "1.0".to_string())];
        let toml = generate_cargo_toml("my_crate", "2021", &deps, true);
        assert!(toml.contains("serde = \"1.0\""));
        assert!(!toml.contains("crate-type"));
    }

    #[test]
    fn test_escape_rust_string_with_quotes() {
        let escaped = escape_rust_string("hello \"world\"");
        assert_eq!(escaped, "hello \\\"world\\\"");
    }
}
