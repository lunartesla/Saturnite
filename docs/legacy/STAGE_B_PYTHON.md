> ⚠️  LEGACY DOCUMENT — NOT THE CURRENT SATURNITE LANGUAGE SPECIFICATION.
> This file is archived for historical reference only. It describes prior
> design audits, roadmaps, or stale architecture from before the native
> syntax migration. Do not use it as a reference for current language
> features, syntax, or compiler behavior. The canonical reference is
> `docs/SATURNITE_SYNTAX.md` and the top-level `README.md`.


# Stage B — Python Interoperability Boundary

Status: CONTRACT DEFINED; FULL CPYTHON EXECUTION DEFERRED.

Reason for deferral: safe Python runtime integration requires interpreter initialization, GIL interaction, reference counting, and exception handling that cannot be safely added without larger runtime architecture review. Faking Python execution would violate the absolute rule.

Implemented:
- `interop_python.rs`: conversion contract (`PythonConversion` enum)
- Fixture preserved (`fixtures/python/test_math.py`)
- Design docs (`PHASE9_INTEROP.md`, `PHASE9_DESIGN.md`) describe value boundary, lifetime requirements, exception behavior, and opaque `PythonObject` design

Documented boundary rules:
- Primitive conversions designed (int↔int, float↔float, bool↔bool, str↔str, List<I64>↔list[int], Unit↔None)
- Unsupported Python objects must map to opaque handle, not arbitrary Saturnite values
- Interpreter lifetime must outlive all Python calls
- Exceptions must become controlled Saturnite errors (no silent swallow, no arbitrary process abort)
- Threading: first implementation intentionally single-threaded; GIL/thread-safety deferred and documented

Deferred to future phase:
- CPython runtime integration
- Actual Python call execution
- Automatic package installation/acquisition
- Python object opaque handles fully implemented
- Multi-threaded Python interoperability
