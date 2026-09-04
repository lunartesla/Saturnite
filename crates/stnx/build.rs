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
    if list_c.exists() {
        build.file(&list_c);
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
             there is no silent fallback to a pre-built object file.\n\
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
    if list_c.exists() {
        println!("cargo:rerun-if-changed={}", list_c.display());
    }
}
