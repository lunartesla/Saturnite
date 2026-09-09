//! Integration tests for the `saturn` toolchain CLI.
//!
//! These tests exercise the full toolchain: project discovery, build,
//! run, check, clean, and init.  Each test creates an isolated temp directory
//! so parallel execution never collides.

mod common;

use saturn::build::BuildContext;
use saturn::profile::BuildProfile;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

/// Write a file at `dir/<rel>` with the given contents, creating parent dirs.
fn write_file(dir: &std::path::Path, rel: &str, contents: &str) -> PathBuf {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("failed to create parent dir");
    }
    fs::write(&path, contents).unwrap_or_else(|e| panic!("failed to write {}: {}", path.display(), e));
    path
}

/// Write a minimal `saturn.toml` with `[build]` section.
fn write_saturn_toml(dir: &std::path::Path, name: &str, version: &str, edition: &str) {
    let toml = format!(
        r#"[package]
name = "{name}"
version = "{version}"
edition = "{edition}"

[dependencies]

[build]
source = "src"
entry = "main"
"#
    );
    write_file(dir, "saturn.toml", &toml);
}

/// Minimal valid native-syntax Saturnite program.
fn hello_program() -> &'static str {
    "module main\n\nmain:\n    say \"Hello, Saturnite!\"\n    give 0\n"
}

/// Check if the output executable exists on this platform.
fn exe_exists(path: &std::path::Path) -> bool {
    if cfg!(windows) {
        path.exists()
    } else {
        path.exists()
    }
}

/// Run the `saturn` binary from within a project directory.
fn run_saturn(dir: &std::path::Path, args: &[&str]) -> (i32, String) {
    let bin = env!("CARGO_BIN_EXE_saturn");
    let output = std::process::Command::new(bin)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("failed to execute saturn: {}", e));
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let combined = format!("{}{}", stdout, stderr);
    (output.status.code().unwrap_or(-1), combined)
}

// ---------------------------------------------------------------------------
// Project discovery
// ---------------------------------------------------------------------------

#[test]
fn test_discover_finds_project_from_root() {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let root = tmp.path();

    write_saturn_toml(root, "test_proj", "0.1.0", "2026");
    write_file(root, "src/main.stn", hello_program());

    let project = saturn::discover::ProjectDiscovery::discover(root)
        .expect("discovery should succeed");
    assert_eq!(project.root, root);
    assert_eq!(project.manifest.package.name, "test_proj");
    assert!(project.entry.ends_with("src/main.stn"));
    assert!(project.entry.is_file());
}

#[test]
fn test_discover_from_subdirectory_walks_up() {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let root = tmp.path();

    write_saturn_toml(root, "sub_proj", "0.2.0", "2026");
    write_file(root, "src/main.stn", hello_program());

    // Create a subdirectory and discover from there.
    let subdir = root.join("src").join("sub");
    fs::create_dir_all(&subdir).expect("failed to create subdir");

    let project = saturn::discover::ProjectDiscovery::discover(&subdir)
        .expect("discovery from subdir should succeed");
    assert_eq!(
        project.root, root,
        "project root should be the dir containing saturn.toml"
    );
    assert_eq!(project.manifest.package.name, "sub_proj");
}

// ---------------------------------------------------------------------------
// Debug build
// ---------------------------------------------------------------------------

#[test]
fn test_build_debug() {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let root = tmp.path();

    write_saturn_toml(root, "debug_test", "0.1.0", "2026");
    write_file(root, "src/main.stn", hello_program());

    let ctx = BuildContext::new(root.to_path_buf(), BuildProfile::Debug)
        .expect("build context should be created");
    let result = ctx.build().expect("build should succeed");

    assert!(exe_exists(&result.output), "executable should exist");
    // Parent directory should be `debug`.
    assert_eq!(
        result.output.parent().unwrap().file_name().unwrap(),
        std::ffi::OsStr::new("debug"),
        "should be in debug dir"
    );

    // Verify it runs and produces output.
    let output = std::process::Command::new(&result.output)
        .output()
        .expect("failed to execute compiled binary");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(0));
    assert!(stdout.contains("Hello, Saturnite!"));
}

// ---------------------------------------------------------------------------
// Release build
// ---------------------------------------------------------------------------

#[test]
fn test_build_release() {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let root = tmp.path();

    write_saturn_toml(root, "release_test", "0.1.0", "2026");
    write_file(root, "src/main.stn", hello_program());

    let ctx = BuildContext::new(root.to_path_buf(), BuildProfile::Release)
        .expect("build context should be created");
    let result = ctx.build().expect("release build should succeed");

    assert!(exe_exists(&result.output), "executable should exist");
    // Parent directory should be `release`.
    assert_eq!(
        result.output.parent().unwrap().file_name().unwrap(),
        std::ffi::OsStr::new("release"),
        "should be in release dir"
    );

    // Verify it runs.
    let output = std::process::Command::new(&result.output)
        .output()
        .expect("failed to execute compiled binary");
    assert_eq!(output.status.code(), Some(0));
}

// ---------------------------------------------------------------------------
// Output naming
// ---------------------------------------------------------------------------

#[test]
fn test_output_naming_uses_package_name() {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let root = tmp.path();

    write_saturn_toml(root, "calculator", "0.1.0", "2026");
    write_file(root, "src/main.stn", hello_program());

    let ctx = BuildContext::new(root.to_path_buf(), BuildProfile::Debug)
        .expect("build context should be created");
    let result = ctx.build().expect("build should succeed");

    // The output name should derive from the package name.
    let exe_name = if cfg!(windows) {
        "calculator.exe"
    } else {
        "calculator"
    };
    assert_eq!(
        result.output.file_name().unwrap(),
        exe_name,
        "executable name should match package name"
    );
}

// ---------------------------------------------------------------------------
// saturn run
// ---------------------------------------------------------------------------

#[test]
fn test_run_builds_and_executes() {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let root = tmp.path();

    write_saturn_toml(root, "run_test", "0.1.0", "2026");
    write_file(root, "src/main.stn", hello_program());

    let ctx = BuildContext::new(root.to_path_buf(), BuildProfile::Debug)
        .expect("build context should be created");
    let exit_code = ctx.run().expect("run should succeed");
    assert_eq!(exit_code, 0);

    // Verify the executable was placed in target/debug/
    let debug_dir = root.join("target").join("debug");
    assert!(debug_dir.exists(), "debug dir should exist");
}

// ---------------------------------------------------------------------------
// saturn check
// ---------------------------------------------------------------------------

#[test]
fn test_check_valid_project() {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let root = tmp.path();

    write_saturn_toml(root, "check_valid", "0.1.0", "2026");
    write_file(root, "src/main.stn", hello_program());

    let ctx = BuildContext::new(root.to_path_buf(), BuildProfile::Debug)
        .expect("build context should be created");
    ctx.check().expect("check should succeed for valid project");
}

#[test]
fn test_check_invalid_project_fails() {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let root = tmp.path();

    write_saturn_toml(root, "check_invalid", "0.1.0", "2026");
    // Invalid Saturnite: unbalanced types.
    write_file(root, "src/main.stn", "fn main() -> i64:\n    return \"not a number\"\n");

    let ctx = BuildContext::new(root.to_path_buf(), BuildProfile::Debug)
        .expect("build context should be created");
    let result = ctx.check();
    assert!(result.is_err(), "check should fail for invalid project");
}

// ---------------------------------------------------------------------------
// saturn clean
// ---------------------------------------------------------------------------

#[test]
fn test_clean_removes_build_artifacts() {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let root = tmp.path();

    write_saturn_toml(root, "clean_test", "0.1.0", "2026");
    write_file(root, "src/main.stn", hello_program());

    let ctx = BuildContext::new(root.to_path_buf(), BuildProfile::Debug)
        .expect("build context should be created");

    // Build first to create artifacts.
    ctx.build().expect("build should succeed");
    assert!(ctx.target_dir.exists(), "target dir should exist after build");

    // Clean and verify.
    ctx.clean_profile().expect("clean should succeed");
    assert!(!ctx.target_dir.exists(), "target dir should be removed after clean");

    // Source files and saturn.toml must still exist.
    assert!(root.join("saturn.toml").exists(), "saturn.toml must survive clean");
    assert!(root.join("src/main.stn").exists(), "source must survive clean");
}

#[test]
fn test_clean_all_removes_full_target() {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let root = tmp.path();

    write_saturn_toml(root, "clean_all_test", "0.1.0", "2026");
    write_file(root, "src/main.stn", hello_program());

    let ctx = BuildContext::new(root.to_path_buf(), BuildProfile::Debug)
        .expect("build context should be created");

    // Build both debug and release.
    let debug_ctx = BuildContext::new(root.to_path_buf(), BuildProfile::Debug)
        .expect("build context should be created");
    let release_ctx = BuildContext::new(root.to_path_buf(), BuildProfile::Release)
        .expect("build context should be created");

    debug_ctx.build().expect("debug build should succeed");
    release_ctx.build().expect("release build should succeed");

    // Clean all and verify.
    ctx.clean_all().expect("clean --all should succeed");
    assert!(!root.join("target").exists(), "target/ must be removed by clean --all");
}

// ---------------------------------------------------------------------------
// saturn init
// ---------------------------------------------------------------------------

#[test]
fn test_init_creates_valid_project() {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let root = tmp.path();

    // Run `saturn init` from within the temp dir, creating a "myproject" subdirectory.
    let (code, output) = run_saturn(root, &["init", "myproject"]);
    assert_eq!(code, 0, "init should succeed: {}", output);
    assert!(output.contains("Created project"));

    let proj_dir = root.join("myproject");
    assert!(proj_dir.join("saturn.toml").exists(), "saturn.toml should exist");
    assert!(proj_dir.join("src").join("main.stn").exists(), "src/main.stn should exist");

    // Verify the generated project can be built.
    let (code, output) = run_saturn(&proj_dir, &["build"]);
    assert_eq!(code, 0, "build should succeed: {}", output);
    assert!(output.contains("Built"));
}

#[test]
fn test_init_in_place() {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let root = tmp.path();

    // Create an existing directory with some content.
    fs::create_dir_all(root).ok();

    let (code, output) = run_saturn(root, &["init", "--in-place"]);
    assert_eq!(code, 0, "init --in-place should succeed: {}", output);

    assert!(root.join("saturn.toml").exists());
    assert!(root.join("src").join("main.stn").exists());
}

// ---------------------------------------------------------------------------
// CLI integration tests
// ---------------------------------------------------------------------------

#[test]
fn test_cli_build_from_project_root() {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let root = tmp.path();

    write_saturn_toml(root, "cli_test", "0.1.0", "2026");
    write_file(root, "src/main.stn", hello_program());

    let (code, output) = run_saturn(root, &["build"]);
    assert_eq!(code, 0, "saturn build should succeed: {}", output);
    assert!(output.contains("Built"));

    // Verify the executable was created.
    let exe_name = if cfg!(windows) { "cli_test.exe" } else { "cli_test" };
    let exe_path = root.join("target").join("debug").join(exe_name);
    assert!(exe_path.exists(), "executable should exist at {:?}", exe_path);
}

#[test]
fn test_cli_build_release() {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let root = tmp.path();

    write_saturn_toml(root, "cli_release", "0.1.0", "2026");
    write_file(root, "src/main.stn", hello_program());

    let (code, output) = run_saturn(root, &["build", "--release"]);
    assert_eq!(code, 0, "saturn build --release should succeed: {}", output);

    let exe_name = if cfg!(windows) { "cli_release.exe" } else { "cli_release" };
    let exe_path = root.join("target").join("release").join(exe_name);
    assert!(exe_path.exists(), "release executable should exist");
}

#[test]
fn test_cli_run() {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let root = tmp.path();

    write_saturn_toml(root, "cli_run", "0.1.0", "2026");
    write_file(root, "src/main.stn", hello_program());

    let (code, output) = run_saturn(root, &["run"]);
    assert_eq!(code, 0, "saturn run should succeed: {}", output);
    assert!(output.contains("Hello, Saturnite!"), "run output should contain the message");
}

#[test]
fn test_cli_check() {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let root = tmp.path();

    write_saturn_toml(root, "cli_check", "0.1.0", "2026");
    write_file(root, "src/main.stn", hello_program());

    let (code, output) = run_saturn(root, &["check"]);
    assert_eq!(code, 0, "saturn check should succeed: {}", output);
    assert!(output.contains("No errors found"));
}

#[test]
fn test_cli_clean() {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let root = tmp.path();

    write_saturn_toml(root, "cli_clean", "0.1.0", "2026");
    write_file(root, "src/main.stn", hello_program());

    // Build first.
    let (code, _) = run_saturn(root, &["build"]);
    assert_eq!(code, 0);

    // Clean.
    let (code, output) = run_saturn(root, &["clean"]);
    assert_eq!(code, 0, "clean should succeed: {}", output);
    assert!(output.contains("Removed"));

    // Source must survive.
    assert!(root.join("saturn.toml").exists());
    assert!(root.join("src").join("main.stn").exists());
}

#[test]
fn test_cli_clean_all() {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let root = tmp.path();

    write_saturn_toml(root, "cli_clean_all", "0.1.0", "2026");
    write_file(root, "src/main.stn", hello_program());

    // Build first.
    let (code, _) = run_saturn(root, &["build", "--release"]);
    assert_eq!(code, 0);

    // Clean all.
    let (code, output) = run_saturn(root, &["clean", "--all"]);
    assert_eq!(code, 0, "clean --all should succeed: {}", output);
    assert!(!root.join("target").exists(), "target/ should be removed");
}

// ---------------------------------------------------------------------------
// Platform awareness
// ---------------------------------------------------------------------------

#[test]
fn test_platform_aware_executable_name() {
    let name = "testapp";
    let exe = saturn::profile::executable_name(name);

    if cfg!(windows) {
        assert_eq!(exe, "testapp.exe", "Windows executable should have .exe suffix");
    } else {
        assert_eq!(exe, "testapp", "Non-Windows executable should not have .exe suffix");
    }
}

#[test]
fn test_platform_info() {
    let _triple = saturn::profile::platform_triple();
    // Just verify it doesn't crash and produces a non-empty string.
    assert!(!_triple.is_empty(), "triple should not be empty");
}
