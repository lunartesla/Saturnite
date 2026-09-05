//! Saturnite Python runtime shim.
//!
//! This is a *thin* C wrapper around the CPython C API. It is NOT a copy of
//! CPython source and does not reimplement the Python interpreter. It only
//! provides a small, explicit, single-threaded surface that the Saturnite
//! runtime calls to:
//!
//! * initialize and shut down a single embedded CPython interpreter;
//! * import a named module from a declared search path;
//! * look up a callable attribute on a module;
//! * call it with a fixed set of Saturnite values and receive a result;
//! * convert results back into Saturnite values (or an opaque handle);
//! * propagate Python exceptions as structured Saturnite errors.
//!
//! The interpreter is initialized once and lives for the lifetime of the
//! Saturnite executable. All Python calls are serialized on the calling
//! thread; the bridge is single-threaded by design (see
//! `PYTHON_SINGLE_THREADED`).

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/// Opaque handle to a Python object that has no direct Saturnite
/// representation (numpy arrays, custom classes, file objects, database
/// connections, ...). The handle owns a reference to the underlying Python
/// object; it must be released with `sat_py_release_handle` when the
/// Saturnite program is done with it.
///
/// The `id` is a small runtime-assigned identifier used for diagnostics.
/// It is NOT a raw CPython pointer exposed to normal Saturnite code.
typedef struct sat_py_object_handle {
    uint64_t id;
    /// The underlying PyObject, owned by this handle. May be NULL if the
    /// object has been released. Declared as an opaque pointer here so the
    /// public header does not depend on Python.h; the implementation casts
    /// it to `PyObject*` internally.
    void *object;
} sat_py_object_handle;

/// A value passed into a Python call from the Saturnite side.
///
/// The `kind` discriminates the union; the caller guarantees that the
/// matching union field is valid.
typedef enum sat_py_value_kind {
    SAT_PY_NONE,
    SAT_PY_BOOL,
    SAT_PY_I64,
    SAT_PY_F64,
    SAT_PY_STR,
} sat_py_value_kind;

typedef struct sat_py_value {
    sat_py_value_kind kind;
    union {
        bool bool_val;
        int64_t i64_val;
        double f64_val;
        /// NUL-terminated UTF-8 string. Owned by the caller for the duration
        /// of the call; the bridge copies it into a Python `str`.
        const char *str_val;
    } as;
} sat_py_value;

/// A value returned from a Python call back to the Saturnite side.
///
/// * `ok == true` and `handle == NULL`: the result is a primitive value
///   stored in `kind`/`as`. For `SAT_PY_STR`, the bridge allocates
///   `as.str_buf` (a NUL-terminated UTF-8 copy) and sets `str_len`; the
///   caller must free it with `sat_py_free_result_str`.
/// * `ok == true` and `handle != NULL`: the result is an opaque Python
///   object the caller owns a reference to via the handle.
/// * `ok == false`: the call raised a Python exception. `error_class` and
///   `error_message` describe it.
typedef struct sat_py_result {
    bool ok;
    sat_py_value_kind kind;
    union {
        bool bool_val;
        int64_t i64_val;
        double f64_val;
        /// Bridge-allocated, NUL-terminated UTF-8 copy of the result string.
        /// Freed by the caller with `sat_py_free_result_str`.
        char *str_buf;
    } as;
    /// Length (in bytes) of the result string, excluding the NUL terminator.
    size_t str_len;
    /// When `ok` is false: the Python exception class name (static buffer;
    /// valid until the next bridge call).
    const char *error_class;
    /// When `ok` is false: the Python exception message (static buffer;
    /// valid until the next bridge call).
    const char *error_message;
    /// Non-NULL when the result is an opaque Python object. The caller owns
    /// a reference via this handle and must release it.
    sat_py_object_handle *handle;
} sat_py_result;

/// Initialize the embedded CPython interpreter.
///
/// This must be called exactly once, before any other Python bridge call,
/// and must not be called again while the interpreter is already
/// initialized. Returns `true` on success.
bool sat_py_init(const char *program_name);

/// Allocate a fresh owned handle wrapping a borrowed reference to `object`.
///
/// Increments the reference count of `object`. The returned handle must be
/// released with `sat_py_release_handle`. Returns NULL on allocation
/// failure. `object` may be NULL (produces a NULL handle).
///
/// `object` is an opaque pointer to a `PyObject`; the public header does
/// not depend on Python.h, so callers pass the pointer cast to `void*`.
sat_py_object_handle *sat_py_handle_new(void *object);

/// Import a Python module by qualified name, searching `search_path` (a
/// PATH-style list of directories separated by the platform path
/// separator). Returns an owned handle on success, or NULL with the last
/// Python error recorded.
sat_py_object_handle *sat_py_import_module(const char *name, const char *search_path);

/// Look up a callable attribute on a module/object. `object` is a handle
/// previously returned by `sat_py_import_module` or another bridge call.
/// Returns an owned handle on success, NULL on failure (attribute missing
/// or not callable).
sat_py_object_handle *sat_py_get_callable(sat_py_object_handle *object, const char *name);

/// Call a callable Python object with the given arguments.
///
/// `callable` is an owned handle from `sat_py_get_callable`. `args` is an
/// array of `arg_count` values. The result is written into `out`. The
/// caller owns any returned handle or string buffer.
///
/// Returns `true` on success (including when the Python code raised, which
/// is reported in `out` with `ok == false`). Returns `false` only when the
/// bridge itself failed (e.g. the callable is NULL).
bool sat_py_call(sat_py_object_handle *callable, const sat_py_value *args, size_t arg_count, sat_py_result *out);

/// High-level helper: resolve and call a Python function by spec.
///
/// `spec` is a `"module::func"` string. `search_path` is a PATH-style
/// list of directories (or NULL). `kinds` and `values` are parallel
/// arrays of `arg_count` entries; `values` is an array of `int64_t`
/// (bool/i64/str-ptr packed as i64; f64 bit-cast to i64). The result is
/// written into `out`.
///
/// This wraps init + import + get_callable + call + release so the
/// Saturnite codegen can emit a single call per external Python call.
///
/// Returns `true` on success (including when the Python code raised, which
/// is reported in `out` with `ok == false`). Returns `false` only when the
/// bridge itself failed (e.g. the interpreter could not be initialized).
bool sat_py_call_flat(const char *spec, const char *search_path, const int32_t *kinds, const int64_t *values, size_t arg_count, sat_py_result *out);

/// Release a Python object handle, decrementing its reference count.
/// Safe to call with NULL.
void sat_py_release_handle(void *handle);

/// Free a result string buffer allocated by the bridge.
void sat_py_free_result_str(char *buf);

/// Shut down the embedded CPython interpreter.
///
/// Must be called exactly once, after all other Python bridge calls have
/// completed and after all object handles have been released. Calling
/// this while handles are still outstanding is undefined behaviour.
void sat_py_shutdown(void);

#ifdef __cplusplus
}
#endif