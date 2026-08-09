use clap::{Parser, Subcommand};
use std::path::PathBuf;
use stnx::codegen;
use stnx::target::{DebugInfo, OptimizationLevel, OutputKind, TargetConfig};
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

/// Build profile: debug or release
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Profile {
    #[default]
    Debug,
    Release,
}

impl Profile {
    fn as_str(&self) -> &'static str {
        match self {
            Profile::Debug => "debug",
            Profile::Release => "release",
        }
    }

    fn is_release(&self) -> bool {
        matches!(self, Profile::Release)
    }
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
        /// Input source file
        #[arg(value_name = "FILE")]
        input: PathBuf,

        /// Cross-compilation target triple (affects target-dependent checks)
        #[arg(long, value_name = "TRIPLE")]
        target: Option<String>,
    },

    /// Run a source file directly (build to a temp dir, then execute)
    Run {
        /// Input source file
        #[arg(value_name = "FILE")]
        input: PathBuf,

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

            // Exactly one input file is required unless --print-target was set.
            let input = input.ok_or_else(|| {
                anyhow::anyhow!(
                    "an input file is required. Usage: saturnite build <FILE> [OPTIONS]"
                )
            })?;

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

            // Build target configuration
            let mut config = if let Some(triple) = &target {
                TargetConfig::from_triple(triple)
                    .map_err(|e| anyhow::anyhow!("Invalid target '{}': {}", triple, e))?
            } else {
                TargetConfig::host()
                    .map_err(|e| anyhow::anyhow!("Failed to initialize native target: {}", e))?
            };

            // Apply optimization level and debug info
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
                None => {
                    if profile.is_release() {
                        config.set_opt_level(OptimizationLevel::Aggressive);
                        config.set_debug_info(DebugInfo::No);
                    } else {
                        config.set_opt_level(OptimizationLevel::None);
                        config.set_debug_info(DebugInfo::Yes);
                    }
                }
            }

            // Determine output path and emit mode
            let (emit_path, output_kind) = resolve_output(
                &input,
                &output,
                &profile,
                emit_ir,
                emit_object,
                emit_exe,
                no_link,
                &target,
            );

            if verbose {
                eprintln!("target: {}", config.triple_str());
                eprintln!("profile: {}", profile.as_str());
                eprintln!("output: {}", emit_path.display());
            }

            let src = std::fs::read_to_string(&input)
                .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", input.display(), e))?;

            let tokens: Vec<_> = stnx::lexer::Lexer::new(&src)
                .by_ref()
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| anyhow::anyhow!("Lex error: {}", e))?;

            let program = stnx::parser::parse(&src, tokens).map_err(render_diagnostic)?;
            let hir = stnx::semantic::analyze_and_lower(&program).map_err(render_diagnostic)?;

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
                    let ir = codegen::generate_ir(&hir)
                        .map_err(|e| anyhow::anyhow!("IR generation failed: {}", e))?;
                    std::fs::write(&emit_path, ir).map_err(|e| {
                        anyhow::anyhow!("Failed to write IR to {}: {}", emit_path.display(), e)
                    })?;
                }
                _ => {
                    codegen::compile_with_target_ext(
                        &hir,
                        emit_path.to_str().unwrap(),
                        config,
                        save_temps,
                    )
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
                println!("Built {} -> {}", input.display(), emit_path.display());
                if verbose {
                    println!("({} ms)", elapsed.as_millis());
                }
            }

            Ok(())
        }

        Commands::Check { input, target } => {
            check_file(&input, target.as_deref())?;
            println!("No errors found in {}", input.display());
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

            let tmp_output = std::env::temp_dir().join(format!(
                "saturnite_run_{}_{}",
                std::process::id(),
                profile.as_str()
            ));
            let _ = build_run_file(&input, &tmp_output, target.as_deref(), profile)?;
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
    emit_ir: Option<PathBuf>,
    emit_object: Option<PathBuf>,
    emit_exe: Option<PathBuf>,
    no_link: bool,
    _target: &Option<String>,
) -> (PathBuf, OutputKind) {
    let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("out");

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
    let src = std::fs::read_to_string(input)
        .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", input.display(), e))?;

    let mut lexer = stnx::lexer::Lexer::new(&src);
    let tokens: Vec<_> = lexer
        .by_ref()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("Lex error: {}", e))?;

    let program =
        stnx::parser::parse(&src, tokens).map_err(|e| anyhow::anyhow!("Parse error: {}", e))?;
    let hir = stnx::semantic::analyze_and_lower(&program)
        .map_err(|e| anyhow::anyhow!("Semantic error: {}", e))?;

    let mut config = if let Some(triple) = target_triple {
        TargetConfig::from_triple(triple)
            .map_err(|e| anyhow::anyhow!("Invalid target '{}': {}", triple, e))?
    } else {
        TargetConfig::host()
            .map_err(|e| anyhow::anyhow!("Failed to initialize native target: {}", e))?
    };

    if profile.is_release() {
        config.set_opt_level(OptimizationLevel::Aggressive);
        config.set_debug_info(DebugInfo::No);
    } else {
        config.set_opt_level(OptimizationLevel::None);
        config.set_debug_info(DebugInfo::Yes);
    }

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

    codegen::compile_with_target(&hir, output.to_str().unwrap(), config)
        .map_err(|e| anyhow::anyhow!("Compilation failed: {}", e))?;

    Ok(output.to_path_buf())
}

fn check_file(input: &std::path::Path, _target_triple: Option<&str>) -> anyhow::Result<()> {
    let src = std::fs::read_to_string(input)
        .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", input.display(), e))?;

    let mut lexer = stnx::lexer::Lexer::new(&src);
    let tokens: Vec<_> = lexer
        .by_ref()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("Lex error: {}", e))?;

    let program =
        stnx::parser::parse(&src, tokens).map_err(|e| anyhow::anyhow!("Parse error: {}", e))?;
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
    codegen::run_diagnostics().map_err(|e| anyhow::anyhow!("diagnostics failed: {}", e))?;
    println!();

    // Show linker availability
    match TargetConfig::host() {
        Ok(config) => match codegen::check_linker(&config) {
            Ok(()) => println!("Linker: available"),
            Err(e) => println!("WARNING: Linker not available: {}", e),
        },
        Err(e) => {
            println!("WARNING: Could not check linker: {}", e);
        }
    }

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
