# Saturnite

A small systems programming language that compiles to native machine code via
LLVM.  Saturnite takes design inspiration from Rust: it supports `i64`, `f64`,
`bool` types, mutable and immutable bindings, control flow, functions with
recursion, and ranges.

> **Note:** Saturnite is a work-in-progress language.  See
> [docs/SATURNITE_0_3_ARCHITECTURE_REVIEW.md](docs/SATURNITE_0_3_ARCHITECTURE_REVIEW.md)
> for the roadmap toward richer language features.

## Quick start

```sh
# Build the compiler (requires a C compiler for the runtime)
cargo build --release

# Run a Saturnite program
./target/release/stnx run examples/hello.stn

# Or build it as a standalone executable
./target/release/stnx build examples/hello.stn
./target/debug/test_build  # the compiled binary
```

### Hello, world

```rust
fn main() -> i64 {
    println(42)
    return 0
}
```

> **Language note (0.2):** The last expression in a function body is **not**
> used as the return value.  Always use an explicit `return` statement.
> See the architecture review for why.

## Compiler architecture

```
Saturnite source
   |
   v
  Lexer          (logos-based tokenization, tokens carry byte spans)
   |
   v
  Parser         (chumsky 0.13, produces a spanned AST)
   |
   v
  Semantic anal. (type checking, mutability enforcement, scope resolution)
   |
   v
  LLVM IR        (inkwell 0.9 -> LLVM 21)
   |
   v
  ObjectEmitter  (TargetMachine writes .o / .ll)
   |
   v
  Linker         (system linker: cc on Linux, clang on macOS, link.exe/gcc on Windows)
   |
   v
  Executable
```

| Component          | Module                          | Notes                                              |
|--------------------|---------------------------------|----------------------------------------------------|
| Lexer              | `src/lexer/`                    | Token + byte span                                    |
| Parser             | `src/parser/`                   | chumsky 0.13, SimpleSpan                             |
| AST                | `src/ast.rs`                    | Every node carries a `Range<usize>` span             |
| Semantic analysis  | `src/semantic.rs`               | Scope-based, mutability-checked                      |
| Code generation    | `src/codegen/`                  | CodeGenerator, ObjectEmitter, Linker                 |
| Target config      | `src/target.rs`                 | Triple validation, optimization & debug levels       |
| Errors             | `src/error.rs`                  | thiserror + miette Diagnostic                         |
| CLI                | `src/main.rs`                    | build / check / run / doctor                         |
| Runtime            | `runtime/println_i64.c`         | Compiled at build time via `build.rs` + `cc`         |

### Mutable variable storage

Mutable variables use stack allocation (`alloca` / `store` / `load`) so that
assignments persist across basic-block boundaries (e.g. inside loops).  This
is the correct 0.2 semantics and remains compatible with a future HIR/MIR
pipeline where values can be promoted back into SSA form.

## CLI reference

```
saturnite build <FILE> [OPTIONS]      # Build to an executable
  --debug                             # Debug profile (opt 0, debug info)
  --release                           # Release profile (opt 3, no debug info)
  --target <TRIPLE>                   # Cross-compilation target (host only in 0.2)
  --opt-level <0|1|2|3>               # Override optimization level
  --emit-ir <FILE>                    # Emit LLVM IR text
  --emit-object <FILE>                # Emit object file only
  --emit-exe <FILE>                   # Emit executable
  --no-link                           # Stop after object emission
  --save-temps                        # Keep intermediate .o files
  --json                              # Structured build report
  --verbose                           # Verbose output
  --print-target                      # Print host triple and exit

saturnite check <FILE>                # Type & semantic check (no codegen)
saturnite run <FILE>                  # Build then execute
saturnite doctor                      # Print environment diagnostics
```

## Build configuration

- **Debug profile:** `target/debug/<name>` — optimization off, debug info on.
- **Release profile:** `target/release/<name>` — optimization level 3, no debug info.

All build artifacts are placed under `target/<profile>/`.

## Dependencies

| Crate        | Version | Purpose                          |
|--------------|---------|----------------------------------|
| `logos`      | 0.16    | Lexer / tokenization             |
| `chumsky`    | 0.13    | Parser combinator framework      |
| `inkwell`    | 0.9     | LLVM bindings (LLVM 21, dynamic) |
| `which`      | 5       | Linker binary discovery           |
| `cc`         | 1       | Runtime C compilation (build)    |
| `miette`     | 7       | Fancy diagnostic rendering        |
| `thiserror`  | 2       | Error derive macros               |
| `clap`       | 4       | CLI argument parsing              |
| `serde`      | 1       | Serialize build report            |
| `serde_json` | 1       | JSON build report (--json)        |
| `anyhow`     | 1       | CLI error handling                |
| `tempfile`   | 3       | Isolated test temp directories    |

## License

Dual-licensed under MIT or Apache-2.0. 
