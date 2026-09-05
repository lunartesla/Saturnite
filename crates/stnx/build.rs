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
    // enabled program can link against it. The Python headers are discovered
    // via `python3-config` when available; if Python is not installed, the
    // shim is skipped and Python interop is unavailable at runtime (a clear
    // diagnostic is produced at compile time, not a silent omission).
    let pyrt_h = runtime_dir.join("pyrt.h");
    let pyrt_impl = runtime_dir.join("pyrt_impl.c");
    if pyrt_impl.exists() {
        let py_inc = std::process::Command::new("python3-config")
            .args(["--includes"])
            .output();
        let py_ldflags = std::process::Command::new("python3-config")
            .args(["--ldflags", "--embed"])
            .output();
        match (py_inc, py_ldflags) {
            (Ok(inc), Ok(ld)) if inc.status.success() && ld.status.success() => {
                let inc_str = String::from_utf8_lossy(&inc.stdout).trim().to_string();
                let ld_str = String::from_utf8_lossy(&ld.stdout).trim().to_string();
                // `python3-config --includes` returns `-I<path>` flags; cc's
                // `include()` takes a bare directory path.
                let mut inc_dirs: Vec<String> = Vec::new();
                for flag in inc_str.split_whitespace() {
                    if let Some(dir) = flag.strip_prefix("-I") {
                        if !dir.is_empty() {
                            inc_dirs.push(dir.to_string());
                            build.include(dir);
                        }
                    }
                }
                build.file(&pyrt_impl);
                build.include(&runtime_dir);
                // The Python library is NOT linked into the runtime archive
                // unconditionally: a pure Saturnite program with no Python
                // dependency must compile and link without Python. The
                // pyrt_impl.o object is only pulled from the archive when a
                // Python-enabled program references it; the linker stage
                // (see `codegen::linker`) adds `-lpython3.13` to the link
                // line only when a Python dependency is present.
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
                    println!(
                        "cargo:rustc-env:SAT_PYTHON_INC={}",
                        inc_dirs.join(",")
                    );
                }
                println!("cargo:rerun-if-changed={}", pyrt_h.display());
                println!("cargo:rerun-if-changed={}", pyrt_impl.display());
            }
            _ => {
                eprintln!(
                    "warning: Python headers/libraries not found via python3-config; \
                     Python interoperability will be unavailable at runtime."
                );
            }
        }
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