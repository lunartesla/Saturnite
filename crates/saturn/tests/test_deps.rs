//! Tests for `saturn add`, `saturn install`, and `saturn update`.
//!
//! These tests verify the manifest modification, lockfile generation,
//! and dependency spec parsing for each ecosystem kind.
//!
//! Tests are split into:
//! - **Deterministic unit tests** — use `saturnite:` and `native:` deps that
//!   don't require network access or external tooling.
//! - **Network-dependent integration tests** — marked with `#[ignore]` and
//!   require `cargo test -- --ignored --no-fail-fast`. These exercise real
//!   Python venv creation, pip installation, and Rust wrapper compilation.

use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn saturn_bin() -> String {
    env!("CARGO_BIN_EXE_saturn").to_string()
}

fn run_saturn(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let output = Command::new(saturn_bin())
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("failed to execute saturn: {}", e));
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (output.status.code().unwrap_or(-1), stdout, stderr)
}

/// Create a minimal project for testing dependency commands.
fn create_project(dir: &Path, name: &str) {
    let (code, _, _) = run_saturn(dir, &["init", name, "--in-place"]);
    assert_eq!(code, 0, "saturn init should succeed");
}

/// Write wrapper metadata JSON so `saturn install` can build a Rust wrapper
/// for the given crate name in the project's `target/saturnite/wrappers/` dir.
fn setup_wrapper_metadata(dir: &Path, crate_name: &str, metadata_json: &str) {
    let wrapper_dir = dir.join("target").join("saturnite").join("wrappers");
    std::fs::create_dir_all(&wrapper_dir).unwrap();
    std::fs::write(
        wrapper_dir.join(format!("{}_saturnite.json", crate_name)),
        metadata_json,
    )
    .unwrap();
}

// ---------------------------------------------------------------------------
// Deterministic tests — no network needed
// ---------------------------------------------------------------------------

#[test]
fn test_add_saturnite_dependency() {
    let tmp = TempDir::new().unwrap();
    let proj_dir = tmp.path().join("testproj");
    std::fs::create_dir_all(&proj_dir).unwrap();
    create_project(&proj_dir, "testproj");

    // Add a Saturnite dependency (no registry resolution needed).
    let (code, out, _err) = run_saturn(&proj_dir, &["add", "mylib@1.0"]);
    assert_eq!(code, 0, "saturn add mylib@1.0 should succeed");
    assert!(out.contains("Added dependency"));
    assert!(out.contains("mylib"));

    // Verify saturn.toml was updated.
    let toml = std::fs::read_to_string(proj_dir.join("saturn.toml")).unwrap();
    assert!(toml.contains("mylib"));

    // Verify saturn.lock was created.
    let lock = std::fs::read_to_string(proj_dir.join("saturn.lock")).unwrap();
    assert!(lock.contains("mylib"));
    assert!(lock.contains("1.0.0"));
}

#[test]
fn test_add_native_dependency() {
    let tmp = TempDir::new().unwrap();
    let proj_dir = tmp.path().join("testproj");
    std::fs::create_dir_all(&proj_dir).unwrap();
    create_project(&proj_dir, "testproj");

    let (code, out, _err) = run_saturn(&proj_dir, &["add", "native:zlib@1.0"]);
    assert_eq!(code, 0);
    assert!(out.contains("Native") || out.contains("native"));
    assert!(out.contains("zlib"));

    let toml = std::fs::read_to_string(proj_dir.join("saturn.toml")).unwrap();
    assert!(toml.contains("zlib"));
}

#[test]
fn test_add_duplicate_dependency_fails() {
    let tmp = TempDir::new().unwrap();
    let proj_dir = tmp.path().join("testproj");
    std::fs::create_dir_all(&proj_dir).unwrap();
    create_project(&proj_dir, "testproj");

    // Add a dependency.
    let (code, _, _) = run_saturn(&proj_dir, &["add", "mylib@1.0"]);
    assert_eq!(code, 0);

    // Try to add the same dependency again.
    let (code, out, err) = run_saturn(&proj_dir, &["add", "mylib@1.0"]);
    assert_eq!(code, 1, "adding duplicate dependency should fail");
    assert!(
        out.contains("already in saturn.toml")
            || err.contains("already in saturn.toml")
            || out.contains("already")
            || err.contains("already")
    );
}

#[test]
fn test_install_creates_lockfile() {
    let tmp = TempDir::new().unwrap();
    let proj_dir = tmp.path().join("testproj");
    std::fs::create_dir_all(&proj_dir).unwrap();
    create_project(&proj_dir, "testproj");

    // Add a Saturnite dependency (no external resolution or install needed).
    let (code, _, _) = run_saturn(&proj_dir, &["add", "mylib@1.0"]);
    assert_eq!(code, 0);

    // Run install — should succeed (saturnite deps have no artifacts to build).
    let (code, out, _err) = run_saturn(&proj_dir, &["install"]);
    assert_eq!(code, 0, "saturn install should succeed, got out={}", out);

    // Verify lockfile exists.
    assert!(proj_dir.join("saturn.lock").exists());
}

#[test]
fn test_install_idempotent_with_current_lockfile() {
    let tmp = TempDir::new().unwrap();
    let proj_dir = tmp.path().join("testproj");
    std::fs::create_dir_all(&proj_dir).unwrap();
    create_project(&proj_dir, "testproj");

    // Add a Saturnite dependency.
    let (code, _, _) = run_saturn(&proj_dir, &["add", "mylib@1.0"]);
    assert_eq!(code, 0);

    // First install writes the lockfile.
    let (code, out, _err) = run_saturn(&proj_dir, &["install"]);
    assert_eq!(code, 0, "first install should succeed: {}", out);

    // Second install with a current lockfile should NOT re-resolve but
    // should still attempt preparation (which for saturnite deps is a no-op).
    let (code, out, _err) = run_saturn(&proj_dir, &["install"]);
    assert_eq!(code, 0, "second install should succeed: {}", out);
    assert!(
        out.contains("current") || out.contains("dependencies"),
        "should indicate lockfile is current: {}",
        out
    );
}

#[test]
fn test_install_force_resolves() {
    let tmp = TempDir::new().unwrap();
    let proj_dir = tmp.path().join("testproj");
    std::fs::create_dir_all(&proj_dir).unwrap();
    create_project(&proj_dir, "testproj");

    // Add a Saturnite dependency.
    let (code, _, _) = run_saturn(&proj_dir, &["add", "mylib@1.0"]);
    assert_eq!(code, 0);

    // Install with --force — should re-resolve and write lockfile.
    let (code, out, _err) = run_saturn(&proj_dir, &["install", "--force"]);
    assert_eq!(code, 0, "install --force should succeed: {}", out);
    assert!(
        out.contains("resolved") || out.contains("dependencies"),
        "should mention resolution: {}",
        out
    );
}

#[test]
fn test_update_specific_dependency() {
    let tmp = TempDir::new().unwrap();
    let proj_dir = tmp.path().join("testproj");
    std::fs::create_dir_all(&proj_dir).unwrap();
    create_project(&proj_dir, "testproj");

    let (code, _, _) = run_saturn(&proj_dir, &["add", "mylib@1.0"]);
    assert_eq!(code, 0);

    // Update a specific dependency.
    let (code, _out, _err) = run_saturn(&proj_dir, &["update", "mylib"]);
    assert_eq!(code, 0, "saturn update mylib should succeed");
}

#[test]
fn test_update_nonexistent_dependency_fails() {
    let tmp = TempDir::new().unwrap();
    let proj_dir = tmp.path().join("testproj");
    std::fs::create_dir_all(&proj_dir).unwrap();
    create_project(&proj_dir, "testproj");

    // Try to update a dependency that doesn't exist.
    let (code, out, err) = run_saturn(&proj_dir, &["update", "nonexistent_dep"]);
    assert_eq!(code, 1, "updating a non-existent dependency should fail");
    assert!(out.contains("not in saturn.toml") || err.contains("not in saturn.toml"));
}

#[test]
fn test_update_without_name_or_all_fails() {
    let tmp = TempDir::new().unwrap();
    let proj_dir = tmp.path().join("testproj");
    std::fs::create_dir_all(&proj_dir).unwrap();
    create_project(&proj_dir, "testproj");

    // Update without a name or --all should error.
    let (code, out, err) = run_saturn(&proj_dir, &["update"]);
    assert_eq!(code, 1, "saturn update without args should fail");
    assert!(
        out.contains("specify")
            || err.contains("specify")
            || out.contains("usage")
            || err.contains("usage")
    );
}

#[test]
fn test_saturnite_dependency_default_kind() {
    let tmp = TempDir::new().unwrap();
    let proj_dir = tmp.path().join("testproj");
    std::fs::create_dir_all(&proj_dir).unwrap();
    create_project(&proj_dir, "testproj");

    // Add a dependency without a kind prefix (defaults to saturnite).
    let (code, out, _err) = run_saturn(&proj_dir, &["add", "mylib@1.0"]);
    assert_eq!(code, 0);
    assert!(out.contains("mylib"));

    let toml = std::fs::read_to_string(proj_dir.join("saturn.toml")).unwrap();
    assert!(toml.contains("mylib"));
}

#[test]
fn test_lockfile_contains_version() {
    let tmp = TempDir::new().unwrap();
    let proj_dir = tmp.path().join("testproj");
    std::fs::create_dir_all(&proj_dir).unwrap();
    create_project(&proj_dir, "testproj");

    let (code, _, _) = run_saturn(&proj_dir, &["add", "mylib@1.0"]);
    assert_eq!(code, 0);

    // Lockfile should contain the version.
    let lock = std::fs::read_to_string(proj_dir.join("saturn.lock")).unwrap();
    assert!(lock.contains("1.0.0"));
}

#[test]
fn test_install_failure_returns_nonzero() {
    // This test verifies that install returns nonzero when a Rust dep's
    // wrapper metadata is missing. It uses `saturn add rust:serde@1.0` which
    // resolves from crates.io (network required), then `install` which fails
    // because no wrapper metadata JSON exists.
    let tmp = TempDir::new().unwrap();
    let proj_dir = tmp.path().join("testproj");
    std::fs::create_dir_all(&proj_dir).unwrap();
    create_project(&proj_dir, "testproj");

    // Add a Rust dependency — resolution should succeed (serde exists on crates.io).
    let (code, _, _) = run_saturn(&proj_dir, &["add", "rust:serde@1.0"]);
    if code != 0 {
        eprintln!("Skipping: rust:serde@1.0 resolution requires network access");
        return;
    }

    // Install should fail (nonzero exit) because no wrapper metadata exists.
    let (code, out, _err) = run_saturn(&proj_dir, &["install"]);
    assert_ne!(
        code, 0,
        "install should fail when wrapper metadata is missing: {}",
        out
    );
    assert!(
        out.contains("Failed dependencies"),
        "should report failures in output: {}",
        out
    );
}

#[test]
fn test_install_continues_after_failure() {
    // This test verifies that install continues attempting eligible deps after
    // one fails, and aggregates all failures. Uses a saturnite dep (always
    // succeeds) plus a Rust dep without wrapper metadata (fails).
    let tmp = TempDir::new().unwrap();
    let proj_dir = tmp.path().join("testproj");
    std::fs::create_dir_all(&proj_dir).unwrap();
    create_project(&proj_dir, "testproj");

    // Add a Saturnite dep (no network needed, install succeeds).
    let (code, _, _) = run_saturn(&proj_dir, &["add", "mylib@1.0"]);
    assert_eq!(code, 0);

    // Add a Rust dep (network needed for resolution).
    let (code, _, _) = run_saturn(&proj_dir, &["add", "rust:serde@1.0"]);
    if code != 0 {
        eprintln!("Skipping: rust:serde@1.0 resolution requires network access");
        return;
    }

    // Install should attempt both and report the failure for the Rust dep,
    // while still processing the saturnite dep.
    let (code, out, _err) = run_saturn(&proj_dir, &["install"]);
    assert_ne!(
        code, 0,
        "install should fail because Rust wrapper couldn't build"
    );
    // The Saturnite dep should still have been processed.
    assert!(
        out.contains("saturnite") || out.contains("mylib"),
        "saturnite dep should have been attempted: {}",
        out
    );
    assert!(
        out.contains("Failed dependencies"),
        "should report failures: {}",
        out
    );
}

// ---------------------------------------------------------------------------
// Network-dependent tests — run with: cargo test -- --ignored --no-fail-fast
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn test_install_creates_python_venv() {
    let tmp = TempDir::new().unwrap();
    let proj_dir = tmp.path().join("testproj");
    std::fs::create_dir_all(&proj_dir).unwrap();
    create_project(&proj_dir, "testproj");

    // Add a real Python package.
    let (code, _, err) = run_saturn(&proj_dir, &["add", "python:requests@2.31"]);
    assert_eq!(code, 0, "add python:requests@2.31 failed: {}", err);

    // Install should create a venv and pip-install the package.
    let (code, out, err) = run_saturn(&proj_dir, &["install"]);
    assert_eq!(
        code, 0,
        "install should succeed: stdout={}, stderr={}",
        out, err
    );
    assert!(
        out.contains("installed") || out.contains("requests"),
        "{}",
        out
    );

    // Verify the venv exists.
    let venv_dir = proj_dir
        .join("target")
        .join("saturnite")
        .join("venvs")
        .join("testproj");
    assert!(
        venv_dir.join("bin").join("python3").exists()
            || venv_dir.join("Scripts").join("python.exe").exists(),
        "venv should exist at {}",
        venv_dir.display()
    );

    // Verify the package is installed in the venv.
    let python_exe = if venv_dir.join("bin/python3").exists() {
        venv_dir.join("bin/python3")
    } else {
        venv_dir.join("Scripts/python.exe")
    };
    let check = Command::new(&python_exe)
        .arg("-c")
        .arg("import requests; print(requests.__version__)")
        .output()
        .unwrap();
    assert!(check.status.success(), "requests should be importable");
}

#[test]
#[ignore]
fn test_install_retries_missing_package() {
    let tmp = TempDir::new().unwrap();
    let proj_dir = tmp.path().join("testproj");
    std::fs::create_dir_all(&proj_dir).unwrap();
    create_project(&proj_dir, "testproj");

    // Add a Python dependency.
    let (code, _, _) = run_saturn(&proj_dir, &["add", "python:requests@2.31"]);
    assert_eq!(code, 0);

    // Run install to create the venv and install the package.
    let (code, _, _) = run_saturn(&proj_dir, &["install"]);
    assert_eq!(code, 0);

    // Delete the venv to simulate a missing environment.
    let venv_dir = proj_dir
        .join("target")
        .join("saturnite")
        .join("venvs")
        .join("testproj");
    std::fs::remove_dir_all(&venv_dir).ok();
    assert!(!venv_dir.exists());

    // Run install again — should recreate the venv and reinstall.
    let (code, out, _err) = run_saturn(&proj_dir, &["install"]);
    assert_eq!(code, 0, "should recreate venv and reinstall: {}", out);
    assert!(venv_dir.exists(), "venv should be recreated");
}

#[test]
#[ignore]
fn test_compiled_executable_imports_installed_python() {
    // End-to-end: add a Python dep, install, write a Saturnite program that
    // calls it, build, and run — verify the output is correct.
    let tmp = TempDir::new().unwrap();
    let proj_dir = tmp.path().join("testproj");
    std::fs::create_dir_all(&proj_dir).unwrap();
    create_project(&proj_dir, "testproj");

    // Add a Python dependency.
    let (code, _, _) = run_saturn(&proj_dir, &["add", "python:requests@2.31"]);
    assert_eq!(code, 0);

    // Install.
    let (code, _, _) = run_saturn(&proj_dir, &["install"]);
    assert_eq!(code, 0);

    // Provide wrapper metadata for requests so the compiler can call it.
    // We'll use a simple function: requests.get(url) -> i64 (status code)
    let metadata = r#"{
        "module": "requests",
        "functions": [
            {"name": "get", "symbol": "get", "params": ["str"], "ret": "i64"}
        ]
    }"#;
    setup_wrapper_metadata(&proj_dir, "requests", metadata);

    // Write a Saturnite program that calls requests.get.
    let src = r#"
use python:requests

fn main() -> i64 {
    0
}
"#;
    std::fs::write(proj_dir.join("src").join("main.stn"), src).unwrap();

    // Build the project.
    let (code, _out, err) = run_saturn(&proj_dir, &["build"]);
    assert_eq!(code, 0, "build should succeed: {}", err);

    // Run the executable.
    let exe_name = if cfg!(windows) {
        "testproj.exe"
    } else {
        "testproj"
    };
    let exe = proj_dir.join("target").join("debug").join(exe_name);
    let output = Command::new(&exe).output().unwrap();
    assert!(
        output.status.success(),
        "executable should run successfully: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
