//! End-to-end smoke test for the `saturn` toolchain.
//!
//! This test mirrors the canonical user workflow:
//!
//! 1. Start from an empty directory.
//! 2. Run `saturn init calculator`.
//! 3. Run `saturn build` — must produce a runnable executable.
//! 4. Run `saturn build --release` — must produce a release executable.
//! 5. Run `saturn run` — must execute the project.

use std::process::Command;
use tempfile::TempDir;

/// Path to the `saturn` binary under test.
fn saturn_bin() -> String {
    // CARGO_BIN_EXE_saturn is set by Cargo for integration tests.
    env!("CARGO_BIN_EXE_saturn").to_string()
}

/// Run the `saturn` binary with given args from within `dir`.
fn run_saturn(dir: &std::path::Path, args: &[&str]) -> (i32, String, String) {
    let output = Command::new(saturn_bin())
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("failed to execute saturn: {}", e));
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (output.status.code().unwrap_or(-1), stdout, stderr)
}

#[test]
fn test_end_to_end_workflow() {
    let tmp = TempDir::new().expect("failed to create isolated temp dir");
    let workspace = tmp.path();

    // Step 1: saturn init
    let (code, out, err) = run_saturn(workspace, &["init", "calculator"]);
    assert_eq!(code, 0, "saturn init calculator failed: stdout={}, stderr={}", out, err);
    assert!(out.contains("Created project 'calculator'"));
    assert!(out.contains("main.stn"));

    let proj_dir = workspace.join("calculator");

    // Verify files exist.
    assert!(proj_dir.join("saturn.toml").exists());
    assert!(proj_dir.join("src/main.stn").exists());

    // Step 2: saturn build (debug)
    let (code, out, err) = run_saturn(&proj_dir, &["build"]);
    assert_eq!(code, 0, "saturn build failed: stdout={}, stderr={}", out, err);
    assert!(out.contains("Built"));

    // Determine the expected executable name for this platform.
    let exe_name = if cfg!(windows) {
        "calculator.exe"
    } else {
        "calculator"
    };
    let debug_exe = proj_dir.join("target").join("debug").join(exe_name);
    assert!(debug_exe.exists(), "debug executable should exist at {:?}", debug_exe);

    // Verify the executable runs and produces output.
    let run_result = Command::new(&debug_exe)
        .output()
        .unwrap_or_else(|e| panic!("failed to execute debug binary: {}", e));
    assert_eq!(run_result.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&run_result.stdout).contains("Hello, Saturnite!"));

    // Step 3: saturn build --release
    let (code, out, err) = run_saturn(&proj_dir, &["build", "--release"]);
    assert_eq!(code, 0, "saturn build --release failed: stdout={}, stderr={}", out, err);
    assert!(out.contains("Built"));

    let release_exe = proj_dir.join("target").join("release").join(exe_name);
    assert!(release_exe.exists(), "release executable should exist at {:?}", release_exe);

    // Step 4: saturn run
    let (code, out, err) = run_saturn(&proj_dir, &["run"]);
    assert_eq!(code, 0, "saturn run failed: stdout={}, stderr={}", out, err);
    assert!(out.contains("Hello, Saturnite!"), "run output: {}", out);

    // Step 5: saturn run --release
    let (code, out, err) = run_saturn(&proj_dir, &["run", "--release"]);
    assert_eq!(code, 0, "saturn run --release failed: stdout={}, stderr={}", out, err);
    assert!(out.contains("Hello, Saturnite!"));

    // Step 6: saturn check
    let (code, out, err) = run_saturn(&proj_dir, &["check"]);
    assert_eq!(code, 0, "saturn check failed: stdout={}, stderr={}", out, err);
    assert!(out.contains("No errors found"));

    // Step 7: saturn clean then build again
    let (code, out, err) = run_saturn(&proj_dir, &["clean"]);
    assert_eq!(code, 0, "saturn clean failed: stdout={}, stderr={}", out, err);
    assert!(!proj_dir.join("target").join("debug").exists());

    // Rebuild after clean to verify it's reproducible.
    let (code, out, err) = run_saturn(&proj_dir, &["build"]);
    assert_eq!(code, 0, "rebuild after clean failed: stdout={}, stderr={}", out, err);
    assert!(debug_exe.exists());
}
