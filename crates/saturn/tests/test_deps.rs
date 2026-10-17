//! Tests for `saturn add`, `saturn install`, and `saturn update`.
//!
//! These tests verify the manifest modification, lockfile generation,
//! and dependency spec parsing for each ecosystem kind.

use std::process::Command;
use tempfile::TempDir;

fn saturn_bin() -> String {
    env!("CARGO_BIN_EXE_saturn").to_string()
}

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

/// Create a minimal project for testing dependency commands.
fn create_project(dir: &std::path::Path, name: &str) {
    let (code, _, _) = run_saturn(dir, &["init", name, "--in-place"]);
    assert_eq!(code, 0, "saturn init should succeed");
}

#[test]
fn test_add_rust_dependency() {
    let tmp = TempDir::new().unwrap();
    let proj_dir = tmp.path().join("testproj");
    std::fs::create_dir_all(&proj_dir).unwrap();
    create_project(&proj_dir, "testproj");

    // Add a Rust dependency.
    let (code, out, err) = run_saturn(&proj_dir, &["add", "rust:serde@1.0"]);
    assert_eq!(
        code, 0,
        "saturn add rust:serde@1.0 failed: stdout={}, stderr={}",
        out, err
    );
    assert!(out.contains("Added dependency"));

    // Verify saturn.toml was updated.
    let toml = std::fs::read_to_string(proj_dir.join("saturn.toml")).unwrap();
    assert!(toml.contains("serde"));
    assert!(toml.contains("rust"));

    // Verify saturn.lock was created.
    let lock = std::fs::read_to_string(proj_dir.join("saturn.lock")).unwrap();
    assert!(lock.contains("serde"));
}

#[test]
fn test_add_python_dependency() {
    let tmp = TempDir::new().unwrap();
    let proj_dir = tmp.path().join("testproj");
    std::fs::create_dir_all(&proj_dir).unwrap();
    create_project(&proj_dir, "testproj");

    // Add a Python dependency.
    let (code, out, err) = run_saturn(&proj_dir, &["add", "python:numpy>=1.24"]);
    assert_eq!(
        code, 0,
        "saturn add python:numpy>=1.24 failed: stdout={}, stderr={}",
        out, err
    );
    assert!(out.contains("Added dependency"));
    assert!(out.contains("numpy"));

    // Verify saturn.toml was updated.
    let toml = std::fs::read_to_string(proj_dir.join("saturn.toml")).unwrap();
    assert!(toml.contains("numpy"));

    // Verify lockfile.
    let lock = std::fs::read_to_string(proj_dir.join("saturn.lock")).unwrap();
    assert!(lock.contains("numpy"));
    assert!(lock.contains("python"));
}

#[test]
fn test_add_rust_dependency_shorthand() {
    let tmp = TempDir::new().unwrap();
    let proj_dir = tmp.path().join("testproj");
    std::fs::create_dir_all(&proj_dir).unwrap();
    create_project(&proj_dir, "testproj");

    // Add a Rust dependency using the shorthand `rust:name@version`.
    let (code, out, _err) = run_saturn(&proj_dir, &["add", "rust:tokio@1.0"]);
    assert_eq!(code, 0, "saturn add rust:tokio@1.0 failed");
    assert!(out.contains("tokio"));
}

#[test]
fn test_add_duplicate_dependency_fails() {
    let tmp = TempDir::new().unwrap();
    let proj_dir = tmp.path().join("testproj");
    std::fs::create_dir_all(&proj_dir).unwrap();
    create_project(&proj_dir, "testproj");

    // Add a dependency.
    let (code, _, _) = run_saturn(&proj_dir, &["add", "rust:serde@1.0"]);
    assert_eq!(code, 0);

    // Try to add the same dependency again.
    let (code, out, err) = run_saturn(&proj_dir, &["add", "rust:serde@1.0"]);
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

    // Add a dependency first.
    let (code, _, _) = run_saturn(&proj_dir, &["add", "rust:serde@1.0"]);
    assert_eq!(code, 0);

    // Run install — should be a no-op since lockfile already exists and is current.
    let (code, out, _err) = run_saturn(&proj_dir, &["install"]);
    assert_eq!(code, 0, "saturn install should succeed");
    assert!(out.contains("up to date") || out.contains("dependencies"));

    // Verify lockfile exists.
    assert!(proj_dir.join("saturn.lock").exists());
}

#[test]
fn test_install_force_resolves() {
    let tmp = TempDir::new().unwrap();
    let proj_dir = tmp.path().join("testproj");
    std::fs::create_dir_all(&proj_dir).unwrap();
    create_project(&proj_dir, "testproj");

    // Add deps.
    let (code, _, _) = run_saturn(&proj_dir, &["add", "rust:serde@1.0"]);
    assert_eq!(code, 0);
    let (code, _, _) = run_saturn(&proj_dir, &["add", "python:numpy@1.24"]);
    assert_eq!(code, 0);

    // Install with --force.
    let (code, out, _err) = run_saturn(&proj_dir, &["install", "--force"]);
    assert_eq!(code, 0);
    assert!(out.contains("resolved") || out.contains("dependencies"));
}

#[test]
fn test_update_all() {
    let tmp = TempDir::new().unwrap();
    let proj_dir = tmp.path().join("testproj");
    std::fs::create_dir_all(&proj_dir).unwrap();
    create_project(&proj_dir, "testproj");

    let (code, _, _) = run_saturn(&proj_dir, &["add", "rust:serde@1.0"]);
    assert_eq!(code, 0);

    // Update all.
    let (code, out, _err) = run_saturn(&proj_dir, &["update", "--all"]);
    assert_eq!(code, 0, "saturn update --all should succeed");
    assert!(out.contains("updated") || out.contains("Lockfile"));
}

#[test]
fn test_update_specific_dependency() {
    let tmp = TempDir::new().unwrap();
    let proj_dir = tmp.path().join("testproj");
    std::fs::create_dir_all(&proj_dir).unwrap();
    create_project(&proj_dir, "testproj");

    let (code, _, _) = run_saturn(&proj_dir, &["add", "rust:serde@1.0"]);
    assert_eq!(code, 0);

    // Update a specific dependency.
    let (code, _out, _err) = run_saturn(&proj_dir, &["update", "serde"]);
    assert_eq!(code, 0, "saturn update serde should succeed");
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

    let (code, _, _) = run_saturn(&proj_dir, &["add", "rust:serde@1.0"]);
    assert_eq!(code, 0);

    // Lockfile should contain the version.
    let lock = std::fs::read_to_string(proj_dir.join("saturn.lock")).unwrap();
    assert!(lock.contains("1.0"));
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
