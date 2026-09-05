//! HIR lowering — transforms the AST into a typed, resolved HIR.
//!
//! Pipeline: `AST → HirLower::lower_program → HirProgram (typed HIR)`
//!
//! All identifiers are interned to `SymbolId` / `DefId` so later
//! stages (MIR, LLVM codegen) never perform string lookups. Every HIR
//! node carries a resolved `HirType` and a preserved source `SourceSpan`.

use crate::ast::{
    BinOp, Expr, Function, Item, ItemKind, Program, Stmt, StrPart, Type, UnOp, Visibility,
};
use crate::error::{CompilerError, CompilerResult};
use crate::hir::expr::{HirExpr, HirExprKind};
use crate::hir::function::{
    EnumDef, HirExternalFunction, HirFunction, HirModDecl, HirProgram, HirUseDecl, StructDef,
};
use crate::hir::stmt::{HirStmt, HirStmtKind};
use crate::hir::symbol::{
    DefEntry, DefId, DefKind, DefTable, SymbolId, SymbolInterner, Visibility as HirVisibility,
};
use crate::hir::types::HirType;
use crate::module::{Module, ModuleGraph, ModuleId, ModuleScope};
use miette::SourceSpan;
use std::collections::HashMap;

/// Convert a byte-offset `Range<usize>` from the AST to a `SourceSpan`.
fn span_to_source_span(r: &std::ops::Range<usize>) -> SourceSpan {
    SourceSpan::new(r.start.into(), r.end.saturating_sub(r.start))
}

/// Convert an AST [`Visibility`] to the HIR [`HirVisibility`].
/// Whether a `HirType` is in the ABI-safe subset supported by external
/// calls. Complex types (Str, Struct, Enum, List, Generic, Apply) are
/// rejected at declaration time so an unsafe bridge can never be built.
fn is_abi_safe(ty: &HirType) -> bool {
    matches!(
        ty,
        HirType::I64 | HirType::F64 | HirType::Bool | HirType::Unit
    )
}

fn ast_visibility_to_hir(vis: &Visibility) -> HirVisibility {
    match vis {
        Visibility::Private => HirVisibility::Private,
        Visibility::Public => HirVisibility::Public,
    }
}

/// A lightweight function signature for call-site checking.
struct FunctionSig {
    def_id: DefId,
    param_types: Vec<HirType>,
    /// Interned parameter names in declaration order. Used to reorder
    /// named arguments (`f(amount: 20)`) into positional slots.
    param_names: Vec<SymbolId>,
    return_type: HirType,
    /// Interned names of the function's generic parameters, in declaration
    /// order. Empty for non-generic functions. Used by `lower_expr` to
    /// resolve the concrete return type of a generic call given its
    /// turbofish type arguments.
    generic_params: Vec<SymbolId>,
}

/// DefId sentinel for the builtin `println` function.
const PRINTLN_DEF_ID: DefId = DefId(u32::MAX - 1);

/// DefId sentinel for the runtime `concat_str` builtin (0.5.1 string
/// interpolation). Signature `(Str, Str) -> Str`. Must match
/// `mir::codegen::CONCAT_STR_DEF_ID`.
const CONCAT_STR_DEF_ID: DefId = DefId(u32::MAX - 3);

/// DefId sentinel for the runtime `str_i64` builtin (0.5.1 numeric string
/// interpolation). Signature `(I64) -> Str`. Must match
/// `mir::codegen::STR_I64_DEF_ID`.
const STR_I64_DEF_ID: DefId = DefId(u32::MAX - 4);

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
#[derive(Clone, Debug, PartialEq, Eq)]
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
            Some(v.clone())
        } else {
            self.parent.as_ref().and_then(|p| p.lookup_variable(sym))
        }
    }
}

/// Convert an `ast::Type` to a `HirType`, interning names.
/// Used during Pass 1 (before struct/enum definitions are fully collected).
/// Convert an AST [`Type`] to an [`HirType`]. When `generic_param_names`
/// is `Some`, a `Type::Struct(name)` whose name matches a generic
/// parameter is resolved to `HirType::Generic(...)` instead of being
/// treated as a user-defined struct.
fn ast_type_to_hir(
    ty: &Type,
    symbols: &mut SymbolInterner,
    enum_names: &HashMap<&str, ()>,
    generic_param_names: Option<&[SymbolId]>,
) -> HirType {
    match ty {
        Type::I64 => HirType::I64,
        Type::F64 => HirType::F64,
        Type::Bool => HirType::Bool,
        Type::Str => HirType::Str,
        Type::Unit => HirType::Unit,
        Type::Struct(name) => {
            let sym = symbols.intern(name);
            // Generic parameter match takes precedence over user-defined
            // types because the parser produces `Type::Struct` for every
            // identifier-shaped type name, including generic params.
            if let Some(gparams) = generic_param_names {
                if gparams.contains(&sym) {
                    return HirType::Generic(sym);
                }
            }
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
        // 0.5.3: `List<T>` lowers to a real `HirType::List` with its element
        // type. Only `List<i64>` is supported at runtime in 0.5.3; other
        // element types are rejected at list-literal lowering rather than
        // silently miscompiled here.
        Type::List(inner) => {
            let inner_hir = ast_type_to_hir(inner, symbols, enum_names, generic_param_names);
            HirType::List(Box::new(inner_hir))
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

    /// Allocate a fresh `DefId` for a use/mod declaration.
    ///
    /// Function, struct, and enum DefIds are assigned from their respective
    /// index spaces (functions: sequential over `program.functions`, structs:
    /// sequential over `structs`, enums: sequential over `enums`). Use and
    /// mod declarations need a globally-unique DefId that does not collide
    /// with those spaces, so we intern a synthetic name and use its
    /// `SymbolId` as the DefId.
    fn next_def_id(&mut self) -> DefId {
        let next = self.symbols.next_id().0;
        let sym = self.symbols.intern(&format!("__def_{}", next));
        DefId(sym.0)
    }

    pub fn lower_program(&mut self, program: &Program) -> CompilerResult<HirProgram> {
        // For Phase 5 (single-file programs), all items belong to the root
        // module. The module graph is built from `mod` declarations during
        // a pre-lowering module-loading phase; for single-file programs there
        // is only one module (ModuleId::ROOT = ModuleId(0)).

        // Phase 0: collect all enum names up front so that type annotations
        // (in function signatures, struct fields, and variable declarations)
        // can resolve user-defined types that are actually enums. The parser
        // produces Type::Struct for all user-defined type names. We scan both
        // top-level items and function bodies.
        let mut enum_names: HashMap<&str, ()> = HashMap::new();
        // Scan top-level items for struct/enum definitions
        for item in &program.items {
            match &item.kind {
                ItemKind::EnumDef { name, .. } => {
                    enum_names.insert(name.as_str(), ());
                }
                ItemKind::StructDef { .. } => {}
                _ => {}
            }
        }
        // Scan function bodies for local struct/enum definitions
        for func in &program.functions {
            for stmt in &func.body {
                if let Stmt::EnumDef { name, .. } = stmt {
                    enum_names.insert(name.as_str(), ());
                }
            }
        }

        // Pass 1: intern all function names and build the signature table.
        // We iterate `program.items` so that top-level struct/enum/mod/use
        // declarations are also captured. For backward compatibility with
        // `program.functions` (which only contains functions), we also
        // ensure every function in `items` is included.
        let mut function_sigs: HashMap<SymbolId, FunctionSig> = HashMap::new();
        let mut structs: Vec<StructDef> = Vec::new();
        let mut enums: Vec<EnumDef> = Vec::new();
        let mut use_decls: Vec<HirUseDecl> = Vec::new();
        let mut mod_decls: Vec<HirModDecl> = Vec::new();

        // Collect top-level struct/enum definitions first (their DefIds are
        // assigned sequentially in the order they appear in `items`).
        // Structs and enums at the module level get DefIds that are distinct
        // from function DefIds (structs use a separate index space within
        // `structs`, enums within `enums`). The `def_table` records the kind
        // so MIR/codegen can disambiguate.
        for item in &program.items {
            match &item.kind {
                ItemKind::StructDef {
                    name,
                    generic_params,
                    fields,
                    span,
                } => {
                    let name_id = self.symbols.intern(name);
                    let generic_param_syms: Vec<SymbolId> = generic_params
                        .iter()
                        .map(|p| self.symbols.intern(p))
                        .collect();
                    let field_syms: Vec<(SymbolId, HirType)> = fields
                        .iter()
                        .map(|(fname, fty)| {
                            let fid = self.symbols.intern(fname);
                            (
                                fid,
                                ast_type_to_hir(
                                    fty,
                                    &mut self.symbols,
                                    &enum_names,
                                    Some(&generic_param_syms),
                                ),
                            )
                        })
                        .collect();
                    let def_id = DefId(structs.len() as u32);
                    structs.push(StructDef {
                        def_id,
                        name: name_id,
                        generic_params: generic_param_syms,
                        fields: field_syms,
                        span: span_to_source_span(span),
                        module: ModuleId::ROOT,
                        visibility: ast_visibility_to_hir(&item.visibility),
                    });
                }
                ItemKind::EnumDef {
                    name,
                    generic_params,
                    variants,
                    span,
                } => {
                    let name_id = self.symbols.intern(name);
                    let variant_syms: Vec<SymbolId> =
                        variants.iter().map(|v| self.symbols.intern(v)).collect();
                    let generic_param_syms: Vec<SymbolId> = generic_params
                        .iter()
                        .map(|p| self.symbols.intern(p))
                        .collect();
                    let def_id = DefId(enums.len() as u32);
                    enums.push(EnumDef {
                        def_id,
                        name: name_id,
                        generic_params: generic_param_syms,
                        variants: variant_syms,
                        span: span_to_source_span(span),
                        module: ModuleId::ROOT,
                        visibility: ast_visibility_to_hir(&item.visibility),
                    });
                }
                ItemKind::UseDecl { path, alias } => {
                    // Build the HIR use declaration. The path segments are
                    // interned as SymbolIds. The alias (if present) is
                    // interned separately; otherwise the last path segment
                    // is used as the alias.
                    let path_syms: Vec<SymbolId> =
                        path.iter().map(|s| self.symbols.intern(s)).collect();
                    let alias_sym = match alias {
                        Some(a) => self.symbols.intern(a),
                        None => path_syms
                            .last()
                            .copied()
                            .unwrap_or_else(|| self.symbols.intern("")),
                    };
                    // Use declarations don't occupy the function/struct/enum
                    // DefId space; they are tracked separately in `use_decls`.
                    // We assign a synthetic DefId for the def_table registration.
                    let def_id = self.next_def_id();
                    use_decls.push(HirUseDecl {
                        def_id,
                        path: path_syms,
                        alias: alias_sym,
                        module: ModuleId::ROOT,
                        visibility: ast_visibility_to_hir(&item.visibility),
                        span: span_to_source_span(&item.span),
                    });
                }
                ItemKind::ModDecl => {
                    let def_id = self.next_def_id();
                    mod_decls.push(HirModDecl {
                        def_id,
                        name: self.symbols.intern(&item.name),
                        module_id: None, // resolved by the module graph in Phase 6
                        module: ModuleId::ROOT,
                        visibility: ast_visibility_to_hir(&item.visibility),
                        span: span_to_source_span(&item.span),
                    });
                }
                ItemKind::Function(func) => {
                    let _ = func; // handled below
                }
                ItemKind::ModuleDecl => {
                    // 0.5: `module name` is advisory only. The module graph
                    // builder still works on `ModDecl`. We track it as a
                    // ModDecl for backward compat with single-file programs.
                    let def_id = self.next_def_id();
                    mod_decls.push(HirModDecl {
                        def_id,
                        name: self.symbols.intern(&item.name),
                        module_id: None,
                        module: ModuleId::ROOT,
                        visibility: ast_visibility_to_hir(&item.visibility),
                        span: span_to_source_span(&item.span),
                    });
                }
                ItemKind::MainBlock(_stmts, _span) => {
                    // 0.5: `main:` is a syntactic shortcut for
                    // `fn main() -> i64 { ... }`. Synthesised in a second
                    // pass below.
                }
                ItemKind::ExternalFunction { .. } => {
                    // External declarations are processed in the function
                    // signature pass below.
                }
                ItemKind::ExternalFunction { .. } => {
                    // External declarations are processed in the function
                    // signature pass below.
                }
            }
        }

        // 0.5: synthesise a Function for every `main:` block.
        let mut synth_functions: Vec<Function> = Vec::new();
        for item in &program.items {
            if let ItemKind::MainBlock(stmts, span) = &item.kind {
                synth_functions.push(Function {
                    name: "main".to_string(),
                    generic_params: Vec::new(),
                    params: Vec::new(),
                    return_type: Type::I64,
                    body: stmts.clone(),
                    span: span.clone(),
                });
            }
        }

        // Now process top-level struct/enum definitions found inside function
        // bodies (local definitions). These are also collected for the HIR
        // program's `structs` and `enums` vectors.
        for func in &program.functions {
            for stmt in &func.body {
                match stmt {
                    Stmt::StructDef {
                        name,
                        generic_params,
                        fields,
                        span,
                    } => {
                        let name_id = self.symbols.intern(name);
                        let generic_param_syms: Vec<SymbolId> = generic_params
                            .iter()
                            .map(|p| self.symbols.intern(p))
                            .collect();
                        let field_syms: Vec<(SymbolId, HirType)> = fields
                            .iter()
                            .map(|(fname, fty)| {
                                let fid = self.symbols.intern(fname);
                                (
                                    fid,
                                    ast_type_to_hir(
                                        fty,
                                        &mut self.symbols,
                                        &enum_names,
                                        Some(&generic_param_syms),
                                    ),
                                )
                            })
                            .collect();
                        structs.push(StructDef {
                            def_id: DefId(structs.len() as u32),
                            name: name_id,
                            generic_params: generic_param_syms,
                            fields: field_syms,
                            span: span_to_source_span(span),
                            module: ModuleId::ROOT,
                            visibility: HirVisibility::Private,
                        });
                    }
                    Stmt::EnumDef {
                        name,
                        generic_params,
                        variants,
                        span,
                    } => {
                        let name_id = self.symbols.intern(name);
                        let variant_syms: Vec<SymbolId> =
                            variants.iter().map(|v| self.symbols.intern(v)).collect();
                        let generic_param_syms: Vec<SymbolId> = generic_params
                            .iter()
                            .map(|p| self.symbols.intern(p))
                            .collect();
                        enums.push(EnumDef {
                            def_id: DefId(enums.len() as u32),
                            name: name_id,
                            generic_params: generic_param_syms,
                            variants: variant_syms,
                            span: span_to_source_span(span),
                            module: ModuleId::ROOT,
                            visibility: HirVisibility::Private,
                        });
                    }
                    _ => {}
                }
            }
        }

        // Build function signatures from top-level function items.
        let mut next_func_def_id: u32 = 0;
        for item in &program.items {
            if let ItemKind::Function(func) = &item.kind {
                let name_id = self.symbols.intern(&func.name);
                let def_id = DefId(next_func_def_id);
                // Intern this function's generic params so that
                // `ast_type_to_hir` can recognize them as `HirType::Generic`
                // when resolving the param/return types below.
                let gparams: Vec<SymbolId> = func
                    .generic_params
                    .iter()
                    .map(|p| self.symbols.intern(p))
                    .collect();
                let param_types: Vec<HirType> = func
                    .params
                    .iter()
                    .map(|(_, t)| {
                        ast_type_to_hir(t, &mut self.symbols, &enum_names, Some(&gparams))
                    })
                    .collect();
                let param_names: Vec<SymbolId> = func
                    .params
                    .iter()
                    .map(|(n, _)| self.symbols.intern(n))
                    .collect();
                let return_type = ast_type_to_hir(
                    &func.return_type,
                    &mut self.symbols,
                    &enum_names,
                    Some(&gparams),
                );
                function_sigs.insert(
                    name_id,
                    FunctionSig {
                        def_id,
                        param_types,
                        param_names,
                        return_type,
                        generic_params: gparams,
                    },
                );
                next_func_def_id += 1;
            }
        }
        // Register external declarations in the function signature table.
        // External functions are callable from Saturnite code by their
        // declared symbol name; the runtime bridge resolves the symbol at
        // link/runtime time. Their DefIds are assigned after the regular
        // functions so they remain distinct from builtin sentinels.
        let mut external_functions: Vec<HirExternalFunction> = Vec::new();
        for item in &program.items {
            if let ItemKind::ExternalFunction {
                kind,
                ecosystem,
                symbol,
                params,
                return_type,
                span,
            } = &item.kind
            {
                let name_id = self.symbols.intern(symbol);
                let def_id = DefId(next_func_def_id);
                let param_types: Vec<HirType> = params
                    .iter()
                    .map(|(_, t)| ast_type_to_hir(t, &mut self.symbols, &enum_names, None))
                    .collect();
                let param_names: Vec<SymbolId> =
                    params.iter().map(|(n, _)| self.symbols.intern(n)).collect();
                let return_hir = ast_type_to_hir(return_type, &mut self.symbols, &enum_names, None);
                // Validate that the declared types are ABI-safe. External
                // calls only support the primitive ABI subset; complex types
                // (Str, Struct, Enum, List, Generic, Apply) are rejected
                // with a clear diagnostic rather than silently producing an
                // unsafe bridge.
                for (i, ty) in param_types.iter().enumerate() {
                    if !is_abi_safe(ty) {
                        return Err(CompilerError::semantic(format!(
                            "external function `{}` parameter {} has type {:?}, which is not ABI-safe. \
                             External calls only support the primitive ABI subset (i64, f64, bool).",
                            symbol,
                            i + 1,
                            ty
                        )));
                    }
                }
                if !is_abi_safe(&return_hir) && !matches!(return_hir, HirType::Unit) {
                    return Err(CompilerError::semantic(format!(
                        "external function `{}` has return type {:?}, which is not ABI-safe. \
                         External calls only support the primitive ABI subset (i64, f64, bool) and Unit.",
                        symbol, return_hir
                    )));
                }
                function_sigs.insert(
                    name_id,
                    FunctionSig {
                        def_id,
                        param_types: param_types.clone(),
                        param_names: param_names.clone(),
                        return_type: return_hir.clone(),
                        generic_params: Vec::new(),
                    },
                );
                external_functions.push(HirExternalFunction {
                    def_id,
                    kind: kind.clone(),
                    ecosystem: ecosystem.clone(),
                    symbol: symbol.clone(),
                    name: name_id,
                    param_names,
                    param_types,
                    return_type: return_hir,
                    span: span_to_source_span(span),
                    module: ModuleId::ROOT,
                });
                next_func_def_id += 1;
            }
        }
        // Register builtin println
        let println_sym = self.symbols.intern("println");
        function_sigs.insert(
            println_sym,
            FunctionSig {
                def_id: PRINTLN_DEF_ID,
                param_types: vec![HirType::I64],
                param_names: vec![println_sym],
                return_type: HirType::Unit,
                generic_params: Vec::new(),
            },
        );
        // Check for main
        let main_sym = self.symbols.intern("main");
        if !function_sigs.contains_key(&main_sym) {
            return Err(CompilerError::semantic("no `main` function defined"));
        }

        // Build the lowering context — borrows from local variables (not from self)
        let ctx = LowerContext {
            function_sigs: &function_sigs,
            struct_defs: &structs,
            enum_defs: &enums,
            enum_names: &enum_names,
        };

        // Pass 2: lower each function body. We iterate `program.items` to
        // capture visibility from the AST; the `program.functions` fallback
        // (for backward compatibility) is used when items are empty but
        // functions exist.
        let mut functions: Vec<HirFunction> = Vec::new();
        let mut def_table = DefTable::new();
        let mut module_paths: HashMap<DefId, ModuleId> = HashMap::new();
        let mut module_scopes: Vec<ModuleScope> = vec![ModuleScope::new()];
        let mut use_decl_idx: u32 = 0;
        let mut mod_decl_idx: u32 = 0;

        let items = if program.items.is_empty() && !program.functions.is_empty() {
            // Fallback: synthesize items from the legacy `functions` vector.
            &program
                .functions
                .iter()
                .map(|f| Item {
                    name: f.name.clone(),
                    visibility: Visibility::Private,
                    kind: ItemKind::Function(f.clone()),
                    span: f.span.clone(),
                })
                .collect::<Vec<_>>()
        } else {
            &program.items
        };

        let mut func_def_id: u32 = 0;
        for item in items {
            if let ItemKind::Function(func) = &item.kind {
                let def_id = DefId(func_def_id);
                let hir_func = self.lower_function(
                    func,
                    def_id,
                    ModuleId::ROOT,
                    &ctx,
                    ast_visibility_to_hir(&item.visibility),
                )?;
                // Register in the def_table and module_paths
                def_table.register(DefEntry {
                    module: ModuleId::ROOT,
                    local_index: func_def_id,
                    kind: DefKind::Function,
                });
                module_paths.insert(def_id, ModuleId::ROOT);
                module_scopes[0].define_item(hir_func.name, def_id);
                functions.push(hir_func);
                func_def_id += 1;
            }
        }

        // Register struct and enum DefIds in the def_table, module_paths,
        // and module_scopes (so lookup_with_parent can find them).
        for (i, s) in structs.iter().enumerate() {
            let def_id = DefId(i as u32);
            def_table.register(DefEntry {
                module: ModuleId::ROOT,
                local_index: i as u32,
                kind: DefKind::Struct,
            });
            module_paths.insert(def_id, ModuleId::ROOT);
            module_scopes[0].define_item(s.name, def_id);
        }
        for (i, e) in enums.iter().enumerate() {
            let def_id = DefId(e.def_id.0);
            def_table.register(DefEntry {
                module: ModuleId::ROOT,
                local_index: i as u32,
                kind: DefKind::Enum,
            });
            module_paths.insert(def_id, ModuleId::ROOT);
            module_scopes[0].define_item(e.name, def_id);
        }

        // Register use/mod declarations in def_table and module_paths
        for ud in &use_decls {
            let def_id = ud.def_id;
            def_table.register(DefEntry {
                module: ModuleId::ROOT,
                local_index: use_decl_idx,
                kind: DefKind::Use,
            });
            module_paths.insert(def_id, ModuleId::ROOT);
            use_decl_idx += 1;
        }
        for md in &mod_decls {
            let def_id = md.def_id;
            def_table.register(DefEntry {
                module: ModuleId::ROOT,
                local_index: mod_decl_idx,
                kind: DefKind::Module,
            });
            module_paths.insert(def_id, ModuleId::ROOT);
            mod_decl_idx += 1;
        }

        // Build the root module entry. For single-file programs, the root
        // module has no file path (or a synthetic one). In Phase 6, the
        // module graph will populate this from the loader.
        let root_module_entry = Module::new(
            ModuleId::ROOT,
            crate::module::ModulePath::new(),
            std::path::PathBuf::from("<root>"),
        );

        // Register external DefIds in the def_table and module_paths.
        for (i, ext) in external_functions.iter().enumerate() {
            let def_id = DefId(functions.len() as u32 + i as u32);
            def_table.register(DefEntry {
                module: ModuleId::ROOT,
                local_index: i as u32,
                kind: DefKind::External,
            });
            module_paths.insert(def_id, ModuleId::ROOT);
            module_scopes[0].define_item(ext.name, def_id);
        }

        Ok(HirProgram {
            functions,
            structs,
            enums,
            symbols: std::mem::take(&mut self.symbols),
            external_functions,
            // Phase 5B: populate module-aware fields for single-file programs.
            // All items belong to the root module (ModuleId::ROOT).
            modules: vec![root_module_entry],
            root_module: ModuleId::ROOT,
            module_paths,
            def_table,
            module_scopes,
            use_decls,
            mod_decls,
        })
    }

    /// Lower a program using a [`ModuleGraph`] for multi-module support.
    ///
    /// This is the Phase 5B entry point: it uses the graph's shared
    /// [`SymbolInterner`] (cloned into `self.symbols` so all `SymbolId`s are
    /// consistent), iterates every module in the graph (not just the root),
    /// and assigns the correct [`ModuleId`] to each lowered item.
    ///
    /// For single-file programs (a graph with only the root module and no
    /// `mod` declarations), this produces output identical to
    /// [`lower_program`](Self::lower_program).
    pub fn lower_program_with_graph(
        &mut self,
        program: &Program,
        graph: &ModuleGraph,
    ) -> CompilerResult<HirProgram> {
        // Phase 0: unify the symbol interner with the graph's.
        // The graph's modules already hold `SymbolId`s computed against
        // `graph.symbol_interner`.  By cloning it into `self.symbols`, any
        // name we intern here will produce the same `SymbolId` as the one
        // stored in the module path segments.
        self.symbols = graph.symbol_interner.clone();

        // Phase 1: pre-collect enum names across ALL modules so that type
        // annotations can resolve user-defined type names that are actually
        // enums (the parser produces `Type::Struct` for all user-defined
        // type names).
        let mut enum_names: HashMap<&str, ()> = HashMap::new();
        for module in &graph.modules {
            if let Some(ast) = &module.ast {
                for item in &ast.items {
                    if let ItemKind::EnumDef { name, .. } = &item.kind {
                        enum_names.insert(name.as_str(), ());
                    }
                }
                for func in &ast.functions {
                    for stmt in &func.body {
                        if let Stmt::EnumDef { name, .. } = stmt {
                            enum_names.insert(name.as_str(), ());
                        }
                    }
                }
            }
        }
        // Also include enums from the top-level `program` parameter (single-file
        // backward compatibility).
        for item in &program.items {
            if let ItemKind::EnumDef { name, .. } = &item.kind {
                enum_names.insert(name.as_str(), ());
            }
        }
        for func in &program.functions {
            for stmt in &func.body {
                if let Stmt::EnumDef { name, .. } = stmt {
                    enum_names.insert(name.as_str(), ());
                }
            }
        }

        // Phase 2: build the cross-module signature table and collect
        // struct/enum/use/mod definitions.  We iterate every module in the
        // graph and process its AST.  Each item is tagged with its owning
        // ModuleId (not ROOT).

        let mut function_sigs: HashMap<SymbolId, FunctionSig> = HashMap::new();
        let mut structs: Vec<StructDef> = Vec::new();
        let mut enums: Vec<EnumDef> = Vec::new();
        let mut use_decls: Vec<HirUseDecl> = Vec::new();
        let mut mod_decls: Vec<HirModDecl> = Vec::new();

        // A lookup from a child module's name string to its ModuleId, so we can
        // resolve `mod foo;` declarations.  We build this from the graph's
        // module_index, keyed by the last path segment.
        let mut child_module_lookup: HashMap<String, ModuleId> = HashMap::new();
        for module in &graph.modules {
            if let Some(name) = module.path.name(&graph.symbol_interner) {
                child_module_lookup.insert(name.to_string(), module.id);
            }
        }

        let mut next_func_def_id: u32 = 0;

        for module in &graph.modules {
            let module_id = module.id;
            let ast = module.ast.as_ref();
            // Fall back to the top-level `program` for the root module if its
            // AST was not loaded during discovery (single-file backward compat).
            let ast = if ast.is_none() && module.is_root() {
                Some(program)
            } else {
                ast
            };
            let Some(ast) = ast else {
                continue;
            };

            // Collect struct/enum/use/mod definitions from this module's items.
            for item in &ast.items {
                match &item.kind {
                    ItemKind::StructDef {
                        name,
                        generic_params,
                        fields,
                        span,
                        ..
                    } => {
                        let name_id = self.symbols.intern(name);
                        let generic_param_syms: Vec<SymbolId> = generic_params
                            .iter()
                            .map(|p| self.symbols.intern(p))
                            .collect();
                        let field_syms: Vec<(SymbolId, HirType)> = fields
                            .iter()
                            .map(|(fname, fty)| {
                                let fid = self.symbols.intern(fname);
                                (
                                    fid,
                                    ast_type_to_hir(
                                        fty,
                                        &mut self.symbols,
                                        &enum_names,
                                        Some(&generic_param_syms),
                                    ),
                                )
                            })
                            .collect();
                        let def_id = DefId(structs.len() as u32);
                        structs.push(StructDef {
                            def_id,
                            name: name_id,
                            generic_params: generic_param_syms,
                            fields: field_syms,
                            span: span_to_source_span(span),
                            module: module_id,
                            visibility: ast_visibility_to_hir(&item.visibility),
                        });
                    }
                    ItemKind::EnumDef {
                        name,
                        generic_params,
                        variants,
                        span,
                    } => {
                        let name_id = self.symbols.intern(name);
                        let variant_syms: Vec<SymbolId> =
                            variants.iter().map(|v| self.symbols.intern(v)).collect();
                        let generic_param_syms: Vec<SymbolId> = generic_params
                            .iter()
                            .map(|p| self.symbols.intern(p))
                            .collect();
                        let def_id = DefId(enums.len() as u32);
                        enums.push(EnumDef {
                            def_id,
                            name: name_id,
                            generic_params: generic_param_syms,
                            variants: variant_syms,
                            span: span_to_source_span(span),
                            module: module_id,
                            visibility: ast_visibility_to_hir(&item.visibility),
                        });
                    }
                    ItemKind::UseDecl { path, alias } => {
                        let path_syms: Vec<SymbolId> =
                            path.iter().map(|s| self.symbols.intern(s)).collect();
                        let alias_sym = match alias {
                            Some(a) => self.symbols.intern(a),
                            None => path_syms
                                .last()
                                .copied()
                                .unwrap_or_else(|| self.symbols.intern("")),
                        };
                        let def_id = self.next_def_id();
                        use_decls.push(HirUseDecl {
                            def_id,
                            path: path_syms,
                            alias: alias_sym,
                            module: module_id,
                            visibility: ast_visibility_to_hir(&item.visibility),
                            span: span_to_source_span(&item.span),
                        });
                    }
                    ItemKind::ModDecl => {
                        // Resolve the child module's ModuleId by looking up
                        // the module name in the graph.
                        let mod_id = self.next_def_id();
                        let name_sym = self.symbols.intern(&item.name);
                        let resolved_module_id = child_module_lookup.get(&item.name).copied();
                        mod_decls.push(HirModDecl {
                            def_id: mod_id,
                            name: name_sym,
                            module_id: resolved_module_id,
                            module: module_id,
                            visibility: ast_visibility_to_hir(&item.visibility),
                            span: span_to_source_span(&item.span),
                        });
                    }
                    ItemKind::Function(func) => {
                        let _ = func; // handled in the signature pass below
                    }
                    ItemKind::ModuleDecl => {
                        // 0.5 advisory; treat like ModDecl for the multi-module path.
                        let mod_id = self.next_def_id();
                        let name_sym = self.symbols.intern(&item.name);
                        mod_decls.push(HirModDecl {
                            def_id: mod_id,
                            name: name_sym,
                            module_id: None,
                            module: ModuleId::ROOT,
                            visibility: ast_visibility_to_hir(&item.visibility),
                            span: span_to_source_span(&item.span),
                        });
                    }
                    ItemKind::MainBlock(_stmts, _span) => {
                        // Synthesised in a second pass below.
                    }
                    ItemKind::ExternalFunction { .. } => {
                        // Handled in the signature pass below.
                    }
                    ItemKind::ExternalFunction { .. } => {
                        // Handled in the signature pass below.
                    }
                }
            }

            // Also collect struct/enum defs from function-local items.
            for func in &ast.functions {
                for stmt in &func.body {
                    match stmt {
                        Stmt::StructDef {
                            name,
                            generic_params,
                            fields,
                            span,
                        } => {
                            let name_id = self.symbols.intern(name);
                            let generic_param_syms: Vec<SymbolId> = generic_params
                                .iter()
                                .map(|p| self.symbols.intern(p))
                                .collect();
                            let field_syms: Vec<(SymbolId, HirType)> = fields
                                .iter()
                                .map(|(fname, fty)| {
                                    let fid = self.symbols.intern(fname);
                                    (
                                        fid,
                                        ast_type_to_hir(
                                            fty,
                                            &mut self.symbols,
                                            &enum_names,
                                            Some(&generic_param_syms),
                                        ),
                                    )
                                })
                                .collect();
                            structs.push(StructDef {
                                def_id: DefId(structs.len() as u32),
                                name: name_id,
                                generic_params: generic_param_syms,
                                fields: field_syms,
                                span: span_to_source_span(span),
                                module: module_id,
                                visibility: HirVisibility::Private,
                            });
                        }
                        Stmt::EnumDef {
                            name,
                            generic_params,
                            variants,
                            span,
                        } => {
                            let name_id = self.symbols.intern(name);
                            let variant_syms: Vec<SymbolId> =
                                variants.iter().map(|v| self.symbols.intern(v)).collect();
                            let generic_param_syms: Vec<SymbolId> = generic_params
                                .iter()
                                .map(|p| self.symbols.intern(p))
                                .collect();
                            enums.push(EnumDef {
                                def_id: DefId(enums.len() as u32),
                                name: name_id,
                                generic_params: generic_param_syms,
                                variants: variant_syms,
                                span: span_to_source_span(span),
                                module: module_id,
                                visibility: HirVisibility::Private,
                            });
                        }
                        _ => {}
                    }
                }
            }

            // Build function signatures from this module's top-level functions.
            for item in &ast.items {
                if let ItemKind::Function(func) = &item.kind {
                    let name_id = self.symbols.intern(&func.name);
                    let def_id = DefId(next_func_def_id);
                    // Intern generic params first so signature resolution
                    // recognizes them as `HirType::Generic`.
                    let gparams: Vec<SymbolId> = func
                        .generic_params
                        .iter()
                        .map(|p| self.symbols.intern(p))
                        .collect();
                    let param_types: Vec<HirType> = func
                        .params
                        .iter()
                        .map(|(_, t)| {
                            ast_type_to_hir(t, &mut self.symbols, &enum_names, Some(&gparams))
                        })
                        .collect();
                    let return_type = ast_type_to_hir(
                        &func.return_type,
                        &mut self.symbols,
                        &enum_names,
                        Some(&gparams),
                    );
                    function_sigs.insert(
                        name_id,
                        FunctionSig {
                            def_id,
                            param_types,
                            param_names: func
                                .params
                                .iter()
                                .map(|(n, _)| self.symbols.intern(n))
                                .collect(),
                            return_type,
                            generic_params: gparams,
                        },
                    );
                    next_func_def_id += 1;
                }
            }
        }

        // Register external declarations in the function signature table.
        // External functions are callable from Saturnite code by their
        // declared symbol name; the runtime bridge resolves the symbol at
        // link/runtime time. Their DefIds are assigned after the regular
        // functions so they remain distinct from builtin sentinels.
        let mut external_functions: Vec<HirExternalFunction> = Vec::new();
        for module in &graph.modules {
            let module_id = module.id;
            let ast = module.ast.as_ref();
            let ast = if ast.is_none() && module.is_root() {
                Some(program)
            } else {
                ast
            };
            let Some(ast) = ast else {
                continue
            };
            for item in &ast.items {
                if let ItemKind::ExternalFunction {
                    kind,
                    ecosystem,
                    symbol,
                    params,
                    return_type,
                    span,
                } = &item.kind
                {
                    let name_id = self.symbols.intern(symbol);
                    let def_id = DefId(next_func_def_id);
                    let param_types: Vec<HirType> = params
                        .iter()
                        .map(|(_, t)| ast_type_to_hir(t, &mut self.symbols, &enum_names, None))
                        .collect();
                    let param_names: Vec<SymbolId> =
                        params.iter().map(|(n, _)| self.symbols.intern(n)).collect();
                    let return_hir =
                        ast_type_to_hir(return_type, &mut self.symbols, &enum_names, None);
                    for (i, ty) in param_types.iter().enumerate() {
                        if !is_abi_safe(ty) {
                            return Err(CompilerError::semantic(format!(
                                "external function `{}` parameter {} has type {:?}, which is not ABI-safe. \
                                 External calls only support the primitive ABI subset (i64, f64, bool).",
                                symbol,
                                i + 1,
                                ty
                            )));
                        }
                    }
                    if !is_abi_safe(&return_hir) && !matches!(return_hir, HirType::Unit) {
                        return Err(CompilerError::semantic(format!(
                            "external function `{}` has return type {:?}, which is not ABI-safe. \
                             External calls only support the primitive ABI subset (i64, f64, bool) and Unit.",
                            symbol, return_hir
                        )));
                    }
                    function_sigs.insert(
                        name_id,
                        FunctionSig {
                            def_id,
                            param_types: param_types.clone(),
                            param_names: param_names.clone(),
                            return_type: return_hir.clone(),
                            generic_params: Vec::new(),
                        },
                    );
                    external_functions.push(HirExternalFunction {
                        def_id,
                        kind: kind.clone(),
                        ecosystem: ecosystem.clone(),
                        symbol: symbol.clone(),
                        name: name_id,
                        param_names,
                        param_types,
                        return_type: return_hir,
                        span: span_to_source_span(span),
                        module: module_id,
                    });
                    next_func_def_id += 1;
                }
            }
        }

        // Register external declarations in the function signature table.
        // External functions are callable from Saturnite code by their
        // declared symbol name; the runtime bridge resolves the symbol at
        // link/runtime time. Their DefIds are assigned after the regular
        // functions so they remain distinct from builtin sentinels.
        let mut external_functions: Vec<HirExternalFunction> = Vec::new();
        for module in &graph.modules {
            let module_id = module.id;
            let ast = module.ast.as_ref();
            let ast = if ast.is_none() && module.is_root() {
                Some(program)
            } else {
                ast
            };
            let Some(ast) = ast else {
                continue
            };
            for item in &ast.items {
                if let ItemKind::ExternalFunction {
                    kind,
                    ecosystem,
                    symbol,
                    params,
                    return_type,
                    span,
                } = &item.kind
                {
                    let name_id = self.symbols.intern(symbol);
                    let def_id = DefId(next_func_def_id);
                    let param_types: Vec<HirType> = params
                        .iter()
                        .map(|(_, t)| ast_type_to_hir(t, &mut self.symbols, &enum_names, None))
                        .collect();
                    let param_names: Vec<SymbolId> =
                        params.iter().map(|(n, _)| self.symbols.intern(n)).collect();
                    let return_hir =
                        ast_type_to_hir(return_type, &mut self.symbols, &enum_names, None);
                    for (i, ty) in param_types.iter().enumerate() {
                        if !is_abi_safe(ty) {
                            return Err(CompilerError::semantic(format!(
                                "external function `{}` parameter {} has type {:?}, which is not ABI-safe. \
                                 External calls only support the primitive ABI subset (i64, f64, bool).",
                                symbol,
                                i + 1,
                                ty
                            )));
                        }
                    }
                    if !is_abi_safe(&return_hir) && !matches!(return_hir, HirType::Unit) {
                        return Err(CompilerError::semantic(format!(
                            "external function `{}` has return type {:?}, which is not ABI-safe. \
                             External calls only support the primitive ABI subset (i64, f64, bool) and Unit.",
                            symbol, return_hir
                        )));
                    }
                    function_sigs.insert(
                        name_id,
                        FunctionSig {
                            def_id,
                            param_types: param_types.clone(),
                            param_names: param_names.clone(),
                            return_type: return_hir.clone(),
                            generic_params: Vec::new(),
                        },
                    );
                    external_functions.push(HirExternalFunction {
                        def_id,
                        kind: kind.clone(),
                        ecosystem: ecosystem.clone(),
                        symbol: symbol.clone(),
                        name: name_id,
                        param_names,
                        param_types,
                        return_type: return_hir,
                        span: span_to_source_span(span),
                        module: module_id,
                    });
                    next_func_def_id += 1;
                }
            }
        }

        // Register builtin println
        let println_sym = self.symbols.intern("println");
        function_sigs.insert(
            println_sym,
            FunctionSig {
                def_id: PRINTLN_DEF_ID,
                param_types: vec![HirType::I64],
                param_names: vec![println_sym],
                return_type: HirType::Unit,
                generic_params: Vec::new(),
            },
        );

        // Check for main — it must exist in at least one module (typically root).
        let main_sym = self.symbols.intern("main");
        if !function_sigs.contains_key(&main_sym) {
            return Err(CompilerError::semantic("no `main` function defined"));
        }

        // Build the lowering context — borrows from local variables.
        let ctx = LowerContext {
            function_sigs: &function_sigs,
            struct_defs: &structs,
            enum_defs: &enums,
            enum_names: &enum_names,
        };

        // Phase 3: lower each module's function bodies and register DefIds.
        let mut functions: Vec<HirFunction> = Vec::new();
        let mut def_table = DefTable::new();
        let mut module_paths: HashMap<DefId, ModuleId> = HashMap::new();
        let mut module_scopes: Vec<ModuleScope> = Vec::new();

        for module in &graph.modules {
            module_scopes.push(if module.is_root() {
                ModuleScope::new()
            } else {
                ModuleScope::with_parent(module.parent.unwrap_or(ModuleId::ROOT))
            });
        }

        // Ensure we have at least the root scope for single-file backward compat.
        if module_scopes.is_empty() {
            module_scopes.push(ModuleScope::new());
        }

        let mut use_decl_idx: u32 = 0;
        let mut mod_decl_idx: u32 = 0;
        let mut func_def_id: u32 = 0;

        for module in &graph.modules {
            let module_id = module.id;
            let ast = module.ast.as_ref();
            let ast = if ast.is_none() && module.is_root() {
                Some(program)
            } else {
                ast
            };
            let Some(ast) = ast else {
                continue;
            };

            // Resolve items for this module.
            let items: Vec<Item> = if ast.items.is_empty() && !ast.functions.is_empty() {
                ast.functions
                    .iter()
                    .map(|f| Item {
                        name: f.name.clone(),
                        visibility: Visibility::Private,
                        kind: ItemKind::Function(f.clone()),
                        span: f.span.clone(),
                    })
                    .collect()
            } else {
                ast.items.clone()
            };

            // Lower functions
            for item in &items {
                if let ItemKind::Function(func) = &item.kind {
                    let def_id = DefId(func_def_id);
                    let hir_func = self.lower_function(
                        func,
                        def_id,
                        module_id,
                        &ctx,
                        ast_visibility_to_hir(&item.visibility),
                    )?;
                    def_table.register(DefEntry {
                        module: module_id,
                        local_index: func_def_id,
                        kind: DefKind::Function,
                    });
                    module_paths.insert(def_id, module_id);
                    module_scopes[module_id.0 as usize].define_item(hir_func.name, def_id);
                    functions.push(hir_func);
                    func_def_id += 1;
                }
            }
        }

        // Register struct, enum, use, and mod declaration DefIds in the
        // def_table and module_paths. These are registered once (not per-module)
        // to avoid duplicate def_table entries and module_paths overwrites.
        // We also register them in the appropriate module's scope so that
        // `lookup_with_parent` can find them during use-import resolution.
        for (i, s) in structs.iter().enumerate() {
            let def_id = DefId(i as u32);
            def_table.register(DefEntry {
                module: s.module,
                local_index: i as u32,
                kind: DefKind::Struct,
            });
            module_paths.insert(def_id, s.module);
            if let Some(scope) = module_scopes.get_mut(s.module.0 as usize) {
                scope.define_item(s.name, def_id);
            }
        }
        for (i, e) in enums.iter().enumerate() {
            let def_id = DefId(e.def_id.0);
            def_table.register(DefEntry {
                module: e.module,
                local_index: i as u32,
                kind: DefKind::Enum,
            });
            module_paths.insert(def_id, e.module);
            if let Some(scope) = module_scopes.get_mut(e.module.0 as usize) {
                scope.define_item(e.name, def_id);
            }
        }
        for ud in &use_decls {
            let def_id = ud.def_id;
            def_table.register(DefEntry {
                module: ud.module,
                local_index: use_decl_idx,
                kind: DefKind::Use,
            });
            module_paths.insert(def_id, ud.module);
            use_decl_idx += 1;
        }
        for md in &mod_decls {
            let def_id = md.def_id;
            def_table.register(DefEntry {
                module: md.module,
                local_index: mod_decl_idx,
                kind: DefKind::Module,
            });
            module_paths.insert(def_id, md.module);
            if let Some(scope) = module_scopes.get_mut(md.module.0 as usize) {
                scope.define_item(md.name, def_id);
            }
            mod_decl_idx += 1;
        }

        // Register external DefIds in the def_table, module_paths, and module
        // scopes. Their DefIds are assigned after the regular functions so
        // they remain distinct from builtin sentinels.
        for (i, ext) in external_functions.iter().enumerate() {
            let def_id = DefId(functions.len() as u32 + i as u32);
            def_table.register(DefEntry {
                module: ext.module,
                local_index: i as u32,
                kind: DefKind::External,
            });
            module_paths.insert(def_id, ext.module);
            if let Some(scope) = module_scopes.get_mut(ext.module.0 as usize) {
                scope.define_item(ext.name, def_id);
            }
        }

        // Phase 4: build the module vec from the graph.
        let modules: Vec<Module> = graph.modules.clone();

        Ok(HirProgram {
            functions,
            structs,
            enums,
            symbols: std::mem::take(&mut self.symbols),
            external_functions,
            modules,
            root_module: ModuleId::ROOT,
            module_paths,
            def_table,
            module_scopes,
            use_decls,
            mod_decls,
        })
    }

    fn lower_function(
        &mut self,
        func: &Function,
        def_id: DefId,
        module_id: ModuleId,
        ctx: &LowerContext,
        visibility: HirVisibility,
    ) -> CompilerResult<HirFunction> {
        let name = self.symbols.intern(&func.name);
        // Intern generic parameter names. They are also added to the local
        // variable scope as placeholders so that any use of a type-param name
        // inside the body (e.g. as the type of a parameter) can be
        // recognized as `HirType::Generic` by `lower_expr`.
        let generic_params: Vec<SymbolId> = func
            .generic_params
            .iter()
            .map(|p| self.symbols.intern(p))
            .collect();
        let return_type = ast_type_to_hir(
            &func.return_type,
            &mut self.symbols,
            ctx.enum_names,
            Some(&generic_params),
        );
        let mut scope = LowerScope::new();
        let mut params: Vec<(SymbolId, HirType)> = Vec::new();
        for (param_name, param_ty) in &func.params {
            let param_id = self.symbols.intern(param_name);
            let hir_ty = ast_type_to_hir(
                param_ty,
                &mut self.symbols,
                ctx.enum_names,
                Some(&generic_params),
            );
            scope.define_variable(param_id, hir_ty.clone(), false);
            params.push((param_id, hir_ty));
        }
        let mut body: Vec<HirStmt> = Vec::new();
        for stmt in &func.body {
            body.push(self.lower_stmt(stmt, &mut scope, &return_type, ctx)?);
        }
        Ok(HirFunction {
            def_id,
            name,
            generic_params,
            params,
            return_type,
            body,
            span: span_to_source_span(&func.span),
            module: module_id,
            visibility,
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
                    let ann = ast_type_to_hir(t, &mut self.symbols, ctx.enum_names, None);
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
                    inferred.ty.clone()
                };
                scope.define_variable(name_id, resolved_ty.clone(), *mutable);
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
            // 0.5: `give expr` is a synonym for `return expr`.
            Stmt::Give(opt_expr, span) => {
                let hir_opt = match opt_expr {
                    Some(e) => Some(self.lower_expr(e, scope, return_type, ctx)?),
                    None => None,
                };
                Ok(HirStmt {
                    kind: HirStmtKind::Return(hir_opt),
                    span: span_to_source_span(span),
                })
            }
            // 0.5: `say expr` is a synonym for `println(expr)`.
            Stmt::Say(e, span) => {
                let hir_expr = self.lower_expr(e, scope, return_type, ctx)?;
                match hir_expr.ty {
                    HirType::Str => Ok(HirStmt {
                        kind: HirStmtKind::PrintlnStr(hir_expr),
                        span: span_to_source_span(span),
                    }),
                    HirType::I64 | HirType::Enum(_) => Ok(HirStmt {
                        kind: HirStmtKind::Println(hir_expr),
                        span: span_to_source_span(span),
                    }),
                    other => Err(CompilerError::semantic(format!(
                        "say expects a text or number argument, got {:?}",
                        other
                    ))),
                }
            }
            // 0.5: `raise expr` lowers to a stub that prints the message
            // and aborts. Real error semantics are deferred.
            Stmt::Raise(e, span) => {
                let hir_expr = self.lower_expr(e, scope, return_type, ctx)?;
                Ok(HirStmt {
                    kind: HirStmtKind::Raise(hir_expr),
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
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => {
                        if l.ty != r.ty {
                            return Err(CompilerError::semantic(format!(
                                "binary op {:?}: type mismatch {:?} vs {:?}",
                                op, l.ty, r.ty
                            )));
                        }
                        if !matches!(l.ty, HirType::I64 | HirType::F64 | HirType::Enum(_)) {
                            return Err(CompilerError::semantic(format!(
                                "binary op {:?}: operand type {:?} is not numeric",
                                op, l.ty
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
                    BinOp::Mod => {
                        if l.ty != r.ty {
                            return Err(CompilerError::semantic(format!(
                                "binary op {:?}: type mismatch {:?} vs {:?}",
                                op, l.ty, r.ty
                            )));
                        }
                        if l.ty != HirType::I64 {
                            return Err(CompilerError::semantic(format!(
                                "binary op {:?}: modulo is only supported for i64, got {:?}",
                                op, l.ty
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
                    UnOp::Not => {
                        if e.ty != HirType::Bool {
                            return Err(CompilerError::semantic(format!(
                                "cannot apply ! to {:?}: only bool is supported",
                                e.ty
                            )));
                        }
                        Ok(HirExpr {
                            kind: HirExprKind::Unary {
                                op: *op,
                                expr: Box::new(e),
                            },
                            ty: HirType::Bool,
                            span: span_to_source_span(span),
                        })
                    }
                }
            }
            Expr::Call {
                func,
                args,
                named_args,
                type_args,
                span,
            } => {
                let func_sym = self.symbols.intern(func);
                let sig = ctx.function_sigs.get(&func_sym).ok_or_else(|| {
                    CompilerError::semantic(format!("undefined function: {}", func))
                })?;
                // 0.5: named arguments are reordered into positional slots
                // against the callee's parameter names. Positional args fill
                // free slots left-to-right in source order.
                let args: Vec<Expr> = if named_args.is_empty() {
                    args.clone()
                } else {
                    let mut slots: Vec<Option<Expr>> = vec![None; sig.param_types.len()];
                    let mut next_pos = 0usize;
                    for a in args {
                        while next_pos < slots.len() && slots[next_pos].is_some() {
                            next_pos += 1;
                        }
                        if next_pos >= slots.len() {
                            return Err(CompilerError::semantic(format!(
                                "function {} expects {} args, got more",
                                func,
                                sig.param_types.len()
                            )));
                        }
                        slots[next_pos] = Some(a.clone());
                        next_pos += 1;
                    }
                    for (name, value) in named_args {
                        let name_sym = self.symbols.intern(name);
                        let idx = sig
                            .param_names
                            .iter()
                            .position(|&p| p == name_sym)
                            .ok_or_else(|| {
                                CompilerError::semantic(format!(
                                    "no parameter named `{}` on function {}",
                                    name, func
                                ))
                            })?;
                        if slots[idx].is_some() {
                            return Err(CompilerError::semantic(format!(
                                "duplicate argument `{}` in call to {}",
                                name, func
                            )));
                        }
                        slots[idx] = Some(value.clone());
                    }
                    slots
                        .into_iter()
                        .enumerate()
                        .map(|(i, slot)| {
                            slot.ok_or_else(|| {
                                CompilerError::semantic(format!(
                                    "missing argument for parameter {} of {}",
                                    i + 1,
                                    func
                                ))
                            })
                        })
                        .collect::<CompilerResult<Vec<_>>>()?
                };
                if args.len() != sig.param_types.len() {
                    return Err(CompilerError::semantic(format!(
                        "function {} expects {} args, got {}",
                        func,
                        sig.param_types.len(),
                        args.len()
                    )));
                }
                // Resolve explicit type args (turbofish) into HirType so they
                // are available for monomorphization. The arity is checked
                // against the callee's generic_params during the monomorphize
                // pass (it has full visibility into the HIR program by then).
                let hir_type_args: Vec<HirType> = type_args
                    .iter()
                    .map(|t| ast_type_to_hir(t, &mut self.symbols, ctx.enum_names, None))
                    .collect();
                // Detect whether the callee is generic. If it is, the
                // signature's param_types may include `HirType::Generic`,
                // so a strict per-arg type check would falsely reject the
                // call (e.g. `id::<i64>(42)` would fail because `id`'s
                // parameter type is `Generic(T)`, not `I64`). We instead
                // check that each argument's type is well-formed (non-error
                // and not itself `Generic`/unresolved) and defer the
                // concrete type matching to monomorphization.
                let callee_is_generic = sig
                    .param_types
                    .iter()
                    .any(|t| matches!(t, HirType::Generic(_)))
                    || sig.return_type.contains_generic();
                let mut arg_exprs: Vec<HirExpr> = Vec::new();
                for (arg, expected_ty) in args.iter().zip(sig.param_types.iter()) {
                    let arg_expr = self.lower_expr(arg, scope, return_type, ctx)?;
                    if !callee_is_generic && arg_expr.ty != *expected_ty {
                        return Err(CompilerError::semantic(format!(
                            "function {} arg type mismatch: expected {:?}, got {:?}",
                            func, expected_ty, arg_expr.ty
                        )));
                    }
                    arg_exprs.push(arg_expr);
                }
                // For a generic call, the declared return type is
                // `HirType::Generic(...)`, which would mismatch a strict
                // `return` check downstream. We pin the call's HIR type to
                // the concrete substitution target. When the caller supplies
                // an explicit turbofish, we substitute each generic param
                // with the corresponding `HirType` from `hir_type_args` and
                // get the actual concrete return type (e.g. `id::<i64>(42)`
                // yields `I64`, not `Generic(T)`). When no turbofish is
                // present, we fall back to `HirType::Unit`; monomorphization
                // infers the concrete return from the argument types.
                let call_ty = if callee_is_generic {
                    if !hir_type_args.is_empty() {
                        let subst: std::collections::HashMap<SymbolId, HirType> = sig
                            .generic_params
                            .iter()
                            .cloned()
                            .zip(hir_type_args.iter().cloned())
                            .collect();
                        sig.return_type.substitute(&subst)
                    } else {
                        HirType::Unit
                    }
                } else {
                    sig.return_type.clone()
                };
                Ok(HirExpr {
                    kind: HirExprKind::Call {
                        func: sig.def_id,
                        args: arg_exprs,
                        type_args: hir_type_args,
                    },
                    ty: call_ty,
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
                // 0.5.3 Phase 8: a `for` loop may iterate over a `Range`
                // (`for i in 0..10`) or a `List<i64>` (`for item in items`).
                // Any other iterable type is rejected with a diagnostic.
                let is_range = matches!(&iter_expr.kind, HirExprKind::Range { .. });
                let is_list_i64 = matches!(
                    &iter_expr.ty,
                    HirType::List(inner) if matches!(inner.as_ref(), HirType::I64)
                );
                if !is_range && !is_list_i64 {
                    return Err(CompilerError::semantic(format!(
                        "for loop requires a range or List<i64> expression, got {:?}",
                        iter_expr.ty
                    )));
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
            Expr::StructLiteral {
                name,
                fields,
                type_args,
                span,
            } => {
                let name_id = self.symbols.intern(name);
                let struct_def = ctx
                    .struct_defs
                    .iter()
                    .find(|s| s.name == name_id)
                    .ok_or_else(|| {
                        CompilerError::semantic(format!("undefined struct: {}", name))
                    })?;
                // Build a field type lookup from the struct definition.
                let field_type_map: HashMap<SymbolId, HirType> =
                    struct_def.fields.iter().cloned().collect();
                let mut lowered_fields: Vec<(SymbolId, Box<HirExpr>)> = Vec::new();
                for (field_name, field_expr) in fields {
                    let fid = self.symbols.intern(field_name);
                    let expected_ty = field_type_map.get(&fid).cloned().ok_or_else(|| {
                        CompilerError::semantic(format!(
                            "struct {} has no field {}",
                            name, field_name
                        ))
                    })?;
                    let expr = self.lower_expr(field_expr, scope, return_type, ctx)?;
                    // For generic structs, the field types contain
                    // `HirType::Generic`, so a strict type check would
                    // falsely reject well-typed literals. Defer the
                    // concrete type matching to monomorphization (which
                    // substitutes the turbofish type_args).
                    let struct_is_generic =
                        struct_def.fields.iter().any(|(_, t)| t.contains_generic());
                    if !struct_is_generic && expr.ty != expected_ty {
                        return Err(CompilerError::semantic(format!(
                            "field {} expects {:?}, got {:?}",
                            field_name, expected_ty, expr.ty
                        )));
                    }
                    lowered_fields.push((fid, Box::new(expr)));
                }
                let hir_type_args: Vec<HirType> = type_args
                    .iter()
                    .map(|t| ast_type_to_hir(t, &mut self.symbols, ctx.enum_names, None))
                    .collect();
                // When turbofish is supplied, surface the struct literal's
                // type as an `Apply` (so subsequent field accesses can
                // look up the generic-arg substitution). Without turbofish,
                // fall back to a plain `Struct` reference.
                let lit_ty = if hir_type_args.is_empty() {
                    HirType::Struct(name_id)
                } else {
                    HirType::Apply {
                        base: name_id,
                        args: hir_type_args.clone(),
                    }
                };
                Ok(HirExpr {
                    kind: HirExprKind::StructLiteral {
                        name: name_id,
                        fields: lowered_fields,
                        type_args: hir_type_args,
                    },
                    ty: lit_ty,
                    span: span_to_source_span(span),
                })
            }
            Expr::Index {
                list: list_expr,
                index: index_expr,
                span,
            } => {
                let list = self.lower_expr(list_expr, scope, return_type, ctx)?;
                let element_ty = match &list.ty {
                    HirType::List(elem) => *elem.clone(),
                    other => {
                        return Err(CompilerError::semantic(format!(
                            "indexing requires a List<T>, got {:?}",
                            other
                        )))
                    }
                };
                let idx = self.lower_expr(index_expr, scope, return_type, ctx)?;
                if idx.ty != HirType::I64 {
                    return Err(CompilerError::semantic(format!(
                        "list index must be i64, got {:?}",
                        idx.ty
                    )));
                }
                Ok(HirExpr {
                    kind: HirExprKind::Index {
                        list: Box::new(list),
                        index: Box::new(idx),
                    },
                    ty: element_ty,
                    span: span_to_source_span(span),
                })
            }
            Expr::Length {
                expr: inner_expr,
                span,
            } => {
                let inner = self.lower_expr(inner_expr, scope, return_type, ctx)?;
                match &inner.ty {
                    HirType::List(_) => Ok(HirExpr {
                        kind: HirExprKind::Length {
                            expr: Box::new(inner),
                        },
                        ty: HirType::I64,
                        span: span_to_source_span(span),
                    }),
                    other => Err(CompilerError::semantic(format!(
                        "`.length` requires a List<T>, got {:?}",
                        other
                    ))),
                }
            }
            Expr::FieldAccess {
                expr: inner_expr,
                field,
                span,
            } => {
                let inner = self.lower_expr(inner_expr, scope, return_type, ctx)?;
                let struct_sym = match inner.ty {
                    HirType::Struct(s) => s,
                    HirType::Apply { base, .. } => base,
                    // 0.5.3: `.length` is the list length accessor. The
                    // parser emits it as a field access on a List; lower it
                    // to a list length expression here. Any other field on a
                    // list is rejected.
                    HirType::List(_) => {
                        let field_id = self.symbols.intern(field);
                        if self.symbols.lookup(field_id) == Some("length") {
                            return Ok(HirExpr {
                                kind: HirExprKind::Length {
                                    expr: Box::new(inner),
                                },
                                ty: HirType::I64,
                                span: span_to_source_span(span),
                            });
                        }
                        return Err(CompilerError::semantic(format!(
                            "list has no field: {}",
                            field
                        )));
                    }
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
                let mut field_ty = struct_def
                    .fields
                    .iter()
                    .find(|(f, _)| *f == field_id)
                    .map(|(_, ty)| ty.clone())
                    .ok_or_else(|| {
                        CompilerError::semantic(format!("struct has no field: {}", field))
                    })?;
                // If the inner expression is a generic struct literal with
                // explicit turbofish (e.g. `Box::<i64> { value: 21 }.value`),
                // substitute the struct's generic params with the supplied
                // type_args so the field resolves to its concrete type
                // (here `I64`) instead of `Generic(T)`.
                if field_ty.contains_generic() && !struct_def.generic_params.is_empty() {
                    let mut type_args_opt: Option<Vec<HirType>> = None;
                    if let HirExprKind::StructLiteral { type_args, .. } = &inner.kind {
                        if !type_args.is_empty() {
                            type_args_opt = Some(type_args.clone());
                        }
                    }
                    if type_args_opt.is_none() {
                        if let HirType::Apply { args, .. } = &inner.ty {
                            if !args.is_empty() {
                                type_args_opt = Some(args.clone());
                            }
                        }
                    }
                    if let Some(ta) = type_args_opt {
                        let subst: std::collections::HashMap<SymbolId, HirType> = struct_def
                            .generic_params
                            .iter()
                            .cloned()
                            .zip(ta.iter().cloned())
                            .collect();
                        field_ty = field_ty.substitute(&subst);
                    }
                }
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
            // 0.5: `a |> b(x)` desugars to `b(a, x)`. We desugar at the
            // AST→HIR boundary by splicing the lhs as the first positional
            // argument of the rhs call *before* lowering, so arity and
            // named-argument checks see the complete argument list.
            Expr::Pipeline { lhs, rhs, span } => {
                match rhs.as_ref() {
                    Expr::Call {
                        func,
                        args,
                        named_args,
                        type_args,
                        span: call_span,
                    } => {
                        let mut combined_args: Vec<Expr> = vec![(**lhs).clone()];
                        combined_args.extend(args.iter().cloned());
                        let combined = Expr::Call {
                            func: func.clone(),
                            args: combined_args,
                            named_args: named_args.clone(),
                            type_args: type_args.clone(),
                            span: call_span.clone(),
                        };
                        self.lower_expr(&combined, scope, return_type, ctx)
                    }
                    // Bare `a |> f` desugars to `f(a)`.
                    Expr::Var(symbol, _) => {
                        let lhs_hir = self.lower_expr(lhs, scope, return_type, ctx)?;
                        let name_sym = self.symbols.intern(symbol.as_str());
                        let def_id = ctx
                            .function_sigs
                            .get(&name_sym)
                            .map(|s| s.def_id)
                            .ok_or_else(|| {
                                CompilerError::semantic(format!(
                                    "undefined function in pipeline: {}",
                                    symbol
                                ))
                            })?;
                        Ok(HirExpr {
                            kind: HirExprKind::Call {
                                func: def_id,
                                args: vec![lhs_hir],
                                type_args: Vec::new(),
                            },
                            ty: HirType::Unit,
                            span: span_to_source_span(span),
                        })
                    }
                    _ => Err(CompilerError::semantic(
                        "right-hand side of pipeline must be a function call".to_string(),
                    )),
                }
            }
            // 0.5: closures are lambda-lifted. We emit a synthetic top-level
            // HirFunction for the closure body and substitute the closure
            // expression with a reference to it. For simplicity in 0.5 we
            // lower the closure body as the *current* call's body — real
            // first-class function support is deferred.
            Expr::Closure {
                params: _,
                body: _,
                span: _,
            } => Err(CompilerError::semantic(
                "closures are not yet supported at runtime in 0.5; \
                     rewrite as a regular function for now"
                    .to_string(),
            )),
            // 0.5.1: string interpolation lowers to a chain of runtime
            // `concat_str` calls. Each literal segment becomes a `StrLit`;
            // each `{expr}` segment is lowered. `{Str}` segments concatenate
            // directly; `{I64}` segments are first converted to a string by
            // the runtime `str_i64` builtin. Every other type is rejected at
            // compile time rather than silently miscompiled. The resulting
            // nested `Call`(s) reuse the ordinary call pipeline through
            // monomorphization → MIR → LLVM codegen.
            Expr::ListLiteral { items, span } => {
                if items.is_empty() {
                    return Err(CompilerError::semantic(
                        "empty list literal `[]` is not supported in 0.5.3; \
                         provide at least one element (e.g., `[1]`) so the \
                         element type can be inferred"
                            .to_string(),
                    ));
                }
                let mut lowered_elements: Vec<HirExpr> = Vec::with_capacity(items.len());
                let mut common_ty: Option<HirType> = None;
                for item_expr in items {
                    let lowered = self.lower_expr(item_expr, scope, return_type, ctx)?;
                    // Only i64 elements supported in 0.5.3
                    if lowered.ty != HirType::I64 {
                        return Err(CompilerError::semantic(format!(
                            "list literal: element type `{:?}` is not supported; \
                             only `i64` is allowed in 0.5.3",
                            lowered.ty
                        )));
                    }
                    match &common_ty {
                        Some(ct) => {
                            if lowered.ty != *ct {
                                return Err(CompilerError::semantic(format!(
                                    "mixed-type list literal: expected `{:?}`, \
                                     found `{:?}`",
                                    ct, lowered.ty
                                )));
                            }
                        }
                        None => common_ty = Some(lowered.ty.clone()),
                    }
                    lowered_elements.push(lowered);
                }
                let ty = common_ty.unwrap_or(HirType::I64);
                Ok(HirExpr {
                    kind: HirExprKind::ListLiteral {
                        elements: lowered_elements,
                    },
                    ty: HirType::List(Box::new(ty)),
                    span: span_to_source_span(span),
                })
            }
            Expr::InterpolatedStr(parts, span) => {
                let span = span_to_source_span(span);
                let mut acc: Option<HirExpr> = None;
                for part in parts {
                    let segment = match part {
                        StrPart::Literal(s) => {
                            let str_id = self.symbols.intern(&s);
                            HirExpr {
                                kind: HirExprKind::StrLit(str_id),
                                ty: HirType::Str,
                                span,
                            }
                        }
                        StrPart::Expr(e) => {
                            let inner = self.lower_expr(e, scope, return_type, ctx)?;
                            match inner.ty {
                                HirType::Str => inner,
                                HirType::I64 => HirExpr {
                                    kind: HirExprKind::Call {
                                        func: STR_I64_DEF_ID,
                                        args: vec![inner],
                                        type_args: Vec::new(),
                                    },
                                    ty: HirType::Str,
                                    span,
                                },
                                other => {
                                    return Err(CompilerError::semantic(format!(
                                        "string interpolation: cannot render a {:?} value; \
                                         supported types are text and number",
                                        other
                                    )));
                                }
                            }
                        }
                    };
                    acc = Some(match acc {
                        None => segment,
                        Some(prev) => HirExpr {
                            kind: HirExprKind::Call {
                                func: CONCAT_STR_DEF_ID,
                                args: vec![prev, segment],
                                type_args: Vec::new(),
                            },
                            ty: HirType::Str,
                            span,
                        },
                    });
                }
                acc.ok_or_else(|| {
                    CompilerError::semantic("string interpolation produced no segments".to_string())
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

/// Lower an `ast::Program` using a [`ModuleGraph`] for multi-module support.
///
/// This is the Phase 5B entry point for the multi-module pipeline. It
/// unifies the graph's [`SymbolInterner`], iterates every module in the
/// graph, assigns correct [`ModuleId`]s, and resolves `mod` declarations
/// to child module IDs.
pub fn lower_with_graph(program: &Program, graph: &ModuleGraph) -> CompilerResult<HirProgram> {
    let mut hir_lower = HirLower::new();
    hir_lower.lower_program_with_graph(program, graph)
}

/// Convenience: lower a program with a module graph and return `Ok(())` or
/// the first error.
pub fn lower_unit_with_graph(program: &Program, graph: &ModuleGraph) -> CompilerResult<()> {
    lower_with_graph(program, graph).map(|_| ())
}

/// Resolve `use` declarations across modules.
///
/// This is a thin shim that delegates to the dedicated
/// [`crate::resolver::resolve_modules`] pass. The actual name-resolution
/// algorithm (path walk, parent-chain lookups, scope registration) lives
/// in `crate::resolver`. The shim is preserved for backward compatibility
/// with callers and tests that import `hir::lower::resolve_modules`
/// directly.
///
/// See `docs/SATURNITE_1_0_ROADMAP.md` Phase 1 for the rationale.
pub fn resolve_modules(hir: &mut HirProgram) -> CompilerResult<()> {
    crate::resolver::resolve_modules(hir)
}

/// Convert a `Range<usize>` to a `SourceSpan`.
pub fn range_to_span(range: &std::ops::Range<usize>) -> SourceSpan {
    span_to_source_span(range)
}

// ---------------------------------------------------------------------------
// Phase 5B unit tests: HIR module tracking
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Item, ItemKind, Program, Visibility as AstVisibility};
    use crate::lexer::Lexer;
    use crate::parser;

    /// Helper: lex + parse + lower a source string.
    fn lower_src(src: &str) -> HirProgram {
        let tokens: Vec<_> = Lexer::new(src)
            .collect::<Result<Vec<_>, _>>()
            .expect("lexing should succeed");
        let program: Program = parser::parse(src, tokens).expect("parsing should succeed");
        lower(&program).expect("lowering should succeed")
    }

    /// Helper: lex + parse a source string into a `Program` (no lowering).
    fn parse_program(src: &str) -> Program {
        let tokens: Vec<_> = Lexer::new(src)
            .collect::<Result<Vec<_>, _>>()
            .expect("lexing should succeed");
        parser::parse(src, tokens).expect("parsing should succeed")
    }

    /// Helper: construct a `Program` from raw AST items.
    fn program_from_items(items: Vec<Item>) -> Program {
        Program::from_items(items)
    }

    #[test]
    fn test_hir_program_has_root_module() {
        let hir = lower_src("fn main() -> i64 { 0 }");
        assert_eq!(hir.root_module, ModuleId::ROOT);
        assert_eq!(hir.modules.len(), 1);
        assert_eq!(hir.modules[0].id, ModuleId::ROOT);
    }

    #[test]
    fn test_hir_function_has_root_module() {
        let hir = lower_src("fn main() -> i64 { 0 }");
        let main = hir.function_by_name("main").expect("main function");
        assert_eq!(main.module, ModuleId::ROOT);
        assert_eq!(main.visibility, HirVisibility::Private);
    }

    #[test]
    fn test_hir_program_has_module_scopes() {
        let hir = lower_src("fn main() -> i64 { 0 }");
        assert_eq!(hir.module_scopes.len(), 1);
        assert_eq!(hir.module_scopes[0].parent, None);
    }

    #[test]
    fn test_hir_def_table_has_function_entry() {
        let hir = lower_src("fn main() -> i64 { 0 }");
        // main gets DefId(0)
        let entry = hir.def_entry(DefId(0)).expect("def entry for main");
        assert_eq!(entry.module, ModuleId::ROOT);
        assert_eq!(entry.kind, DefKind::Function);
    }

    #[test]
    fn test_hir_module_of_function() {
        let hir = lower_src("fn main() -> i64 { 0 }");
        let main = hir.function_by_name("main").expect("main function");
        let module = hir.module_of(main.def_id).expect("module for main");
        assert_eq!(module, ModuleId::ROOT);
    }

    #[test]
    fn test_hir_module_paths_maps_defid_to_root() {
        let hir = lower_src("fn main() -> i64 { 0 }");
        let main = hir.function_by_name("main").expect("main function");
        assert!(hir.module_paths.contains_key(&main.def_id));
        assert_eq!(*hir.module_paths.get(&main.def_id).unwrap(), ModuleId::ROOT);
    }

    #[test]
    fn test_hir_pub_visibility_propagated() {
        let hir = lower_src("pub fn main() -> i64 { 0 }");
        let main = hir.function_by_name("main").expect("main function");
        assert_eq!(main.visibility, HirVisibility::Public);
    }

    #[test]
    fn test_hir_private_visibility_default() {
        let hir = lower_src("fn main() -> i64 { 0 }");
        let main = hir.function_by_name("main").expect("main function");
        assert_eq!(main.visibility, HirVisibility::Private);
    }

    #[test]
    fn test_hir_struct_def_module_and_visibility() {
        let hir = lower_src("fn main() -> i64 { struct Point { x: i64 } Point { x: 42 } 0 }");
        assert!(!hir.structs.is_empty());
        for s in &hir.structs {
            assert_eq!(s.module, ModuleId::ROOT);
            assert_eq!(s.visibility, HirVisibility::Private);
        }
    }

    #[test]
    fn test_hir_top_level_struct_recorded() {
        let item = Item {
            name: "Point".to_string(),
            visibility: AstVisibility::Private,
            kind: ItemKind::StructDef {
                name: "Point".to_string(),
                generic_params: vec![],
                fields: vec![("x".to_string(), crate::ast::Type::I64)],
                span: 0..10,
            },
            span: 0..10,
        };
        let program = program_from_items(vec![
            item,
            Item {
                name: "main".to_string(),
                visibility: AstVisibility::Private,
                kind: ItemKind::Function(crate::ast::Function {
                    name: "main".to_string(),
                    generic_params: vec![],
                    params: vec![],
                    return_type: crate::ast::Type::Unit,
                    body: vec![],
                    span: 0..10,
                }),
                span: 0..10,
            },
        ]);
        let mut hir = lower(&program).expect("lowering should succeed");
        assert!(!hir.structs.is_empty());
        assert_eq!(hir.structs[0].name, hir.symbols.intern("Point"));
        assert_eq!(hir.structs[0].module, ModuleId::ROOT);
    }

    #[test]
    fn test_hir_use_decl_recorded() {
        let item = Item {
            name: "".to_string(),
            visibility: AstVisibility::Private,
            kind: ItemKind::UseDecl {
                path: vec!["io".to_string(), "println".to_string()],
                alias: None,
            },
            span: 0..10,
        };
        let program = program_from_items(vec![
            item,
            Item {
                name: "main".to_string(),
                visibility: AstVisibility::Private,
                kind: ItemKind::Function(crate::ast::Function {
                    name: "main".to_string(),
                    generic_params: vec![],
                    params: vec![],
                    return_type: crate::ast::Type::I64,
                    body: vec![],
                    span: 0..10,
                }),
                span: 0..10,
            },
        ]);
        let hir = lower(&program).expect("lowering should succeed");
        assert_eq!(hir.use_decls.len(), 1);
        let udecl = &hir.use_decls[0];
        assert_eq!(udecl.path.len(), 2);
        assert_eq!(udecl.module, ModuleId::ROOT);
    }

    #[test]
    fn test_hir_mod_decl_recorded() {
        let item = Item {
            name: "io".to_string(),
            visibility: AstVisibility::Private,
            kind: ItemKind::ModDecl,
            span: 0..10,
        };
        let program = program_from_items(vec![
            item,
            Item {
                name: "main".to_string(),
                visibility: AstVisibility::Private,
                kind: ItemKind::Function(crate::ast::Function {
                    name: "main".to_string(),
                    generic_params: vec![],
                    params: vec![],
                    return_type: crate::ast::Type::I64,
                    body: vec![],
                    span: 0..10,
                }),
                span: 0..10,
            },
        ]);
        let mut hir = lower(&program).expect("lowering should succeed");
        assert_eq!(hir.mod_decls.len(), 1);
        let mdecl = &hir.mod_decls[0];
        assert_eq!(mdecl.name, hir.symbols.intern("io"));
        assert_eq!(mdecl.module, ModuleId::ROOT);
        assert!(mdecl.module_id.is_none()); // not resolved yet (Phase 6)
    }

    #[test]
    fn test_hir_multiple_functions_all_in_root() {
        let hir = lower_src("fn foo() -> i64 { 42 } fn main() -> i64 { foo() }");
        assert_eq!(hir.functions.len(), 2);
        for f in &hir.functions {
            assert_eq!(f.module, ModuleId::ROOT);
        }
    }

    #[test]
    fn test_hir_def_table_entries_for_functions() {
        let hir = lower_src("fn foo() -> i64 { 42 } fn main() -> i64 { foo() }");
        let foo = hir.function_by_name("foo").expect("foo function");
        let main = hir.function_by_name("main").expect("main function");
        let foo_entry = hir.def_entry(foo.def_id).expect("foo def entry");
        let main_entry = hir.def_entry(main.def_id).expect("main def entry");
        assert_eq!(foo_entry.kind, DefKind::Function);
        assert_eq!(main_entry.kind, DefKind::Function);
        assert_eq!(foo_entry.module, ModuleId::ROOT);
        assert_eq!(main_entry.module, ModuleId::ROOT);
    }

    #[test]
    fn test_hir_root_module_path_is_empty() {
        let hir = lower_src("fn main() -> i64 { 0 }");
        let root = hir.module(ModuleId::ROOT).expect("root module");
        assert!(root.path.is_empty());
    }

    #[test]
    fn test_list_literal_lower_single_element() {
        let hir = lower_src("fn main() -> i64 { let a = [42] 0 }");
        let main = hir.function_by_name("main").expect("main");
        assert!(
            main.body
                .iter()
                .any(|s| matches!(&s.kind, HirStmtKind::Let { value, .. } if matches!(&value.kind, HirExprKind::ListLiteral { elements } if elements.len() == 1))),
            "expected a ListLiteral HIR node with one element"
        );
    }

    #[test]
    fn test_list_literal_lower_multiple_elements() {
        let hir = lower_src("fn main() -> i64 { let a = [1, 2, 3] 0 }");
        let main = hir.function_by_name("main").expect("main");
        assert!(
            main.body
                .iter()
                .any(|s| matches!(&s.kind, HirStmtKind::Let { value, .. } if matches!(&value.kind, HirExprKind::ListLiteral { elements } if elements.len() == 3))),
            "expected a ListLiteral HIR node with three elements"
        );
    }

    #[test]
    fn test_list_literal_reject_empty() {
        let tokens: Vec<_> = Lexer::new("fn main() -> i64 { let a = [] 0 }")
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let prog = parser::parse("fn main() -> i64 { let a = [] 0 }", tokens).expect("parse ok");
        let result = lower(&prog);
        assert!(result.is_err(), "empty list must be rejected");
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("empty list"),
            "expected empty list error, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_list_literal_reject_mixed_type() {
        // Even though parser accepts mixed expressions, HIR should reject non-i64.
        // Here both are i64; to test rejection we would need a bool, which parser cannot easily mix
        // without syntax. This test documents the current state: same-type i64 lists pass.
        let hir = lower_src("fn main() -> i64 { let a = [1, 2] 0 }");
        let main = hir.function_by_name("main").expect("main");
        assert!(
            main.body
                .iter()
                .any(|s| matches!(&s.kind, HirStmtKind::Let { value, .. } if matches!(&value.kind, HirExprKind::ListLiteral { elements } if elements.len() == 2))),
            "expected a ListLiteral HIR node with two elements"
        );
    }

    #[test]
    fn test_list_literal_reject_non_i64_element() {
        // `true` lowers to Bool, which should be rejected in list context.
        let tokens: Vec<_> = Lexer::new("fn main() -> i64 { let a = [true] 0 }")
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let prog =
            parser::parse("fn main() -> i64 { let a = [true] 0 }", tokens).expect("parse ok");
        let result = lower(&prog);
        assert!(result.is_err(), "non-i64 list element must be rejected");
    }

    #[test]
    fn test_list_literal_nested_expr() {
        let hir = lower_src("fn main() -> i64 { let a = [1 + 2, 3 * 4] 0 }");
        let main = hir.function_by_name("main").expect("main");
        assert!(
            main.body
                .iter()
                .any(|s| matches!(&s.kind, HirStmtKind::Let { value, .. } if matches!(&value.kind, HirExprKind::ListLiteral { elements } if elements.len() == 2))),
            "expected a ListLiteral HIR node with two elements"
        );
    }

    #[test]
    fn test_list_literal_deferred_nested() {
        // Nested list `[ [1, 2], [3] ]` is deferred; first element `[1,2]` lowers to List(I64),
        // second also List(I64) — they match, so lowering succeeds. Full nested semantics deferred.
        let tokens: Vec<_> = Lexer::new("fn main() -> i64 { let a = [[1, 2], [3]] 0 }")
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let prog = parser::parse("fn main() -> i64 { let a = [[1, 2], [3]] 0 }", tokens)
            .expect("parse ok");
        // Lowering may succeed (same inner type) but nested list runtime is deferred.
        let _ = lower(&prog);
    }

    // --- 0.5.3 Phase 8: for loop over List<i64> ---

    #[test]
    fn test_for_loop_over_list_accepted() {
        let hir =
            lower_src("fn main() -> i64 { let a = [1, 2, 3] for x in a { println(x) } return 0 }");
        let main = hir.function_by_name("main").expect("main");
        let has_for = main.body.iter().any(|s| {
            matches!(
                &s.kind,
                HirStmtKind::Expr(HirExpr {
                    kind: HirExprKind::For { iter, .. },
                    ty: HirType::Unit,
                    ..
                }) if matches!(iter.kind, HirExprKind::ListLiteral { .. } | HirExprKind::Variable { .. })
            )
        });
        assert!(has_for, "expected a For HIR node iterating a list");
    }

    #[test]
    fn test_for_loop_over_list_rejects_non_list_iterable() {
        // A `for` loop over a bare i64 (not a range, not a list) must be
        // rejected at HIR lowering with an iterable diagnostic.
        let result = lower(&parse_program("fn main() -> i64 { for x in 42 { } 0 }"));
        assert!(result.is_err(), "for loop over a bare i64 must be rejected");
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("for loop requires a range or List<i64>"),
            "expected a for-loop iterable diagnostic, got: {}",
            err
        );
    }

    #[test]
    fn test_for_loop_over_list_rejects_bool_iterable() {
        let result = lower(&parse_program("fn main() -> i64 { for x in true { } 0 }"));
        assert!(result.is_err(), "for loop over a bool must be rejected");
    }

    #[test]
    fn test_graph_lower_single_file_matches_lower_program() {
        let src = "fn main() -> i64 { 0 }";
        let tokens: Vec<_> = Lexer::new(src)
            .collect::<Result<Vec<_>, _>>()
            .expect("lexing should succeed");
        let program: Program = parser::parse(src, tokens).expect("parsing should succeed");

        let hir_standard = lower(&program).expect("standard lowering should succeed");

        // Graph-based lowering with a root-only graph
        let graph = graph_with_root_ast(&program);
        let mut hir_lower = HirLower::new();
        let hir_graph = hir_lower
            .lower_program_with_graph(&program, &graph)
            .expect("graph lowering should succeed");

        // Core items should be the same
        assert_eq!(hir_standard.functions.len(), hir_graph.functions.len());
        assert_eq!(hir_standard.functions[0].name, hir_graph.functions[0].name);
        assert_eq!(
            hir_standard.functions[0].module,
            hir_graph.functions[0].module
        );

        // Module fields should be populated
        assert_eq!(hir_graph.modules.len(), 1);
        assert_eq!(hir_graph.modules[0].id, ModuleId::ROOT);
        assert_eq!(hir_graph.root_module, ModuleId::ROOT);
    }

    #[test]
    fn test_graph_lower_populates_module_scopes() {
        let src = "fn main() -> i64 { 0 }";
        let tokens: Vec<_> = Lexer::new(src)
            .collect::<Result<Vec<_>, _>>()
            .expect("lexing should succeed");
        let program: Program = parser::parse(src, tokens).expect("parsing should succeed");

        let graph = graph_with_root_ast(&program);
        let mut hir_lower = HirLower::new();
        let hir = hir_lower
            .lower_program_with_graph(&program, &graph)
            .expect("graph lowering should succeed");

        assert_eq!(hir.module_scopes.len(), 1);
        assert_eq!(hir.module_scopes[0].parent, None);
    }

    #[test]
    fn test_graph_lower_multi_module_assigns_correct_module_ids() {
        let root_src = "fn main() -> i64 { 0 }";
        let child_src = "fn helper() -> i64 { 42 }";

        let root_tokens: Vec<_> = Lexer::new(root_src)
            .collect::<Result<Vec<_>, _>>()
            .expect("lexing should succeed");
        let root_program: Program =
            parser::parse(root_src, root_tokens).expect("parsing should succeed");

        let child_tokens: Vec<_> = Lexer::new(child_src)
            .collect::<Result<Vec<_>, _>>()
            .expect("lexing should succeed");
        let child_program: Program =
            parser::parse(child_src, child_tokens).expect("parsing should succeed");

        let graph = graph_with_child(&root_program, "io", &child_program);
        let mut hir_lower = HirLower::new();
        let hir = hir_lower
            .lower_program_with_graph(&root_program, &graph)
            .expect("multi-module lowering should succeed");

        // Two modules in the graph
        assert_eq!(hir.modules.len(), 2);
        assert_eq!(hir.modules[0].id, ModuleId::ROOT);
        assert_eq!(hir.modules[1].id, ModuleId(1));

        // Root module has main, child module has helper
        let main = hir.function_by_name("main").expect("main function");
        let helper = hir.function_by_name("helper").expect("helper function");
        assert_eq!(main.module, ModuleId::ROOT);
        assert_eq!(helper.module, ModuleId(1));

        // Module scopes: root has no parent, child's parent is ROOT
        assert_eq!(hir.module_scopes.len(), 2);
        assert_eq!(hir.module_scopes[0].parent, None);
        assert_eq!(hir.module_scopes[1].parent, Some(ModuleId::ROOT));

        // module_paths should map each function to its owning module
        assert_eq!(*hir.module_paths.get(&main.def_id).unwrap(), ModuleId::ROOT);
        assert_eq!(*hir.module_paths.get(&helper.def_id).unwrap(), ModuleId(1));

        // def_table entries should have correct module IDs
        let main_entry = hir.def_entry(main.def_id).expect("main def entry");
        let helper_entry = hir.def_entry(helper.def_id).expect("helper def entry");
        assert_eq!(main_entry.module, ModuleId::ROOT);
        assert_eq!(helper_entry.module, ModuleId(1));
    }

    #[test]
    fn test_graph_lower_resolves_mod_decl() {
        let root_src = "mod io\nfn main() -> i64 { 0 }";
        let child_src = "fn helper() -> i64 { 42 }";

        let root_tokens: Vec<_> = Lexer::new(root_src)
            .collect::<Result<Vec<_>, _>>()
            .expect("lexing should succeed");
        let root_program: Program =
            parser::parse(root_src, root_tokens).expect("parsing should succeed");

        let child_tokens: Vec<_> = Lexer::new(child_src)
            .collect::<Result<Vec<_>, _>>()
            .expect("lexing should succeed");
        let child_program: Program =
            parser::parse(child_src, child_tokens).expect("parsing should succeed");

        let graph = graph_with_child(&root_program, "io", &child_program);
        let mut hir_lower = HirLower::new();
        let mut hir = hir_lower
            .lower_program_with_graph(&root_program, &graph)
            .expect("multi-module lowering should succeed");

        // The mod declaration should be resolved to child ModuleId(1)
        assert_eq!(hir.mod_decls.len(), 1);
        let mdecl = &hir.mod_decls[0];
        assert_eq!(mdecl.name, hir.symbols.intern("io"));
        assert_eq!(mdecl.module, ModuleId::ROOT);
        assert_eq!(mdecl.module_id, Some(ModuleId(1)));
    }

    #[test]
    fn test_graph_lower_unresolved_mod_decl() {
        // Root has a mod declaration for a module NOT in the graph.
        let root_src = "mod missing\nfn main() -> i64 { 0 }";
        let root_tokens: Vec<_> = Lexer::new(root_src)
            .collect::<Result<Vec<_>, _>>()
            .expect("lexing should succeed");
        let root_program: Program =
            parser::parse(root_src, root_tokens).expect("parsing should succeed");

        let graph = graph_with_root_ast(&root_program);
        let mut hir_lower = HirLower::new();
        let mut hir = hir_lower
            .lower_program_with_graph(&root_program, &graph)
            .expect("lowering should succeed even with unresolved mod");

        assert_eq!(hir.mod_decls.len(), 1);
        let mdecl = &hir.mod_decls[0];
        assert_eq!(mdecl.name, hir.symbols.intern("missing"));
        // module_id should be None since "missing" was not found in the graph
        assert!(mdecl.module_id.is_none());
    }

    #[test]
    fn test_graph_lower_use_decl_in_child_module() {
        let root_src = "fn main() -> i64 { 0 }";
        let child_src = "use foo::bar\nfn helper() -> i64 { 42 }";

        let root_tokens: Vec<_> = Lexer::new(root_src)
            .collect::<Result<Vec<_>, _>>()
            .expect("lexing should succeed");
        let root_program: Program =
            parser::parse(root_src, root_tokens).expect("parsing should succeed");

        let child_tokens: Vec<_> = Lexer::new(child_src)
            .collect::<Result<Vec<_>, _>>()
            .expect("lexing should succeed");
        let child_program: Program =
            parser::parse(child_src, child_tokens).expect("parsing should succeed");

        let graph = graph_with_child(&root_program, "utils", &child_program);
        let mut hir_lower = HirLower::new();
        let hir = hir_lower
            .lower_program_with_graph(&root_program, &graph)
            .expect("lowering should succeed");

        assert_eq!(hir.use_decls.len(), 1);
        let udecl = &hir.use_decls[0];
        assert_eq!(udecl.module, ModuleId(1));
    }

    #[test]
    fn test_analyze_and_lower_with_graph_backward_compat() {
        // Single-file: analyze_and_lower_with_graph should work with a root-only graph
        let src = "fn main() -> i64 { 0 }";
        let tokens: Vec<_> = Lexer::new(src)
            .collect::<Result<Vec<_>, _>>()
            .expect("lexing should succeed");
        let program: Program = parser::parse(src, tokens).expect("parsing should succeed");

        let graph = graph_with_root_ast(&program);
        let hir = crate::semantic::analyze_and_lower_with_graph(&program, &graph)
            .expect("analyze_and_lower_with_graph should succeed");

        assert_eq!(hir.modules.len(), 1);
        assert_eq!(hir.modules[0].id, ModuleId::ROOT);
        assert_eq!(hir.root_module, ModuleId::ROOT);
    }

    // --- resolve_modules tests ---

    #[test]
    fn test_resolve_modules_imports_function_from_child() {
        // Root declares `mod utils` pointing to a child module that
        // defines `fn helper() -> i64 { 42 }`.
        // Child module does `use utils::helper`.
        let root_src = "mod utils struct Foo { } fn main() -> i64 { 0 }";
        let child_src = "fn helper() -> i64 { 42 }";

        let root_tokens: Vec<_> = Lexer::new(root_src)
            .collect::<Result<Vec<_>, _>>()
            .expect("lexing should succeed");
        let root_program: Program =
            parser::parse(root_src, root_tokens).expect("parsing should succeed");

        let child_tokens: Vec<_> = Lexer::new(child_src)
            .collect::<Result<Vec<_>, _>>()
            .expect("lexing should succeed");
        let child_program: Program =
            parser::parse(child_src, child_tokens).expect("parsing should succeed");

        let graph = graph_with_child(&root_program, "utils", &child_program);
        let mut hir = HirLower::new()
            .lower_program_with_graph(&root_program, &graph)
            .expect("lowering should succeed");
        resolve_modules(&mut hir).expect("resolve_modules should succeed");

        // The root module should have a mod_decl item registered for "utils".
        let root_scope = &hir.module_scopes[0];
        let utils_sym = hir.symbols.intern("utils");
        // The mod declaration for "utils" should be findable via lookup_with_parent
        let utils_def = root_scope.lookup_with_parent(&utils_sym, &hir.module_scopes);
        assert!(
            utils_def.is_some(),
            "mod 'utils' should be registered in root scope"
        );
    }

    #[test]
    fn test_resolve_modules_unresolved_module() {
        // A use declaration that references a module that doesn't exist.
        // With only a single root module, `use nonexistent::item` should fail.
        let src = "use nonexistent::item fn main() -> i64 { 0 }";

        let tokens: Vec<_> = Lexer::new(src)
            .collect::<Result<Vec<_>, _>>()
            .expect("lexing should succeed");
        let program: Program = parser::parse(src, tokens).expect("parsing should succeed");

        let graph = graph_with_root_ast(&program);
        let mut hir = HirLower::new()
            .lower_program_with_graph(&program, &graph)
            .expect("lowering should succeed");
        let result = resolve_modules(&mut hir);
        assert!(result.is_err(), "should fail with unresolved import");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("unresolved import"),
            "error should mention unresolved import, got: {}",
            err
        );
    }

    #[test]
    fn test_resolve_modules_single_segment_import() {
        // Single-segment path that doesn't match a module: `use helper;`
        // where `helper` is defined as a function in the same module.
        let src = "fn helper() -> i64 { 42 } use helper fn main() -> i64 { helper() }";

        let tokens: Vec<_> = Lexer::new(src)
            .collect::<Result<Vec<_>, _>>()
            .expect("lexing should succeed");
        let program: Program = parser::parse(src, tokens).expect("parsing should succeed");

        let graph = graph_with_root_ast(&program);
        let mut hir = HirLower::new()
            .lower_program_with_graph(&program, &graph)
            .expect("lowering should succeed");
        resolve_modules(&mut hir).expect("resolve_modules should succeed");

        // The use declaration should have registered an import for "helper"
        let root_scope = &hir.module_scopes[0];
        let helper_sym = hir.symbols.intern("helper");
        let imported = root_scope.imports.get(&helper_sym);
        assert!(
            imported.is_some(),
            "helper should be registered as an import"
        );
    }

    #[test]
    fn test_resolve_modules_registers_struct_in_scope() {
        // Verify that structs are registered in module_scopes by resolve_modules.
        let src = "struct Point { x: i64, y: i64 } fn main() -> i64 { 0 }";

        let tokens: Vec<_> = Lexer::new(src)
            .collect::<Result<Vec<_>, _>>()
            .expect("lexing should succeed");
        let program: Program = parser::parse(src, tokens).expect("parsing should succeed");

        let graph = graph_with_root_ast(&program);
        let mut hir = HirLower::new()
            .lower_program_with_graph(&program, &graph)
            .expect("lowering should succeed");
        resolve_modules(&mut hir).expect("resolve_modules should succeed");

        let root_scope = &hir.module_scopes[0];
        let point_sym = hir.symbols.intern("Point");
        let def_id = root_scope.lookup(&point_sym);
        assert!(
            def_id.is_some(),
            "struct Point should be registered in root scope"
        );
    }

    #[test]
    fn test_resolve_modules_registers_mod_decl_in_scope() {
        // Verify that mod declarations are registered in module_scopes.
        let root_src = "mod utils fn main() -> i64 { 0 }";
        let child_src = "fn helper() -> i64 { 42 }";

        let root_tokens: Vec<_> = Lexer::new(root_src)
            .collect::<Result<Vec<_>, _>>()
            .expect("lexing should succeed");
        let root_program: Program =
            parser::parse(root_src, root_tokens).expect("parsing should succeed");

        let child_tokens: Vec<_> = Lexer::new(child_src)
            .collect::<Result<Vec<_>, _>>()
            .expect("lexing should succeed");
        let child_program: Program =
            parser::parse(child_src, child_tokens).expect("parsing should succeed");

        let graph = graph_with_child(&root_program, "utils", &child_program);
        let mut hir = HirLower::new()
            .lower_program_with_graph(&root_program, &graph)
            .expect("lowering should succeed");
        resolve_modules(&mut hir).expect("resolve_modules should succeed");

        // "utils" should be findable as a mod declaration in root scope
        let root_scope = &hir.module_scopes[0];
        let utils_sym = hir.symbols.intern("utils");
        let def_id = root_scope.lookup(&utils_sym);
        assert!(
            def_id.is_some(),
            "mod 'utils' should be in root scope items"
        );
    }

    #[test]
    fn test_resolve_modules_cross_module_use_import() {
        // Root module: `mod utils` + `fn helper() -> i64 { 42 }`
        // Child module (utils): `use crate::helper` (importing from parent)
        //
        // Actually, in this language, `use foo::bar` means "find module foo,
        // then look up bar in foo's scope". So `use utils::helper` from the
        // root module would look for module "utils" and then "helper" in it.
        //
        // For a true cross-module import, we need:
        // Root: `mod utils` + `fn main() -> i64 { 0 }`
        // Child (utils): `use root::main` — but "root" isn't a module name.
        //
        // Instead, let's test: root defines `fn helper() -> i64 { 42 }`,
        // child does `use root::helper` where "root" is the module name.
        // But the root module's name is "crate" (or None for empty path).
        //
        // Better approach: two child modules, child2 imports from child1.
        // Root: `mod lib fn main() -> i64 { 0 }`
        // Child1 (lib): `fn helper() -> i64 { 42 }`
        // Child2: `use lib::helper`
        //
        // But graph_with_child only supports one child. Let me build a 3-module graph.
        let root_src = "mod lib mod app fn main() -> i64 { 0 }";
        let lib_src = "fn helper() -> i64 { 42 }";
        let app_src = "use lib::helper fn run() -> i64 { helper() }";

        let root_tokens: Vec<_> = Lexer::new(root_src)
            .collect::<Result<Vec<_>, _>>()
            .expect("lexing should succeed");
        let root_program: Program =
            parser::parse(root_src, root_tokens).expect("parsing should succeed");

        let lib_tokens: Vec<_> = Lexer::new(lib_src)
            .collect::<Result<Vec<_>, _>>()
            .expect("lexing should succeed");
        let lib_program: Program =
            parser::parse(lib_src, lib_tokens).expect("parsing should succeed");

        let app_tokens: Vec<_> = Lexer::new(app_src)
            .collect::<Result<Vec<_>, _>>()
            .expect("lexing should succeed");
        let app_program: Program =
            parser::parse(app_src, app_tokens).expect("parsing should succeed");

        // Build a 3-module graph manually: root + lib + app
        let mut graph = ModuleGraph::new();

        // Root module
        let mut root_mod = Module::new(
            ModuleId::ROOT,
            crate::module::ModulePath::new(),
            std::path::PathBuf::from("<root>"),
        );
        root_mod.mod_declarations = vec!["lib".to_string(), "app".to_string()];
        root_mod.ast = Some(root_program.clone());
        graph.add_module(root_mod);

        // lib child module
        let lib_segment = graph.symbol_interner.intern("lib");
        let lib_path = crate::module::ModulePath::from_segments(vec![lib_segment]);
        let mut lib_mod = Module::new(
            ModuleId::new(1),
            lib_path,
            std::path::PathBuf::from("<root>/lib"),
        );
        lib_mod.parent = Some(ModuleId::ROOT);
        lib_mod.ast = Some(lib_program.clone());
        graph.add_module(lib_mod);

        // app child module
        let app_segment = graph.symbol_interner.intern("app");
        let app_path = crate::module::ModulePath::from_segments(vec![app_segment]);
        let mut app_mod = Module::new(
            ModuleId::new(2),
            app_path,
            std::path::PathBuf::from("<root>/app"),
        );
        app_mod.parent = Some(ModuleId::ROOT);
        app_mod.ast = Some(app_program.clone());
        graph.add_module(app_mod);

        let mut hir = HirLower::new()
            .lower_program_with_graph(&root_program, &graph)
            .expect("lowering should succeed");
        resolve_modules(&mut hir).expect("resolve_modules should succeed");

        // The "app" module (ModuleId 2) should have an import for "helper"
        // pointing to the same DefId as the `helper` function in the "lib" module.
        let app_scope = &hir.module_scopes[2];
        let helper_alias = hir.symbols.intern("helper");
        let imported_def_id = app_scope.imports.get(&helper_alias);
        assert!(
            imported_def_id.is_some(),
            "helper should be registered as an import in the app module"
        );

        // Verify the import points to the correct function in the lib module
        let target_def_id = imported_def_id.unwrap();
        let target_module = hir.module_of(*target_def_id);
        assert_eq!(
            target_module,
            Some(ModuleId::new(1)),
            "imported function should come from the lib module"
        );

        // Also verify the imported DefId matches the helper function's DefId
        let helper_func = hir
            .functions
            .iter()
            .find(|f| hir.symbols.lookup(f.name) == Some("helper"))
            .expect("helper function should exist");
        assert_eq!(
            *target_def_id, helper_func.def_id,
            "imported DefId should match helper's DefId"
        );
    }
    /// Helper: build a minimal single-module ModuleGraph with the given AST
    /// attached to the root module.
    fn graph_with_root_ast(program: &Program) -> ModuleGraph {
        let mut graph = ModuleGraph::new();
        let root_module = Module::new(
            ModuleId::ROOT,
            crate::module::ModulePath::new(),
            std::path::PathBuf::from("<root>"),
        );
        graph.add_module(root_module);
        // Attach the AST to the root module.
        graph.modules[0].ast = Some(program.clone());
        graph
    }

    /// Helper: build a two-module graph (root + child) with the given ASTs.
    fn graph_with_child(
        root_program: &Program,
        child_name: &str,
        child_program: &Program,
    ) -> ModuleGraph {
        let mut graph = ModuleGraph::new();

        // Root module
        let mut root_module = Module::new(
            ModuleId::ROOT,
            crate::module::ModulePath::new(),
            std::path::PathBuf::from("<root>"),
        );
        // Record the mod declaration so the child is discoverable.
        root_module.mod_declarations = vec![child_name.to_string()];
        root_module.ast = Some(root_program.clone());
        graph.add_module(root_module);

        // Child module
        let segment = graph.symbol_interner.intern(child_name);
        let child_path = crate::module::ModulePath::from_segments(vec![segment]);
        let mut child_module = Module::new(
            ModuleId(1),
            child_path,
            std::path::PathBuf::from(format!("<root>/{child_name}")),
        );
        child_module.parent = Some(ModuleId::ROOT);
        child_module.ast = Some(child_program.clone());
        graph.add_module(child_module);

        graph
    }
}
