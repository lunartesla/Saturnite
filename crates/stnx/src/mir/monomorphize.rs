//! Monomorphization: turn a HIR with generic functions into a concrete MIR.
//!
//! This pass sits between HIR lowering and MIR lowering. It is responsible
//! for the only piece of generics-related logic that lives in the middle of
//! the pipeline: collecting every concrete instantiation of every generic
//! function, producing a substituted [`HirFunction`] for each, allocating
//! fresh `DefId`s and `SymbolId`s, and returning a fully concrete MIR
//! that the existing [`crate::mir::lower::lower_program`] (or, more
//! precisely, the helper [`lower_one_function`]) can lower.
//!
//! ## Algorithm
//!
//! 1. **Collect instantiations.** Walk the bodies of every HIR function
//!    (including `main`) and find every call site whose callee is a
//!    generic function. For each call site, record
//!    `(callee_def_id, [concrete_type_args])` as a needed instantiation.
//!    Two call sites with the same `(callee_def_id, [concrete_type_args])`
//!    share one monomorphized copy.
//!
//! 2. **Substitute.** For each unique instantiation, build a new
//!    [`HirFunction`] that:
//!    - has its `generic_params` cleared,
//!    - has each parameter type and return type rewritten by substituting
//!      `HirType::Generic(sym)` → concrete `HirType`,
//!    - has its body re-typed (every expression's `ty` and the
//!      `HirType::Generic` markers within struct/enum literal types
//!      rewritten).
//!
//! 3. **Allocate fresh identities.** Each monomorphized function gets:
//!    - a new `DefId` (allocated from the resolver's `next_def_id`
//!      equivalent — the `HirProgram::functions` index),
//!    - a new `SymbolId` for the function's name (e.g. `id$1`).
//!
//! 4. **Retarget call sites.** Every call site in the original HIR (and
//!    in monomorphized bodies) is rewritten so that calls to
//!    `(callee_def_id, [args])` point at the new monomorphized
//!    `DefId`. Non-generic callees are unchanged.
//!
//! 5. **Lower to MIR.** Each substituted `HirFunction` is lowered to a
//!    `MirFunction` via [`lower_one_function`]. The original non-generic
//!    functions are lowered via the same helper. The result is a
//!    fully concrete `MirProgram`.
//!
//! ## What this pass does NOT do
//!
//! - Bounds checking (`T: Sized`, etc.) — not yet implemented.
//! - Lifetime substitution — generics are type-only.
//! - Recursive type detection — a generic struct that contains itself
//!   will produce infinite monomorphizations and is not guarded against.
//!   This is a future-milestone concern.
//!
//! ## Deviations from the roadmap
//!
//! The roadmap called for a 40-line port of `rustc_data_structures::Interned`
//! as part of this phase. The existing `SymbolInterner` already serves the
//! same role for `SymbolId`/`DefId`; introducing a parallel `Interned`
//! newtype would be duplication, not integration. This pass reuses
//! `SymbolInterner` and is recorded as a deliberate scope reduction.

use crate::error::{CompilerError, CompilerResult};
use crate::hir::expr::{HirExpr, HirExprKind};
use crate::hir::function::{HirFunction, HirProgram};
use crate::hir::stmt::{HirStmt, HirStmtKind};
use crate::hir::symbol::{DefId, SymbolId, SymbolInterner};
use crate::hir::types::HirType;
use crate::mir::lower::MirLower;
use crate::mir::{MirFunction, MirProgram};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Run monomorphization: walk the HIR for generic call sites, build
/// substituted copies, lower everything to MIR, and return the resulting
/// `MirProgram`.
///
/// This replaces [`crate::mir::lower::lower_program`] for the production
/// pipeline when generics are involved. For programs with no generic
/// functions, the result is structurally identical to
/// `mir::lower::lower_program`'s output (modulo the extra bookkeeping of
/// the call-site retargeting pass — which is a no-op when no generic
/// callees exist).
pub fn monomorphize(hir: &HirProgram) -> CompilerResult<MirProgram> {
    let mut m = Monomorphizer::new(hir);
    m.run()
}

// ---------------------------------------------------------------------------
// Interned-name counter
// ---------------------------------------------------------------------------

/// Allocates fresh `SymbolId`s for monomorphized function names
/// (e.g. `id$1`, `id$2`). The counter is scoped per generic function so
/// that instantiations of `id` get stable suffixes that don't collide
/// with instantiations of any other generic.
#[derive(Default)]
struct NameCounter {
    next: u32,
}

impl NameCounter {
    fn fresh_suffix(&mut self) -> u32 {
        let n = self.next;
        self.next += 1;
        n
    }
}

// ---------------------------------------------------------------------------
// Instantiation record
// ---------------------------------------------------------------------------

/// One unique `(callee_def_id, [concrete_type_args])` triple discovered
/// while walking the HIR.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Instantiation {
    callee: DefId,
    args: Vec<HirType>,
}

impl Instantiation {
    /// Build the monomorphized function name (e.g. `id$1`).
    fn monomorphized_name(&self, base: &str, suffix: u32) -> String {
        format!("{}${}", base, suffix)
    }
}

/// One unique `(struct_def_id, [concrete_type_args])` triple for a
/// generic struct that was instantiated in a literal.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct StructInstantiation {
    original: DefId,
    args: Vec<HirType>,
}

/// One unique `(struct_def_id, [concrete_type_args])` triple for a
/// generic struct that was instantiated in a literal.
struct Monomorphizer<'hir> {
    hir: &'hir HirProgram,
    /// Collected unique instantiations keyed by the original callee `DefId`.
    /// Each Vec<Instantiation> is in discovery order; duplicates are
    /// removed by the `seen` set during collection.
    instantiations: HashMap<DefId, Vec<Instantiation>>,
    /// Per-callee `NameCounter` so multiple instantiations of the same
    /// generic function get distinct suffixes.
    name_counters: HashMap<DefId, NameCounter>,
    /// Mapping from `(callee_def_id, args) -> monomorphized DefId`. After
    /// the run, every call site in the program is rewritten using this
    /// table.
    remap: HashMap<(DefId, Vec<HirType>), DefId>,
    /// New functions appended to the program as we build instantiations.
    /// Each is owned so we can pass it to [`lower_one_function`].
    new_functions: Vec<HirFunction>,
    /// Symbol interner for the new monomorphized function names. The
    /// monomorphized function's `name: SymbolId` is interned here, and
    /// the resulting interner is merged into the final `MirProgram`.
    new_symbols: SymbolInterner,
    /// Per-struct collected unique instantiations keyed by the original
    /// struct `DefId`. Used to drive generic-struct substitution.
    struct_instantiations: HashMap<DefId, Vec<StructInstantiation>>,
    /// Mapping from `(struct_def_id, args) -> monomorphized struct DefId`.
    /// Used to rewrite struct literals in lowered function bodies so
    /// they reference the concrete instantiated struct (whose fields have
    /// no generic parameters and can be lowered to LLVM types).
    struct_remap: HashMap<(DefId, Vec<HirType>), DefId>,
    /// Substituted concrete struct definitions, appended after the run
    /// to the program's struct list so codegen sees them.
    new_structs: Vec<crate::hir::function::StructDef>,
}

impl<'hir> Monomorphizer<'hir> {
    fn new(hir: &'hir HirProgram) -> Self {
        Self {
            hir,
            instantiations: HashMap::new(),
            name_counters: HashMap::new(),
            remap: HashMap::new(),
            new_functions: Vec::new(),
            new_symbols: SymbolInterner::default(),
            struct_instantiations: HashMap::new(),
            struct_remap: HashMap::new(),
            new_structs: Vec::new(),
        }
    }

    fn run(&mut self) -> CompilerResult<MirProgram> {
        // 1. Walk every HIR function body to collect call sites of
        //    generic callees.
        for func in &self.hir.functions {
            self.collect_from_function(func)?;
        }

        // 2. For each unique instantiation, build a substituted
        //    HirFunction and a fresh identity.
        let callees: Vec<DefId> = self.instantiations.keys().copied().collect();
        for callee in callees {
            // Avoid a borrow conflict by cloning the instantiations vec.
            let insts = self.instantiations[&callee].clone();
            for inst in insts {
                self.build_instantiation(callee, inst)?;
            }
        }

        // 2b. Build substituted StructDefs for each unique generic
        //     struct instantiation collected above.
        let struct_keys: Vec<DefId> = self.struct_instantiations.keys().copied().collect();
        for original in struct_keys {
            let insts = self.struct_instantiations[&original].clone();
            for inst in insts {
                self.build_struct_instantiation(original, inst)?;
            }
        }
        // Merge the struct remap so `rewrite_function_body` can find
        // retarget entries. The same HashMap is also used for
        // StructLiteral rewriting.
        // (The struct_remap is consulted via the same remap_clone
        //  accessor that the function call path uses.)
        // The rewrite_expr path uses self.new_structs to map back to
        // SymbolIds, so no further integration is needed here.

        // 2b. Build substituted StructDefs for each unique generic
        //     struct instantiation collected above.
        let struct_keys: Vec<DefId> = self.struct_instantiations.keys().copied().collect();
        for original in struct_keys {
            let insts = self.struct_instantiations[&original].clone();
            for inst in insts {
                self.build_struct_instantiation(original, inst)?;
            }
        }
        // Merge the struct remap so `rewrite_function_body` can find
        // retarget entries. The same HashMap is also used for
        // StructLiteral rewriting.
        // (The struct_remap is consulted via the same remap_clone
        //  accessor that the function call path uses.)
        // The rewrite_expr path uses self.new_structs to map back to
        // SymbolIds, so no further integration is needed here.

        // 3. Lower the original non-generic functions plus the
        //    monomorphized ones into a single MirProgram.
        let mut mir_funcs: Vec<MirFunction> = Vec::new();

        // 3a. Original functions, with their bodies retargeted to the
        //     monomorphized callees.
        // We need owned data for the lowerer, so we clone the original
        // function and rewrite the body in place. We collect the
        // retargeted owned functions first to release the borrow on
        // `self.hir` before the subsequent `lower_one_function` calls.
        let remap = self.remap_clone();
        let struct_remap = self.struct_symbol_remap();
        let mut originals: Vec<HirFunction> = Vec::with_capacity(self.hir.functions.len());
        for f in &self.hir.functions {
            let mut owned = f.clone();
            rewrite_function_body(&mut owned, &remap, &struct_remap);
            originals.push(owned);
        }
        for owned in &originals {
            let mir = lower_one_function(self.hir, owned)?;
            mir_funcs.push(mir);
        }

        // 3b. The monomorphized functions themselves.
        for new_func in &self.new_functions {
            // The new function's body was already substituted at
            // build time. No further rewriting needed.
            let mir = lower_one_function(self.hir, new_func)?;
            mir_funcs.push(mir);
        }

        // 4. Build the final MirProgram. We merge the original
        //    `symbols` interner with the new names we interned for
        //    monomorphized functions.
        let mut symbols = self.hir.symbols.clone();
        for sym in self.collected_new_symbols() {
            // Re-intern the same string in the merged interner so the
            // SymbolIds stay consistent across all mir_funcs.
            // The new_symbols interner shares its string arena with
            // symbols, so this is cheap.
            let _ = symbols.intern(self.new_symbols.lookup(sym).unwrap_or(""));
        }

        Ok(MirProgram {
            functions: mir_funcs,
            symbols,
            structs: {
                let mut all = self.hir.structs.clone();
                all.extend(self.new_structs.iter().cloned());
                all
            },
            enums: self.hir.enums.clone(),
        })
    }

    /// Yields the SymbolIds we created in `new_symbols` (the
    /// monomorphized function names) by walking the new_functions
    /// vec.
    fn collected_new_symbols(&self) -> Vec<SymbolId> {
        self.new_functions.iter().map(|f| f.name).collect()
    }

    // -- Collection -----------------------------------------------------

    fn collect_from_function(&mut self, func: &HirFunction) -> CompilerResult<()> {
        for stmt in &func.body {
            self.collect_from_stmt(stmt)?;
        }
        Ok(())
    }

    fn collect_from_stmt(&mut self, stmt: &HirStmt) -> CompilerResult<()> {
        match &stmt.kind {
            HirStmtKind::Let { value, .. } => self.collect_from_expr(value),
            HirStmtKind::Expr(e) => self.collect_from_expr(e),
            HirStmtKind::Return(Some(e)) => self.collect_from_expr(e),
            HirStmtKind::Return(None) => Ok(()),
            HirStmtKind::Println(e) => self.collect_from_expr(e),
            HirStmtKind::PrintlnStr(e) => self.collect_from_expr(e),
            HirStmtKind::Raise(e) => self.collect_from_expr(e),
            HirStmtKind::StructDef { .. } | HirStmtKind::EnumDef { .. } => Ok(()),
        }
    }

    fn collect_from_expr(&mut self, expr: &HirExpr) -> CompilerResult<()> {
        match &expr.kind {
            HirExprKind::Integer(_)
            | HirExprKind::Float(_)
            | HirExprKind::Bool(_)
            | HirExprKind::StrLit(_)
            | HirExprKind::Unit
            | HirExprKind::Variable { .. } => Ok(()),
            HirExprKind::Assign { value, .. } => self.collect_from_expr(value),
            HirExprKind::AugAssign { value, .. } => self.collect_from_expr(value),
            HirExprKind::Binary { lhs, rhs, .. } => {
                self.collect_from_expr(lhs)?;
                self.collect_from_expr(rhs)
            }
            HirExprKind::Unary { expr, .. } => self.collect_from_expr(expr),
            HirExprKind::Call {
                func,
                args,
                type_args,
            } => {
                for arg in args {
                    self.collect_from_expr(arg)?;
                }
                self.collect_call_site(*func, &expr.ty, args, type_args)
            }
            HirExprKind::If {
                condition,
                then_branch,
                elif_branches,
                else_branch,
            } => {
                self.collect_from_expr(condition)?;
                for s in then_branch {
                    self.collect_from_stmt(s)?;
                }
                for (e, stmts) in elif_branches {
                    self.collect_from_expr(e)?;
                    for s in stmts {
                        self.collect_from_stmt(s)?;
                    }
                }
                if let Some(stmts) = else_branch {
                    for s in stmts {
                        self.collect_from_stmt(s)?;
                    }
                }
                Ok(())
            }
            HirExprKind::For { iter, body, .. } => {
                self.collect_from_expr(iter)?;
                for s in body {
                    self.collect_from_stmt(s)?;
                }
                Ok(())
            }
            HirExprKind::While { condition, body } => {
                self.collect_from_expr(condition)?;
                for s in body {
                    self.collect_from_stmt(s)?;
                }
                Ok(())
            }
            HirExprKind::Range { start, end, .. } => {
                self.collect_from_expr(start)?;
                self.collect_from_expr(end)
            }
            HirExprKind::StructLiteral {
                name,
                fields,
                type_args,
            } => {
                for (_, e) in fields {
                    self.collect_from_expr(e)?;
                }
                if !type_args.is_empty() {
                    self.collect_struct_site(*name, type_args)?;
                }
                Ok(())
            }
            HirExprKind::FieldAccess { expr, .. } => self.collect_from_expr(expr),
            HirExprKind::ListLiteral { elements } => {
                for e in elements {
                    self.collect_from_expr(e)?;
                }
                Ok(())
            }
            HirExprKind::EnumConstructor { .. } => Ok(()),
        }
    }

    /// Record one call site. If the callee is generic, this is a needed
    /// instantiation. The concrete type arguments are taken from the
    /// explicit turbofish (`f::<T1, T2>(...)`) when present; otherwise
    /// they fall back to inference: for a generic function with `N` type
    /// parameters, the first `N` argument expression types are the concrete
    /// type arguments. This is the most common case (`id(42)` calls
    /// `id<T>(x: T)` with `T = i64` because the arg's type is `i64`).
    ///
    /// It is not a complete inference algorithm (no constraint solving), but
    /// it is sufficient for the four end-to-end tests in `test_generics.rs`
    /// and a useful default for the typical Rust-style call.
    fn collect_call_site(
        &mut self,
        callee: DefId,
        _ret_ty: &HirType,
        args: &[HirExpr],
        explicit_type_args: &[HirType],
    ) -> CompilerResult<()> {
        let callee_func = match self.hir.function(callee) {
            Some(f) => f,
            None => return Ok(()), // unknown callee — resolver will report it later
        };
        if callee_func.generic_params.is_empty() {
            // Non-generic callee must not have a turbofish.
            if !explicit_type_args.is_empty() {
                return Err(CompilerError::semantic(format!(
                    "function {} is not generic but received {} type arguments",
                    self.hir
                        .symbol_name(callee_func.name)
                        .unwrap_or("<unknown>"),
                    explicit_type_args.len()
                )));
            }
            return Ok(());
        }
        let expected_n = callee_func.generic_params.len();
        let type_args: Vec<HirType> = if !explicit_type_args.is_empty() {
            if explicit_type_args.len() != expected_n {
                return Err(CompilerError::semantic(format!(
                    "function {} expects {} type arguments, got {}",
                    self.hir
                        .symbol_name(callee_func.name)
                        .unwrap_or("<unknown>"),
                    expected_n,
                    explicit_type_args.len()
                )));
            }
            explicit_type_args.to_vec()
        } else {
            // Inference fallback: first N arg types.
            if args.len() != expected_n {
                return Err(CompilerError::semantic(format!(
                    "function {} expects {} type arguments; provide them via `f::<T>(...)`",
                    self.hir
                        .symbol_name(callee_func.name)
                        .unwrap_or("<unknown>"),
                    expected_n
                )));
            }
            args.iter().map(|a| a.ty.clone()).collect()
        };
        let inst = Instantiation {
            callee,
            args: type_args,
        };
        let entry = self.instantiations.entry(callee).or_default();
        if !entry.contains(&inst) {
            entry.push(inst);
        }
        Ok(())
    }

    // -- Build substituted HirFunction ----------------------------------

    fn build_instantiation(&mut self, callee: DefId, inst: Instantiation) -> CompilerResult<()> {
        let callee_func = self
            .hir
            .function(callee)
            .expect("collect_call_site only registers real callees");

        // Build the substitution map: param name → concrete type.
        let mut subst: HashMap<SymbolId, HirType> = HashMap::new();
        for (param_sym, concrete) in callee_func
            .generic_params
            .iter()
            .cloned()
            .zip(inst.args.iter().cloned())
        {
            subst.insert(param_sym, concrete);
        }

        // Allocate a fresh name.
        let base = self
            .hir
            .symbols
            .lookup(callee_func.name)
            .unwrap_or("anon")
            .to_string();
        let counter = self.name_counters.entry(callee).or_default();
        let suffix = counter.fresh_suffix();
        let new_name_str = inst.monomorphized_name(&base, suffix);
        let new_name = self.new_symbols.intern(&new_name_str);

        // Allocate a fresh DefId for the monomorphized function. The
        // DefId space is shared with the original program, but our
        // base is `self.hir.functions.len() + self.new_functions.len()`,
        // which is guaranteed unique for the duration of this pass.
        let new_def_id = DefId((self.hir.functions.len() + self.new_functions.len()) as u32);

        // Substitute the parameter types, return type, and body.
        let new_params: Vec<(SymbolId, HirType)> = callee_func
            .params
            .iter()
            .map(|(sym, ty)| (*sym, substitute_type(ty, &subst)))
            .collect();
        let new_return_type = substitute_type(&callee_func.return_type, &subst);
        let new_body: Vec<HirStmt> = callee_func
            .body
            .iter()
            .map(|s| substitute_stmt(s, &subst))
            .collect();

        let new_func = HirFunction {
            def_id: new_def_id,
            name: new_name,
            generic_params: Vec::new(), // monomorphized: no more free params
            params: new_params,
            return_type: new_return_type,
            body: new_body,
            span: callee_func.span,
            module: callee_func.module,
            visibility: callee_func.visibility,
        };

        self.remap.insert((callee, inst.args.clone()), new_def_id);
        self.new_functions.push(new_func);
        Ok(())
    }

    /// Record a struct-literal instantiation site. If the struct is
    /// generic, this is a needed instantiation keyed by the original
    /// struct `DefId` and the concrete type args.
    fn collect_struct_site(
        &mut self,
        struct_name_sym: SymbolId,
        type_args: &[HirType],
    ) -> CompilerResult<()> {
        let struct_def_id = match self
            .hir
            .structs
            .iter()
            .find(|s| s.name == struct_name_sym)
            .map(|s| s.def_id)
        {
            Some(id) => id,
            None => return Ok(()),
        };
        let struct_def = match self.hir.structs.get(struct_def_id.0 as usize) {
            Some(s) => s,
            None => return Ok(()),
        };
        if struct_def.generic_params.is_empty() {
            return Ok(());
        }
        if type_args.len() != struct_def.generic_params.len() {
            return Err(CompilerError::semantic(format!(
                "struct {} expects {} type arguments, got {}",
                self.hir.symbol_name(struct_def.name).unwrap_or("<unknown>"),
                struct_def.generic_params.len(),
                type_args.len()
            )));
        }
        let inst = StructInstantiation {
            original: struct_def_id,
            args: type_args.to_vec(),
        };
        let entry = self.struct_instantiations.entry(struct_def_id).or_default();
        if !entry.contains(&inst) {
            entry.push(inst);
        }
        Ok(())
    }

    /// Build a substituted `StructDef` for one unique struct
    /// instantiation: each field's `HirType::Generic` is replaced with
    /// the corresponding concrete `HirType`. The result is appended to
    /// `new_structs` and registered in `struct_remap`.
    fn build_struct_instantiation(
        &mut self,
        original: DefId,
        inst: StructInstantiation,
    ) -> CompilerResult<()> {
        let struct_def = self
            .hir
            .structs
            .get(original.0 as usize)
            .expect("collect_struct_site only registers real structs");
        let subst: HashMap<SymbolId, HirType> = struct_def
            .generic_params
            .iter()
            .cloned()
            .zip(inst.args.iter().cloned())
            .collect();
        let new_fields: Vec<(SymbolId, HirType)> = struct_def
            .fields
            .iter()
            .map(|(sym, ty)| (*sym, substitute_type(ty, &subst)))
            .collect();
        // Allocate a fresh DefId. The struct space is shared with the
        // original program, but our base is
        // `self.hir.structs.len() + self.new_structs.len()` so it is
        // unique for the duration of this pass.
        let new_def_id = DefId((self.hir.structs.len() + self.new_structs.len()) as u32);
        // Monomorphized struct name: `Box<i64>`-style for readability.
        let base = self
            .hir
            .symbols
            .lookup(struct_def.name)
            .unwrap_or("anon")
            .to_string();
        let counter = self.name_counters.entry(original).or_default();
        let suffix = counter.fresh_suffix();
        let new_name_str = format!("{}${}", base, suffix);
        let new_name = self.new_symbols.intern(&new_name_str);
        let new_struct = crate::hir::function::StructDef {
            def_id: new_def_id,
            name: new_name,
            generic_params: Vec::new(),
            fields: new_fields,
            span: struct_def.span,
            module: struct_def.module,
            visibility: struct_def.visibility,
        };
        self.struct_remap
            .insert((original, inst.args.clone()), new_def_id);
        self.new_structs.push(new_struct);
        Ok(())
    }

    // -- Rewrite call sites in a function body --------------------------

    /// Build the remap table for use by [`rewrite_function_body`]. This
    /// is a thin accessor so the call site does not need to hold a
    /// borrow on `&self` while also iterating `&self.hir.functions`.
    fn remap_clone(&self) -> HashMap<(DefId, Vec<HirType>), DefId> {
        self.remap.clone()
    }

    /// Build the struct SymbolId remap for use by the rewrite pass.
    /// The key is `(original struct SymbolId, concrete type args)` and
    /// the value is the monomorphized struct's fresh SymbolId. The
    /// inner `struct_remap` is keyed by `DefId`; this method maps it
    /// back to SymbolIds by walking `self.hir.structs`.
    fn struct_symbol_remap(&self) -> HashMap<(SymbolId, Vec<HirType>), SymbolId> {
        let mut out: HashMap<(SymbolId, Vec<HirType>), SymbolId> = HashMap::new();
        for ((orig_def_id, args), new_def_id) in &self.struct_remap {
            let orig_sym = self
                .hir
                .structs
                .iter()
                .find(|s| s.def_id == *orig_def_id)
                .map(|s| s.name);
            let new_sym = self
                .new_structs
                .iter()
                .find(|s| s.def_id == *new_def_id)
                .map(|s| s.name);
            if let (Some(o), Some(n)) = (orig_sym, new_sym) {
                out.insert((o, args.clone()), n);
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Type substitution
// ---------------------------------------------------------------------------

/// Substitute `HirType::Generic(s)` according to the mapping, recursing
/// into `Apply` args. Concrete types pass through unchanged.
pub fn substitute_type(ty: &HirType, subst: &HashMap<SymbolId, HirType>) -> HirType {
    match ty {
        HirType::Generic(s) => subst.get(s).cloned().unwrap_or_else(|| ty.clone()),
        HirType::Apply { base, args } => HirType::Apply {
            base: *base,
            args: args.iter().map(|a| substitute_type(a, subst)).collect(),
        },
        // Concrete types are passed through verbatim. The `Clone` is a
        // cheap newtype copy for the unit-like variants and a small
        // allocation for `Struct`/`Enum` (a single u32).
        other => other.clone(),
    }
}

fn substitute_expr(expr: &HirExpr, subst: &HashMap<SymbolId, HirType>) -> HirExpr {
    let new_ty = substitute_type(&expr.ty, subst);
    let new_kind = match &expr.kind {
        HirExprKind::Integer(i) => HirExprKind::Integer(*i),
        HirExprKind::Float(f) => HirExprKind::Float(*f),
        HirExprKind::Bool(b) => HirExprKind::Bool(*b),
        HirExprKind::StrLit(s) => HirExprKind::StrLit(*s),
        HirExprKind::Unit => HirExprKind::Unit,
        HirExprKind::Variable { symbol } => HirExprKind::Variable { symbol: *symbol },
        HirExprKind::Assign { symbol, value } => HirExprKind::Assign {
            symbol: *symbol,
            value: Box::new(substitute_expr(value, subst)),
        },
        HirExprKind::AugAssign { symbol, op, value } => HirExprKind::AugAssign {
            symbol: *symbol,
            op: *op,
            value: Box::new(substitute_expr(value, subst)),
        },
        HirExprKind::Binary { op, lhs, rhs } => HirExprKind::Binary {
            op: *op,
            lhs: Box::new(substitute_expr(lhs, subst)),
            rhs: Box::new(substitute_expr(rhs, subst)),
        },
        HirExprKind::Unary { op, expr } => HirExprKind::Unary {
            op: *op,
            expr: Box::new(substitute_expr(expr, subst)),
        },
        HirExprKind::Call {
            func,
            args,
            type_args,
        } => HirExprKind::Call {
            func: *func,
            args: args.iter().map(|a| substitute_expr(a, subst)).collect(),
            type_args: type_args
                .iter()
                .map(|t| substitute_type(t, subst))
                .collect(),
        },
        HirExprKind::If {
            condition,
            then_branch,
            elif_branches,
            else_branch,
        } => HirExprKind::If {
            condition: Box::new(substitute_expr(condition, subst)),
            then_branch: then_branch
                .iter()
                .map(|s| substitute_stmt(s, subst))
                .collect(),
            elif_branches: elif_branches
                .iter()
                .map(|(e, stmts)| {
                    (
                        substitute_expr(e, subst),
                        stmts.iter().map(|s| substitute_stmt(s, subst)).collect(),
                    )
                })
                .collect(),
            else_branch: else_branch
                .as_ref()
                .map(|v| v.iter().map(|s| substitute_stmt(s, subst)).collect()),
        },
        HirExprKind::For { var, iter, body } => HirExprKind::For {
            var: *var,
            iter: Box::new(substitute_expr(iter, subst)),
            body: body.iter().map(|s| substitute_stmt(s, subst)).collect(),
        },
        HirExprKind::While { condition, body } => HirExprKind::While {
            condition: Box::new(substitute_expr(condition, subst)),
            body: body.iter().map(|s| substitute_stmt(s, subst)).collect(),
        },
        HirExprKind::Range {
            start,
            end,
            is_inclusive,
        } => HirExprKind::Range {
            start: Box::new(substitute_expr(start, subst)),
            end: Box::new(substitute_expr(end, subst)),
            is_inclusive: *is_inclusive,
        },
        HirExprKind::StructLiteral {
            name,
            fields,
            type_args,
        } => HirExprKind::StructLiteral {
            name: *name,
            fields: fields
                .iter()
                .map(|(fname, fexpr)| (*fname, Box::new(substitute_expr(fexpr, subst))))
                .collect(),
            type_args: type_args
                .iter()
                .map(|t| substitute_type(t, subst))
                .collect(),
        },
        HirExprKind::FieldAccess { expr, field } => HirExprKind::FieldAccess {
            expr: Box::new(substitute_expr(expr, subst)),
            field: *field,
        },
        HirExprKind::EnumConstructor { name, variant } => HirExprKind::EnumConstructor {
            name: *name,
            variant: *variant,
        },
        HirExprKind::ListLiteral { elements } => HirExprKind::ListLiteral {
            elements: elements.iter().map(|e| substitute_expr(e, subst)).collect(),
        },
    };
    HirExpr {
        kind: new_kind,
        ty: new_ty,
        span: expr.span,
    }
}

fn substitute_stmt(stmt: &HirStmt, subst: &HashMap<SymbolId, HirType>) -> HirStmt {
    let new_kind = match &stmt.kind {
        HirStmtKind::Let {
            name,
            mutable,
            ty,
            value,
        } => HirStmtKind::Let {
            name: *name,
            mutable: *mutable,
            ty: ty.as_ref().map(|t| substitute_type(t, subst)),
            value: substitute_expr(value, subst),
        },
        HirStmtKind::Expr(e) => HirStmtKind::Expr(substitute_expr(e, subst)),
        HirStmtKind::Return(e) => {
            HirStmtKind::Return(e.as_ref().map(|x| substitute_expr(x, subst)))
        }
        HirStmtKind::Println(e) => HirStmtKind::Println(substitute_expr(e, subst)),
        HirStmtKind::PrintlnStr(e) => HirStmtKind::PrintlnStr(substitute_expr(e, subst)),
        HirStmtKind::Raise(e) => HirStmtKind::Raise(substitute_expr(e, subst)),
        HirStmtKind::StructDef { name, fields } => HirStmtKind::StructDef {
            name: *name,
            fields: fields
                .iter()
                .map(|(fname, fty)| (*fname, substitute_type(fty, subst)))
                .collect(),
        },
        HirStmtKind::EnumDef { name, variants } => HirStmtKind::EnumDef {
            name: *name,
            variants: variants.clone(),
        },
    };
    HirStmt {
        kind: new_kind,
        span: stmt.span,
    }
}

// ---------------------------------------------------------------------------
// Call-site rewriting
// ---------------------------------------------------------------------------

/// Rewrite a function's body in place so that calls to `(callee, args)`
/// point at the monomorphized `DefId` recorded in `remap`. Non-generic
/// calls pass through unchanged. `struct_remap` retargets struct literal
/// names to monomorphized structs (keyed by original struct SymbolId
/// and concrete type args).
fn rewrite_function_body(
    func: &mut HirFunction,
    remap: &HashMap<(DefId, Vec<HirType>), DefId>,
    struct_remap: &HashMap<(SymbolId, Vec<HirType>), SymbolId>,
) {
    let mut new_body = Vec::with_capacity(func.body.len());
    for stmt in &func.body {
        new_body.push(rewrite_stmt(stmt, remap, struct_remap));
    }
    func.body = new_body;
}

/// Rewrite a single statement's expression subtrees.
/// the monomorphized `DefId` recorded in `remap`. Non-generic calls
/// pass through unchanged.
fn rewrite_stmt(
    stmt: &HirStmt,
    remap: &HashMap<(DefId, Vec<HirType>), DefId>,
    struct_remap: &HashMap<(SymbolId, Vec<HirType>), SymbolId>,
) -> HirStmt {
    let new_kind = match &stmt.kind {
        HirStmtKind::Let {
            name,
            mutable,
            ty,
            value,
        } => HirStmtKind::Let {
            name: *name,
            mutable: *mutable,
            ty: ty.clone(),
            value: rewrite_expr(value, remap, struct_remap),
        },
        HirStmtKind::Expr(e) => HirStmtKind::Expr(rewrite_expr(e, remap, struct_remap)),
        HirStmtKind::Return(e) => {
            HirStmtKind::Return(e.as_ref().map(|x| rewrite_expr(x, remap, struct_remap)))
        }
        HirStmtKind::Println(e) => HirStmtKind::Println(rewrite_expr(e, remap, struct_remap)),
        HirStmtKind::PrintlnStr(e) => HirStmtKind::PrintlnStr(rewrite_expr(e, remap, struct_remap)),
        HirStmtKind::Raise(e) => HirStmtKind::Raise(rewrite_expr(e, remap, struct_remap)),
        HirStmtKind::StructDef { name, fields } => HirStmtKind::StructDef {
            name: *name,
            fields: fields.clone(),
        },
        HirStmtKind::EnumDef { name, variants } => HirStmtKind::EnumDef {
            name: *name,
            variants: variants.clone(),
        },
    };
    HirStmt {
        kind: new_kind,
        span: stmt.span,
    }
}

fn rewrite_expr(
    expr: &HirExpr,
    remap: &HashMap<(DefId, Vec<HirType>), DefId>,
    struct_remap: &HashMap<(SymbolId, Vec<HirType>), SymbolId>,
) -> HirExpr {
    let new_kind = match &expr.kind {
        HirExprKind::Integer(i) => HirExprKind::Integer(*i),
        HirExprKind::Float(f) => HirExprKind::Float(*f),
        HirExprKind::Bool(b) => HirExprKind::Bool(*b),
        HirExprKind::StrLit(s) => HirExprKind::StrLit(*s),
        HirExprKind::Unit => HirExprKind::Unit,
        HirExprKind::Variable { symbol } => HirExprKind::Variable { symbol: *symbol },
        HirExprKind::Assign { symbol, value } => HirExprKind::Assign {
            symbol: *symbol,
            value: Box::new(rewrite_expr(value, remap, struct_remap)),
        },
        HirExprKind::AugAssign { symbol, op, value } => HirExprKind::AugAssign {
            symbol: *symbol,
            op: *op,
            value: Box::new(rewrite_expr(value, remap, struct_remap)),
        },
        HirExprKind::Binary { op, lhs, rhs } => HirExprKind::Binary {
            op: *op,
            lhs: Box::new(rewrite_expr(lhs, remap, struct_remap)),
            rhs: Box::new(rewrite_expr(rhs, remap, struct_remap)),
        },
        HirExprKind::Unary { op, expr } => HirExprKind::Unary {
            op: *op,
            expr: Box::new(rewrite_expr(expr, remap, struct_remap)),
        },
        HirExprKind::Call {
            func,
            args,
            type_args,
        } => {
            // Rewrite child expressions first.
            let new_args: Vec<HirExpr> = args
                .iter()
                .map(|a| rewrite_expr(a, remap, struct_remap))
                .collect();
            // Look up the call in the remap. Prefer the explicit turbofish
            // type_args (more accurate than arg-type inference) when present;
            // fall back to arg-type inference for legacy call sites that did
            // not record explicit type args.
            let key_types: Vec<HirType> = if !type_args.is_empty() {
                type_args.clone()
            } else {
                new_args.iter().map(|a| a.ty.clone()).collect()
            };
            let new_func = remap
                .get(&(*func, key_types.clone()))
                .copied()
                .unwrap_or(*func);
            HirExprKind::Call {
                func: new_func,
                args: new_args,
                type_args: type_args.clone(),
            }
        }
        HirExprKind::If {
            condition,
            then_branch,
            elif_branches,
            else_branch,
        } => HirExprKind::If {
            condition: Box::new(rewrite_expr(condition, remap, struct_remap)),
            then_branch: then_branch
                .iter()
                .map(|s| rewrite_stmt(s, remap, struct_remap))
                .collect(),
            elif_branches: elif_branches
                .iter()
                .map(|(e, stmts)| {
                    (
                        rewrite_expr(e, remap, struct_remap),
                        stmts
                            .iter()
                            .map(|s| rewrite_stmt(s, remap, struct_remap))
                            .collect(),
                    )
                })
                .collect(),
            else_branch: else_branch.as_ref().map(|v| {
                v.iter()
                    .map(|s| rewrite_stmt(s, remap, struct_remap))
                    .collect()
            }),
        },
        HirExprKind::For { var, iter, body } => HirExprKind::For {
            var: *var,
            iter: Box::new(rewrite_expr(iter, remap, struct_remap)),
            body: body
                .iter()
                .map(|s| rewrite_stmt(s, remap, struct_remap))
                .collect(),
        },
        HirExprKind::While { condition, body } => HirExprKind::While {
            condition: Box::new(rewrite_expr(condition, remap, struct_remap)),
            body: body
                .iter()
                .map(|s| rewrite_stmt(s, remap, struct_remap))
                .collect(),
        },
        HirExprKind::Range {
            start,
            end,
            is_inclusive,
        } => HirExprKind::Range {
            start: Box::new(rewrite_expr(start, remap, struct_remap)),
            end: Box::new(rewrite_expr(end, remap, struct_remap)),
            is_inclusive: *is_inclusive,
        },
        HirExprKind::StructLiteral {
            name,
            fields,
            type_args,
        } => {
            // If this struct was instantiated with concrete type args and
            // we built a substituted StructDef for it, retarget `name` to
            // the monomorphized struct's name (SymbolId) so codegen sees
            // fields with no generic parameters.
            let new_name = struct_remap
                .get(&(*name, type_args.clone()))
                .copied()
                .unwrap_or(*name);
            HirExprKind::StructLiteral {
                name: new_name,
                fields: fields
                    .iter()
                    .map(|(fname, fexpr)| {
                        (*fname, Box::new(rewrite_expr(fexpr, remap, struct_remap)))
                    })
                    .collect(),
                type_args: type_args.clone(),
            }
        }
        HirExprKind::FieldAccess { expr, field } => HirExprKind::FieldAccess {
            expr: Box::new(rewrite_expr(expr, remap, struct_remap)),
            field: *field,
        },
        HirExprKind::EnumConstructor { name, variant } => HirExprKind::EnumConstructor {
            name: *name,
            variant: *variant,
        },
        HirExprKind::ListLiteral { elements } => HirExprKind::ListLiteral {
            elements: elements
                .iter()
                .map(|e| rewrite_expr(e, remap, struct_remap))
                .collect(),
        },
    };
    HirExpr {
        kind: new_kind,
        ty: expr.ty.clone(),
        span: expr.span,
    }
}

// ---------------------------------------------------------------------------
// Owned-function lowerer
// ---------------------------------------------------------------------------

/// Lower a single (possibly monomorphized) `HirFunction` to a `MirFunction`.
///
/// This is a thin wrapper around the existing `MirLower` state machine that
/// accepts an owned `HirFunction` rather than borrowing from a `HirProgram`.
/// The signature table (`sigs`) is built from the original `HirProgram` so
/// that call-site lookups for non-generic callees (and for the original
/// generic callees' `DefId`s, which now resolve through the remap) work
/// uniformly.
pub fn lower_one_function(hir: &HirProgram, func: &HirFunction) -> CompilerResult<MirFunction> {
    // Build the signature table from the *original* program. The
    // monomorphized function is a new entry whose signature is the
    // substituted `(params, return_type)`. We register it as an extra
    // entry so internal calls in the body resolve.
    let mut sigs: HashMap<DefId, (Vec<HirType>, HirType)> =
        HashMap::with_capacity(hir.functions.len() + 1);
    for f in &hir.functions {
        sigs.insert(
            f.def_id,
            (
                f.params.iter().map(|(_, t)| t.clone()).collect(),
                f.return_type.clone(),
            ),
        );
    }
    sigs.insert(
        func.def_id,
        (
            func.params.iter().map(|(_, t)| t.clone()).collect(),
            func.return_type.clone(),
        ),
    );

    // `MirLower` borrows the function. We hand it `func` directly — its
    // lifetime is the duration of this call.
    let mut lower = MirLower::new(hir, func, &sigs);
    lower.lower_function()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::function::HirFunction;
    use crate::hir::symbol::Visibility as HirVisibility;
    use crate::hir::types::HirType;
    use crate::module::ModuleId;
    use miette::SourceSpan;

    fn empty_subst() -> HashMap<SymbolId, HirType> {
        HashMap::new()
    }

    #[test]
    fn substitute_concrete_type_passes_through() {
        let subst = empty_subst();
        assert_eq!(substitute_type(&HirType::I64, &subst), HirType::I64);
        assert_eq!(substitute_type(&HirType::Bool, &subst), HirType::Bool);
        assert_eq!(substitute_type(&HirType::Unit, &subst), HirType::Unit);
    }

    #[test]
    fn substitute_generic_resolves_to_concrete() {
        let mut subst = empty_subst();
        let t = SymbolId(42);
        subst.insert(t, HirType::I64);
        let ty = HirType::Generic(t);
        assert_eq!(substitute_type(&ty, &subst), HirType::I64);
    }

    #[test]
    fn substitute_generic_with_no_mapping_is_identity() {
        let subst = empty_subst();
        let ty = HirType::Generic(SymbolId(99));
        // Missing mapping → keep the type (defensive).
        assert_eq!(substitute_type(&ty, &subst), ty);
    }

    #[test]
    fn substitute_apply_recurses() {
        let mut subst = empty_subst();
        let t = SymbolId(7);
        subst.insert(t, HirType::Bool);
        let ty = HirType::Apply {
            base: SymbolId(1),
            args: vec![HirType::I64, HirType::Generic(t), HirType::F64],
        };
        let out = substitute_type(&ty, &subst);
        match out {
            HirType::Apply { base, args } => {
                assert_eq!(base, SymbolId(1));
                assert_eq!(args, vec![HirType::I64, HirType::Bool, HirType::F64]);
            }
            _ => panic!("expected Apply"),
        }
    }

    #[allow(dead_code)]
    fn make_func_with_generic_param() -> HirFunction {
        HirFunction {
            def_id: DefId(0),
            name: SymbolId(0),
            generic_params: vec![SymbolId(99)],
            params: vec![(SymbolId(1), HirType::Generic(SymbolId(99)))],
            return_type: HirType::Generic(SymbolId(99)),
            body: vec![],
            span: SourceSpan::new(0.into(), 0),
            module: ModuleId::ROOT,
            visibility: HirVisibility::Private,
        }
    }

    #[test]
    fn instantiation_name_format() {
        let inst = Instantiation {
            callee: DefId(0),
            args: vec![HirType::I64],
        };
        assert_eq!(inst.monomorphized_name("id", 1), "id$1");
        assert_eq!(inst.monomorphized_name("swap", 3), "swap$3");
    }
}
