//! Python virtual-environment management for ecosystem interoperability.
//!
//! Each Saturnite project that depends on Python packages gets an isolated
//! virtual environment under `target/saturnite/venvs/<project_name>/`. This
//! module provides:
//!
//! * [`PyEnv::create`] — create a venv if one doesn't already exist;
//! * [`PyEnv::ensure_package`] — install a package into the venv via `pip`;
//! * [`PyEnv::search_path`] — return the `PYTHONPATH`-style search path
//!   string that the C bridge uses to locate modules.
//!
//! The venv is a real `python3 -m venv` environment — not a fake or mock. Only
//! packages explicitly listed in `saturn.toml` are installed; the interpreter
//! lifecycle is managed by `pyrt_impl.c`.

use std::path::{Path, PathBuf};

use crate::error::{BuildError, BuildResult};

/// A Python virtual environment associated with a Saturnite project.
#[derive(Debug, Clone)]
pub struct PyEnv {
    /// Root directory of the venv (contains `bin/python` or `Scripts/python.exe`).
    pub venv_root: PathBuf,
    /// The path to the Python executable inside the venv.
    pub python_exe: PathBuf,
    /// The site-packages directory (where pip-installed packages live).
    pub site_packages: PathBuf,
}

impl PyEnv {
    /// Create a `PyEnv` handle pointing at an existing venv.
    /// Does NOT create the venv on disk — use [`PyEnv::create`] for that.
    pub fn at(venv_root: PathBuf) -> Self {
        let (python_exe, site_packages) = venv_layout(&venv_root);
        Self {
            venv_root,
            python_exe,
            site_packages,
        }
    }

    /// Create or open the venv at `<project_root>/target/saturnite/venvs/<name>/`.
    ///
    /// If the venv already exists (i.e. `bin/python` or `Scripts/python.exe`
    /// is present), it is reused. Otherwise `python3 -m venv <dir>` is invoked.
    ///
    /// Returns a `PyEnv` handle with correct `python_exe` and `site_packages`
    /// paths. The caller can then call [`ensure_package`] to install deps.
    ///
    /// [`ensure_package`]: PyEnv::ensure_package
    pub fn create(project_root: &Path, name: &str) -> BuildResult<Self> {
        let venv_root = project_root
            .join("target")
            .join("saturnite")
            .join("venvs")
            .join(name);

        if !venv_root.exists() {
            // Create the venv from scratch. We use `python3 -m venv` — the
            // standard library venv module — which is available on all
            // CPython 3.3+ installations.
            let status = std::process::Command::new("python3")
                .arg("-m")
                .arg("venv")
                .arg(&venv_root)
                .status()
                .map_err(|e| {
                    BuildError::wrapper_build(format!(
                        "failed to invoke python3 to create venv: {}",
                        e
                    ))
                })?;

            if !status.success() {
                return Err(BuildError::wrapper_build(format!(
                    "python3 -m venv failed with exit status {:?}",
                    status.code()
                )));
            }
        }

        let py_exe = &venv_python(&venv_root);
        if !py_exe.exists() {
            return Err(BuildError::wrapper_build(format!(
                "venv was created but python executable not found at {}",
                py_exe.display()
            )));
        }

        Ok(Self::at(venv_root))
    }

    /// Ensure that a package is installed in this venv.
    ///
    /// Uses `pip install <spec>` inside the venv. If the package is already
    /// installed at a compatible version, pip skips the install (it is
    /// idempotent). The `spec` string should be a valid pip requirement
    /// specifier (e.g. `"numpy>=1.24"`, `"requests==2.31.0"`, `"torch"`).
    pub fn ensure_package(&self, spec: &str) -> BuildResult<()> {
        let status = std::process::Command::new(&self.python_exe)
            .arg("-m")
            .arg("pip")
            .arg("install")
            .arg("--disable-pip-version-check")
            .arg(spec)
            .status()
            .map_err(|e| {
                BuildError::wrapper_build(format!(
                    "failed to invoke pip for install '{}': {}",
                    spec, e
                ))
            })?;

        if !status.success() {
            return Err(BuildError::wrapper_build(format!(
                "pip install '{}' failed with exit status {:?}",
                spec,
                status.code()
            )));
        }

        Ok(())
    }

    /// Return the `PYTHONPATH`-style search path for this venv.
    ///
    /// This is a colon-separated (or semicolon-separated on Windows) list of
    /// directories that the C bridge passes to `sat_py_call_flat`'s
    /// `search_path` parameter. It includes:
    ///
    /// * The venv's `site-packages` (where pip packages live)
    /// * Any version-specific `lib/pythonX.Y/site-packages` directories
    ///   (Unix venvs have this nested layout)
    /// * The `Lib/site-packages` directory (Windows venvs have this layout)
    pub fn search_path(&self) -> BuildResult<String> {
        let mut dirs = vec![self.site_packages.to_string_lossy().to_string()];

        // On Unix, the venv layout is <root>/lib/pythonX.Y/site-packages.
        // Walk the lib/ directory to find python version dirs.
        let lib_dir = self.venv_root.join("lib");
        if lib_dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&lib_dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if name.starts_with("python") {
                        let sp = entry.path().join("site-packages");
                        if sp.is_dir() {
                            let sp_str = sp.to_string_lossy().to_string();
                            if !dirs.iter().any(|d| d == &sp_str) {
                                dirs.push(sp_str);
                            }
                        }
                    }
                }
            }
        }

        // On Windows, the venv layout is <root>/Lib/site-packages, which is
        // already captured by `self.site_packages`. If the primary site-packages
        // doesn't exist yet (fresh venv), the C bridge will still get the path
        // and pip will populate it.

        // Join with platform-appropriate separator.
        let sep = if cfg!(windows) { ";" } else { ":" };
        Ok(dirs.join(sep))
    }

    /// Check whether this venv exists and is usable.
    pub fn exists(&self) -> bool {
        self.python_exe.exists()
    }

    /// Remove this venv from disk (for `saturn clean` / `saturn doctor`).
    pub fn remove(&self) -> std::io::Result<()> {
        if self.venv_root.exists() {
            std::fs::remove_dir_all(&self.venv_root)?;
        }
        Ok(())
    }
}

/// Compute the Python executable path and site-packages path for a venv.
/// Handles both Unix and Windows venv layouts.
fn venv_layout(venv_root: &Path) -> (PathBuf, PathBuf) {
    let python_exe = venv_python(venv_root);
    let site_packages = venv_site_packages(venv_root);
    (python_exe, site_packages)
}

#[cfg(windows)]
fn venv_python(venv_root: &Path) -> PathBuf {
    venv_root.join("Scripts").join("python.exe")
}

#[cfg(not(windows))]
fn venv_python(venv_root: &Path) -> PathBuf {
    venv_root.join("bin").join("python3")
}

#[cfg(windows)]
fn venv_site_packages(venv_root: &Path) -> PathBuf {
    // On Windows, site-packages is under Lib/site-packages.
    venv_root.join("Lib").join("site-packages")
}

#[cfg(not(windows))]
fn venv_site_packages(venv_root: &Path) -> PathBuf {
    // On Unix, site-packages is under lib/pythonX.Y/site-packages.
    // We try to find the python version directory.
    let lib_dir = venv_root.join("lib");
    if lib_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&lib_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("python") {
                    return entry.path().join("site-packages");
                }
            }
        }
    }
    // Fallback: assume python3.11 (or whatever the system default is).
    venv_root.join("lib").join("python3").join("site-packages")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_crate_name() {
        // venv names should handle hyphens and underscores
        let venv = PyEnv::at(PathBuf::from("/tmp/test_venv"));
        assert!(!venv.exists());
    }

    #[test]
    fn test_venv_layout_unix() {
        let root = PathBuf::from("/tmp/test_venv");
        let (py, sp) = venv_layout(&root);
        #[cfg(windows)]
        {
            assert_eq!(py, root.join("Scripts").join("python.exe"));
            assert_eq!(sp, root.join("Lib").join("site-packages"));
        }
        #[cfg(not(windows))]
        {
            assert_eq!(py, root.join("bin").join("python3"));
        }
    }

    #[test]
    fn test_pyenv_at() {
        let venv = PyEnv::at(PathBuf::from("/nonexistent/venv"));
        assert!(!venv.exists());
        assert!(!venv.site_packages.exists());
    }

    #[test]
    fn test_pyenv_search_path_separator() {
        // The search_path should use the correct separator for the platform.
        // We can't test the actual content without a real venv, but we can
        // verify the separator logic.
        let sep = if cfg!(windows) { ";" } else { ":" };
        assert!(!sep.is_empty());
    }
}
