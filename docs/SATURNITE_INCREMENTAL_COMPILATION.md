# Incremental Compilation Design — Phase 14

## Status: Design Proposal

## Overview

This document describes the incremental compilation design for Saturnite 0.3.
Incremental compilation avoids recompiling unchanged code by caching compilation
artifacts and only reprocessing modules whose source (or dependencies) have changed.

## 1. Motivation

- **Fast edit-compile cycles:** Developers see near-instant rebuilds when editing a single file.
- **Scalable builds:** Large projects only recompile the subset of modules affected.
- **CI efficiency:** Incremental builds enable caching of build artifacts in CI.

## 2. High-Level Architecture

```
Source → Fingerprint → (cache hit? skip to cached HIR : Parse+Lower → Semantic check → Codegen) → Cache → Link
```

## 3. Fingerprinting Strategy

```
Fingerprint = SHA-256(source_content || dependencies_hashes || config_hash)
```

### Cache Layout

```
target/
├── incremental/
│   ├── fingerprints.json
│   ├── hir/<fingerprint>.hir
│   ├── objects/<fingerprint>.o
│   └── metadata.json
└── debug/
```

## 4. Implementation Plan

### Phase 14a: Fingerprinting
- Add `fingerprint` module with `Fingerprint::compute()`.
- Store fingerprints in `target/incremental/fingerprints.json`.

### Phase 14b: HIR Caching
- Add `Serialize`/`Deserialize` derives to `HirProgram`.
- Cache serialized HIR keyed by fingerprint.

### Phase 14c: Object File Caching
- Cache LLVM object files keyed by fingerprint.
- Skip codegen on cache hit; only re-link when objects change.

### Phase 14d: Dependency-Aware Invalidation
- Track import relationships in HIR.
- Invalidate all transitive dependents when a module changes.

## 5. Integration with Existing Pipeline

Current: `Source → Lexer → Parser → AST → HIR Lowering → LLVM Codegen`
Incremental: `Source → [Fingerprint check] → (cached HIR | Parse + Lower) → (cached obj | Codegen) → Link`

## 6. Cache Invalidation Policy

| Event | Action |
|-------|--------|
| Source file changed | Recompile + all dependents |
| saturn.toml changed | Full rebuild |
| Compiler version changed | Full rebuild |
| opt level / debug changed | Full rebuild |
| New file added | Compile new + dependents |
| File deleted | Recompile dependents |

## 7. Correctness Considerations

- **Cache poisoning:** Detect fingerprint mismatch → fall back to full rebuild. Provide `--clean`.
- **Concurrency:** File locking on `target/incremental/`.
- **Determinism:** Byte-identical output for same input.

## 8. Performance Targets

| Project size | Cold build | Incremental | Target speedup |
|-------------|-----------|-------------|----------------|
| 10 files | < 2s | < 0.5s | 4x |
| 100 files | < 10s | < 1s | 10x |
| 1000 files | < 60s | < 5s | 12x |

## 9. Alternatives Considered

- **Rustc-style query system:** Defered to future phase due to complexity.
- **mtime-based invalidation:** Rejected; fingerprints are more reliable.
