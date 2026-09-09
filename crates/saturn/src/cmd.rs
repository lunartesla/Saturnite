//! High-level toolchain commands: `init` and `doctor`.

use crate::manifest::Manifest;
use anyhow::Result;
use std::path::PathBuf;

/// Scaffold a new Saturnite project.
///
/// Creates `saturn.toml`, `src/`, and `src/main.stn` with canonical native
/// syntax.
///
/// `name` may be a simple project name (creates `./name/`) or a path
/// (`mkdir -p` style). The package name is derived from the final directory
/// component of the resolved path.
pub fn init_project(
    name: Option<&str>,
    in_place: bool,
    pkg_version: Option<&str>,
) -> Result<()> {
    let version = pkg_version.unwrap_or("0.1.0").to_string();
    let edition = "2026".to_string();

    let (project_dir, project_name) = if in_place {
        let dir = std::env::current_dir()?;
        let pkg = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("project")
            .to_string();
        (dir, pkg)
    } else {
        let name_str = name.unwrap_or("myproject");
        let dir = PathBuf::from(name_str);
        let pkg = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("myproject")
            .to_string();
        // Create parent directories if a path was given.
        if let Some(parent) = dir.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| anyhow::anyhow!("Failed to create parent directory: {}", e))?;
            }
        }
        (dir, pkg)
    };

    if project_dir.exists() && !in_place {
        // If the directory exists and is non-empty, error.
        if project_dir.read_dir()?.next().is_some() {
            anyhow::bail!(
                "Directory '{}' already exists and is not empty. Use --in-place to initialize in the current directory.",
                project_dir.display()
            );
        }
    }

    // Create directories
    let src_dir = project_dir.join("src");
    std::fs::create_dir_all(&src_dir)
        .map_err(|e| anyhow::anyhow!("Failed to create project directory: {}", e))?;

    // Write saturn.toml
    let manifest = Manifest {
        package: crate::manifest::Package {
            name: project_name.clone(),
            version: version.clone(),
            edition: edition.clone(),
        },
        dependencies: Default::default(),
        build: crate::manifest::BuildSection {
            source: "src".to_string(),
            entry: "main".to_string(),
        },
    };
    manifest.write_to(&project_dir)?;

    // Write src/main.stn (canonical native syntax)
    let main_content = "module main\n\nmain:\n    say \"Hello, Saturnite!\"\n    give 0\n";
    std::fs::write(src_dir.join("main.stn"), main_content)
        .map_err(|e| anyhow::anyhow!("Failed to write src/main.stn: {}", e))?;

    let display_dir = if in_place { "." } else { &project_name };

    println!("Created project '{}'", display_dir);
    println!("  |-- saturn.toml");
    println!("  |-- src/");
    println!("  |   |-- main.stn");
    println!();
    println!("To build:   saturn build");
    println!("To run:     saturn run");
    println!("To check:   saturn check");
    println!("To test:    saturn test");

    Ok(())
}

/// Print diagnostics about the compiler environment.
pub fn run_doctor() -> Result<()> {
    println!("Saturnite Toolchain Diagnostics");
    println!("===============================");
    println!();

    println!("Saturnite toolchain version: {}", env!("CARGO_PKG_VERSION"));
    println!();

    // Platform info
    println!("Platform:");
    println!("  OS:       {}", std::env::consts::OS);
    println!("  Arch:     {}", std::env::consts::ARCH);
    println!("  Triple:   {}", crate::profile::platform_triple());
    println!();

    // Compiler (stnx) info
    match stnx::codegen::host_triple() {
        Ok(triple) => {
            println!("Compiler (stnx) host triple: {}", triple);
        }
        Err(e) => {
            println!("WARNING: Failed to determine compiler host triple: {}", e);
        }
    }
    println!();

    // Linker availability
    match stnx::target::TargetConfig::host() {
        Ok(config) => {
            match stnx::codegen::check_linker(&config) {
                Ok(()) => println!("Linker: available"),
                Err(e) => println!("WARNING: Linker not available: {}", e),
            }
        }
        Err(e) => {
            println!("WARNING: Failed to initialize target config: {}", e);
        }
    }
    println!();

    println!("Runtime: compiled with saturnite-stdlib (stnx build output)");
    println!();

    Ok(())
}
