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
        let has_python = externals
            .iter()
            .any(|e| e.kind == crate::mir::MirExternalKind::Python);

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

        let has_python = externals
            .iter()
            .any(|e| e.kind == crate::mir::MirExternalKind::Python);

        let linker_name = linker_cmd.first().map(|s| s.as_str()).unwrap_or("cc");

        // Locate the linker binary. On Windows MSVC, `which::which` may find
        // an MSYS2/MinGW `link.exe` (`/usr/bin/link.exe`) instead of the real
        // MSVC linker — which doesn't understand MSVC-style `/DEFAULTLIB:` flags.
        // Use `vswhere` to find the VS installation's `link.exe` first.
        let linker_path = resolve_linker_path(linker_name, os, env).map_err(|_| {
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
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let details = if stderr.is_empty() && !stdout.is_empty() {
                Some(stdout)
            } else if stderr.is_empty() {
                None
            } else {
                Some(stderr)
            };
            return Err(LinkError::linking_failed(
                output_path.display().to_string(),
                details,
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
                    vec![
                        format!("lib{}.dylib", ext.name),
                        format!("lib{}.so", ext.name),
                    ]
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
                    let nested = dir.join(&ext.name).join("target/release").join(file_name);
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
                vec![
                    obj_str,
                    "-o".to_string(),
                    output_str,
                    runtime_str,
                    "-no-pie".to_string(),
                ]
            }
            (OperatingSystem::Windows, Environment::Msvc) => {
                let mut args = vec![
                    obj_str,
                    format!("/OUT:{}", output_str),
                    runtime_str,
                    "/SUBSYSTEM:CONSOLE".to_string(),
                    // Request the CRT libs so the linker resolves
                    // mainCRTStartup and the C runtime symbols. The runtime C
                    // code is compiled with the /MD default CRT (dynamic
                    // linking), so we need the shared CRT and UCRT libs.
                    "/DEFAULTLIB:msvcrt".to_string(),
                    "/DEFAULTLIB:ucrt".to_string(),
                    "/DEFAULTLIB:vcruntime".to_string(),
                ];
                // Add MSVC CRT and Windows SDK lib paths so the linker can
                // find system libraries (libcmt, ucrt, etc.) without the LIB
                // environment variable being set.
                for dir in find_msvc_lib_dirs() {
                    args.push(format!("/LIBPATH:{}", dir.display()));
                }
                args
            }
            (OperatingSystem::Windows, Environment::Gnu) => {
                vec![
                    obj_str,
                    "-o".to_string(),
                    output_str,
                    runtime_str,
                    "-no-pie".to_string(),
                ]
            }
            (OperatingSystem::Windows, _) => {
                vec![obj_str, format!("/OUT:{}", output_str), runtime_str]
            }
            _ => {
                vec![
                    obj_str,
                    "-o".to_string(),
                    output_str,
                    runtime_str,
                    "-no-pie".to_string(),
                ]
            }
        }
    }
}

pub fn check_linker_available(target_config: &TargetConfig) -> Result<(), CompilerError> {
    let os = target_config.os();
    let env = target_config.environment();

    let linker = Linker::new(target_config);
    let linker_name = linker.select_linker(os, env)[0].clone();

    // Resolve the linker path — on Windows MSVC, this may use `vswhere`
    // to find the real MSVC `link.exe` (not the MSYS2 shim).
    match resolve_linker_path(&linker_name, os, env) {
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

/// Locate a linker binary by name, with Windows MSVC special handling.
///
/// On Windows with the MSVC environment, `which::which("link.exe")` can find an
/// MSYS2/MinGW `link.exe` at `/usr/bin/link.exe` instead of the real MSVC linker.
/// The MSYS2 linker doesn't understand MSVC-style `/DEFAULTLIB:` flags. This
/// function uses `vswhere` to find the Visual Studio installation's `link.exe`
/// first, falling back to `which` if that fails.
fn resolve_linker_path(
    name: &str,
    os: &OperatingSystem,
    env: &Environment,
) -> std::io::Result<PathBuf> {
    // On Windows MSVC, try vswhere to find the real link.exe.
    if matches!((os, env), (OperatingSystem::Windows, Environment::Msvc))
        && name.eq_ignore_ascii_case("link.exe")
    {
        if let Some(path) = find_msvc_linker_via_vswhere() {
            if path.is_file() {
                return Ok(path);
            }
        }
    }
    which::which(name).map_err(|e| std::io::Error::new(std::io::ErrorKind::NotFound, e.to_string()))
}

/// Use `vswhere` to locate the Visual Studio installation path and return the
/// MSVC version directory (`<installPath>/VC/Tools/MSVC/<ver>`).
fn find_msvc_version_dir() -> Option<PathBuf> {
    // vswhere is installed alongside Visual Studio at this well-known path.
    let vswhere = Path::new("C:/Program Files (x86)/Microsoft Visual Studio/Installer/vswhere.exe");
    if !vswhere.is_file() {
        return None;
    }

    let output = std::process::Command::new(vswhere)
        .args([
            "-latest",
            "-products",
            "*",
            "-requires",
            "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
            "-property",
            "installationPath",
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let install_path = String::from_utf8_lossy(&output.stdout);
    let install_path = install_path.trim();
    if install_path.is_empty() {
        return None;
    }

    // MSVC version directory: <installPath>/VC/Tools/MSVC/<ver>
    let msvc_dir = Path::new(install_path).join("VC/Tools/MSVC");
    if !msvc_dir.is_dir() {
        return None;
    }

    // Find the first MSVC version directory.
    if let Ok(entries) = std::fs::read_dir(&msvc_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name
                .chars()
                .next()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
            {
                return Some(entry.path());
            }
        }
    }

    None
}

/// Use `vswhere` to locate the MSVC `link.exe` inside a Visual Studio
/// installation. Returns `None` if `vswhere` is not available or no suitable
/// installation was found.
fn find_msvc_linker_via_vswhere() -> Option<PathBuf> {
    let version_dir = find_msvc_version_dir()?;

    // The x64-hosted, x64-target MSVC tools live under:
    //   <installPath>/VC/Tools/MSVC/<ver>/bin/HostX64/x64/
    let link_path = version_dir.join("bin/HostX64/x64/link.exe");
    if link_path.is_file() {
        return Some(link_path);
    }

    // Fall back to HostX64/x86 and HostX86/x64.
    let fallbacks = [
        "bin/HostX64/x86/link.exe",
        "bin/HostX86/x64/link.exe",
        "bin/HostX86/x86/link.exe",
    ];
    for rel in &fallbacks {
        let candidate = version_dir.join(rel);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}

/// Discover the MSVC CRT library directory and the Windows SDK UCRT library
/// directory for x64. Returns a list of paths suitable for `/LIBPATH:` flags.
fn find_msvc_lib_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    // MSVC CRT libs: <msvc_version>/lib/spectre/x64
    if let Some(version_dir) = find_msvc_version_dir() {
        let crt_lib = version_dir.join("lib/spectre/x64");
        if crt_lib.is_dir() {
            dirs.push(crt_lib);
        }
    }

    // Windows SDK libs: C:/Program Files (x86)/Windows Kits/10/Lib/<sdk_ver>/{um|x86|ucrt}/x64
    let kits_root = Path::new("C:/Program Files (x86)/Windows Kits/10/Lib");
    if kits_root.is_dir() {
        if let Ok(entries) = std::fs::read_dir(kits_root) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name
                    .chars()
                    .next()
                    .map(|c| c.is_ascii_digit())
                    .unwrap_or(false)
                {
                    let sdk_dir = entry.path();
                    // um/x64 has kernel32.lib, user32.lib, etc.
                    let um_lib = sdk_dir.join("um/x64");
                    if um_lib.is_dir() {
                        dirs.push(um_lib);
                    }
                    // ucrt/x64 has ucrt.lib
                    let ucrt_lib = sdk_dir.join("ucrt/x64");
                    if ucrt_lib.is_dir() {
                        dirs.push(ucrt_lib);
                    }
                    break;
                }
            }
        }
    }

    dirs
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
