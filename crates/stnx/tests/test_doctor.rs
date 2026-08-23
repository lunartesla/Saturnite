//! Integration tests for the `saturn doctor` command.
//!
//! `run_doctor()` lives in `main.rs` (the binary crate) and is therefore not
//! directly callable from integration tests (which link against the library
//! crate only).  We instead verify:
//!
//! 1. **Library seams** — the public functions `host_triple()` and
//!    `check_linker()` that `run_doctor()` depends on.  These must succeed on
//!    the host machine.
//!
//! 2. **End-to-end smoke test** — invoke the compiled `stnx` binary with the
//!    `doctor` subcommand and assert that the output contains every section
//!    the doctor is supposed to report: host target, host configuration,
//!    linker availability, LLVM information, and runtime availability.
//!
//! 3. **No redundant linker double-check** — the output must mention "Linker"
//!    exactly once (see Phase 2 task: ensure Agent 2A's cleanup eliminated the
//!    old double-check where `run_diagnostics()` printed linker status and
//!    `run_doctor()` printed it again separately).

mod common;

use std::process::Command;
use stnx::codegen::{check_linker, host_triple};
use stnx::target::TargetConfig;

// ---------------------------------------------------------------------------
// Library seam tests
// ---------------------------------------------------------------------------

#[test]
fn test_host_triple_returns_valid_triple() {
    // `host_triple()` is one of the public library calls that `run_doctor()`
    // delegates to.  On any reasonable host it should succeed and return a
    // non-empty string containing the OS name.
    let triple = host_triple().expect("host_triple() should succeed on the host machine");
    assert!(!triple.is_empty(), "host triple must not be empty");
    let lower = triple.to_lowercase();
    assert!(
        lower.contains("linux") || lower.contains("windows") || lower.contains("darwin"),
        "host triple '{}' should contain a known OS",
        triple
    );
}

#[test]
fn test_check_linker_does_not_panic_on_host() {
    // `check_linker()` is the public library call for linker verification.
    // On a properly set-up host it should succeed; if the linker is missing
    // we still want to ensure the function returns (Ok or Err) without panicking.
    let config = TargetConfig::host().expect("TargetConfig::host() should succeed");
    let result = check_linker(&config);
    // We don't assert Ok here — CI environments might lack a C linker — but
    // the call must return cleanly (not panic).
    match &result {
        Ok(()) => {}
        Err(e) => {
            // If it errors, the error message should be informative.
            let msg = e.to_string();
            assert!(!msg.is_empty(), "linker error message should not be empty");
        }
    }
}

#[test]
fn test_target_config_host_exposes_expected_fields() {
    // Verify that the TargetConfig fields that run_doctor() prints are
    // accessible and non-empty.
    let config = TargetConfig::host().expect("TargetConfig::host() should succeed");

    assert!(
        !config.triple_str().is_empty(),
        "triple should be non-empty"
    );
    // These enums derive Debug but not Display, so use {:?} formatting.
    assert_ne!(
        format!("{:?}", config.architecture()).len(),
        0,
        "architecture should be non-empty"
    );
    assert_ne!(
        format!("{:?}", config.os()).len(),
        0,
        "os should be non-empty"
    );
    assert_ne!(
        format!("{:?}", config.environment()).len(),
        0,
        "environment should be non-empty"
    );
    assert_ne!(
        format!("{:?}", config.opt_level()).len(),
        0,
        "opt_level should be non-empty"
    );
}

// ---------------------------------------------------------------------------
// End-to-end smoke test: run the binary's `doctor` subcommand
// ---------------------------------------------------------------------------

/// Locate the compiled `stnx` binary.
///
/// Integration tests run after the crate is compiled, so the debug binary
/// should exist at `target/debug/stnx` relative to the workspace root.  We
/// use `CARGO_BIN_EXE_stnx` (set by Cargo when a `[[bin]]` target named
/// `stnx` exists) and fall back to a relative path for robustness.
fn stnx_binary_path() -> std::path::PathBuf {
    if let Some(path) = std::env::var_os("CARGO_BIN_EXE_stnx") {
        return std::path::PathBuf::from(path);
    }

    // Fallback: derive from OUT_DIR or CARGO_MANIFEST_DIR.
    let manifest_dir =
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR should be set during tests");
    let workspace_root = std::path::Path::new(&manifest_dir)
        .ancestors()
        .nth(2) // crates/stnx/src -> crates/stnx -> crates -> <workspace root>
        .unwrap_or(std::path::Path::new(&manifest_dir));
    workspace_root.join("target").join("debug").join("stnx")
}

/// Run `stnx doctor` and return (exit_code, stdout).
fn run_doctor_command() -> (i32, String) {
    let bin = stnx_binary_path();
    assert!(
        bin.exists(),
        "binary not found at {}; run `cargo build -p stnx` first",
        bin.display()
    );

    let output = Command::new(&bin)
        .arg("doctor")
        .output()
        .unwrap_or_else(|e| panic!("failed to execute {}: {}", bin.display(), e));

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let exit_code = output.status.code().unwrap_or(-1);
    (exit_code, stdout)
}

#[test]
fn test_doctor_command_exits_zero() {
    let (code, _) = run_doctor_command();
    assert_eq!(code, 0, "doctor command should exit successfully (code 0)");
}

#[test]
fn test_doctor_command_reports_host_target() {
    let (code, stdout) = run_doctor_command();
    assert_eq!(code, 0, "doctor command should succeed");
    assert!(
        stdout.contains("Host target triple:"),
        "doctor output should report host target triple"
    );
    assert!(
        stdout.contains("Host configuration:"),
        "doctor output should report host configuration"
    );
}

#[test]
fn test_doctor_command_reports_linker() {
    let (code, stdout) = run_doctor_command();
    assert_eq!(code, 0, "doctor command should succeed");

    // Count occurrences of "Linker:" — there should be exactly one.
    // This verifies the redundant linker double-check was eliminated
    // (previously run_diagnostics() and run_doctor() both printed linker status).
    let linker_count = stdout.matches("Linker:").count();
    assert_eq!(
        linker_count, 1,
        "doctor should report linker status exactly once (redundant double-check removed); \
         found {} occurrence(s) of 'Linker:' in output:\n{}",
        linker_count, stdout
    );
}

#[test]
fn test_doctor_command_reports_llvm_info() {
    let (code, stdout) = run_doctor_command();
    assert_eq!(code, 0, "doctor command should succeed");
    assert!(
        stdout.contains("inkwell") && stdout.contains("LLVM"),
        "doctor output should report LLVM/inkwell information"
    );
}

#[test]
fn test_doctor_command_reports_runtime() {
    let (code, stdout) = run_doctor_command();
    assert_eq!(code, 0, "doctor command should succeed");
    assert!(
        stdout.contains("Runtime:"),
        "doctor output should report runtime availability"
    );
}

#[test]
fn test_doctor_command_output_contains_all_sections() {
    // Comprehensive check: the doctor output should mention every section
    // the command is documented to report.
    let (code, stdout) = run_doctor_command();
    assert_eq!(code, 0);

    let expected_fragments = [
        "Saturnite Compiler Diagnostics",
        "Host target triple:",
        "Host configuration:",
        "Architecture",
        "OS:",
        "Environment",
        "Opt level:",
        "Linker:",
        "inkwell",
        "LLVM",
        "Runtime:",
    ];

    for fragment in &expected_fragments {
        assert!(
            stdout.contains(fragment),
            "doctor output should contain '{}'; full output was:\n{}",
            fragment,
            stdout
        );
    }
}
