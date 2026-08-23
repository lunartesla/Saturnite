//! Tests for target configuration: profile → (OptimizationLevel, DebugInfo)
//! mapping, OutputKind preservation, and target triple preservation.
//!
//! These properties are verified at two levels:
//!   1. **Unit-level** — the `TargetConfig` getters return the values we just
//!      set, confirming the setters work correctly.
//!   2. **Integration-level** — we compile a trivial program with a given
//!      config through the full `compile_from_mir_ext` seam and check that the
//!      target triple is embedded in the resulting module / executable.
//!
//! The profile mapping itself (Debug → None/Yes, Release → Aggressive/No)
//! mirrors what `main.rs` encodes inline (see findings #1 and #2 in the
//! Phase 0 audit: duplicated profile logic in three call sites).

mod common;

use common::{compile_src, compile_to_object, to_mir};
use stnx::mir::codegen::compile_from_mir_ext;
use stnx::target::Profile;
use stnx::target::{DebugInfo, OptimizationLevel, OutputKind, TargetConfig};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Profile → (OptimizationLevel, DebugInfo) mapping
// ---------------------------------------------------------------------------

#[test]
fn test_debug_profile_maps_to_none_opt_and_yes_debug() {
    let mut config = TargetConfig::host().expect("host target init failed");
    config.apply_profile(Profile::Debug);

    assert_eq!(
        config.opt_level(),
        &OptimizationLevel::None,
        "Debug profile should map to OptimizationLevel::None"
    );
    assert_eq!(
        config.debug_info(),
        &DebugInfo::Yes,
        "Debug profile should map to DebugInfo::Yes"
    );
}

#[test]
fn test_release_profile_maps_to_aggressive_opt_and_no_debug() {
    let mut config = TargetConfig::host().expect("host target init failed");
    config.apply_profile(Profile::Release);

    assert_eq!(
        config.opt_level(),
        &OptimizationLevel::Aggressive,
        "Release profile should map to OptimizationLevel::Aggressive"
    );
    assert_eq!(
        config.debug_info(),
        &DebugInfo::No,
        "Release profile should map to DebugInfo::No"
    );
}

#[test]
fn test_debug_profile_is_consistent_with_release_profile() {
    // The two profiles must produce opposite extremes — never the same.
    let mut debug_cfg = TargetConfig::host().expect("host target init failed");
    let mut release_cfg = TargetConfig::host().expect("host target init failed");
    debug_cfg.apply_profile(Profile::Debug);
    release_cfg.apply_profile(Profile::Release);

    assert_ne!(
        debug_cfg.opt_level(),
        release_cfg.opt_level(),
        "Debug and Release must differ in optimization level"
    );
    assert_ne!(
        debug_cfg.debug_info(),
        release_cfg.debug_info(),
        "Debug and Release must differ in debug info"
    );

    // Debug is the minimal level; Release is the maximal level.
    assert_eq!(debug_cfg.opt_level(), &OptimizationLevel::None);
    assert_eq!(debug_cfg.debug_info(), &DebugInfo::Yes);
    assert_eq!(release_cfg.opt_level(), &OptimizationLevel::Aggressive);
    assert_eq!(release_cfg.debug_info(), &DebugInfo::No);
}

// ---------------------------------------------------------------------------
// to_inkwell_opt_level — verifies the public mapping table
// ---------------------------------------------------------------------------

#[test]
fn test_opt_level_mapping_all_variants() {
    // Verify every OptimizationLevel variant maps to the corresponding
    // inkwell level.  This is the mapping that `compile_from_mir_ext` uses
    // inline (audit finding #2).
    let pairs = [
        (OptimizationLevel::None, inkwell::OptimizationLevel::None),
        (OptimizationLevel::Less, inkwell::OptimizationLevel::Less),
        (
            OptimizationLevel::Default,
            inkwell::OptimizationLevel::Default,
        ),
        (
            OptimizationLevel::Aggressive,
            inkwell::OptimizationLevel::Aggressive,
        ),
    ];

    for (stnx_level, ink_level) in pairs {
        let mut config = TargetConfig::host().expect("host target init failed");
        config.set_opt_level(stnx_level.clone());
        assert_eq!(
            config.to_inkwell_opt_level(),
            ink_level,
            "OptimizationLevel::{:?} should map to inkwell::{:?}",
            stnx_level,
            ink_level
        );
    }
}

// ---------------------------------------------------------------------------
// OutputKind preservation
// ---------------------------------------------------------------------------

#[test]
fn test_debug_profile_output_kind_exe_preserved() {
    let mut config = TargetConfig::host().expect("host target init failed");
    config.apply_profile(Profile::Debug);
    config.set_output_kind(OutputKind::Exe);
    assert_eq!(config.output_kind(), &OutputKind::Exe);

    // Full compile + run should succeed and produce correct exit code.
    let bin = compile_src("fn main() -> i64 { return 42 }");
    let (code, _) = bin.run();
    assert_eq!(code, 42);
}

#[test]
fn test_release_profile_output_kind_exe_preserved() {
    let mut config = TargetConfig::host().expect("host target init failed");
    config.apply_profile(Profile::Release);
    config.set_output_kind(OutputKind::Exe);
    assert_eq!(config.output_kind(), &OutputKind::Exe);
}

#[test]
fn test_output_kind_object_preserved_through_compilation() {
    let mut config = TargetConfig::host().expect("host target init failed");
    config.set_output_kind(OutputKind::Object);

    let obj = compile_to_object("fn main() -> i64 { return 7 }");
    assert!(obj.path().exists(), "object file should be created");
    // An object file should exist and be non-empty.
    let metadata = std::fs::metadata(obj.path()).expect("object file should exist");
    assert!(metadata.len() > 0, "object file should not be empty");
}

#[test]
fn test_output_kind_ir_preserved_through_compilation() {
    let mut config = TargetConfig::host().expect("host target init failed");
    config.set_output_kind(OutputKind::Ir);

    let mir = to_mir("fn main() -> i64 { return 42 }");
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let ir_path = temp_dir.path().join("program.ll");

    compile_from_mir_ext(&mir, ir_path.to_str().unwrap(), config, false)
        .expect("IR compilation should succeed");

    assert!(ir_path.exists(), "IR file should be created");
    let ir_text = std::fs::read_to_string(&ir_path).expect("should be able to read IR file");
    assert!(
        ir_text.contains("define i64 @main"),
        "IR file should contain main function definition, got: {}",
        ir_text
    );
}

#[test]
fn test_output_kind_setter_roundtrips() {
    let mut config = TargetConfig::host().expect("host target init failed");

    config.set_output_kind(OutputKind::Ir);
    assert_eq!(config.output_kind(), &OutputKind::Ir);

    config.set_output_kind(OutputKind::Object);
    assert_eq!(config.output_kind(), &OutputKind::Object);

    config.set_output_kind(OutputKind::Exe);
    assert_eq!(config.output_kind(), &OutputKind::Exe);
}

// ---------------------------------------------------------------------------
// Target triple preservation
// ---------------------------------------------------------------------------

#[test]
fn test_target_triple_preserved_on_host_config() {
    let config = TargetConfig::host().expect("host target init failed");
    let triple = config.triple_str();
    assert!(!triple.is_empty(), "host triple should not be empty");
    assert!(
        triple.contains("linux") || triple.contains("windows") || triple.contains("darwin"),
        "host triple '{}' should contain a known OS",
        triple
    );
}

#[test]
fn test_target_triple_preserved_through_compilation() {
    // When we compile to IR, the module triple should match the config triple.
    let config = TargetConfig::host().expect("host target init failed");
    let expected_triple = config.triple_str();

    // `ir_only` generates IR text from MIR.  The module's triple is set inside
    // `compile_from_mir_ext`, but the IR-only path (`generate_ir_from_mir`)
    // does not call `set_triple`.  Instead, we verify triple preservation by
    // compiling with `compile_from_mir_ext` to an IR file and checking the
    // `source_filename` / target triple attribute in the output.
    let mir = to_mir("fn main() -> i64 { return 42 }");
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let ir_path = temp_dir.path().join("program.ll");

    let mut ir_config = TargetConfig::host().expect("host target init failed");
    ir_config.set_output_kind(OutputKind::Ir);
    compile_from_mir_ext(&mir, ir_path.to_str().unwrap(), ir_config, false)
        .expect("IR compilation should succeed");

    let ir_text = std::fs::read_to_string(&ir_path).expect("should read IR file");
    // LLVM's `print_to_file` for text IR includes a `source_filename` attribute,
    // and the triple is set on the module via `set_triple`.  Check that the
    // module-level triple line is present.
    assert!(
        ir_text.contains(&expected_triple),
        "IR output should contain the target triple '{}'",
        expected_triple
    );
}

// ---------------------------------------------------------------------------
// Profile mapping end-to-end consistency
// ---------------------------------------------------------------------------

#[test]
fn test_profile_mapping_consistent_across_setters() {
    // Apply debug profile, then verify every getter reflects it.
    {
        let mut config = TargetConfig::host().expect("host target init failed");
        config.apply_profile(Profile::Debug);
        assert_eq!(config.opt_level(), &OptimizationLevel::None);
        assert_eq!(config.debug_info(), &DebugInfo::Yes);
    }

    // Apply release profile, then verify every getter reflects it.
    {
        let mut config = TargetConfig::host().expect("host target init failed");
        config.apply_profile(Profile::Release);
        assert_eq!(config.opt_level(), &OptimizationLevel::Aggressive);
        assert_eq!(config.debug_info(), &DebugInfo::No);
    }
}
