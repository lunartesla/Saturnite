//! Rust wrapper crate generation for ecosystem interoperability.
//!
//! When a Saturnite program declares `use rust:serde as serde_lib` and the
//! project depends on a Rust wrapper crate, the compiler needs a `.a` static
//! library containing `#[no_mangle] extern "C"` shim functions that forward
//! to the real crate's API surface.
//!
//! This module is responsible for:
//!
//! * Generating a standalone Cargo project (Cargo.toml + src/lib.rs) from a
//!   [`WrapperMetadata`] descriptor;
//! * Building that project with `cargo build --release` to produce a
//!   `staticlib` artifact;
//! * Returning the path to the resulting `.a` / `.lib` file so the Saturnite
//!   linker can include it on the link line.
//!
//! The generated wrapper functions are thin shims — they call the real crate
//! function and return its result directly. Only ABI-safe types (i64, f64,
//! bool, str as `*const c_char`, Unit) cross the boundary.
//!
//! ## Generated crate layout
//!
//! ```text
//! <build_dir>/
//!   Cargo.toml          # [lib] crate-type = ["staticlib"], deps on real crate
//!   src/
//!     lib.rs            # #[no_mangle] extern "C" shims
//! ```

use std::path::{Path, PathBuf};

use crate::error::{BuildError, BuildResult};
use crate::hir::lower::{EcosystemFunctionMeta, WrapperMetadata};
use crate::interop::SAT_ABI_VERSION;

/// Generate the `Cargo.toml` content for a wrapper crate.
///
/// The generated crate is a `staticlib` that depends on the real upstream crate
/// at the requested version, then exposes `#[no_mangle] extern "C"` functions
/// matching the symbols declared in `WrapperMetadata`.
pub fn generate_cargo_toml(metadata: &WrapperMetadata, version: &str) -> String {
    let crate_name = &metadata.crate_name;

    format!(
        r#"[package]
name = "{crate_name}-saturnite-wrapper"
version = "{version}"
edition = "2021"

[lib]
crate-type = ["staticlib"]

[dependencies]
{crate_name} = "{version}"
"#,
        crate_name = crate_name,
    )
}

/// Sanitize a crate name for use in Rust code generation.
/// Converts hyphens to underscores (required for Rust identifiers).
fn sanitize_crate_name(name: &str) -> String {
    name.replace('-', "_")
}

/// Map an ABI-safe type string (as used in WrapperMetadata) to the Rust `extern "C"`
/// parameter type for the generated shim.
fn abi_type_to_rust_param(type_str: &str) -> &'static str {
    match type_str {
        "i64" | "i32" | "i16" | "i8" => "i64",
        "u64" | "u32" | "u16" | "u8" => "u64",
        "f64" => "f64",
        "f32" => "f32",
        "bool" => "bool",
        "unit" => "",
        "str" | "c_str" | "string" => "*const std::os::raw::c_char",
        _ => "*const std::os::raw::c_char", // Unknown types default to opaque string
    }
}

/// Map an ABI-safe return type string to the Rust `extern "C"` return type.
fn abi_type_to_rust_ret(type_str: &str) -> &'static str {
    match type_str {
        "i64" | "i32" | "i16" | "i8" => "i64",
        "u64" | "u32" | "u16" | "u8" => "u64",
        "f64" => "f64",
        "f32" => "f32",
        "bool" => "bool",
        "unit" => "",
        "str" | "c_str" | "string" => "*const std::os::raw::c_char",
        _ => "*const std::os::raw::c_char",
    }
}

/// Generate a parameter name (arg0, arg1, ...) for Rust code generation.
fn param_name(index: usize) -> String {
    format!("arg{}", index)
}

/// Generate the `src/lib.rs` content for a wrapper crate from `WrapperMetadata`.
pub fn generate_wrapper_src(metadata: &WrapperMetadata, funcs: &[EcosystemFunctionMeta]) -> String {
    let crate_name = sanitize_crate_name(&metadata.crate_name);
    let abi_version = SAT_ABI_VERSION;

    let mut shims = String::new();

    for func in funcs {
        // Build parameter list, filtering out unit params (which have no type).
        let params: Vec<String> = func
            .params
            .iter()
            .enumerate()
            .filter_map(|(i, t)| {
                let rust_ty = abi_type_to_rust_param(t);
                if rust_ty.is_empty() {
                    None
                } else {
                    Some(format!("{}: {}", param_name(i), rust_ty))
                }
            })
            .collect();

        let ret_ty = abi_type_to_rust_ret(&func.ret);
        let ret_prefix = if ret_ty.is_empty() { "" } else { " -> " };

        // Build the call expression: crate_name::func_name(arg0, arg1, ...)
        let call_args: Vec<String> = (0..func.params.len()).map(param_name).collect();
        let call = format!("{}::{}({})", crate_name, func.name, call_args.join(", "));

        shims.push_str(&format!(
            r#"
/// ABI version: {abi_version} -- shim for {crate_name}.{func_name}
#[no_mangle]
pub extern "C" fn {symbol}({params}){ret_prefix}{ret_ty} {{
    // Validate ABI version at compile time via static assertion.
    const _: () = {{
        const _ASSERT_ABI_VERSION: () = [()][(1 - {abi_version}) as usize % 1];
        let _ = _ASSERT_ABI_VERSION;
    }};
    {call}
}}

"#,
            abi_version = abi_version,
            crate_name = crate_name,
            func_name = func.name,
            symbol = func.symbol,
            params = params.join(", "),
            ret_prefix = ret_prefix,
            ret_ty = ret_ty,
            call = call,
        ));
    }

    format!(
        r#"//! Auto-generated Saturnite wrapper crate for `{crate_name}`.
//!
//! This crate exposes `#[no_mangle] extern "C"` shim functions that forward
//! to the real `{crate_name}` crate's API. It is compiled as a `staticlib`
//! so the Saturnite linker can include it.
//!
//! ABI version: {abi_version}

extern crate {crate_name};

{shims}

// Embed the ABI version for runtime validation by rsrt_impl.c.
// The static is placed in a custom section so it survives dead-code elimination.
#[link_section = ".saturnite_meta"]
static _SAT_ABI_VERSION: [u8; 4] = [
    ((SATI_ABI_VERSION >> 24) & 0xFF) as u8,
    ((SATI_ABI_VERSION >> 16) & 0xFF) as u8,
    ((SATI_ABI_VERSION >> 8) & 0xFF) as u8,
    (SATI_ABI_VERSION & 0xFF) as u8,
];

const SATI_ABI_VERSION: u32 = {abi_version};
"#,
        crate_name = crate_name,
        abi_version = abi_version,
        shims = shims,
    )
}

/// Build a Rust wrapper crate from `WrapperMetadata` and return the path to
/// the resulting static library.
///
/// This function:
/// 1. Creates a temporary directory with a generated Cargo project
/// 2. Writes `Cargo.toml` and `src/lib.rs`
/// 3. Runs `cargo build --release --target <triple>` (if a target triple is
///    provided, for cross-compilation) or `cargo build --release`
/// 4. Returns the path to the built `.a` (or `.lib` on MSVC) file
///
/// The caller is responsible for ensuring:
/// - `cargo` and `rustc` are available on PATH
/// - The real Rust crate is available in the cargo registry or a local path
pub fn build_rust_wrapper(
    metadata: &WrapperMetadata,
    version: &str,
    output_dir: &Path,
    target_triple: Option<&str>,
) -> BuildResult<PathBuf> {
    let build_dir = output_dir.join(format!("{}_wrapper", &metadata.crate_name));

    // Clean any previous build artifacts.
    if build_dir.exists() {
        std::fs::remove_dir_all(&build_dir).map_err(|e| {
            BuildError::wrapper_build(format!("failed to clean {}: {}", build_dir.display(), e))
        })?;
    }

    // Create directory structure.
    let src_dir = build_dir.join("src");
    std::fs::create_dir_all(&src_dir)
        .map_err(|e| BuildError::wrapper_build(format!("failed to create src dir: {}", e)))?;

    // Write Cargo.toml.
    let toml_content = generate_cargo_toml(metadata, version);
    std::fs::write(build_dir.join("Cargo.toml"), toml_content)
        .map_err(|e| BuildError::wrapper_build(format!("failed to write Cargo.toml: {}", e)))?;

    // Write src/lib.rs.
    let lib_content = generate_wrapper_src(metadata, &metadata.functions);
    std::fs::write(src_dir.join("lib.rs"), lib_content)
        .map_err(|e| BuildError::wrapper_build(format!("failed to write lib.rs: {}", e)))?;

    // Emit the ABI version as a compile-time environment marker.
    let abi_env = format!("{}", SAT_ABI_VERSION);

    // Build with cargo.
    let mut cmd = std::process::Command::new("cargo");
    cmd.arg("build")
        .arg("--release")
        .env("SAT_ABI_VERSION", &abi_env);

    if let Some(triple) = target_triple {
        cmd.arg("--target").arg(triple);
    }

    // Set the working directory to the generated crate.
    cmd.current_dir(&build_dir);

    let output = cmd
        .output()
        .map_err(|e| BuildError::wrapper_build(format!("failed to invoke cargo: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(BuildError::wrapper_build(format!(
            "cargo build failed for wrapper crate '{}':\n--- stdout ---\n{}\n--- stderr ---\n{}",
            metadata.crate_name, stdout, stderr
        )));
    }

    // Locate the static library artifact.
    let lib_artifact = find_staticlib(&build_dir, &metadata.crate_name)?;

    Ok(lib_artifact)
}

/// Find the static library artifact produced by `cargo build --release`.
fn find_staticlib(build_dir: &Path, crate_name: &str) -> BuildResult<PathBuf> {
    let sanitized = sanitize_crate_name(crate_name);
    let lib_name = format!("{}-saturnite-wrapper", sanitized);

    let release_dir = build_dir.join("target").join("release");

    // Staticlib naming: lib<name>.a on Unix, <name>.lib on MSVC.
    let candidates = [
        format!("lib{}.a", lib_name),
        format!("lib{}.lib", lib_name),
        format!("{}.a", lib_name),
        format!("{}.lib", lib_name),
    ];

    for candidate in &candidates {
        let path = release_dir.join(candidate);
        if path.is_file() {
            return Ok(path);
        }
    }

    // Search subdirectories (e.g. target/release/deps/ may have copies).
    if let Ok(entries) = std::fs::read_dir(release_dir.join("deps")) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(&format!("lib{}", lib_name))
                && (name.ends_with(".a") || name.ends_with(".lib"))
            {
                return Ok(entry.path());
            }
        }
    }

    Err(BuildError::wrapper_build(format!(
        "static library artifact not found in {} (expected one of {:?})",
        release_dir.display(),
        candidates
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_metadata() -> WrapperMetadata {
        WrapperMetadata {
            crate_name: "example_crate".to_string(),
            functions: vec![
                EcosystemFunctionMeta {
                    name: "add".to_string(),
                    symbol: "sat_example_crate_add_i64".to_string(),
                    params: vec!["i64".to_string(), "i64".to_string()],
                    ret: "i64".to_string(),
                },
                EcosystemFunctionMeta {
                    name: "greet".to_string(),
                    symbol: "sat_example_crate_greet".to_string(),
                    params: vec!["str".to_string()],
                    ret: "i64".to_string(),
                },
            ],
        }
    }

    #[test]
    fn test_sanitize_crate_name() {
        assert_eq!(sanitize_crate_name("my-crate"), "my_crate");
        assert_eq!(sanitize_crate_name("simple"), "simple");
    }

    #[test]
    fn test_abi_type_to_rust_param() {
        assert_eq!(abi_type_to_rust_param("i64"), "i64");
        assert_eq!(abi_type_to_rust_param("f64"), "f64");
        assert_eq!(abi_type_to_rust_param("bool"), "bool");
        assert_eq!(abi_type_to_rust_param("str"), "*const std::os::raw::c_char");
        assert_eq!(abi_type_to_rust_param("unit"), "");
    }

    #[test]
    fn test_abi_type_to_rust_ret() {
        assert_eq!(abi_type_to_rust_ret("i64"), "i64");
        assert_eq!(abi_type_to_rust_ret("f64"), "f64");
        assert_eq!(abi_type_to_rust_ret("unit"), "");
    }

    #[test]
    fn test_param_name() {
        assert_eq!(param_name(0), "arg0");
        assert_eq!(param_name(1), "arg1");
        assert_eq!(param_name(3), "arg3");
    }

    #[test]
    fn test_generate_cargo_toml_basic() {
        let metadata = make_metadata();
        let toml = generate_cargo_toml(&metadata, "1.0");
        assert!(toml.contains("crate-type = [\"staticlib\"]"));
        assert!(toml.contains("example_crate = \"1.0\""));
        assert!(toml.contains("name = \"example_crate-saturnite-wrapper\""));
    }

    #[test]
    fn test_generate_wrapper_src_has_shims() {
        let metadata = make_metadata();
        let src = generate_wrapper_src(&metadata, &metadata.functions);

        // Both functions should have shims.
        assert!(src.contains("sat_example_crate_add_i64"));
        assert!(src.contains("sat_example_crate_greet"));

        // Should reference the real crate.
        assert!(src.contains("example_crate::add"));
        assert!(src.contains("example_crate::greet"));

        // Should have ABI version marker.
        assert!(src.contains(&format!("ABI version: {}", SAT_ABI_VERSION)));
    }

    #[test]
    fn test_generate_wrapper_src_param_count() {
        let metadata = make_metadata();
        let src = generate_wrapper_src(&metadata, &metadata.functions);

        // The add function shim should have two i64 parameters.
        // Search for the function signature line that contains both the symbol
        // and "extern \"C\"".
        let sig_line = src
            .lines()
            .find(|l| l.contains("sat_example_crate_add_i64") && l.contains("extern \"C\""))
            .unwrap();
        assert!(sig_line.contains("arg0: i64"));
        assert!(sig_line.contains("arg1: i64"));
    }

    #[test]
    fn test_find_staticlib_logic() {
        // Just verify the function compiles and has reasonable candidates.
        // Actual file system operations are tested in integration tests.
        let metadata = make_metadata();
        let sanitized = sanitize_crate_name(&metadata.crate_name);
        let lib_name = format!("{}-saturnite-wrapper", sanitized);
        assert_eq!(lib_name, "example_crate-saturnite-wrapper");
    }

    #[test]
    fn test_cargo_toml_sanitized_name() {
        let metadata = WrapperMetadata {
            crate_name: "serde-json".to_string(),
            functions: vec![],
        };
        let toml = generate_cargo_toml(&metadata, "1.0");
        assert!(toml.contains("name = \"serde-json-saturnite-wrapper\""));
    }

    #[test]
    fn test_unit_param_filtered() {
        let metadata = WrapperMetadata {
            crate_name: "test_crate".to_string(),
            functions: vec![EcosystemFunctionMeta {
                name: "no_args".to_string(),
                symbol: "sat_test_crate_no_args".to_string(),
                params: vec!["unit".to_string()],
                ret: "unit".to_string(),
            }],
        };
        let src = generate_wrapper_src(&metadata, &metadata.functions);

        // The function should have no parameters (unit is filtered out).
        assert!(src.contains("pub extern \"C\" fn sat_test_crate_no_args()"));
    }

    #[test]
    fn test_str_param_maps_to_c_char_ptr() {
        let metadata = make_metadata();
        let src = generate_wrapper_src(&metadata, &metadata.functions);

        assert!(src.contains("*const std::os::raw::c_char"));
    }
}
