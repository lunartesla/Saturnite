# Saturnite

Saturnite is a statically-typed, compiled systems programming language that
compiles to native machine code via LLVM. It is designed for human readability:
the surface syntax resembles Python (indentation, `main:`, `say`, `give`),
while retaining strong static typing, native compilation, and direct
interoperability with the Rust and Python ecosystems.

Saturnite is:

- **Human-friendly** — indentation-based blocks, minimal punctuation
- **Statically typed** — `number`, `text`, `bool`, and user-defined structs/enums
- **Compiled to native code** — LLVM-backed, produces standalone executables
- **Rust-interoperable** — `use rust:serde` imports Rust ecosystems
- **Python-interoperable** — `use python:numpy` imports Python libraries

> See [`docs/SATURNITE_SYNTAX.md`](docs/SATURNITE_SYNTAX.md) for the
> authoritative language syntax reference.
>
> See [`docs/INTEROPERABILITY.md`](docs/INTEROPERABILITY.md) for the
> interoperability model.
>
> See [`docs/DEPENDENCIES.md`](docs/DEPENDENCIES.md) for the dependency model.
>
> See [`docs/README.md`](docs/README.md) for the full documentation index.
>
> **Legacy documentation** under [`docs/legacy/`](docs/legacy/) is
> archived for historical reference and is **not** the current language
> specification.

---

## Quick start

```sh
# Build the compiler (requires a system C compiler for the runtime)
cargo build --release

# Run a Saturnite program (builds to a temp exe, then executes)
./target/release/stnx run examples/hello.stn

# Or build it as a standalone executable
./target/release/stnx build examples/hello.stn
./target/debug/test_build  # the compiled binary

# Type-check a program without producing code
./target/release/stnx check examples/smoke_test.stnx

# Scaffold a new project (creates ./myproj/saturn.toml + src/main.stnx)
./target/release/stnx init myproj
```

### Hello, world

```saturnite
main:
    say "Hello, world!"
    give 0
```

### A canonical example

This program demonstrates modules, structs, functions, types, conditionals,
loops, string interpolation, and the `main:` entry point:

```saturnite
module calculator

struct Item:
    name: text
    price: number
    quantity: number

fn total_value(items: List<Item>) -> number:
    let total = 0
    for item in items:
        total += item.price * item.quantity
    give total

fn restock(price: number, amount: number) -> number:
    if amount <= 0:
        raise "restock amount must be positive"
    give price + amount

fn add(a: number, b: number) -> number:
    give a + b

main:
    let values = [10, 20, 30]
    let result = add(20, 22)
    say "Result: {result}"
    say "Values: {values.length}"
    give 0
```

### Legacy brace syntax

Saturnite also accepts the legacy brace-based syntax for backward compatibility:

```rust
fn main() -> i64 {
    println(42)
    return 0
}
```

Both syntaxes may be mixed in a single file. The native colon-indented syntax
is the canonical, preferred style for all new code.

---

## Language reference

### Modules and imports

```saturnite
module calculator

use math
use collections
use rust:serde
use python:numpy
```

| Import form              | Meaning                                    |
|--------------------------|--------------------------------------------|
| `use math`               | Import a Saturnite-native module           |
| `use rust:serde`         | Import a Rust crate (wrapper-backed)       |
| `use python:numpy`       | Import a Python library (runtime bridge)   |
| `use native:sqlite`      | Import a native C-ABI library              |

### Functions and entry point

```saturnite
fn calculate_total(price: number, quantity: number) -> number:
    let total = price * quantity
    give total

main:
    let result = calculate_total(20, 5)
    say "Result: {result}"
```

### Variables

```saturnite
let name = "Saturnite"
let age = 16
let items = [1, 2, 3]
```

Reassignment requires the variable to be declared `mut`:

```saturnite
let mut count = 0
count = count + 1
```

### Types

| Saturnite (canonical) | Internal representation |
|-----------------------|------------------------|
| `number`              | `i64`                  |
| `text`                | `str`                  |
| `bool`                | `bool`                 |
| `List<T>`             | `List<T>`              |

### Conditions

```saturnite
if age >= 18:
    say "Adult"
elif age > 13:
    say "Teen"
else:
    say "Minor"
```

### Loops

```saturnite
for item in items:
    say item

for i in 0..10:
    say i

while count < 10:
    count += 1
```

### Structs

```saturnite
struct Person:
    name: text
    age: number

let person = Person:
    name: "Alex"
    age: 25
```

### Lists

```saturnite
let numbers = [10, 20, 30]
let first = numbers[0]
let count = numbers.length

for number in numbers:
    say number
```

### String interpolation

```saturnite
let name = "Saturnite"
let age = 16

say "Hello {name}, you are {age}."
```

### Named arguments

```saturnite
let user = create_user(
    name: "Alex"
    age: 25
    admin: false
)
```

### Pipelines and closures

```saturnite
let result = numbers
    |> filter(x -> x > 10)
    |> sort()
```

---

## Features at a glance

- **Numeric & basic types** — `number`, `text`, `bool`, `List<T>`
- **Bindings** — `let x = expr` (immutable), `let mut x = expr` (mutable)
- **Compound assignment** — `+=`, `-=`, `*=`, `/=` on mutable locals
- **Control flow** — `if / elif / else`, `while`, `for` (ranges and lists)
- **Functions** — first-class, with recursion and `give` return
- **Ranges** — `a..b` (exclusive), `a...b` (inclusive)
- **Structs** — `struct Name:` with colon-indented fields and construction
- **Enums** — `enum Color:` with variants and constructors (`Color::Red`)
- **Modules & visibility** — `module name`, `use`, `pub`, `mod`
- **Generics** — turbofish syntax (`id::<number>(42)`)
- **Builtins** — `say` (print text/number), `raise` (error stub)
- **String interpolation** — `"Value: {x}"` with runtime concatenation
- **Pipelines** — `a |> f(x)` left-associative desugaring to `f(a, x)`
- **Named arguments** — `f(name: "Alex", age: 25)`
- **Native codegen** — typed MIR → LLVM 21 IR → object → link
- **Cross-platform linkers** — `cc` (Linux), `clang` (macOS),
  `link.exe`/`gcc` (Windows)
- **Diagnostics** — every compile stage has a `thiserror` error type
  rendered via `miette`
- **Multi-module projects** — `saturn.toml` discovery, `src/main.stnx`
  default entry, recursive module resolution

---

## Compiler architecture

```text
Saturnite source (.stn / .stnx)
   │
   ▼
 Lexer             (logos, byte-spanned tokens + indent pre-pass)
   │
   ▼
 Parser            (chumsky 0.13 → spanned AST)
   │
   ▼
 Token preparation (desugars colon-blocks → braces; native syntax → AST)
   │
   ▼
 Semantic          (AST → HIR; type-check, mutability, scope)
   │
   ▼
 Resolver          (dedicated name-resolution pass)
   │
   ▼
 Monomorphize      (resolve generic call sites, substitute types)
   │
   ▼
 MIR               (typed CFG: locals, blocks, terminators)
   │
   ▼
 MIR → LLVM IR     (inkwell 0.9 → LLVM 21; the only codegen path)
   │
   ▼
 ObjectEmitter     (TargetMachine → .o / .ll)
   │
   ▼
 Linker            (system linker: cc / clang / link.exe)
   │
   ▼
 Executable
```

| Component          | Module / file                  | Notes                                              |
|--------------------|--------------------------------|----------------------------------------------------|
| Lexer              | `src/lexer/`                   | logos; tokens carry byte spans; indent pre-pass    |
| Token preparation  | `src/lexer/prepare.rs`         | Desugars native colon-blocks to braces             |
| Parser             | `src/parser/`                  | chumsky 0.13, `SimpleSpan`                         |
| AST                | `src/ast.rs`                   | every node carries a `Range<usize>` span            |
| Semantic analysis  | `src/semantic.rs`              | AST → HIR: scope-based, mutability-checked          |
| HIR                | `src/hir/`                     | typed, span-bearing; the authoritative IR           |
| Resolver           | `src/resolver.rs`              | dedicated name-resolution pass                      |
| MIR                | `src/mir/`                     | typed CFG (lower, verify, optimize)                 |
| Monomorphize       | `src/mir/monomorphize.rs`      | generic-function substitution                       |
| MIR codegen        | `src/mir/codegen.rs`           | **sole** MIR → LLVM path                            |
| Object emission    | `src/codegen/emitter.rs`       | `ObjectEmitter`: writes `.o` / `.ll` via TargetMachine |
| Linking            | `src/codegen/linker.rs`        | system linker invocation                            |
| Target config      | `src/target.rs`                | triple validation, optimization & debug levels      |
| Runtime            | `crates/stnx/runtime/`         | C runtime compiled at build time via `build.rs`     |
| Module system      | `src/module.rs`                | `ModuleGraph`, `Project`, discovery                 |
| CLI                | `src/main.rs`                  | `build` / `check` / `run` / `doctor` / `init`       |
| Errors             | `src/error.rs`                 | `thiserror` + `miette::Diagnostic`                  |

---

## CLI reference

```
stnx build <FILE> [OPTIONS]      # Build to an executable, object, or IR
  --debug                         # Debug profile (opt 0, debug info)
  --release                       # Release profile (opt 3, no debug info)
  --target <TRIPLE>               # Cross-compilation target (host-only in current release)
  --opt-level <0|1|2|3>           # Override optimization level
  --emit-ir <FILE>                # Emit LLVM IR text
  --emit-object <FILE>           # Emit object file only
  --emit-exe <FILE>              # Emit executable
  --no-link                      # Stop after object emission
  --save-temps                   # Keep intermediate .o files
  --json                         # Structured build report
  --verbose                      # Verbose output
  --print-target                 # Print host triple and exit

stnx check <FILE>                # Type & semantic check (no codegen)
stnx run <FILE>                  # Build then execute
stnx doctor                      # Print environment diagnostics
stnx init [NAME]                 # Scaffold a new project
```

If `<FILE>` is omitted, the CLI looks for a `saturn.toml` project and
defaults to `src/main.stnx`.

---

## Project layout

```text
.
├── Cargo.toml                # workspace manifest
├── saturn.toml               # default sample project manifest
├── examples/
│   ├── hello.stn             # minimal "hello world"
│   ├── interpolation_demo.stn # string interpolation demo
│   ├── list_demo.stnx        # list literals, indexing, iteration
│   ├── native_demo.stn       # native syntax feature showcase
│   └── smoke_test.stnx       # full native syntax demo
├── docs/                     # canonical language docs
│   ├── SATURNITE_SYNTAX.md   # canonical language syntax spec
│   ├── INTEROPERABILITY.md   # Rust/Python native interop
│   ├── DEPENDENCIES.md       # dependency model
│   ├── SATURNITE_CRATE_DEPENDENCY_AUDIT.md  # crate dependency audit
│   ├── SATURNITE_0_4_ARCHITECTURE.md        # compiler architecture
│   ├── README.md             # documentation index
│   └── legacy/               # archived (NOT canonical) documentation
└── crates/stnx/
    ├── Cargo.toml
    ├── build.rs              # compiles the C runtime via cc
    ├── runtime/              # C runtime (println, lists, Python bridge)
    └── src/
        ├── main.rs           # CLI entry point
        ├── lib.rs            # public API re-exports
        ├── ast.rs            # AST nodes
        ├── lexer/            # logos tokenizer + indent pre-pass
        ├── parser/           # chumsky parser
        ├── semantic.rs       # AST → HIR entry point
        ├── hir/              # typed HIR (types, exprs, stmts, symbols)
        ├── resolver.rs       # dedicated name-resolution pass
        ├── mir/              # typed CFG (lower, verify, optimize)
        ├── codegen/          # ObjectEmitter, Linker (shared seams)
        ├── target.rs         # Profile, TargetConfig, triple handling
        ├── module.rs         # ModuleGraph, Project, discovery
        ├── config.rs         # saturn.toml parsing
        ├── interop.rs        # dependency model + external-call contract
        ├── interop_rust.rs   # Rust ABI bridge
        ├── interop_python.rs # Python runtime bridge
        └── error.rs          # CompilerError + miette Diagnostic
```

---

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
| `serde_json` | 1       | JSON build report (`--json`)      |
| `anyhow`     | 1       | CLI error handling                |
| `tempfile`   | 3       | Isolated test temp directories    |

---

## Build configuration

- **Debug profile:** `target/debug/<name>` — optimization off, debug info on.
- **Release profile:** `target/release/<name>` — optimization level 3, no debug info.

All build artifacts are placed under `target/<profile>/`. Cross-compilation
to a non-host triple is rejected with a clear diagnostic, because the
runtime is compiled for the host only.

---

## Testing

Saturnite ships a comprehensive test suite covering the lexer, parser, AST,
HIR lowering, resolver, MIR, codegen, native compilation, string interpolation,
list operations, generics, and multi-module projects.

```sh
cargo test --workspace
```

---

## Interoperability boundaries

Saturnite does **not** claim to support arbitrary Rust or Python APIs.
`use rust:serde` means the compiler resolves the declared dependency and
exposes its supported adapter boundary — not that Saturnite understands
every Rust type. Similarly, `use python:numpy` means the Python bridge can
resolve and call the declared module, not that every NumPy function is
available.

See [`docs/INTEROPERABILITY.md`](docs/INTEROPERABILITY.md) for details.

---

## Source policy

- **No rustc source** is copied or vendored. Saturnite is implemented in
  Rust but does not reuse the Rust compiler's source.
- **No CPython source** is copied or vendored. Python interoperability
  uses the CPython C API as the runtime boundary.
- Saturnite may use Rust crates and interoperate with Python; it does not
  become a clone of either.

---

## License

Dual-licensed under MIT or Apache-2.0.
