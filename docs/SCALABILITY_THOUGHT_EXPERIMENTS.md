# Phase 0.5.2 Scalability Thought Experiments

Status: ARCHITECTURAL EXERCISES (not implementation)

A. 20 builtins
- `BuiltinRegistry` holds metadata; special lowering (interpolation chain) remains explicit. Adding 15 more standard builtins should only require adding entries to registry and their runtime C functions. No redesign needed. RISK: low.

B. List<T>
- Would require a new `HirType` variant (e.g. `List(Box<HirType>)`) and runtime allocation. The runtime boundary document makes this feasible without redesigning the built-in registry. The registry does NOT become a redesign point; runtime ABI interface is the relevant boundary. RISK: medium (runtime ABI design needed, but architecture allows it).

C. Closures
- Would require new AST expression kind (`ExprKind::Closure` or lambda), new HIR expression kind, new MIR representation (function pointer / environment struct), and new codegen mapping. The pipeline stages remain appropriate; no stage elimination needed. The architecture does not destabilize unrelated stages. RISK: medium (new representations needed at each stage, but pipeline is sound).

D. External Rust-lang crate
- Could be exposed without rustc source reuse via an FFI layer or a crate loader that links against `rustc` library interfaces (not copying source). The architecture leaves room because the compiler does not embed rustc internals; it uses its own HIR/MIR/codegen pipeline. RISK: medium (FFI design required, but no source reuse needed).

E. Python library
- Could be exposed through a deliberate interoperability layer (PyO3, CFFI, or a Python runtime embedding layer). The architecture does not block it; runtime boundary is the insertion point. RISK: medium.

F. 100-module project
- `ModuleGraph` uses `Vec<Module>` and `HashMap<ModulePath, ModuleId>`. Lookup is numeric equality for `SymbolId` paths. Cycle detection uses DFS and is O(V+E). No global string `HashMap`. Should scale. RISK: low.

G. 1,000 diagnostics
- `ErrCategory` supports categories; no structured code assignment yet. With the category convention (`E0xxx` etc.) and a documented numbering scheme, 1,000 diagnostics become manageable. RISK: low once numbering is completed.
