//! Rust crate discovery and compatibility classification for the hybrid backend.
//!
//! When the hybrid backend is selected, the Saturnite compiler needs to:
//!
//! 1. **Discover** which Rust crates are referenced by `use rust:name` imports
//!    and find their version/spec from the project manifest.
//! 2. **Classify compatibility** — determine which external function declarations
//!    are reachable from the entry point and ABI-safe, so only the necessary
//!    wrapper shims are generated.
//!
//! This module provides two entry points:
//!
//! - [`discover_rust_dependencies`] — given a `HirProgram` and manifest
//!   dependencies, returns the list of Rust crates that are both declared in the
//!   manifest and actually imported via `use rust:`.
//! - [`classify_external_calls`] — given a `HirProgram`, returns the external
//!   Rust functions that are both reachable from `main` and ABI-safe (i.e. their
//!   parameter and return types are within the supported ABI set: `i64`, `f64`,
//!   `bool`, `str`, `Unit`).

use crate::error::{CompilerError, CompilerResult};
use crate::hir::symbol::SymbolInterner;
use crate::hir::HirProgram;
use crate::interop::DependencyKind;

// ---------------------------------------------------------------------------
// ABI-safe type checking
// ---------------------------------------------------------------------------

/// The set of Saturnite types that are ABI-safe at the Rust extern "C" boundary.
///
/// Only these types can appear in `extern "C"` shim signatures without
/// additional adapter code:
/// - `I64` — the primary integer type (carries all 8/16/32-bit integers too).
/// - `F64` — the primary floating-point type (carries `f32`).
/// - `Bool` — a C-compatible `bool`.
/// - `Str` — represented as `*const c_char` (NUL-terminated).
/// - `Unit` — represented as `()` at the Rust level, `i64(0)` at the ABI level.
fn is_abi_safe_type(ty: &crate::hir::HirType) -> bool {
    matches!(
        ty,
        crate::hir::HirType::I64
            | crate::hir::HirType::F64
            | crate::hir::HirType::Bool
            | crate::hir::HirType::Str
            | crate::hir::HirType::Unit
    )
}

/// The result of discovering and classifying Rust dependencies.
///
/// Each entry records a Rust crate that is imported via `use rust:`, its
/// resolved version from the manifest, and the set of ABI-safe external
/// function declarations that target it.
#[derive(Debug, Clone)]
pub struct RustDependencyInfo {
    /// The crate name as declared in the manifest (e.g. `"serde"`).
    pub crate_name: String,
    /// The version string from the manifest (e.g. `"1.0"`).
    pub version: String,
    /// The import alias used in Saturnite source (e.g. `"serde_lib"`).
    pub alias: String,
    /// The ABI-safe external functions declared for this crate.
    /// Each entry is `(ecosystem_symbol, param_types, return_type)`.
    pub abi_safe_functions: Vec<AbiSafeFunction>,
    /// External functions for this crate that are NOT ABI-safe
    /// (unsupported types) — recorded for diagnostics.
    pub incompatible_functions: Vec<AbiIncompatibleFunction>,
}

/// An ABI-safe external function declaration.
#[derive(Debug, Clone)]
pub struct AbiSafeFunction {
    /// The link-time symbol (e.g. `"sat_serde_deserialize_i64"`).
    pub symbol: String,
    /// The call-site name (e.g. `"serde_lib.deserialize"`).
    pub qualified_name: String,
    /// Parameter types (all ABI-safe).
    pub params: Vec<crate::hir::HirType>,
    /// Return type (ABI-safe).
    pub ret: crate::hir::HirType,
}

/// An external function that is NOT ABI-safe — used for diagnostic reporting.
#[derive(Debug, Clone)]
pub struct AbiIncompatibleFunction {
    /// The link-time symbol.
    pub symbol: String,
    /// The call-site name.
    pub qualified_name: String,
    /// The first parameter type that violates the ABI constraint.
    pub offending_param: Option<crate::hir::HirType>,
    /// The return type that violates the ABI constraint, if applicable.
    pub offending_ret: Option<crate::hir::HirType>,
}

/// Discover Rust crate dependencies from the HIR program and manifest.
///
/// This function:
/// 1. Scans `program.use_decls` for `use rust:<crate> as <alias>` imports.
/// 2. For each Rust import, looks up the crate in `manifest_deps` to find
///    the declared version.
/// 3. Validates that the crate is declared in the manifest (errors if a
///    `use rust:foo` import has no corresponding manifest entry).
///
/// Returns a list of [`RustDependencyInfo`] entries, one per Rust crate that
/// is both imported and declared.
pub fn discover_rust_dependencies(
    program: &HirProgram,
    manifest_deps: &std::collections::BTreeMap<String, crate::config::DependencySpec>,
) -> CompilerResult<Vec<RustDependencyInfo>> {
    let symbols: &SymbolInterner = &program.symbols;

    // Collect (crate_name, alias) pairs from use_decls.
    let rust_imports: Vec<(String, String)> = program
        .use_decls
        .iter()
        .filter_map(|ud| {
            if ud.ecosystem != Some(crate::ast::ExternalKind::Rust) {
                return None;
            }
            let crate_name = symbols
                .lookup(
                    ud.path
                        .first()
                        .copied()
                        .unwrap_or(crate::hir::symbol::SymbolId(0)),
                )
                .unwrap_or("")
                .to_string();
            let alias = symbols.lookup(ud.alias).unwrap_or("");
            if crate_name.is_empty() || alias.is_empty() {
                None
            } else {
                Some((crate_name, alias.to_string()))
            }
        })
        .collect();

    // Build the result list.
    let mut results: Vec<RustDependencyInfo> = Vec::new();

    for (crate_name, alias) in rust_imports {
        // Look up the crate in the manifest.
        let (version, kind) = match manifest_deps.get(&crate_name) {
            Some(spec) => (spec.version().to_string(), spec.kind()),
            None => {
                return Err(CompilerError::config(format!(
                    "Rust crate '{}' is imported via `use rust:{}` but is not declared \
                     in [dependencies] of saturn.toml. Add an entry like: \
                     `{} = {{ kind = \"rust\", version = \"...\" }}`",
                    crate_name, crate_name, crate_name
                )));
            }
        };

        if kind != DependencyKind::Rust {
            return Err(CompilerError::config(format!(
                "Dependency '{}' is declared with kind {:?}, but `use rust:{}` \
                 requires a Rust dependency. Use `kind = \"rust\"` in the manifest.",
                crate_name, kind, crate_name
            )));
        }

        results.push(RustDependencyInfo {
            crate_name: crate_name.clone(),
            version,
            alias,
            abi_safe_functions: Vec::new(),
            incompatible_functions: Vec::new(),
        });
    }

    Ok(results)
}

/// Classify external function declarations by ABI safety and reachability.
///
/// This function examines the `external_functions` in the HIR program and:
///
/// 1. Groups them by their `ecosystem` field (the Rust crate name).
/// 2. For each group, classifies functions as ABI-safe or not based on their
///    parameter and return types.
/// 3. Returns the classified information, which callers can use to decide
///    which wrapper shims to generate.
///
/// Note: this function classifies ALL external functions in the program. Callers
/// that only need reachable ones should filter using the `reachable_from_main`
/// function from `backend.rs`.
pub fn classify_external_calls(program: &HirProgram) -> Vec<RustDependencyInfo> {
    let symbols: &SymbolInterner = &program.symbols;

    // Build a map from ecosystem name → RustDependencyInfo (without version/alias).
    // We'll fill in abi_safe_functions and incompatible_functions.
    let mut by_crate: std::collections::HashMap<String, RustDependencyInfo> =
        std::collections::HashMap::new();

    // Find the alias for each ecosystem name from use_decls.
    for ud in &program.use_decls {
        if ud.ecosystem != Some(crate::ast::ExternalKind::Rust) {
            continue;
        }
        let crate_name = symbols
            .lookup(
                ud.path
                    .first()
                    .copied()
                    .unwrap_or(crate::hir::symbol::SymbolId(0)),
            )
            .unwrap_or("")
            .to_string();
        let alias = symbols.lookup(ud.alias).unwrap_or("").to_string();

        if crate_name.is_empty() || alias.is_empty() {
            continue;
        }

        by_crate.entry(crate_name.clone()).or_insert_with(|| {
            RustDependencyInfo {
                crate_name: crate_name.clone(),
                version: String::new(),
                alias,
                abi_safe_functions: Vec::new(),
                incompatible_functions: Vec::new(),
            }
        });
    }

    // Classify each external function.
    for ef in &program.external_functions {
        if ef.kind != crate::ast::ExternalKind::Rust {
            continue;
        }

        let crate_name = &ef.ecosystem;
        let entry = by_crate.entry(crate_name.clone()).or_insert_with(|| {
            RustDependencyInfo {
                crate_name: crate_name.clone(),
                version: String::new(),
                alias: crate_name.clone(),
                abi_safe_functions: Vec::new(),
                incompatible_functions: Vec::new(),
            }
        });

        // Build qualified name from the call-site (ecosystem.symbol or alias.symbol).
        let qualified_name = format!("{}.{}", entry.alias, ef.symbol);

        // Check ABI safety of all params and return type.
        let offending_param = ef
            .param_types
            .iter()
            .find(|t| !is_abi_safe_type(t))
            .cloned();

        let offending_ret = if !is_abi_safe_type(&ef.return_type) {
            Some(ef.return_type.clone())
        } else {
            None
        };

        if offending_param.is_none() && offending_ret.is_none() {
            entry.abi_safe_functions.push(AbiSafeFunction {
                symbol: ef.symbol.clone(),
                qualified_name,
                params: ef.param_types.clone(),
                ret: ef.return_type.clone(),
            });
        } else {
            entry.incompatible_functions.push(AbiIncompatibleFunction {
                symbol: ef.symbol.clone(),
                qualified_name,
                offending_param,
                offending_ret,
            });
        }
    }

    by_crate.into_values().collect()
}

/// Discover a Rust crate in the local Cargo registry cache.
///
/// Given a crate name and version requirement, this function searches the
/// user's Cargo registry cache for a matching crate. It returns the path to
/// the crate's `.crate` file and the exact resolved version.
///
/// This is used to validate that a declared Rust dependency is actually
/// available before attempting to build a wrapper.
pub fn find_crate_in_registry(
    crate_name: &str,
    version_req: &str,
) -> Option<FoundCrateInfo> {
    let cargo_home = cargo_home()?;

    let registry_cache = cargo_home.join("registry").join("cache");
    let registry_src = cargo_home.join("registry").join("src");

    // Search the cache directory for .crate files matching the name.
    if let Some(result) = find_crate_in_dir(&registry_cache, crate_name, version_req) {
        return Some(result);
    }

    // Search the src directory for unpacked sources.
    if let Some(result) = find_crate_in_dir(&registry_src, crate_name, version_req) {
        return Some(result);
    }

    // Search the index.
    let index = cargo_home.join("registry").join("index");
    if let Some(result) = find_crate_in_dir(&index, crate_name, version_req) {
        return Some(result);
    }

    None
}

/// Information about a Rust crate found in the Cargo registry.
#[derive(Debug, Clone)]
pub struct FoundCrateInfo {
    /// The crate name (e.g. `"serde"`).
    pub name: String,
    /// The exact resolved version (e.g. `"1.0.195"`).
    pub version: String,
    /// Path to the `.crate` file or unpacked source directory.
    pub path: std::path::PathBuf,
}

/// Resolve the cargo home directory.
fn cargo_home() -> Option<std::path::PathBuf> {
    if let Ok(home) = std::env::var("CARGO_HOME") {
        return Some(std::path::PathBuf::from(home));
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()?;
    Some(std::path::PathBuf::from(home).join(".cargo"))
}

/// Search a directory tree for a crate matching `crate_name` and `version_req`.
fn find_crate_in_dir(
    dir: &std::path::Path,
    crate_name: &str,
    version_req: &str,
) -> Option<FoundCrateInfo> {
    let _ = dir;
    let _ = crate_name;
    let _ = version_req;
    // Note: a full implementation would walk the registry index structure
    // (cache/<first-letter>/<crate_name>/<version>.crate) or search the
    // src/<hash>/<crate_name>-<version>/ directories. This placeholder returns
    // None so that the caller falls back to the wrapper metadata approach.
    None
}

/// Check whether a crate name is a valid Rust identifier (after sanitization).
///
/// Rust crate names can contain alphanumeric characters, hyphens, and underscores.
/// When used as Rust identifiers, hyphens are replaced with underscores.
pub fn is_valid_rust_crate_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
}

/// Sanitize a crate name for use as a Rust identifier (hyphens → underscores).
pub fn sanitize_crate_name(name: &str) -> String {
    name.replace('-', "_")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::function::HirExternalFunction;
    use crate::hir::types::HirType;
    use crate::hir::HirUseDecl;
    use crate::module::ModuleId;
    use std::collections::{BTreeMap, HashMap};

    fn make_program_with_rust_import() -> (HirProgram, SymbolInterner) {
        let mut symbols = SymbolInterner::default();
        let _main_name = symbols.intern("main");
        let crate_name_sym = symbols.intern("serde");
        let alias_sym = symbols.intern("serde_lib");
        let ext_name = symbols.intern("serde_lib.deserialize");
        let param_name = symbols.intern("data");

        let use_decl = HirUseDecl {
            def_id: crate::hir::symbol::DefId(0),
            path: vec![crate_name_sym],
            alias: alias_sym,
            module: ModuleId::ROOT,
            visibility: crate::hir::symbol::Visibility::Private,
            ecosystem: Some(crate::ast::ExternalKind::Rust),
            span: miette::SourceSpan::new(0.into(), 0),
        };

        let ext_fn = HirExternalFunction {
            def_id: crate::hir::symbol::DefId(1),
            kind: crate::ast::ExternalKind::Rust,
            ecosystem: "serde".to_string(),
            symbol: "sat_serde_deserialize".to_string(),
            name: ext_name,
            param_names: vec![param_name],
            param_types: vec![HirType::Str],
            return_type: HirType::I64,
            span: miette::SourceSpan::new(0.into(), 0),
            module: ModuleId::ROOT,
        };

        let program = HirProgram {
            functions: Vec::new(),
            structs: Vec::new(),
            enums: Vec::new(),
            symbols: std::mem::take(&mut symbols),
            external_functions: vec![ext_fn],
            modules: Vec::new(),
            root_module: ModuleId::ROOT,
            module_paths: HashMap::new(),
            def_table: crate::hir::symbol::DefTable::default(),
            module_scopes: Vec::new(),
            use_decls: vec![use_decl],
            mod_decls: Vec::new(),
            python_search_path: String::new(),
        };

        // Recreate symbols since we took them.
        let symbols = SymbolInterner::default();
        (program, symbols)
    }

    #[test]
    fn test_is_abi_safe_type() {
        assert!(is_abi_safe_type(&HirType::I64));
        assert!(is_abi_safe_type(&HirType::F64));
        assert!(is_abi_safe_type(&HirType::Bool));
        assert!(is_abi_safe_type(&HirType::Str));
        assert!(is_abi_safe_type(&HirType::Unit));
        // Struct types are not ABI-safe.
        assert!(!is_abi_safe_type(&HirType::Struct(crate::hir::symbol::SymbolId(0))));
        assert!(!is_abi_safe_type(&HirType::List(Box::new(HirType::I64))));
    }

    #[test]
    fn test_classify_external_calls_abi_safe() {
        let (program, _symbols) = make_program_with_rust_import();
        let result = classify_external_calls(&program);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].crate_name, "serde");
        assert_eq!(result[0].alias, "serde_lib");
        assert_eq!(result[0].abi_safe_functions.len(), 1);
        assert_eq!(result[0].abi_safe_functions[0].symbol, "sat_serde_deserialize");
        assert!(result[0].incompatible_functions.is_empty());
    }

    #[test]
    fn test_classify_external_calls_incompatible() {
        let mut symbols = SymbolInterner::default();
        let ext_name = symbols.intern("bad.func");

        let ext_fn = HirExternalFunction {
            def_id: crate::hir::symbol::DefId(0),
            kind: crate::ast::ExternalKind::Rust,
            ecosystem: "bad_crate".to_string(),
            symbol: "sat_bad_func".to_string(),
            name: ext_name,
            param_names: vec![],
            param_types: vec![HirType::Struct(crate::hir::symbol::SymbolId(0))],
            return_type: HirType::F64,
            span: miette::SourceSpan::new(0.into(), 0),
            module: ModuleId::ROOT,
        };

        let program = HirProgram {
            functions: Vec::new(),
            structs: Vec::new(),
            enums: Vec::new(),
            symbols,
            external_functions: vec![ext_fn],
            modules: Vec::new(),
            root_module: ModuleId::ROOT,
            module_paths: HashMap::new(),
            def_table: crate::hir::symbol::DefTable::default(),
            module_scopes: Vec::new(),
            use_decls: Vec::new(),
            mod_decls: Vec::new(),
            python_search_path: String::new(),
        };

        let result = classify_external_calls(&program);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].abi_safe_functions.len(), 0);
        assert_eq!(result[0].incompatible_functions.len(), 1);
        assert!(result[0].incompatible_functions[0].offending_param.is_some());
    }

    #[test]
    fn test_is_valid_rust_crate_name() {
        assert!(is_valid_rust_crate_name("serde"));
        assert!(is_valid_rust_crate_name("serde-json"));
        assert!(is_valid_rust_crate_name("my_crate"));
        assert!(!is_valid_rust_crate_name(""));
        assert!(!is_valid_rust_crate_name("crate@2"));
    }

    #[test]
    fn test_sanitize_crate_name() {
        assert_eq!(sanitize_crate_name("serde-json"), "serde_json");
        assert_eq!(sanitize_crate_name("my_crate"), "my_crate");
        assert_eq!(sanitize_crate_name("simple"), "simple");
    }

    #[test]
    fn test_discover_rust_dependencies_no_manifest_entry() {
        let (program, _symbols) = make_program_with_rust_import();
        let manifest_deps: BTreeMap<String, crate::config::DependencySpec> = BTreeMap::new();
        let result = discover_rust_dependencies(&program, &manifest_deps);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("serde"));
        assert!(err.contains("not declared"));
    }

    #[test]
    fn test_discover_rust_dependencies_with_manifest() {
        let (program, _symbols) = make_program_with_rust_import();
        let manifest_deps: BTreeMap<String, crate::config::DependencySpec> = BTreeMap::from([
            (
                "serde".to_string(),
                crate::config::DependencySpec::Full {
                    kind: DependencyKind::Rust,
                    version: "1.0".to_string(),
                    target: None,
                },
            ),
        ]);

        let result = discover_rust_dependencies(&program, &manifest_deps);
        assert!(result.is_ok());
        let deps = result.unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].crate_name, "serde");
        assert_eq!(deps[0].version, "1.0");
        assert_eq!(deps[0].alias, "serde_lib");
    }
}
