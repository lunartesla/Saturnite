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
        let os = self.target_config.os();
        let env = self.target_config.environment();

        let linker_cmd = self.select_linker(os, env);
        let args = self.build_linker_args(os, env, obj_path, output_path);

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
                vec![obj_str, "-o".to_string(), output_str, runtime_str]
            }
            (OperatingSystem::Windows, Environment::Msvc) => {
                vec![
                    obj_str,
                    format!("/OUT:{}", output_str),
                    format!("/DEFAULTLIB:{}", runtime_str),
                ]
            }
            (OperatingSystem::Windows, Environment::Gnu) => {
                vec![obj_str, "-o".to_string(), output_str, runtime_str]
            }
            (OperatingSystem::Windows, _) => {
                vec![obj_str, format!("/OUT:{}", output_str), runtime_str]
            }
            _ => {
                vec![obj_str, "-o".to_string(), output_str, runtime_str]
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
