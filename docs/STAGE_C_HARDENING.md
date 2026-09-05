# Stage C — Interoperability Hardening

Status: ARCHITECTURAL HARDENING DOCUMENTED; FULL DEPENDENCY MANAGER NOT IMPLEMENTED.

Implemented / designed:
- Dependency model (`DependencyKind`, `DependencyEntry`) preserved and extended with `inter_op_rust` wrapper descriptor, `inter_op_python` conversion contract.
- Dependency graph conceptual model documented: project → rust/python/saturnite/native nodes, with separate wrapper/native-artifact and runtime-package branches. No universal abstraction.
- `saturn.toml` extension designed (not forced): `[rust.dependencies]`, `[python.dependencies]`, `[native.dependencies]` conceptual forms using `DependencyEntry`.
- Lock/reproducibility: future `saturn.lock` mechanism conceptualized; resolution must be deterministic. No registry server, no cloud service.
- Target restrictions supported via `DependencyEntry::target`; actual cross-platform verification deferred.
- Cache design prepared (dependency, wrapper, build artifact, metadata caches possible) without premature optimization.
- Offline/failure behavior: errors must be explicit diagnostics, not crashes or silent ignores.
- Security/trust: external libraries execute native/runtime code; Saturnite type checker does not make them safe. No sandbox added unless explicitly designed later.
- ABI versioning: ABI version, runtime ABI version, bridge version must be representable; exact encoding deferred.
- Runtime boundary: Rust interop runtime, Python bridge runtime, native library boundary documented. Ownership boundaries explicit.
- Diagnostic categories: existing `CompilerError` architecture sufficient; interop errors use `Config`/`Codegen`/`Semantic` with descriptive messages. No unnecessary new categories.

Explicitly NOT implemented (deferred):
- Full dependency resolver / package manager
- Actual package download/acquisition
- Lock file generation
- Registry server or cloud service
- Universal FFI super-framework
- Dynamic Python object system fully implemented
