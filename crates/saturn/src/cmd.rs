//! High-level toolchain commands: `init`, `doctor`, `add`, `install`, `update`.

use crate::lockfile::Lockfile;
use crate::manifest::{DependencyKind, DependencySpec, Manifest};
use anyhow::Result;
use std::path::{Path, PathBuf};

/// Scaffold a new Saturnite project.
///
/// Creates `saturn.toml`, `src/`, and `src/main.stn` with canonical native
/// syntax.
///
/// `name` may be a simple project name (creates `./name/`) or a path
/// (`mkdir -p` style). The package name is derived from the final directory
/// component of the resolved path.
pub fn init_project(name: Option<&str>, in_place: bool, pkg_version: Option<&str>) -> Result<()> {
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
        Ok(config) => match stnx::codegen::check_linker(&config) {
            Ok(()) => println!("Linker: available"),
            Err(e) => println!("WARNING: Linker not available: {}", e),
        },
        Err(e) => {
            println!("WARNING: Failed to initialize target config: {}", e);
        }
    }
    println!();

    println!("Runtime: compiled with saturnite-stdlib (stnx build output)");
    println!();

    Ok(())
}

// ---------------------------------------------------------------------------
// Dependency management: add, install, update
// ---------------------------------------------------------------------------

/// Parse a dependency spec string from the CLI.
///
/// Accepted formats:
/// - `name` → Saturnite dep with "*" version
/// - `name@1.0` → Saturnite dep with exact version "1.0"
/// - `name>=1.0` → Saturnite dep with range ">=1.0"
/// - `rust:name` → Rust dep
/// - `rust:name@1.0` → Rust dep with version "1.0"
/// - `python:name>=1.0` → Python dep with range
/// - `native:name@1.0` → Native dep with version
/// - `saturnite:name@1.0` → Saturnite dep (explicit)
fn parse_dep_spec(input: &str) -> Result<(String, DependencySpec)> {
    // Split on the first ':' to separate kind from the rest.
    let (kind_str, rest) = match input.find(':') {
        Some(idx) => (&input[..idx], &input[idx + 1..]),
        None => ("saturnite", input),
    };

    let kind = match kind_str {
        "rust" => DependencyKind::Rust,
        "python" => DependencyKind::Python,
        "native" => DependencyKind::Native,
        "saturnite" => DependencyKind::Saturnite,
        other => {
            anyhow::bail!(
                "unknown dependency kind '{}'. Valid kinds: rust, python, native, saturnite. \
                 Or omit the kind prefix for a Saturnite dependency.",
                other
            );
        }
    };

    // Split the rest into name and version spec.
    let (name, version) =
        match rest.find(|c: char| c == '@' || c == '>' || c == '<' || c == '=' || c == '^') {
            Some(idx) => (&rest[..idx], Some(&rest[idx..])),
            None => (rest, None),
        };

    // Strip the leading '@' if present (e.g. "@1.0" → "1.0").
    let version_str = version
        .map(|v| v.strip_prefix('@').unwrap_or(v))
        .unwrap_or("*");
    let spec = DependencySpec::Full {
        kind,
        version: version_str.to_string(),
        target: None,
    };

    Ok((name.to_string(), spec))
}

/// Add a dependency to the project.
///
/// Resolves the dependency against its registry FIRST, then writes both the
/// manifest and lockfile only if resolution succeeds. This is transactional:
/// if resolution fails, the project files are left untouched.
pub fn add_dependency(project_root: &Path, dep_spec: &str) -> Result<()> {
    let mut manifest = Manifest::from_dir(project_root)?;
    let mut lockfile = Lockfile::load(project_root)?;

    let (name, spec) = parse_dep_spec(dep_spec)?;

    if manifest.dependencies.contains_key(&name) {
        anyhow::bail!(
            "dependency '{}' is already in saturn.toml. Use `saturn update {}` to change its version.",
            name, name
        );
    }

    // Stage the new dependency in the manifest (in memory only).
    manifest.dependencies.insert(name.clone(), spec.clone());

    // Resolve BEFORE writing anything to disk — if this fails, the project
    // files remain untouched.
    let resolved = lockfile.add(&manifest.dependencies, &name, &spec)?;

    // Now that resolution succeeded, persist both files.
    manifest.write_to(project_root)?;
    lockfile.save(project_root)?;

    println!(
        "Added dependency: {} ({:?}) {} → {}",
        name,
        spec.kind(),
        spec.version(),
        resolved.version
    );
    println!("  → saturn.toml updated");
    println!("  → saturn.lock updated");

    Ok(())
}

/// Install (resolve and prepare) all dependencies declared in the manifest.
///
/// For Rust deps, this generates wrapper crates and builds static libraries.
/// For Python deps, this creates virtual environments and pip-installs packages.
/// For Native deps, this registers them for linking.
pub fn install_dependencies(project_root: &Path, force: bool) -> Result<()> {
    let manifest = Manifest::from_dir(project_root)?;

    // Load or create the lockfile.
    let mut lockfile = Lockfile::load(project_root)?;

    if !force && !lockfile.is_stale(&manifest.dependencies) {
        println!("All dependencies are up to date (lockfile is current).");
        println!("  → {} dependencies locked", lockfile.dependencies.len());
        return Ok(());
    }

    if force {
        println!("Forcing re-resolution of all dependencies...");
    } else {
        println!("Resolving dependencies (lockfile is stale or missing)...");
    }

    // Resolve all dependencies into the lockfile.
    let prev_count = lockfile.dependencies.len();
    lockfile.resolve(&manifest.dependencies, force)?;

    // Save the lockfile.
    lockfile.save(project_root)?;

    // Report what changed.
    let new_count = lockfile.dependencies.len();
    if new_count != prev_count {
        let added = new_count.saturating_sub(prev_count);
        if added > 0 {
            println!("  → {} new dependencies resolved", added);
        }
    }

    // Prepare runtime artifacts for each dependency.
    let wrappers_dir = project_root
        .join("target")
        .join("saturnite")
        .join("wrappers");
    std::fs::create_dir_all(&wrappers_dir)
        .map_err(|e| anyhow::anyhow!("failed to create wrappers dir: {}", e))?;

    for (name, locked) in &lockfile.dependencies {
        match locked.kind {
            DependencyKind::Rust => {
                println!("  [rust]   {} → {}", name, locked.version);
                // Look for wrapper metadata; if present, build the wrapper.
                let meta_path = wrappers_dir.join(format!("{}_saturnite.json", name));
                if !meta_path.exists() {
                    eprintln!(
                        "  [rust]   WARNING: No wrapper metadata found for '{}'. \
                         The crate '{}' was resolved but cannot be used from Saturnite. \
                         Provide a wrapper metadata file at {} to expose its API.",
                        name,
                        name,
                        meta_path.display()
                    );
                    continue;
                }
                // Read the metadata and build the wrapper.
                let meta_contents = std::fs::read_to_string(&meta_path)
                    .map_err(|e| anyhow::anyhow!("failed to read wrapper metadata: {}", e))?;
                let metadata: serde_json::Value = serde_json::from_str(&meta_contents)
                    .map_err(|e| anyhow::anyhow!("invalid wrapper metadata JSON: {}", e))?;
                let crate_name = metadata["crate_name"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("wrapper metadata missing 'crate_name'"))?;

                let wrapper_meta = stnx::hir::lower::WrapperMetadata {
                    crate_name: crate_name.to_string(),
                    functions: metadata["functions"]
                        .as_array()
                        .ok_or_else(|| {
                            anyhow::anyhow!("wrapper metadata missing 'functions' array")
                        })?
                        .iter()
                        .map(|f| stnx::hir::lower::EcosystemFunctionMeta {
                            name: f["name"].as_str().unwrap_or("").to_string(),
                            symbol: f["symbol"].as_str().unwrap_or("").to_string(),
                            params: f["params"]
                                .as_array()
                                .map(|a| {
                                    a.iter()
                                        .map(|p| p.as_str().unwrap_or("").to_string())
                                        .collect()
                                })
                                .unwrap_or_default(),
                            ret: f["ret"].as_str().unwrap_or("").to_string(),
                        })
                        .collect(),
                };

                let target_triple = stnx::target::TargetConfig::host()
                    .map(|c| c.triple_str())
                    .unwrap_or_default();
                match stnx::rust_wrapper::build_rust_wrapper(
                    &wrapper_meta,
                    &locked.version,
                    &wrappers_dir,
                    Some(&target_triple),
                ) {
                    Ok(artifact) => {
                        println!("  [rust]   built wrapper → {}", artifact.display());
                    }
                    Err(e) => {
                        eprintln!("  [rust]   ERROR building wrapper for '{}': {}", name, e);
                    }
                }
            }
            DependencyKind::Python => {
                println!("  [python] {} → {}", name, locked.version);
                // Install into the project venv.
                match stnx::pyenv::PyEnv::create(project_root, &manifest.package.name) {
                    Ok(venv) => {
                        let spec = format!("{}=={}", name, locked.version);
                        if let Err(e) = venv.ensure_package(&spec) {
                            eprintln!(
                                "  [python] ERROR installing '{}=={}': {}",
                                name, locked.version, e
                            );
                        } else {
                            println!("  [python] installed {}=={}", name, locked.version);
                        }
                    }
                    Err(e) => {
                        eprintln!(
                            "  [python] WARNING: failed to create Python venv for '{}': {}",
                            name, e
                        );
                    }
                }
            }
            DependencyKind::Native => {
                println!("  [native] {} → {} (system library)", name, locked.version);
                // Native deps are linked via system libraries; no installation needed.
            }
            DependencyKind::Saturnite => {
                println!("  [saturnite] {} → {}", name, locked.version);
            }
        }
    }

    println!(
        "\nLockfile written to {} ({} dependencies)",
        lockfile_path(project_root).display(),
        new_count
    );

    Ok(())
}

/// Update dependencies to their latest compatible versions.
///
/// - If `dep_name` is Some, only that dependency is updated.
/// - If `all` is true, all dependencies are updated.
/// - If neither, prints usage.
pub fn update_dependencies(project_root: &Path, dep_name: Option<&str>, all: bool) -> Result<()> {
    let manifest = Manifest::from_dir(project_root)?;
    let mut lockfile = Lockfile::load(project_root)?;

    if !all && dep_name.is_none() {
        anyhow::bail!(
            "specify a dependency name or use --all:\n  saturn update <name>\n  saturn update --all"
        );
    }

    if all {
        // Re-resolve everything: force=true bypasses cached lockfile entries.
        for (name, _) in &manifest.dependencies {
            if let Some(locked) = lockfile.dependencies.get(name) {
                println!("  re-resolving {} (was {})", name, locked.version);
            } else {
                println!("  added {}", name);
            }
        }
        lockfile.resolve(&manifest.dependencies, true)?;
    } else if let Some(name) = dep_name {
        let spec = manifest
            .dependencies
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("dependency '{}' is not in saturn.toml", name))?;
        let resolved_before = lockfile
            .dependencies
            .get(name)
            .map(|d| d.version.clone())
            .unwrap_or_else(|| "(not locked)".to_string());
        let resolved = lockfile.update(&manifest.dependencies, name, spec)?;
        println!(
            "  updated {}: {} → {}",
            name, resolved_before, resolved.version
        );
    }

    lockfile.save(project_root)?;
    println!("\nLockfile updated.");
    Ok(())
}

/// Path to the lockfile within a project root.
pub fn lockfile_path(project_root: &Path) -> PathBuf {
    crate::lockfile::lockfile_path(project_root)
}
