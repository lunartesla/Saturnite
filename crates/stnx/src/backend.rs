//! Backend selection and dependency analysis for the hybrid Saturnite compiler.
//!
//! The hybrid compiler supports two backends:
//!
//! - **LLVM-only**: The entire program is compiled through the MIR→LLVM pipeline
//!   and linked with `cc`/`link.exe`/`rust-lld`. Rust crate dependencies are
//!   handled via pre-built static-library wrappers.
//!
//! - **Rust-aware (hybrid)**: Saturnite source compiles through the LLVM
//!   backend (HIR → MIR → object). Pre-built Rust wrapper static libraries
//!   (built during `saturn install`) are linked alongside the LLVM object
//!   to resolve `use rust:` calls. No Saturnite source is translated to Rust.
//!
//! ## Automatic selection rules
//!
//! When `backend = "auto"` (the default), the selector inspects the module graph
//! and ecosystem imports:
//!
//! 1. If the project has **no** `use rust:` imports that are actually *reachable*
//!    (i.e. referenced from the entry-point call graph), the LLVM-only backend
//!    is selected. Unused Rust dependencies are skipped.
//! 2. If the project has **any** `use rust:` imports that are reachable, the
//!    hybrid backend is selected.
//! 3. If the project has only `use python:` imports (no Rust), the LLVM-only
//!    backend is selected (Python interop works through the LLVM backend).
//! 4. If `backend` is set to `"llvm"` or `"hybrid"` explicitly, that backend is
//!    used regardless of dependencies.
//! 5. If `backend` is set to any other value, selection fails with a clear error.
//!
//! ## Reachability analysis
//!
//! A `use rust:foo` import is only considered "reachable" if at least one
//! qualified call (`foo.bar(...)`) appears in a function that is reachable from
//! the `main` entry point. This ensures that adding a Rust dependency to the
//! manifest but never importing it does not force the hybrid backend.

use crate::ast::ExternalKind;
use crate::error::CompilerResult;
use crate::hir::function::HirFunction;
use crate::hir::function::HirProgram;
use crate::hir::symbol::DefId;
use crate::hir::{HirExpr, HirExprKind, HirStmt, HirStmtKind};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

/// The backend mode requested by the user or default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendMode {
    /// Automatic selection based on dependency analysis.
    Auto,
    /// Force the LLVM-only backend.
    Llvm,
    /// Force the Rust-aware hybrid backend.
    Hybrid,
}

impl Default for BackendMode {
    fn default() -> Self {
        BackendMode::Auto
    }
}

impl std::str::FromStr for BackendMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "auto" => Ok(BackendMode::Auto),
            "llvm" => Ok(BackendMode::Llvm),
            "hybrid" => Ok(BackendMode::Hybrid),
            other => Err(format!(
                "unknown backend mode '{}'. Valid values: \"auto\", \"llvm\", \"hybrid\"",
                other
            )),
        }
    }
}

/// The backend that was selected after analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectedBackend {
    /// LLVM-only: all Saturnite source → LLVM IR → object → executable.
    /// Rust crates are reached only through pre-built static-library wrappers.
    Llvm,
    /// Rust-aware hybrid: some Saturnite source is translated to Rust source
    /// and compiled with rustc/cargo. Non-Rust code goes through LLVM.
    Hybrid,
}

/// The result of backend selection: which backend to use and why.
#[derive(Debug, Clone)]
pub struct BackendSelection {
    /// The selected backend.
    pub backend: SelectedBackend,
    /// Human-readable justification for the selection.
    pub reason: String,
}

/// Perform backend selection based on the manifest's `backend` field and the
/// HIR program's ecosystem imports.
///
/// # Arguments
/// * `backend_mode` - The requested backend mode (from `saturn.toml [build] backend`).
/// * `program` - The lowered HIR program (after semantic analysis).
///
/// # Selection logic
/// - `BackendMode::Llvm` → always LLVM-only.
/// - `BackendMode::Hybrid` → always hybrid.
/// - `BackendMode::Auto` → inspect reachable Rust imports:
///   - None → LLVM-only.
///   - Some → hybrid.
pub fn select_backend(
    backend_mode: BackendMode,
    program: &HirProgram,
) -> CompilerResult<BackendSelection> {
    let has_reachable_rust = !find_reachable_rust_imports(program).is_empty();

    let backend = match backend_mode {
        BackendMode::Llvm => SelectedBackend::Llvm,
        BackendMode::Hybrid => SelectedBackend::Hybrid,
        BackendMode::Auto => {
            if has_reachable_rust {
                SelectedBackend::Hybrid
            } else {
                SelectedBackend::Llvm
            }
        }
    };

    let reason = match (backend_mode, backend) {
        (BackendMode::Llvm, _) => "backend explicitly set to \"llvm\"".to_string(),
        (BackendMode::Hybrid, _) => "backend explicitly set to \"hybrid\"".to_string(),
        (BackendMode::Auto, SelectedBackend::Llvm) => {
            "no reachable Rust imports; using LLVM-only backend".to_string()
        }
        (BackendMode::Auto, SelectedBackend::Hybrid) => {
            "reachable Rust imports detected; using hybrid backend".to_string()
        }
    };

    Ok(BackendSelection { backend, reason })
}

/// Collect all `DefId`s called transitively from `main`, using the HIR
/// function bodies. Returns the set of reachable `DefId`s.
fn reachable_from_main(program: &HirProgram) -> HashSet<DefId> {
    // Find main's DefId
    let main_id = program
        .functions
        .iter()
        .find(|f| program.symbols.lookup(f.name) == Some("main"))
        .map(|f| f.def_id);

    let Some(main_id) = main_id else {
        return HashSet::new();
    };

    // Build a map from DefId to function body for quick lookup.
    let func_map: HashMap<DefId, &HirFunction> = program
        .functions
        .iter()
        .map(|f| (f.def_id, f))
        .collect();

    let mut visited: HashSet<DefId> = HashSet::new();
    let mut worklist: VecDeque<DefId> = VecDeque::new();
    worklist.push_back(main_id);

    while let Some(current_id) = worklist.pop_front() {
        if !visited.insert(current_id) {
            continue;
        }

        if let Some(func) = func_map.get(&current_id) {
            for (callee_id, _is_external) in collect_call_targets(&func.body) {
                if !visited.contains(&callee_id) {
                    worklist.push_back(callee_id);
                }
            }
        }
    }

    visited
}

/// Collect all `DefId`s called by a list of HIR statements (the function body).
fn collect_call_targets(body: &[HirStmt]) -> Vec<(DefId, bool)> {
    let mut targets = Vec::new();
    for stmt in body {
        collect_call_targets_stmt(stmt, &mut targets);
    }
    targets
}

/// Walk a HIR statement and collect call targets.
fn collect_call_targets_stmt(
    stmt: &HirStmt,
    targets: &mut Vec<(DefId, bool)>,
) {
    match &stmt.kind {
        HirStmtKind::Expr(expr)
        | HirStmtKind::Let { value: expr, .. }
        | HirStmtKind::Println(expr)
        | HirStmtKind::PrintlnStr(expr)
        | HirStmtKind::Raise(expr) => {
            collect_call_targets_expr(expr, targets);
        }
        HirStmtKind::Return(Some(expr)) => {
            collect_call_targets_expr(expr, targets);
        }
        _ => {}
    }
}

/// Recursively walk an HIR expression and collect call targets (DefIds).
fn collect_call_targets_expr(expr: &HirExpr, targets: &mut Vec<(DefId, bool)>) {
    match &expr.kind {
        HirExprKind::Call { func, args, .. } => {
            targets.push((*func, false));
            for arg in args {
                collect_call_targets_expr(arg, targets);
            }
        }
        HirExprKind::If {
            condition,
            then_branch,
            elif_branches,
            else_branch,
        } => {
            collect_call_targets_expr(condition, targets);
            for stmt in then_branch {
                collect_call_targets_stmt(stmt, targets);
            }
            for (cond, body) in elif_branches {
                collect_call_targets_expr(cond, targets);
                for stmt in body {
                    collect_call_targets_stmt(stmt, targets);
                }
            }
            if let Some(eb) = else_branch {
                for stmt in eb {
                    collect_call_targets_stmt(stmt, targets);
                }
            }
        }
        HirExprKind::For { iter, body, .. } => {
            collect_call_targets_expr(iter, targets);
            for stmt in body {
                collect_call_targets_stmt(stmt, targets);
            }
        }
        HirExprKind::While { condition, body } => {
            collect_call_targets_expr(condition, targets);
            for stmt in body {
                collect_call_targets_stmt(stmt, targets);
            }
        }
        HirExprKind::Binary { lhs, rhs, .. } => {
            collect_call_targets_expr(lhs, targets);
            collect_call_targets_expr(rhs, targets);
        }
        HirExprKind::Unary { expr, .. } => {
            collect_call_targets_expr(expr, targets);
        }
        HirExprKind::Assign { value, .. } | HirExprKind::AugAssign { value, .. } => {
            collect_call_targets_expr(value, targets);
        }
        HirExprKind::FieldAccess { expr, .. } | HirExprKind::Index { list: expr, .. } => {
            collect_call_targets_expr(expr, targets);
        }
        HirExprKind::StructLiteral { fields, .. } => {
            for (_, val) in fields {
                collect_call_targets_expr(val, targets);
            }
        }
        HirExprKind::Length { expr } => {
            collect_call_targets_expr(expr, targets);
        }
        HirExprKind::Range {
            start,
            end,
            is_inclusive: _,
        } => {
            collect_call_targets_expr(start, targets);
            collect_call_targets_expr(end, targets);
        }
        // Leaf expressions with no sub-expressions
        HirExprKind::Integer(_)
        | HirExprKind::Float(_)
        | HirExprKind::Bool(_)
        | HirExprKind::StrLit(_)
        | HirExprKind::Unit
        | HirExprKind::Variable { .. }
        | HirExprKind::EnumConstructor { .. } => {}
        HirExprKind::ListLiteral { elements } => {
            for elem in elements {
                collect_call_targets_expr(elem, targets);
            }
        }
    }
}

/// Analyze the HIR program to determine which `use rust:` imports have at
/// least one reachable qualified call.
///
/// Returns a list of ecosystem crate names (the package name from the import
/// path) that are both imported and reachable from `main`.
pub fn find_reachable_rust_imports(program: &HirProgram) -> Vec<String> {
    let reachable = reachable_from_main(program);

    // For each `use rust:<crate>` import, find the alias symbol and check
    // if any reachable external function's symbol starts with that alias.
    let mut reachable_rust: Vec<String> = Vec::new();

    for ud in &program.use_decls {
        if ud.ecosystem != Some(ExternalKind::Rust) {
            continue;
        }

        let pkg_str = ud
            .path
            .iter()
            .filter_map(|s| program.symbols.lookup(*s))
            .next()
            .unwrap_or("");

        let alias_str = program.symbols.lookup(ud.alias).unwrap_or("");

        // Check if any external function in this crate's namespace is reachable.
        let is_reachable = program.external_functions.iter().any(|ef| {
            // The external function's ecosystem field should match the crate name.
            let matches_crate = ef.ecosystem == pkg_str || ef.ecosystem == alias_str;
            matches_crate && reachable.contains(&ef.def_id)
        });

        if is_reachable {
            if !reachable_rust.contains(&pkg_str.to_string()) {
                reachable_rust.push(pkg_str.to_string());
            }
        }
    }

    reachable_rust
}

/// Analyze the manifest dependencies to determine which Rust crates are
/// declared but unused (skipped by `saturn install`).
///
/// Returns `(used_rust_deps, unused_rust_deps)` where `used_rust_deps` are
/// the Rust crate names that should be installed/prepared, and
/// `unused_rust_deps` are Rust crates declared in the manifest but not
/// imported anywhere in the source.
pub fn analyze_dependencies(
    manifest_deps: &std::collections::BTreeMap<String, crate::config::DependencySpec>,
    program: &HirProgram,
) -> (Vec<String>, Vec<String>) {
    let mut used = Vec::new();
    let mut unused = Vec::new();

    for (dep_name, spec) in manifest_deps {
        if spec.kind() == crate::interop::DependencyKind::Rust {
            // Check if this Rust crate is imported via `use rust:<name>`
            let imported = program.use_decls.iter().any(|ud| {
                if ud.ecosystem == Some(ExternalKind::Rust) {
                    let pkg = ud
                        .path
                        .iter()
                        .filter_map(|s| program.symbols.lookup(*s))
                        .next()
                        .unwrap_or("");
                    pkg == dep_name
                } else {
                    false
                }
            });

            if imported {
                used.push(dep_name.clone());
            } else {
                unused.push(dep_name.clone());
            }
        }
    }

    (used, unused)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_backend_mode_from_str() {
        assert_eq!(BackendMode::from_str("auto").unwrap(), BackendMode::Auto);
        assert_eq!(BackendMode::from_str("llvm").unwrap(), BackendMode::Llvm);
        assert_eq!(BackendMode::from_str("hybrid").unwrap(), BackendMode::Hybrid);
        assert!(BackendMode::from_str("invalid").is_err());
        assert_eq!(BackendMode::from_str("AUTO").unwrap(), BackendMode::Auto);
    }

    #[test]
    fn test_backend_mode_default() {
        assert_eq!(BackendMode::default(), BackendMode::Auto);
    }

    #[test]
    fn test_backend_mode_ser_de() {
        let json = serde_json::to_string(&BackendMode::Auto).unwrap();
        assert_eq!(json, "\"auto\"");
        let json = serde_json::to_string(&BackendMode::Llvm).unwrap();
        assert_eq!(json, "\"llvm\"");
        let json = serde_json::to_string(&BackendMode::Hybrid).unwrap();
        assert_eq!(json, "\"hybrid\"");
    }

    #[test]
    fn test_selected_backend_values() {
        assert_ne!(SelectedBackend::Llvm, SelectedBackend::Hybrid);
    }
}
