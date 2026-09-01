# Saturnite

**Saturnite** is a small systems-programming language that compiles to native
machine code via LLVM. It draws its design from Rust: it supports `i64`, `f64`,
`bool`, and `str` types, mutable and immutable bindings, control flow,
first-class functions with recursion, ranges, user-defined structs and enums,
modules, a dedicated name-resolution pass, and a generic-function system with
turbofish syntax.

The current line — **Saturnite 0.4** — is built around a typed
control-flow graph IR (**MIR**) that is the *sole* production codegen path.
All programs go through one pipeline: `lex → parse → AST → HIR → resolve →
monomorphize → MIR → verify → optimize → MIR→LLVM → object → link`.

> See [`docs/SATURNITE_0_4_ARCHITECTURE.md`](docs/SATURNITE_0_4_ARCHITECTURE.md)
> for the authoritative architectural description and
> [`docs/SATURNITE_1_0_ROADMAP.md`](docs/SATURNITE_1_0_ROADMAP.md) for the
> path to 1.0.

---

## Quick start

```sh
# Build the compiler (requires a system C compiler for the runtime)
cargo build --release

# Run a Saturnite program (build to a temp exe, then execute)
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

```rust
fn main() -> i64 {
    println(42)
    return 0
}
```

> **Language note (0.4):** the last expression of a function body is **not**
> used as the return value — always use an explicit `return`. This matches the
> MIR model where every block ends in a terminator and `Return` is the only
> way out of a function.

> **Native syntax (0.5):** Saturnite also accepts a Python-inspired,
> indentation-aware surface syntax (`fn f() -> i64:` with an indented body,
> `give` for `return`, `say` for `println`, `main:` entry block, `module`,
> `text`/`number` type aliases). Both the legacy brace style and the native
> style may be mixed in one file. See
> [`docs/SATURNITE_SYNTAX.md`](docs/SATURNITE_SYNTAX.md). The example below is
> a native-syntax program (it lives at `examples/native_demo.stn`):

```text
module inventory_demo

struct Item:
    name: text
    price: number
    quantity: number

fn total_value(price: number, qty: number) -> number:
    give price * qty

fn restock(price: number, amount: number) -> number:
    if amount <= 0:
        raise "restock amount must be positive"
    give price + amount

main:
    let x = total_value(4, 10)
    say x
    give 0
```

---

## Features at a glance

- **Numeric & basic types** — `i64`, `f64`, `bool`, `str`, `unit`.
- **Bindings** — `let x = expr` (immutable) and `let mut x = expr` (mutable).
- **Compound assignment** — `+=`, `-=`, `*=`, `/=` on mutable locals.
- **Control flow** — `if / elif / else`, `while cond { … }`, `for i in a..b { … }`.
- **Functions** — first-class, with recursion and explicit `return`.
- **Ranges** — `a..b` (exclusive), `a..=b` / `a...b` (inclusive).
- **Structs** — `struct Point { x: i64, y: i64 }` with field-access syntax `p.x`.
- **Enums** — `enum Color { Red, Green, Blue }` and variant constructors
  (`Color::Red`) lowered to integer tags.
- **Modules & visibility** — `mod foo;`, `use math::add;`, `pub` items,
  `as` aliasing, nested directories with `mod.stnx`.
- **Generics** — type-parameterised functions and structs with turbofish
  syntax (`id::<i64>(42)`, `Box::<i64> { value: 21 }`), monomorphized to
  concrete MIR before codegen.
- **Builtins** — `println(value)` writes an integer to stdout.
- **Native codegen** — typed MIR → LLVM 21 IR → object file → system linker.
- **Cross-platform linkers** — `cc` (Linux), `clang` (macOS),
  `link.exe`/`gcc` (Windows).
- **Diagnostics** — every compile stage has a `thiserror` error type
  rendered via `miette`.
- **Multi-module projects** — `saturn.toml` discovery, `src/main.stnx`
  default entry, recursive module resolution.

---

## Code tour

The whole compiler lives under `crates/stnx/src/`. Here is what each part
does, in pipeline order.

### 1. Lexer — `src/lexer/`

A [`logos`](https://crates.io/crates/logos)-based tokenizer that produces
typed tokens with byte spans. Every keyword (`fn`, `let`, `if`, `while`,
`mod`, `use`, …) and every operator has its own variant; identifiers,
integers, floats, and strings are matched by regex.

```rust
// crates/stnx/src/lexer/token.rs
pub enum TokenKind {
    Fn, Let, Mut, If, Elif, Else, For, While, In, Return,
    I64, F64, Bool, Str, Unit, True, False, Println,
    Struct, Enum, Mod, Use, Pub, As,
    Ident(String), Integer(i64), Float(f64), StrLit(String),
    Plus, Minus, Star, Slash, Percent,
    Assign, PlusAssign, MinusAssign, StarAssign, SlashAssign,
    EqEq, NotEq, Lt, Gt, LtEq, GtEq, And, Or, Bang,
    DotDot, DotDotEllipsis, Dot, DoubleColon,
    LParen, RParen, LBrace, RBrace, Comma, Colon, RArrow,
    Error, Eof,
}
```

### 2. Parser — `src/parser/`

A [`chumsky`](https://crates.io/crates/chumsky) 0.13 parser combinator
that produces a spanned `ast::Program`. Every node carries a
`Range<usize>` so downstream phases can produce source-aware diagnostics.

### 3. AST — `src/ast.rs`

The surface syntax tree. Top-level items are functions, struct/enum
definitions, module declarations, and use declarations; statements include
`let`, assignment, augmented assignment, `if`, `while`, `for`, `return`,
and expression statements; expressions include literals, identifiers,
binary/unary ops, calls, struct literals, field access, and ranges.

### 4. HIR — `src/hir/`

The compiler's **single authoritative semantic representation**. The AST is
lowered into a typed HIR (`HirProgram`) that carries:

- interned symbol and definition tables (`SymbolId`, `DefId`),
- struct and enum definitions,
- per-module scopes (items + imports),
- the resolved `mod` graph and `use` resolutions,
- the type of every expression.

HIR types are reused as MIR types — there is no parallel type system.

### 5. Resolver — `src/resolver.rs`

A dedicated name-resolution pass introduced in 0.4 (commit `ec5138b`).
Before 0.4, name resolution was embedded inside `hir::lower`; in 0.4 it
lives in its own module so that:

- it is testable in isolation (`tests/test_resolver_dedicated.rs`),
- it can be re-run on incremental changes,
- future privacy / visibility / use-glob work has a single home.

The resolver does four things in order:

1. **Duplicate-definition detection** across functions, structs, enums,
   and `mod` declarations (per-module, keyed on `(ModuleId, SymbolId)`).
2. **Defensive re-registration** of items into their owning module's scope
   so resolution succeeds even if lowering pre-populated the scope
   only partially.
3. **Path-walk resolution** of every `use` declaration, with parent-chain
   lookups (`lookup_with_parent`) so child modules can import items from
   their parents, Rust 2018 style.
4. **Application** of resolutions to module scopes' imports table.

The CLI now invokes `analyze_and_lower_with_graph` in all three command
paths (Build, Run, Check), so multi-module projects compile end-to-end.
A real example:

```text
mod math;
use math::add;

fn main() -> i64 {
    return add(21, 21)   // → 42
}
```

### 6. Monomorphization — `src/mir/monomorphize.rs`

Generics live here. Added in commit `577dd55` as Milestone 2 of the 1.0
roadmap. The pass walks every HIR function and collects each call site
whose callee is generic; for each unique `(callee_def_id, [concrete_args])`
pair it builds a substituted `HirFunction` (parameters, return type, and
body all rewritten via `HirType::substitute`) and a fresh `DefId` +
`SymbolId`. Original call sites are retargeted to point at the new
concrete functions. The result is then lowered to MIR.

The parser supports turbofish syntax:

```rust
fn id<T>(x: T) -> T { return x }

fn main() -> i64 {
    return id::<i64>(42)   // monomorphized to id$1
}
```

Generic structs work the same way:

```rust
struct Box<T> { value: T }

fn main() -> i64 {
    let b = Box::<i64> { value: 21 }
    return b.value        // → 21
}
```

### 7. MIR — `src/mir/`

A typed control-flow graph. Every function is a `MirFunction` containing
locals and `MirBasicBlock`s; every block holds straight-line statements
and ends in exactly one `MirTerminator` (`Goto`, `SwitchInt`, `Call`,
`Return`, or `Unreachable`). The MIR is **the sole production codegen
seam**: there is no other path from HIR to LLVM.

```text
MirProgram
  ├─ functions: Vec<MirFunction>
  ├─ structs: Vec<StructDef>
  ├─ enums:   Vec<EnumDef>
  └─ symbols: SymbolInterner

MirFunction { locals, blocks, start_block }
MirBasicBlock { stmts, terminator }
```

Locals are stack-allocated (`alloca` / `store` / `load`) so that mutability
semantics survive across basic-block boundaries — including loops. This
is the correct behaviour for `let mut x = …; x = x + 1` inside a `while`.

`MirRvalue` carries the surface expressions (`Use`, `Binary`, `Unary`,
`StructLit`, `FieldAccess`, `EnumCtor`, `StrLit`), and `MirTerminator`
carries control flow. `SwitchInt` keeps a `ty: MirType` so codegen can
pick the right LLVM integer width (`i1` for `bool`, `i64` otherwise).

### 8. MIR → LLVM — `src/mir/codegen.rs`

The single codegen entry point. Walks the MIR and emits LLVM IR via
[`inkwell`](https://crates.io/crates/inkwell) 0.9 against LLVM 21. The
produced IR is handed to the shared `codegen` infrastructure for object
emission and linking.

### 9. Object emission & linking — `src/codegen/`

`ObjectEmitter` wraps an LLVM module + `TargetMachine` to write `.o`
object files or `.ll` IR text. `Linker` invokes the system linker
(`cc`/`clang`/`link.exe`) to produce the final executable. These are
shared seams — not tied to any particular IR — so future backends can
plug in without rewriting the platform glue.

### 10. Runtime — `crates/stnx/runtime/println_i64.c`

A 6-line C runtime that implements `long long println_i64(long long)`.
It is compiled at build time by `build.rs` via the `cc` crate and linked
into every Saturnite executable.

```c
#include <stdio.h>
#include <stdint.h>

long long println_i64(long long value) {
    printf("%lld\n", (long long)value);
    return 0;
}
```

### 11. Module system — `src/module.rs`

`Project::discover(start)` walks upward looking for `saturn.toml`,
parses the package metadata, and sets `source_root = <root>/src`.
`ModuleGraph::discover_modules` then scans `mod foo;` declarations and
follows `mod foo;` → either `<dir>/foo.stnx` or `<dir>/foo/mod.stnx`.

### 12. CLI — `src/main.rs`

`clap`-driven subcommands. Each command runs the full pipeline (or just
the semantic stage for `check`). The pipeline is the same one-line call
in every command:

```rust
let hir    = analyze_and_lower_with_graph(&program, &project.graph)?;
let mut mir = monomorphize(&hir)?;
mir.verify()?;
match output_kind {
    OutputKind::Ir    => generate_ir_from_mir(&mir)?,
    _                 => compile_from_mir_ext(&mir, …)?,
}
```

---

## Compiler architecture

```
Saturnite source (.stn / .stnx)
   │
   ▼
 Lexer             (logos, byte-spanned tokens)
   │
   ▼
 Parser            (chumsky 0.13 → spanned AST)
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
 MIR verify        (CFG structural integrity)
   │
   ▼
 MIR optimize      (constant folding on arithmetic / comparison / logical)
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
| Lexer              | `src/lexer/`                   | logos; tokens carry byte spans                      |
| Parser             | `src/parser/`                  | chumsky 0.13, `SimpleSpan`                          |
| AST                | `src/ast.rs`                   | every node carries a `Range<usize>` span            |
| Semantic analysis  | `src/semantic.rs`              | AST → HIR: scope-based, mutability-checked          |
| HIR                | `src/hir/`                     | typed, span-bearing; the authoritative IR           |
| Resolver           | `src/resolver.rs`              | dedicated name-resolution pass                     |
| MIR                | `src/mir/`                     | typed CFG (lower, verify, optimize)                 |
| Monomorphize       | `src/mir/monomorphize.rs`      | generic-function substitution                       |
| MIR codegen        | `src/mir/codegen.rs`           | **sole** MIR → LLVM path                            |
| Object emission    | `src/codegen/emitter.rs`       | `ObjectEmitter`: writes `.o` / `.ll` via TargetMachine |
| Linking            | `src/codegen/linker.rs`        | system linker invocation                            |
| Target config      | `src/target.rs`                | triple validation, optimization & debug levels      |
| Errors             | `src/error.rs`                 | `thiserror` + `miette::Diagnostic`                  |
| Module system      | `src/module.rs`                | `ModuleGraph`, `Project`, discovery                 |
| CLI                | `src/main.rs`                  | `build` / `check` / `run` / `doctor` / `init`       |
| Runtime            | `runtime/println_i64.c`        | compiled at build time via `build.rs` + `cc`        |

---

## A complete example

From `examples/smoke_test.stnx` — exercises functions, recursion, mutable
locals, `while`, `for`, `if/elif/else`, range iteration, and `println`:

```rust
fn factorial(n: i64) -> i64 {
    if n <= 1 {
        return 1
    }
    return n * factorial(n - 1)
}

fn is_even(n: i64) -> bool {
    return n % 2 == 0
}

fn sum_even_squares(limit: i64) -> i64 {
    let mut sum = 0
    let mut i = 0
    while i < limit {
        if is_even(i) {
            sum = sum + i * i
        }
        i = i + 1
    }
    return sum
}

fn sum_range(start: i64, end: i64) -> i64 {
    let mut sum = 0
    for i in start..end {
        sum = sum + i
    }
    return sum
}

fn classify(n: i64) -> i64 {
    if n == 0 {
        return 0
    } elif n < 0 {
        return -1
    } else {
        return 1
    }
}

fn main() -> i64 {
    let fact5    = factorial(5)
    let even_sum = sum_range(0, 10)
    let squares  = sum_even_squares(10)
    let sign     = classify(0)
    let total    = fact5 + even_sum + squares + sign
    println(fact5)
    println(even_sum)
    println(squares)
    println(sign)
    println(total)
    return total
}
```

---

## CLI reference

```
stnx build <FILE> [OPTIONS]      # Build to an executable, object, or IR
  --debug                         # Debug profile (opt 0, debug info)
  --release                       # Release profile (opt 3, no debug info)
  --target <TRIPLE>               # Cross-compilation target (host-only in 0.4)
  --opt-level <0|1|2|3>           # Override optimization level
  --emit-ir <FILE>                # Emit LLVM IR text
  --emit-object <FILE>            # Emit object file only
  --emit-exe <FILE>               # Emit executable
  --no-link                       # Stop after object emission
  --save-temps                    # Keep intermediate .o files
  --json                          # Structured build report
  --verbose                       # Verbose output
  --print-target                  # Print host triple and exit

stnx check <FILE>                # Type & semantic check (no codegen)
stnx run <FILE>                  # Build then execute
stnx doctor                      # Print environment diagnostics
stnx init [NAME]                 # Scaffold a new project
```

If `<FILE>` is omitted, the CLI looks for a `saturn.toml` project and
defaults to `src/main.stnx`.

---

## Build configuration

- **Debug profile:** `target/debug/<name>` — optimization off, debug info on.
- **Release profile:** `target/release/<name>` — optimization level 3, no debug info.

All build artifacts are placed under `target/<profile>/`. Cross-compilation
to a non-host triple is rejected with a clear diagnostic, because the
runtime (`println_i64.c`) is compiled for the host only.

---

## Project layout

```
.
├── Cargo.toml                # workspace manifest
├── saturn.toml               # default sample project manifest
├── examples/
│   ├── hello.stn             # minimal "hello world"
│   └── smoke_test.stnx       # factorials, ranges, mutability, conditionals
├── docs/                     # architecture, audits, roadmap, security
└── crates/stnx/
    ├── Cargo.toml
    ├── build.rs              # compiles the C runtime via cc
    ├── runtime/println_i64.c
    └── src/
        ├── main.rs           # CLI entrypoint
        ├── lib.rs            # public API re-exports
        ├── ast.rs            # AST nodes
        ├── lexer/            # logos tokenizer
        ├── parser/           # chumsky parser
        ├── semantic.rs       # AST → HIR entry point
        ├── hir/              # typed HIR (types, exprs, stmts, symbols)
        ├── resolver.rs       # dedicated name-resolution pass
        ├── mir/              # typed CFG (lower, verify, opt, codegen, mono)
        ├── codegen/          # ObjectEmitter, Linker (shared seams)
        ├── target.rs         # Profile, TargetConfig, triple handling
        ├── module.rs         # ModuleGraph, Project, discovery
        ├── config.rs         # saturn.toml parsing
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

## Testing

The 0.4 line ships **381 tests** across the unit, integration, and
end-to-end suites (codegen, native compilation, MIR lowering, module
graph, project loading, resolver, generics, doctor, target config,
target machine). The generics suite (`tests/test_generics.rs`)
exercises monomorphization end-to-end from source to native execution:

```rust
fn id<T>(x: T) -> T { return x }
fn main() -> i64 { return id::<i64>(42) }   // exit code 42
```

---

## What's new since 0.2

- **MIR is now the sole production codegen path** (commit `79beea8`).
  The HIR-to-LLVM direct path was removed; all programs flow through
  `lex → parse → AST → HIR → resolve → MIR → verify → optimize → MIR→LLVM`.
- **Dedicated resolver pass** (commit `ec5138b`). Name resolution moved
  out of `hir::lower` into its own module, with a structured `Resolution`
  report and full CLI integration across `build`, `check`, and `run`.
- **Generic functions and structs** (commit `577dd55`). Turbofish syntax,
  `HirType::Generic` substitution, monomorphization into concrete MIR,
  and an end-to-end test suite in `tests/test_generics.rs`.
- **Module system** with `saturn.toml` discovery, recursive `mod` walking,
  `use` resolution with parent-chain lookups, and per-module scopes.

---

## License

Dual-licensed under MIT or Apache-2.0.