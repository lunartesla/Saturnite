//! Integration tests for HIR→MIR compatibility across modules (Phase 7B).
//!
//! These tests verify that the DefId-based signature lookup in
//! `mir::lower::lower_program` works correctly when functions are
//! defined in different modules. After Phase 7A, the `sigs` table is a
//! `HashMap<DefId, (Vec<HirType>, HirType)>` keyed by the function's actual
//! `DefId` (not a positional `Vec` index). These tests exercise the HashMap
//! lookup path — especially the case where a child module function's `DefId`
//! is not at the same array position that old Vec-indexing would have used.
//!
//! Three scenarios are covered:
//!
//! 1. **Intra-module function call (regression)** — single-file program with
//!    two root functions; `main` calls `helper`. Verifies the MIR contains a
//!    `Call` terminator with the correct `DefId` and return type.
//!
//! 2. **Cross-module call with DefId lookup** — root module calls a function
//!    defined in a child module. Verifies the `Call` terminator's `DefId`
//!    matches the child function's `DefId` and the HashMap resolved the return
//!    type correctly.
//!
//! 3. **Multi-function DefId ordering** — a program with multiple functions
//!    spread across root and child modules. Verifies the `sigs` HashMap
//!    correctly resolves each function's return type regardless of DefId
//!    ordering across module boundaries.

mod common;

use std::collections::HashMap;
use std::fs;
use stnx::hir::{HirProgram, HirType};
use stnx::mir::lower::lower_program;
use stnx::mir::opt::optimize;
use stnx::mir::{MirFunction, MirProgram, MirTerminator};
use stnx::module::{ModuleGraph, ModuleId};
use stnx::semantic::analyze_and_lower_with_graph;
use stnx::DefId;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Write a file at `dir/<rel>` with the given contents, creating parent dirs.
fn write_file(dir: &TempDir, rel: &str, contents: &str) {
    let path = dir.path().join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("failed to create parent dir");
    }
    fs::write(&path, contents)
        .unwrap_or_else(|e| panic!("failed to write {}: {}", path.display(), e));
}

/// Write a minimal `saturn.toml` for project discovery.
fn write_saturn_toml(dir: &TempDir, name: &str) {
    write_file(
        dir,
        "saturn.toml",
        &format!(
            r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2026"

[dependencies]
"#
        ),
    );
}

/// Build a full multi-module HIR from a temp directory layout.
///
/// Returns the HIR program, ready for MIR lowering.
fn build_hir(root_src: &str, child_src: &str, tmp: &TempDir) -> HirProgram {
    write_saturn_toml(tmp, "multimodule_test");
    write_file(tmp, "src/main.stnx", root_src);
    write_file(tmp, "src/child.stnx", child_src);

    let main_path = tmp.path().join("src").join("main.stnx");
    let graph = ModuleGraph::discover_modules(main_path.clone()).expect("module discovery failed");

    assert_eq!(graph.len(), 2, "graph should contain root + child modules");

    let root_ast = graph
        .root_module()
        .ast
        .as_ref()
        .expect("root module AST should be available");

    analyze_and_lower_with_graph(root_ast, &graph).expect("HIR lowering with graph should succeed")
}

/// Lower a HIR program to verified, optimized MIR.
fn hir_to_mir(hir: &HirProgram) -> MirProgram {
    let mut mir = lower_program(hir).expect("MIR lowering failed");
    if let Err(errs) = mir.verify() {
        let msgs: Vec<String> = errs.iter().map(|e| e.to_string()).collect();
        panic!("MIR verification failed: {}", msgs.join(", "));
    }
    optimize(&mut mir);
    mir
}

/// Find a MIR function by name.
fn mir_function<'a>(prog: &'a MirProgram, name: &str) -> &'a MirFunction {
    prog.functions
        .iter()
        .find(|f| prog.symbols.lookup(f.name) == Some(name))
        .unwrap_or_else(|| panic!("MIR function '{name}' not found in program"))
}

/// Find a HIR function by name string.
fn hir_function<'a>(hir: &'a HirProgram, name: &str) -> &'a stnx::hir::HirFunction {
    hir.functions
        .iter()
        .find(|f| hir.symbol_name(f.name) == Some(name))
        .unwrap_or_else(|| panic!("HIR function '{name}' not found in HIR program"))
}

/// Collect all `Call` terminators across all blocks of a MIR function,
/// returning `(DefId, LocalId)` for each (the callee and the destination local).
fn collect_calls(func: &MirFunction) -> Vec<(DefId, stnx::mir::LocalId)> {
    func.blocks
        .iter()
        .filter_map(|b| match &b.terminator {
            MirTerminator::Call {
                func, destination, ..
            } => Some((*func, *destination)),
            _ => None,
        })
        .collect()
}

/// Find the type of a local by `LocalId` in a MIR function.
fn local_type(func: &MirFunction, local_id: stnx::mir::LocalId) -> Option<HirType> {
    func.locals.iter().find(|l| l.id == local_id).map(|l| l.ty)
}

/// Find a module by name in the HIR's module list.
fn find_module_by_name(hir: &HirProgram, name: &str) -> ModuleId {
    hir.modules
        .iter()
        .find(|m| m.path.name(&hir.symbols) == Some(name))
        .unwrap_or_else(|| panic!("module '{name}' not found in HIR"))
        .id
}

// ---------------------------------------------------------------------------
// Test 1: Intra-module function call (regression)
// ---------------------------------------------------------------------------

/// A single-file program where `main` calls `helper` in the root module.
/// This is a regression test: the old Vec-indexed `sigs` lookup
/// (`sigs.get(def_id.0 as usize)`) worked because root functions were
/// sequentially indexed. The HashMap-based lookup must still resolve the
/// correct return type and produce a `Call` terminator with the right `DefId`.
#[test]
fn test_intra_module_call_regression() {
    let src = "fn main() -> i64 { helper() }\nfn helper() -> i64 { 42 }\n";

    // Use the shared single-file pipeline (no module graph needed).
    let mir = common::to_mir(src);

    // Find helper's DefId — MirFunction.def_id carries the HIR DefId.
    let helper_fn = mir_function(&mir, "helper");
    let helper_def_id = helper_fn.def_id;

    let main_fn = mir_function(&mir, "main");
    let calls = collect_calls(main_fn);
    assert!(
        !calls.is_empty(),
        "main should contain at least one Call terminator"
    );

    // The Call's `func` DefId must match helper's DefId — this is the
    // HashMap key lookup path in lower_call.
    let found_call = calls.iter().any(|(def_id, _)| *def_id == helper_def_id);
    assert!(
        found_call,
        "main's Call terminator should reference helper's DefId ({:?}), \
         found calls: {:?}",
        helper_def_id,
        calls.iter().map(|(d, _)| d).collect::<Vec<_>>()
    );

    // The destination local should be typed i64 (helper returns i64).
    // This verifies the HashMap resolved the return type correctly.
    let dest_ty = calls
        .iter()
        .find(|(def_id, _)| *def_id == helper_def_id)
        .and_then(|(_, dest_local_id)| local_type(main_fn, *dest_local_id));
    assert_eq!(
        dest_ty,
        Some(HirType::I64),
        "Call destination local for helper() should be typed i64 (helper's return type)"
    );

    // IR-level checks: function name and call instruction present.
    let ir = common::ir_only(src);
    assert!(
        ir.contains("helper"),
        "generated IR should contain the helper function name, got: {}",
        ir
    );
    assert!(
        ir.contains("call i64 @helper"),
        "IR should contain a call to @helper returning i64, got: {}",
        ir
    );
}

// ---------------------------------------------------------------------------
// Test 2: Cross-module call with DefId lookup
// ---------------------------------------------------------------------------

/// A multi-module program where the root module calls a function defined in
/// a child module. This exercises the HashMap lookup path: the child
/// function's `DefId` is assigned after root functions, and the old
/// Vec-indexed approach (`sigs.get(def_id.0 as usize)`) could have resolved
/// the wrong signature if the DefId didn't match the array position.
#[test]
fn test_cross_module_call_defid_lookup() {
    let tmp = TempDir::new().expect("failed to create temp dir");

    // Root: declares `mod child`, imports `use child::helper`, calls helper from main.
    let root_src = "mod child\nuse child::helper\nfn main() -> i64 {\n    helper()\n}\n";

    // Child: defines `helper` returning i64.
    let child_src = "fn helper() -> i64 {\n    return 99\n}\n";

    let hir = build_hir(root_src, child_src, &tmp);

    // --- Locate the child function's DefId via the module scope ---
    let child_id = find_module_by_name(&hir, "child");
    let child_scope = hir
        .module_scope(child_id)
        .expect("child module scope must exist");

    // Find "helper" in the child module's item scope.
    let helper_name_sym = hir
        .functions
        .iter()
        .find(|f| hir.symbol_name(f.name) == Some("helper"))
        .map(|f| f.name)
        .expect("helper function should exist in HIR");

    let helper_def_id = child_scope
        .items
        .get(&helper_name_sym)
        .copied()
        .unwrap_or_else(|| {
            // Fallback: scan all functions for "helper" in the child module.
            hir.functions
                .iter()
                .find(|f| f.name == helper_name_sym && f.module == child_id)
                .map(|f| f.def_id)
                .expect("child module scope should contain 'helper'")
        });

    // Verify the DefId matches the HirFunction's def_id.
    let helper_hir = hir_function(&hir, "helper");
    assert_eq!(
        helper_def_id, helper_hir.def_id,
        "child scope's helper DefId should match HirFunction.def_id"
    );

    // --- Lower to MIR ---
    let mir = hir_to_mir(&hir);

    let main_mir = mir_function(&mir, "main");
    let mir_calls = collect_calls(main_mir);

    assert!(
        !mir_calls.is_empty(),
        "main's MIR should contain at least one Call terminator"
    );

    // The Call's `func` DefId must match helper's DefId — this proves the
    // HashMap lookup in lower_call resolved the child function correctly.
    let mir_call_matches = mir_calls.iter().any(|(def_id, _)| *def_id == helper_def_id);
    assert!(
        mir_call_matches,
        "MIR Call terminator should reference helper's DefId ({:?}), \
         found calls: {:?}",
        helper_def_id,
        mir_calls.iter().map(|(d, _)| d).collect::<Vec<_>>()
    );

    // The destination local should be typed i64 (helper returns i64).
    let dest_ty = mir_calls
        .iter()
        .find(|(def_id, _)| *def_id == helper_def_id)
        .and_then(|(_, dest_local_id)| local_type(main_mir, *dest_local_id));
    assert_eq!(
        dest_ty,
        Some(HirType::I64),
        "Cross-module call destination local should be typed i64 (helper's return type)"
    );

    // --- Verify: helper appears in the child module's MIR function ---
    let helper_mir = mir_function(&mir, "helper");
    assert_eq!(
        helper_mir.def_id, helper_def_id,
        "MIR helper function DefId should match HIR DefId"
    );

    // --- Verify: the function name appears in generated IR ---
    let ir = stnx::mir::codegen::generate_ir_from_mir(&mir).expect("IR generation failed");
    assert!(
        ir.contains("helper"),
        "generated IR should contain the helper function name, got: {}",
        ir
    );
    assert!(
        ir.contains("call i64 @helper"),
        "IR should contain a call to @helper returning i64, got: {}",
        ir
    );
}

// ---------------------------------------------------------------------------
// Test 3: Multi-function DefId ordering across modules
// ---------------------------------------------------------------------------

/// A program with multiple functions spread across root and child modules,
/// where the call graph crosses module boundaries. Each child function
/// returns a different type (i64, f64, bool). Verify the `sigs` HashMap
/// correctly resolves each function's return type through the Call
/// destination local — even though the child functions' DefIds are not
/// at sequential positions matching the root's.
#[test]
fn test_multi_function_defid_ordering_across_modules() {
    let tmp = TempDir::new().expect("failed to create temp dir");

    // Root module: main calls three child functions, each returning a different type.
    let root_src = "\
mod child
use child::value_i64
use child::value_f64
use child::value_bool
fn main() -> i64 {
    let a = value_i64()
    let b = value_f64()
    let c = value_bool()
    return a
}
";

    // Child module: three functions returning different types.
    let child_src = "\
fn value_i64() -> i64 { return 42 }
fn value_f64() -> f64 { return 3.14 }
fn value_bool() -> bool { return true }
";

    let hir = build_hir(root_src, child_src, &tmp);

    // --- Build a name → DefId map for all child functions ---
    let child_id = find_module_by_name(&hir, "child");
    let mut child_def_ids: HashMap<String, DefId> = HashMap::new();
    for func in &hir.functions {
        if func.module == child_id {
            if let Some(name) = hir.symbol_name(func.name) {
                child_def_ids.insert(name.to_string(), func.def_id);
            }
        }
    }

    for expected in &["value_i64", "value_f64", "value_bool"] {
        assert!(
            child_def_ids.contains_key(*expected),
            "child module should define function '{expected}'; \
             found child functions: {:?}",
            child_def_ids.keys().collect::<Vec<_>>()
        );
    }

    let value_i64_did = child_def_ids["value_i64"];
    let value_f64_did = child_def_ids["value_f64"];
    let value_bool_did = child_def_ids["value_bool"];

    // The child function DefIds must be distinct from each other.
    assert_ne!(
        value_i64_did, value_f64_did,
        "value_i64 and value_f64 must differ"
    );
    assert_ne!(
        value_i64_did, value_bool_did,
        "value_i64 and value_bool must differ"
    );
    assert_ne!(
        value_f64_did, value_bool_did,
        "value_f64 and value_bool must differ"
    );

    // The child function DefIds must differ from root main's DefId.
    let main_did = hir_function(&hir, "main").def_id;
    assert_ne!(
        value_i64_did, main_did,
        "child function DefId should differ from root main's DefId"
    );

    // --- Lower to MIR ---
    let mir = hir_to_mir(&hir);

    // --- Verify each Call's destination local has the correct return type ---
    let main_mir = mir_function(&mir, "main");
    let calls = collect_calls(main_mir);
    assert!(
        calls.len() >= 3,
        "main should have at least 3 Call terminators (one per cross-module call), got {}",
        calls.len()
    );

    // For each call, check the destination local type matches the callee's
    // return type via the sigs HashMap.
    let check = |calls: &[(DefId, stnx::mir::LocalId)],
                 target_def_id: DefId,
                 expected_ty: HirType,
                 label: &str| {
        let entry = calls.iter().find(|(d, _)| *d == target_def_id);
        assert!(
            entry.is_some(),
            "should find a Call to {} (DefId {:?}); found calls: {:?}",
            label,
            target_def_id,
            calls.iter().map(|(d, _)| d).collect::<Vec<_>>()
        );
        let (_, dest_local_id) = entry.unwrap();
        let actual_ty = local_type(main_mir, *dest_local_id);
        assert_eq!(
            actual_ty,
            Some(expected_ty),
            "Call to {} should produce a destination local typed {:?}, got {:?}",
            label,
            expected_ty,
            actual_ty
        );
    };

    check(&calls, value_i64_did, HirType::I64, "value_i64");
    check(&calls, value_f64_did, HirType::F64, "value_f64");
    check(&calls, value_bool_did, HirType::Bool, "value_bool");

    // --- Verify: IR generation produces correct return types for each call ---
    let ir = stnx::mir::codegen::generate_ir_from_mir(&mir).expect("IR generation failed");
    assert!(
        ir.contains("call i64 @value_i64"),
        "IR should contain call to @value_i64 returning i64, got: {}",
        ir
    );
    assert!(
        ir.contains("call double @value_f64"),
        "IR should contain call to @value_f64 returning double, got: {}",
        ir
    );
    assert!(
        ir.contains("call i1 @value_bool"),
        "IR should contain call to @value_bool returning i1, got: {}",
        ir
    );
}
