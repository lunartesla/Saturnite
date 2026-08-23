use clap::{Parser, Subcommand};
use std::path::PathBuf;
use stnx::codegen;
use stnx::mir::codegen::{compile_from_mir_ext, generate_ir_from_mir};
use stnx::mir::lower::lower_program;
use stnx::mir::opt::optimize;
use stnx::module::Project;
use stnx::target::{DebugInfo, OptimizationLevel, OutputKind, Profile, TargetConfig};
// `CompilerError` carries the variants that `render_diagnostic` pattern-matches on.
use stnx::CompilerError;

/// Saturnite programming language compiler
#[derive(Parser)]
#[command(name = "saturnite")]
#[command(about = "Saturnite programming language compiler", long_about = None)]
#[command(version, propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build a .sat source file to an executable (or emit IR / object / exe)
    Build {
        /// Input source file
        #[arg(value_name = "FILE", required = false)]
        input: Option<PathBuf>,

        /// Output executable path (default: target/<profile>/<name>)
        #[arg(short, long, value_name = "FILE")]
        output: Option<PathBuf>,

        /// Cross-compilation target triple (e.g. x86_64-pc-windows-msvc)
        #[arg(long, value_name = "TRIPLE")]
        target: Option<String>,

        /// Emit LLVM IR text to the given file
        #[arg(long, value_name = "FILE")]
        emit_ir: Option<PathBuf>,

        /// Emit a relocatable object file to the given path
        #[arg(long, value_name = "FILE")]
        emit_object: Option<PathBuf>,

        /// Emit an executable to the given path
        #[arg(long, value_name = "FILE")]
        emit_exe: Option<PathBuf>,

        /// Print the host target triple and exit
        #[arg(long)]
        print_target: bool,

        /// Debug build (default: low optimization, debug info)
        #[arg(long, conflicts_with = "release")]
        debug: bool,

        /// Release build (optimization enabled, no debug info)
        #[arg(long, conflicts_with = "debug")]
        release: bool,

        /// Set optimization level explicitly (0-3). Overrides --debug/--release
        #[arg(long, value_name = "LEVEL")]
        opt_level: Option<u8>,

        /// Output structured JSON build report
        #[arg(long)]
        json: bool,

        /// Verbose output
        #[arg(short, long)]
        verbose: bool,

        /// Emit object file only; skip the linking step
        #[arg(long)]
        no_link: bool,

        /// Keep intermediate object files (do not delete after linking)
        #[arg(long)]
        save_temps: bool,
    },

    /// Check a source file for type and semantic errors without generating code
    Check {
        /// Input source file (defaults to src/main.stnx)
        #[arg(value_name = "FILE", required = false)]
        input: Option<PathBuf>,

        /// Cross-compilation target triple (affects target-dependent checks)
        #[arg(long, value_name = "TRIPLE")]
        target: Option<String>,
    },

    /// Run a source file directly (build to a temp dir, then execute)
    Run {
        /// Input source file (defaults to src/main.stnx)
        #[arg(value_name = "FILE", required = false)]
        input: Option<PathBuf>,

        /// Debug build
        #[arg(long, conflicts_with = "release")]
        debug: bool,

        /// Release build
        #[arg(long, conflicts_with = "debug")]
        release: bool,

        /// Cross-compilation target triple (affects codegen)
        #[arg(long, value_name = "TRIPLE")]
        target: Option<String>,
    },

    /// Print diagnostics about the compiler environment
    Doctor,

    /// Create a new Saturnite project with scaffolding
    Init {
        /// Directory name for the new project (will be created if it doesn't exist)
        #[arg(value_name = "NAME", required = false)]
        name: Option<String>,

        /// Create the project in the current directory instead of a subdirectory
        #[arg(short, long, default_value_t = false)]
        in_place: bool,

        /// Package version string (default: 0.1.0)
        #[arg(short, long, value_name = "VERS")]
        pkg_version: Option<String>,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Build {
            input,
            output,
            target,
            emit_ir,
            emit_object,
            emit_exe,
            print_target,
            debug,
            release,
            opt_level,
            json,
            verbose,
            no_link,
            save_temps,
        } => {
            // --print-target short-circuits before we need a file.
            if print_target {
                let triple = codegen::host_triple()
                    .map_err(|e| anyhow::anyhow!("failed to get host triple: {}", e))?;
                println!("{}", triple);
                return Ok(());
            }

            // Determine profile from flags
            let profile = if release {
                Profile::Release
            } else if debug {
                Profile::Debug
            } else {
                Profile::default()
            };

            // Resolve the entry point: when no input file is given, discover
            // the project from the current directory and use its default entry
            // (src/main.stnx). When a file is given, behavior is unchanged.
            let (entry_path, package_name) = match &input {
                Some(input) => (input.clone(), None),
                None => {
                    let cwd = std::env::current_dir()?;
                    let project = Project::discover(&cwd)?;
                    let entry = project.source_root.join("main.stnx");
                    if !entry.is_file() {
                        return Err(anyhow::anyhow!(
                            "no entry point found: expected {} (create a saturn.toml project with src/main.stnx or pass a file explicitly)",
                            entry.display()
                        ));
                    }
                    let pkg = project.config.package.name.clone();
                    (entry, Some(pkg))
                }
            };

            // Validate that at most one emit mode is selected.
            let emit_count =
                emit_ir.is_some() as u8 + emit_object.is_some() as u8 + emit_exe.is_some() as u8;
            if emit_count > 1 {
                return Err(anyhow::anyhow!(
                    "at most one of --emit-ir, --emit-object, --emit-exe may be specified"
                ));
            }
            if no_link && (emit_ir.is_some() || emit_exe.is_some()) {
                return Err(anyhow::anyhow!(
                    "--no-link can only be used with --emit-object or default (exe) output"
                ));
            }

            // Build target configuration and apply the profile defaults
            // (optimization level + debug-info).  An explicit `--opt-level`
            // override is applied afterwards so it still takes precedence.
            let mut config = if let Some(triple) = &target {
                TargetConfig::from_triple(triple)
                    .map_err(|e| anyhow::anyhow!("Invalid target '{}': {}", triple, e))?
            } else {
                TargetConfig::host()
                    .map_err(|e| anyhow::anyhow!("Failed to initialize native target: {}", e))?
            };
            config.apply_profile(profile);

            match opt_level {
                Some(0) => {
                    config.set_opt_level(OptimizationLevel::None);
                    config.set_debug_info(DebugInfo::Yes);
                }
                Some(1) => config.set_opt_level(OptimizationLevel::Less),
                Some(2) => config.set_opt_level(OptimizationLevel::Default),
                Some(3) => config.set_opt_level(OptimizationLevel::Aggressive),
                Some(_) => {
                    return Err(anyhow::anyhow!(
                        "optimization level must be between 0 and 3"
                    ))
                }
                None => {} // profile defaults already applied above
            }

            // Determine output path and emit mode
            let (emit_path, output_kind) = resolve_output(
                &entry_path,
                &output,
                &profile,
                package_name.as_deref(),
                emit_ir,
                emit_object,
                emit_exe,
                no_link,
            );

            if verbose {
                eprintln!("target: {}", config.triple_str());
                eprintln!("profile: {}", profile.as_str());
                eprintln!("output: {}", emit_path.display());
            }

            let mut project = Project::discover(&entry_path)?;
            let program = if input.is_some() {
                project.load_from(&entry_path)?
            } else {
                project.load()?
            };
            let hir = stnx::semantic::analyze_and_lower(&program).map_err(render_diagnostic)?;

            // Lower HIR → MIR (the single production codegen seam).
            let mut mir =
                lower_program(&hir).map_err(|e| anyhow::anyhow!("MIR lowering failed: {}", e))?;

            // Verify the MIR CFG before handing it to LLVM.
            if let Err(errs) = mir.verify() {
                let msgs: Vec<String> = errs.iter().map(|e| e.to_string()).collect();
                return Err(anyhow::anyhow!(
                    "MIR verification failed: {}",
                    msgs.join(", ")
                ));
            }

            // Apply any MIR-level optimizations that exist.
            optimize(&mut mir);

            config.set_output_kind(output_kind);

            // Cross-compilation guard: the Saturnite runtime is compiled via
            // build.rs from C source targeting the *host* platform only. If the
            // user requests a target that differs from the host, the resulting
            // binary would be linked against an incompatible runtime object, so
            // we fail clearly instead of producing a broken executable.
            if let Some(ref requested) = target {
                let host_triple = codegen::host_triple()
                    .map_err(|e| anyhow::anyhow!("Failed to determine host triple: {}", e))?;
                if requested != &host_triple {
                    return Err(anyhow::anyhow!(
                        "Cross-compilation to '{}' is not yet supported in Saturnite 0.2.\n\
                         The runtime is compiled for the host target only.\n\
                         Requested target: {}\n\
                         Host target:      {}",
                        requested,
                        requested,
                        host_triple
                    ));
                }
            }

            // Ensure the parent directory of the output path exists.
            if let Some(parent) = emit_path.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        anyhow::anyhow!(
                            "Failed to create output directory '{}': {}",
                            parent.display(),
                            e
                        )
                    })?;
                }
            }

            let start = std::time::Instant::now();

            match output_kind {
                OutputKind::Ir => {
                    let ir = generate_ir_from_mir(&mir)
                        .map_err(|e| anyhow::anyhow!("IR generation failed: {}", e))?;
                    std::fs::write(&emit_path, ir).map_err(|e| {
                        anyhow::anyhow!("Failed to write IR to {}: {}", emit_path.display(), e)
                    })?;
                }
                _ => {
                    compile_from_mir_ext(&mir, emit_path.to_str().unwrap(), config, save_temps)
                        .map_err(|e| anyhow::anyhow!("Compilation failed: {}", e))?;
                }
            }

            let elapsed = start.elapsed();

            if json {
                let size_bytes = std::fs::metadata(&emit_path).ok().map(|m| m.len());
                let artifact = ArtifactInfo {
                    output_path: emit_path.to_string_lossy().to_string(),
                    kind: match output_kind {
                        OutputKind::Ir => "ir",
                        OutputKind::Object => "object",
                        OutputKind::Exe => "executable",
                    },
                    target: {
                        if let Some(ref triple) = target {
                            triple.clone()
                        } else {
                            codegen::host_triple().unwrap_or_default()
                        }
                    },
                    profile: profile.as_str().to_string(),
                    elapsed_ms: elapsed.as_millis() as u64,
                    size_bytes,
                };
                let report = BuildReport {
                    success: true,
                    artifacts: vec![artifact],
                    errors: vec![],
                };
                println!("{}", serde_json::to_string_pretty(&report).unwrap());
            } else {
                println!("Built {} -> {}", entry_path.display(), emit_path.display());
                if verbose {
                    println!("({} ms)", elapsed.as_millis());
                }
            }

            Ok(())
        }

        Commands::Check { input, target: _ } => {
            let entry = if let Some(ref input) = input {
                input.clone()
            } else {
                let cwd = std::env::current_dir()?;
                let project = Project::discover(&cwd)?;
                let entry = project.source_root.join("main.stnx");
                if !entry.is_file() {
                    return Err(anyhow::anyhow!(
                        "no entry point found: expected {} (create a saturn.toml project with src/main.stnx or pass a file explicitly)",
                        entry.display()
                    ));
                }
                entry
            };
            check_file(&entry)?;
            println!("No errors found in {}", entry.display());
            Ok(())
        }

        Commands::Run {
            input,
            debug,
            release,
            target,
        } => {
            let profile = if release {
                Profile::Release
            } else if debug {
                Profile::Debug
            } else {
                Profile::default()
            };

            let entry = if let Some(ref input) = input {
                input.clone()
            } else {
                let cwd = std::env::current_dir()?;
                let project = Project::discover(&cwd)?;
                let entry = project.source_root.join("main.stnx");
                if !entry.is_file() {
                    return Err(anyhow::anyhow!(
                        "no entry point found: expected {} (create a saturn.toml project with src/main.stnx or pass a file explicitly)",
                        entry.display()
                    ));
                }
                entry
            };

            let tmp_output = std::env::temp_dir().join(format!(
                "saturnite_run_{}_{}",
                std::process::id(),
                profile.as_str()
            ));
            let _ = build_run_file(&entry, &tmp_output, target.as_deref(), profile)?;
            let status = std::process::Command::new(&tmp_output)
                .status()
                .map_err(|e| anyhow::anyhow!("failed to execute: {}", e))?;
            let _ = std::fs::remove_file(&tmp_output);
            std::process::exit(status.code().unwrap_or(0));
        }

        Commands::Doctor => {
            run_doctor()?;
            Ok(())
        }

        Commands::Init {
            name,
            in_place,
            pkg_version,
        } => {
            init_project(name, in_place, pkg_version)?;
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Output path resolution
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn resolve_output(
    input: &std::path::Path,
    output: &Option<PathBuf>,
    profile: &Profile,
    package_name: Option<&str>,
    emit_ir: Option<PathBuf>,
    emit_object: Option<PathBuf>,
    emit_exe: Option<PathBuf>,
    no_link: bool,
) -> (PathBuf, OutputKind) {
    let stem = package_name
        .or_else(|| input.file_stem().and_then(|s| s.to_str()))
        .unwrap_or("out");

    if let Some(path) = emit_ir {
        (path, OutputKind::Ir)
    } else if let Some(path) = emit_object {
        (path, OutputKind::Object)
    } else if let Some(path) = emit_exe {
        (path, OutputKind::Exe)
    } else if let Some(path) = output {
        // --no-link with -o: emit an object file to the specified path.
        if no_link {
            (path.clone(), OutputKind::Object)
        } else {
            (path.clone(), OutputKind::Exe)
        }
    } else if no_link {
        // --no-link: emit object file to target/<profile>/<name>.o
        let target_dir = PathBuf::from("target").join(profile.as_str());
        (target_dir.join(format!("{}.o", stem)), OutputKind::Object)
    } else {
        // Default: target/<profile>/<name>
        let target_dir = PathBuf::from("target").join(profile.as_str());
        (target_dir.join(stem), OutputKind::Exe)
    }
}

// ---------------------------------------------------------------------------
// Core helpers
// ---------------------------------------------------------------------------

fn build_run_file(
    input: &std::path::Path,
    output: &std::path::Path,
    target_triple: Option<&str>,
    profile: Profile,
) -> anyhow::Result<std::path::PathBuf> {
    let mut project = Project::discover(input)?;
    let program = project.load_from(input)?;
    let hir = stnx::semantic::analyze_and_lower(&program)
        .map_err(|e| anyhow::anyhow!("Semantic error: {}", e))?;

    // Lower HIR → MIR (the single production codegen seam).
    let mut mir = lower_program(&hir).map_err(|e| anyhow::anyhow!("MIR lowering failed: {}", e))?;

    if let Err(errs) = mir.verify() {
        let msgs: Vec<String> = errs.iter().map(|e| e.to_string()).collect();
        return Err(anyhow::anyhow!(
            "MIR verification failed: {}",
            msgs.join(", ")
        ));
    }

    optimize(&mut mir);

    let mut config = if let Some(triple) = target_triple {
        TargetConfig::from_triple(triple)
            .map_err(|e| anyhow::anyhow!("Invalid target '{}': {}", triple, e))?
    } else {
        TargetConfig::host()
            .map_err(|e| anyhow::anyhow!("Failed to initialize native target: {}", e))?
    };

    config.apply_profile(profile);

    // Cross-compilation guard: the runtime is host-only (see Build command).
    if let Some(requested) = target_triple {
        let host_triple = codegen::host_triple()
            .map_err(|e| anyhow::anyhow!("Failed to determine host triple: {}", e))?;
        if requested != host_triple {
            return Err(anyhow::anyhow!(
                "Cross-compilation to '{}' is not yet supported in Saturnite 0.2.\n\
                 The runtime is compiled for the host target only.\n\
                 Requested target: {}\n\
                 Host target:      {}",
                requested,
                requested,
                host_triple
            ));
        }
    }

    config.set_output_kind(OutputKind::Exe);

    compile_from_mir_ext(&mir, output.to_str().unwrap(), config, false)
        .map_err(|e| anyhow::anyhow!("Compilation failed: {}", e))?;

    Ok(output.to_path_buf())
}

fn check_file(input: &std::path::Path) -> anyhow::Result<()> {
    let mut project = Project::discover(input)?;
    let program = project.load_from(input)?;
    stnx::semantic::analyze(&program).map_err(|e| anyhow::anyhow!("Semantic error: {}", e))?;

    Ok(())
}

/// Scaffold a new Saturnite project: creates `saturn.toml`, `src/` directory,
/// and a minimal `src/main.stnx` entry point.
fn init_project(
    name: Option<String>,
    in_place: bool,
    pkg_version: Option<String>,
) -> anyhow::Result<()> {
    let project_name = name.unwrap_or_else(|| "myproject".to_string());
    let version = pkg_version.unwrap_or_else(|| "0.1.0".to_string());

    let project_dir = if in_place {
        std::env::current_dir()
            .map_err(|e| anyhow::anyhow!("Failed to get current directory: {}", e))?
    } else {
        std::path::PathBuf::from(&project_name)
    };

    if project_dir.exists() && !in_place {
        return Err(anyhow::anyhow!(
            "Directory '{}' already exists. Use --in-place to initialize in the current directory.",
            project_dir.display()
        ));
    }

    // Create directories
    let src_dir = project_dir.join("src");
    std::fs::create_dir_all(&src_dir).map_err(|e| {
        anyhow::anyhow!(
            "Failed to create project directory '{}': {}",
            project_dir.display(),
            e
        )
    })?;

    // Write saturn.toml
    let toml_content = format!(
        "[package]\nname = \"{}\"\nversion = \"{}\"\nedition = \"2026\"\n\n[dependencies]\n# saturnite-stdlib = \"0.1\"\n",
        project_name, version
    );
    std::fs::write(project_dir.join("saturn.toml"), toml_content)
        .map_err(|e| anyhow::anyhow!("Failed to write saturn.toml: {}", e))?;

    // Write default src/main.stnx
    let main_content = "// Generated by `saturn init`.\n//\n// A simple Saturnite program:\n\nfn main() -> i64 {\n    println(42)\n    return 0\n}\n";
    std::fs::write(src_dir.join("main.stnx"), main_content)
        .map_err(|e| anyhow::anyhow!("Failed to write src/main.stnx: {}", e))?;

    let display_dir = if in_place { "." } else { &project_name };

    println!("Created project '{}'", display_dir);
    println!("  |-- saturn.toml");
    println!("  |-- src/");
    println!("  |   |-- main.stnx");
    println!();
    println!("To build:   saturnite build src/main.stnx");
    println!("To run:     saturnite run src/main.stnx");
    println!("To check:   saturnite check src/main.stnx");

    Ok(())
}

fn run_doctor() -> anyhow::Result<()> {
    println!("Saturnite Compiler Diagnostics");
    println!("==============================");
    println!();

    // Show host target triple
    match codegen::host_triple() {
        Ok(triple) => {
            println!("Host target triple: {}", triple);
            println!();
        }
        Err(e) => {
            println!("ERROR: Failed to determine host target triple: {}", e);
            println!();
            return Ok(());
        }
    }

    // Show host configuration and linker availability in a single pass
    match TargetConfig::host() {
        Ok(config) => {
            println!("Host configuration:");
            println!("  Triple:      {}", config.triple_str());
            println!("  Architecture: {:?}", config.architecture());
            println!("  OS:          {:?}", config.os());
            println!("  Environment: {:?}", config.environment());
            println!("  Opt level:   {:?}", config.opt_level());
            println!();

            match codegen::check_linker(&config) {
                Ok(()) => println!("Linker: available"),
                Err(e) => println!("WARNING: Linker not available: {}", e),
            }
        }
        Err(e) => {
            println!("ERROR: Failed to initialize target config: {}", e);
            println!();
            println!("WARNING: Could not check linker: {}", e);
        }
    }
    println!();

    println!("inkwell 0.9 with LLVM 21.x (dynamic linking)");

    println!();

    // Show runtime availability
    let out_dir = env!("OUT_DIR");
    let runtime_path = std::path::PathBuf::from(out_dir).join("libsaturnite_runtime.a");
    if runtime_path.exists() {
        println!("Runtime: compiled (libsaturnite_runtime.a)");
    } else {
        println!("WARNING: Runtime object not found");
    }

    println!();

    Ok(())
}

// ---------------------------------------------------------------------------
// Structured build report types
// ---------------------------------------------------------------------------

#[derive(serde::Serialize)]
struct ArtifactInfo {
    output_path: String,
    kind: &'static str,
    target: String,
    profile: String,
    elapsed_ms: u64,
    size_bytes: Option<u64>,
}

#[derive(serde::Serialize)]
struct BuildReport {
    success: bool,
    artifacts: Vec<ArtifactInfo>,
    errors: Vec<String>,
}

/// Render a compiler error through miette if it carries source-span information,
/// falling back to plain Display otherwise.
fn render_diagnostic(e: CompilerError) -> anyhow::Error {
    use miette::GraphicalReportHandler;

    // Render through miette for variants whose inner type implements `Diagnostic`
    // (carries source code + span). Other variants fall back to plain Display.
    let report: String = match &e {
        CompilerError::Lexer(lex_err) => {
            let mut buf = String::new();
            let _ = GraphicalReportHandler::new().render_report(&mut buf, lex_err);
            buf
        }
        CompilerError::Parse(parse_err) => {
            let mut buf = String::new();
            let _ = GraphicalReportHandler::new().render_report(&mut buf, parse_err);
            buf
        }
        _ => e.to_string(),
    };
    anyhow::anyhow!("{}", report)
}
