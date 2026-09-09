//! Tests for the toolchain's manifest parsing and config handling.

use saturn::manifest::Manifest;
use tempfile::TempDir;

#[test]
fn test_parse_minimal_manifest() {
    let toml = r#"[package]
name = "hello"
version = "0.1.0"
edition = "2026"
"#;
    let manifest = Manifest::from_str(toml).expect("should parse");
    assert_eq!(manifest.package.name, "hello");
    assert_eq!(manifest.package.version, "0.1.0");
    assert_eq!(manifest.package.edition, "2026");
    assert_eq!(manifest.build.source, "src");
    assert_eq!(manifest.build.entry, "main");
}

#[test]
fn test_parse_manifest_with_build_section() {
    let toml = r#"[package]
name = "custom"
version = "0.2.0"
edition = "2026"

[build]
source = "source"
entry = "app"
"#;
    let manifest = Manifest::from_str(toml).expect("should parse");
    assert_eq!(manifest.build.source, "source");
    assert_eq!(manifest.build.entry, "app");
}

#[test]
fn test_manifest_defaults() {
    let manifest = Manifest::default();
    assert_eq!(manifest.build.source, "src");
    assert_eq!(manifest.build.entry, "main");
}

#[test]
fn test_manifest_from_name() {
    let manifest = Manifest::from_name("myapp").expect("should create");
    assert_eq!(manifest.package.name, "myapp");
    assert_eq!(manifest.package.version, "0.1.0");
    assert_eq!(manifest.package.edition, "2026");
}

#[test]
fn test_manifest_generate_toml() {
    let manifest = Manifest {
        package: saturn::manifest::Package {
            name: "gen_test".to_string(),
            version: "1.0.0".to_string(),
            edition: "2026".to_string(),
        },
        dependencies: Default::default(),
        build: saturn::manifest::BuildSection::default(),
    };
    let toml_str = manifest.generate_toml();
    assert!(toml_str.contains(r#"name = "gen_test""#));
    assert!(toml_str.contains(r#"[build]"#));
    assert!(toml_str.contains(r#"source = "src""#));
    assert!(toml_str.contains(r#"entry = "main""#));
}

#[test]
fn test_manifest_from_dir() {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let toml = r#"[package]
name = "from_dir_test"
version = "0.1.0"
edition = "2026"
"#;
    std::fs::write(tmp.path().join("saturn.toml"), toml).expect("failed to write");

    let manifest = Manifest::from_dir(tmp.path()).expect("should load from dir");
    assert_eq!(manifest.package.name, "from_dir_test");
}

#[test]
fn test_manifest_resolve_entry_stn_first() {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let toml = r#"[package]
name = "resolve_test"
version = "0.1.0"
edition = "2026"

[build]
source = "src"
entry = "main"
"#;
    std::fs::write(tmp.path().join("saturn.toml"), toml).expect("failed to write");
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::write(tmp.path().join("src/main.stn"), "module main\n").unwrap();

    let manifest = Manifest::from_dir(tmp.path()).expect("should load");
    let entry = manifest.resolve_entry(tmp.path());
    assert_eq!(entry.file_name().unwrap(), "main.stn");
}

#[test]
fn test_manifest_resolve_entry_stnx_fallback() {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let toml = r#"[package]
name = "resolve_legacy"
version = "0.1.0"
edition = "2026"

[build]
source = "src"
entry = "main"
"#;
    std::fs::write(tmp.path().join("saturn.toml"), toml).expect("failed to write");
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::write(tmp.path().join("src/main.stnx"), "fn main() -> i64 { 0 }").unwrap();

    let manifest = Manifest::from_dir(tmp.path()).expect("should load");
    let entry = manifest.resolve_entry(tmp.path());
    assert_eq!(entry.file_name().unwrap(), "main.stnx");
}

#[test]
fn test_build_config_defaults() {
    let bc = saturn::manifest::BuildSection::default();
    assert_eq!(bc.source, "src");
    assert_eq!(bc.entry, "main");
}

#[test]
fn test_platform_aware_executable_naming() {
    if cfg!(windows) {
        assert_eq!(
            saturn::profile::executable_name("myapp"),
            "myapp.exe"
        );
    } else {
        assert_eq!(
            saturn::profile::executable_name("myapp"),
            "myapp"
        );
    }
}
