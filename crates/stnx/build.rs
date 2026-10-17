use std::path::PathBuf;

fn main() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let runtime_dir = PathBuf::from(manifest_dir).join("runtime");
    let runtime_c = runtime_dir.join("println_i64.c");
    let list_c = runtime_dir.join("list.c");

    // Emit a clear diagnostic if the runtime source is missing, rather than
    // silently falling back to a stale, architecture-specific checked-in object.
    if !runtime_c.exists() {
        eprintln!(
            "error: Saturnite runtime source is missing.\n\
             The Saturnite runtime must be compiled from source; there is no\n\
             silent fallback to a pre-built, architecture-specific object file.\n\
             A C compiler is required to build the runtime (println_i64).\n\
             Searched for the runtime source at: {}\n\
             Target platform: {} ({})\n\
             To fix this, restore the runtime source file and ensure a C compiler\n\
             (cc/gcc/clang) is installed.",
            runtime_c.display(),
            std::env::consts::ARCH,
            std::env::consts::OS,
        );
        std::process::exit(1);
    }

    let mut build = cc::Build::new();
    build.file(&runtime_c);
    if list_c.exists() {
        build.file(&list_c);
    }

    // Rust interoperability runtime shim (rsrt.h / rsrt.c / rsrt_impl.c).
    let rsrt_h = runtime_dir.join("rsrt.h");
    let rsrt_c = runtime_dir.join("rsrt.c");
    let rsrt_impl = runtime_dir.join("rsrt_impl.c");
    if rsrt_c.exists() && rsrt_impl.exists() {
        build.file(&rsrt_c);
        build.file(&rsrt_impl);
        build.include(&runtime_dir);
        println!("cargo:rerun-if-changed={}", rsrt_h.display());
        println!("cargo:rerun-if-changed={}", rsrt_c.display());
        println!("cargo:rerun-if-changed={}", rsrt_impl.display());
    }

    // Python interoperability runtime shim (pyrt.h / pyrt_impl.c).
    //
    // This links against the system CPython library. It is opt-in: a pure
    // Saturnite program with no Python dependency does not need it, but the
    // shim is always compiled into the runtime archive so that a Python-
    // enabled program can link against it.
    //
    // Python headers and library paths are discovered via `python3-config`
    // on Unix, or via `python` + `sysconfig` on Windows. If Python is not
    // installed, the shim is skipped and Python interop is unavailable at
    // runtime (a warning is produced at compile time).
    let pyrt_h = runtime_dir.join("pyrt.h");
    let pyrt_impl = runtime_dir.join("pyrt_impl.c");
    // Only compile the full pyrt_impl.c (which needs Python.h) if a Python
    // development environment is available. The pyrt.h header is always
    // installed so the C compiler succeeds even without Python.
    let py_ok = pyrt_impl.exists() && {
        // On Windows, `python3-config` doesn't exist; use `python` + sysconfig.
        if std::env::consts::OS == "windows" {
            let query = |expr: &str| -> Option<String> {
                let out = std::process::Command::new("python")
                    .arg("-c")
                    .arg(format!(
                        "import sysconfig; print(sysconfig.get_config_var({:?}))",
                        expr
                    ))
                    .output()
                    .ok()?;
                if !out.status.success() {
                    return None;
                }
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if s.is_empty() {
                    None
                } else {
                    Some(s)
                }
            };
            query("INCLUDEPY").is_some() && query("LIBDIR").is_some()
        } else {
            let py_inc = std::process::Command::new("python3-config")
                .args(["--includes"])
                .output();
            let py_ldflags = std::process::Command::new("python3-config")
                .args(["--ldflags", "--embed"])
                .output();
            py_inc.is_ok()
                && py_ldflags.is_ok()
                && py_inc.as_ref().map(|o| o.status.success()).unwrap_or(false)
                && py_ldflags
                    .as_ref()
                    .map(|o| o.status.success())
                    .unwrap_or(false)
        }
    };
    if py_ok {
        let (inc_str, ld_str) = if std::env::consts::OS != "windows" {
            let py_inc = std::process::Command::new("python3-config")
                .args(["--includes"])
                .output()
                .ok();
            let py_ldflags = std::process::Command::new("python3-config")
                .args(["--ldflags", "--embed"])
                .output()
                .ok();
            (
                String::from_utf8_lossy(
                    &py_inc
                        .as_ref()
                        .and_then(|o| Some(&o.stdout[..]))
                        .unwrap_or(&[][..]),
                )
                .trim()
                .to_string(),
                String::from_utf8_lossy(
                    &py_ldflags
                        .as_ref()
                        .and_then(|o| Some(&o.stdout[..]))
                        .unwrap_or(&[][..]),
                )
                .trim()
                .to_string(),
            )
        } else if std::env::consts::OS == "windows" {
            // On Windows, `python3-config` doesn't exist. Use `python` to query
            // sysconfig for include and library paths.
            let query = |expr: &str| -> Option<String> {
                let out = std::process::Command::new("python")
                    .arg("-c")
                    .arg(format!(
                        "import sysconfig; print(sysconfig.get_config_var({:?}))",
                        expr
                    ))
                    .output()
                    .ok()?;
                if !out.status.success() {
                    return None;
                }
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if s.is_empty() {
                    None
                } else {
                    Some(s)
                }
            };

            let inc = query("INCLUDEPY");
            let libdir = query("LIBDIR");
            let inc_dir = inc.unwrap_or_default();
            let lib_dir = libdir.unwrap_or_default();

            if inc_dir.is_empty() && lib_dir.is_empty() {
                (String::new(), String::new())
            } else {
                let inc_flag = if inc_dir.is_empty() {
                    String::new()
                } else {
                    format!("-I{}", inc_dir)
                };
                // MSVC Python ships the import library as "python3.lib"
                // (the standard name on all CPython 3.x Windows installs).
                let lib_name = "python3".to_string();
                (inc_flag, format!("-L{} -l{}", lib_dir, lib_name))
            }
        } else {
            (String::new(), String::new())
        };

        if inc_str.is_empty() && ld_str.is_empty() {
            eprintln!(
                "warning: Python headers/libraries not found; \
                 Python interoperability will be unavailable at runtime."
            );
            return;
        }

        // Parse include flags from `-I<path>` or bare path.
        let mut inc_dirs: Vec<String> = Vec::new();
        for flag in inc_str.split_whitespace() {
            let dir = if let Some(d) = flag.strip_prefix("-I") {
                d
            } else {
                flag
            };
            if !dir.is_empty() {
                inc_dirs.push(dir.to_string());
                build.include(dir);
            }
        }

        build.file(&pyrt_impl);
        build.include(&runtime_dir);

        // Parse library and libdir from the link flags string.
        let mut lib_dir = String::new();
        let mut lib_name = String::new();
        for flag in ld_str.split_whitespace() {
            if let Some(dir) = flag.strip_prefix("-L") {
                if !dir.is_empty() {
                    lib_dir = dir.to_string();
                }
            } else if let Some(lib) = flag.strip_prefix("-l") {
                if !lib.is_empty() {
                    lib_name = lib.to_string();
                }
            }
        }
        if !lib_name.is_empty() {
            println!("cargo:rustc-env:SAT_PYTHON_LIB={}", lib_name);
        }
        if !lib_dir.is_empty() {
            println!("cargo:rustc-env:SAT_PYTHON_LIBDIR={}", lib_dir);
        }
        if !inc_dirs.is_empty() {
            println!("cargo:rustc-env:SAT_PYTHON_INC={}", inc_dirs.join(","));
        }
        println!("cargo:rerun-if-changed={}", pyrt_h.display());
        println!("cargo:rerun-if-changed={}", pyrt_impl.display());
    }

    // Detect the host C compiler so we can report what was searched for.
    let host_cc = build.get_compiler();
    let cc_name = host_cc.path().display().to_string();

    // Use try_compile (not compile) so a missing/too-old compiler produces a
    // structured error instead of an opaque panic.
    if let Err(e) = build.try_compile("saturnite_runtime") {
        eprintln!(
            "error: Saturnite runtime failed to compile.\n\
             The Saturnite runtime must be compiled from source at build time;\n\
             there is no silent fallback to a pre-built, architecture-specific object file.\n\
             A C compiler is required to build the runtime (println_i64).\n\
             Compiler searched for: {}\n\
             Target platform being built: {} ({})\n\
             Underlying error: {}",
            cc_name,
            std::env::consts::ARCH,
            std::env::consts::OS,
            e,
        );
        std::process::exit(1);
    }

    println!("cargo:rerun-if-changed={}", runtime_c.display());
    if list_c.exists() {
        println!("cargo:rerun-if-changed={}", list_c.display());
    }
}
