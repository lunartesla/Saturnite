use crate::error::{CompilerError, LinkError, LinkResult};
use crate::target::{Environment, OperatingSystem, TargetConfig};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Path to the Saturnite runtime support object file.
/// This is compiled from runtime/println_i64.c (via build.rs) and provides
/// the `println_i64` builtin. The object is placed in `OUT_DIR` by the
/// build script.
fn runtime_object_path() -> PathBuf {
    let out_dir = env!("OUT_DIR");
    PathBuf::from(out_dir).join("libsaturnite_runtime.a")
}

pub struct Linker<'cfg> {
    target_config: &'cfg TargetConfig,
}

impl<'cfg> Linker<'cfg> {
    pub fn new(target_config: &'cfg TargetConfig) -> Self {
        Self { target_config }
    }

    pub fn link(&self, obj_path: &Path, output_path: &Path) -> LinkResult<()> {
        self.link_with_externals(obj_path, output_path, &[])
    }

    /// Link an object file into an executable, additionally linking any
    /// declared external interop libraries.
    ///
    /// Each declared Rust library resolves to `lib<name>.a` and each Native
    /// library to `lib<name>.so` (or `.dylib` on macOS). Artifacts are
    /// searched in the following directories, in order:
    ///
    /// 1. The directory of the output executable (project `target/` dir)
    /// 2. The current working directory
    /// 3. `<cwd>/libs/<name>` (conventional per-project interop dir)
    /// 4. `/usr/local/lib`
    ///
    /// The first matching artifact is passed to the linker by full path.
    /// If no artifact is found, the link fails with a diagnostic naming the
    /// missing library and the searched locations.
    pub fn link_with_externals(
        &self,
        obj_path: &Path,
        output_path: &Path,
        externals: &[crate::mir::ExternalLibrary],
    ) -> LinkResult<()> {
        let os = self.target_config.os();
        let env = self.target_config.environment();

        let linker_cmd = self.select_linker(os, env);
        let mut args = self.build_linker_args(os, env, obj_path, output_path);

        // Resolve declared external libraries to concrete artifacts and
        // append them to the link line (after the runtime object so their
        // symbols are available).
        // Track if we have Python externals (need -lpython and -lpthread)
        let has_python = externals.iter().any(|e| e.kind == crate::mir::MirExternalKind::Python);

        for ext in externals {
            match self.resolve_external_library(ext, output_path) {
                Ok(artifact) => {
                    // Python externals don't have a link-time artifact;
                    // they need the Python library added to the link line.
                    if ext.kind != crate::mir::MirExternalKind::Python {
                        args.push(artifact.display().to_string());
                    }
                }
                Err(searched) => {
                    return Err(LinkError::linking_failed(
                        output_path.display().to_string(),
                        Some(format!(
                            "cannot resolve external {} library '{}'.\n\
                             Searched:\n{}",
                            match ext.kind {
                                crate::mir::MirExternalKind::Rust => "rust",
                                crate::mir::MirExternalKind::Native => "native",
                                crate::mir::MirExternalKind::Python => "python",
                            },
                            ext.name,
                            searched
                        )),
                    ));
                }
            }
        }

        // Add Python library if any Python externals are declared.
        if has_python {
            // Try to get the Python library from the build script env var.
            // Fall back to python3-config at link time, then to a default.
            let py_lib = std::env::var("SAT_PYTHON_LIB")
                .ok()
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    // Try python3-config at link time.
                    std::process::Command::new("python3-config")
                        .args(["--ldflags", "--embed"])
                        .output()
                        .ok()
                        .filter(|o| o.status.success())
                        .and_then(|o| {
                            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                            for flag in s.split_whitespace() {
                                if let Some(lib) = flag.strip_prefix("-l") {
                                    return Some(lib.to_string());
                                }
                            }
                            None
                        })
                })
                .unwrap_or_else(|| "python3.13".to_string());

            // Also add library search path if available.
            if let Ok(py_libdir) = std::env::var("SAT_PYTHON_LIBDIR") {
                if !py_libdir.is_empty() {
                    args.push(format!("-L{}", py_libdir));
                }
            } else if let Ok(out) = std::process::Command::new("python3-config")
                .args(["--ldflags", "--embed"])
                .output()
            {
                if out.status.success() {
                    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    for flag in s.split_whitespace() {
                        if let Some(dir) = flag.strip_prefix("-L") {
                            if !dir.is_empty() {
                                args.push(format!("-L{}", dir));
                            }
                        }
                    }
                }
            }

            args.push(format!("-l{}", py_lib));
            args.push("-lpthread".to_string());
            args.push("-ldl".to_string());
            args.push("-lutil".to_string());
            args.push("-lm".to_string());
        }

        let linker_name = linker_cmd.first().map(|s| s.as_str()).unwrap_or("cc");

        // Use `which` to locate the linker on PATH before spawning.
        // If unavailable, fail with a clear diagnostic.
        let linker_path = which::which(linker_name).map_err(|_| {
            LinkError::linker_not_found(format!(
                "{} (searched PATH for {:?})\n\
                 Target platform: {} with {} environment.\n\
                 A C compiler/linker is required to link executables.",
                linker_name,
                linker_name,
                describe_os(os),
                describe_env(env)
            ))
        })?;

        let output = Command::new(&linker_path)
            .args(&linker_cmd[1..])
            .args(&args)
            .output()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    LinkError::linker_not_found(linker_name)
                } else {
                    LinkError::linking_failed(
                        output_path.display().to_string(),
                        Some(e.to_string()),
                    )
                }
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(LinkError::linking_failed(
                output_path.display().to_string(),
                if stderr.is_empty() {
                    None
                } else {
                    Some(stderr)
                },
            ));
        }

        Ok(())
    }

    /// Search for the linkable artifact of a declared external library.
    ///
    /// Returns the full artifact path on success, or a formatted list of the
    /// searched locations on failure. `lib<name>.a` is expected for Rust
    /// staticlibs and `lib<name>.so` / `lib<name>.dylib` for Native
    /// shared libraries. Rust artifacts may also live in
    /// `<dir>/<name>/target/release/` (a rustc build of the wrapper crate).
    fn resolve_external_library(
        &self,
        ext: &crate::mir::ExternalLibrary,
        output_path: &Path,
    ) -> Result<PathBuf, String> {
        let file_names: Vec<String> = match ext.kind {
            crate::mir::MirExternalKind::Rust => vec![format!("lib{}.a", ext.name)],
            crate::mir::MirExternalKind::Native => match self.target_config.os() {
                OperatingSystem::Darwin => {
                    vec![format!("lib{}.dylib", ext.name), format!("lib{}.so", ext.name)]
                }
                _ => vec![format!("lib{}.so", ext.name)],
            },
            // Python needs no link-time artifact; treat as resolved.
            crate::mir::MirExternalKind::Python => return Ok(PathBuf::from("")),
        };

        let out_dir = output_path.parent().unwrap_or_else(|| Path::new("."));
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let search_dirs: Vec<PathBuf> = vec![
            out_dir.to_path_buf(),
            cwd.clone(),
            cwd.join("libs").join(&ext.name),
            cwd.join("libs"),
            PathBuf::from("/usr/local/lib"),
        ];

        let mut searched = String::new();
        for dir in &search_dirs {
            for file_name in &file_names {
                let candidate = dir.join(file_name);
                searched.push_str(&format!("  {}\n", candidate.display()));
                if candidate.is_file() {
                    return Ok(candidate);
                }
                // Rust wrapper crates are also searched as nested rustc
                // build outputs: <dir>/<name>/target/release/lib<name>.a
                if ext.kind == crate::mir::MirExternalKind::Rust {
                    let nested = dir
                        .join(&ext.name)
                        .join("target/release")
                        .join(file_name);
                    searched.push_str(&format!("  {}\n", nested.display()));
                    if nested.is_file() {
                        return Ok(nested);
                    }
                }
            }
        }
        Err(searched)
    }

    fn select_linker(&self, os: &OperatingSystem, env: &Environment) -> Vec<String> {
        match (os, env) {
            (OperatingSystem::Linux, _) => vec!["cc".to_string()],
            (OperatingSystem::Darwin, _) => vec!["clang".to_string()],
            (OperatingSystem::Windows, Environment::Msvc) => {
                vec!["link.exe".to_string()]
            }
            (OperatingSystem::Windows, Environment::Gnu) => {
                vec!["gcc".to_string()]
            }
            _ => vec!["cc".to_string()],
        }
    }

    fn build_linker_args(
        &self,
        os: &OperatingSystem,
        env: &Environment,
        obj_path: &Path,
        output_path: &Path,
    ) -> Vec<String> {
        let obj_str = obj_path.display().to_string();
        let output_str = output_path.display().to_string();

        // Include the runtime library (provides println_i64)
        let runtime = runtime_object_path();
        let runtime_str = runtime.display().to_string();

        match (os, env) {
            (OperatingSystem::Linux, _) | (OperatingSystem::Darwin, _) => {
                // PIC object + non-PIE executable. The runtime object was built
                // without PIE, so we must link with `-no-pie` to avoid
                // "R_X86_64_32 against `.rodata' can not be used when making a PIE object".
                vec![obj_str, "-o".to_string(), output_str, runtime_str, "-no-pie".to_string()]
            }
            (OperatingSystem::Windows, Environment::Msvc) => {
                vec![
                    obj_str,
                    format!("/OUT:{}", output_str),
                    format!("/DEFAULTLIB:{}", runtime_str),
                ]
            }
            (OperatingSystem::Windows, Environment::Gnu) => {
                vec![obj_str, "-o".to_string(), output_str, runtime_str, "-no-pie".to_string()]
            }
            (OperatingSystem::Windows, _) => {
                vec![obj_str, format!("/OUT:{}", output_str), runtime_str]
            }
            _ => {
                vec![obj_str, "-o".to_string(), output_str, runtime_str, "-no-pie".to_string()]
            }
        }
    }
}

pub fn check_linker_available(target_config: &TargetConfig) -> Result<(), CompilerError> {
    let os = target_config.os();
    let env = target_config.environment();

    let linker = Linker::new(target_config);
    let linker_name = linker.select_linker(os, env)[0].clone();

    // Use `which` to verify the linker binary is discoverable on PATH.
    match which::which(&linker_name) {
        Ok(path) => {
            // MSVC's `link.exe` uses `/?` for help; all others accept `--version`.
            let check_arg = match (os, env) {
                (OperatingSystem::Windows, Environment::Msvc) => "/?",
                _ => "--version",
            };
            let check = Command::new(&path).arg(check_arg).output();
            match check {
                Ok(output) if output.status.success() => Ok(()),
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                    let msg = format!(
                        "Linker '{}' was found at {} but exited with status {:?}.\n\
                         Detected platform: {} with {} environment.\n\
                         stderr: {}",
                        linker_name,
                        path.display(),
                        output.status.code(),
                        describe_os(os),
                        describe_env(env),
                        if stderr.is_empty() {
                            "(empty)".to_string()
                        } else {
                            stderr
                        },
                    );
                    Err(CompilerError::Link(LinkError::linking_failed(
                        linker_name,
                        Some(msg),
                    )))
                }
                Err(e) => Err(CompilerError::Link(LinkError::linking_failed(
                    linker_name,
                    Some(e.to_string()),
                ))),
            }
        }
        Err(_) => Err(CompilerError::Link(LinkError::linker_not_found(format!(
            "{} (searched PATH for {:?})",
            linker_name, linker_name
        )))),
    }
}

fn describe_os(os: &OperatingSystem) -> &'static str {
    match os {
        OperatingSystem::Linux => "Linux",
        OperatingSystem::Darwin => "macOS",
        OperatingSystem::Windows => "Windows",
        OperatingSystem::FreeBSD => "FreeBSD",
        OperatingSystem::Unknown => "Unknown",
    }
}

fn describe_env(env: &Environment) -> &'static str {
    match env {
        Environment::Msvc => "MSVC",
        Environment::Gnu => "GNU",
        Environment::Musl => "Musl",
        Environment::Unknown => "Unknown",
    }
}
