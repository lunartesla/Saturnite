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

#include <Python.h>

#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>
#include <string.h>

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
    /// object has been released.
    PyObject *object;
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
/// `ok` is `true` when the call succeeded and `result` holds the value.
/// When `ok` is `false`, `error_kind` and `error_message` describe the
/// Python exception that was raised.
typedef struct sat_py_result {
    bool ok;
    sat_py_value_kind kind;
    union {
        bool bool_val;
        int64_t i64_val;
        double f64_val;
        /// Caller-owned buffer filled with the UTF-8 result string. The
        /// caller must provide a buffer of at least `str_len + 1` bytes.
        char *str_buf;
    } as;
    /// Length (in bytes) of the result string, excluding the NUL terminator.
    size_t str_len;
    /// When `ok` is false: the Python exception class name (caller-owned,
    /// NUL-terminated).
    const char *error_class;
    /// When `ok` is false: the Python exception message (caller-owned,
    /// NUL-terminated).
    const char *error_message;
} sat_py_result;

/// Initialize the embedded CPython interpreter.
///
/// This must be called exactly once, before any other Python bridge call,
/// and must not be called again while the interpreter is already
/// initialized. Returns `true` on success.
bool sat_py_init(const char *program_name);

/// Import a Python module by qualified name, searching `search_path` (a
/// PATH-style list of directories separated by the platform path
/// separator). Returns a non-NULL `PyObject*` cast to `void*` on success,
/// or NULL with the last Python error recorded.
void *sat_py_import_module(const char *name, const char *search_path);

/// Look up a callable attribute on a module/object. `object` is a handle
/// previously returned by `sat_py_import_module` or another bridge call.
/// Returns a non-NULL `PyObject*` cast to `void*` on success, NULL on
/// failure (attribute missing or not callable).
void *sat_py_get_callable(void *object, const char *name);

/// Call a callable Python object with the given arguments.
///
/// `callable` is a handle from `sat_py_get_callable`. `args` is a array of
/// `arg_count` values. The result is written into `out`.
///
/// Returns `true` on success. On `false`, `out->ok` is false and the
/// exception details are filled in.
bool sat_py_call(void *callable, const sat_py_value *args, size_t arg_count, sat_py_result *out);

/// Release a Python object handle, decrementing its reference count.
/// Safe to call with NULL.
void sat_py_release_handle(void *handle);

/// Release the result string buffer back to the caller.
///
/// The bridge allocates the string for `SAT_PY_STR` results with
/// `PyUnicode_AsUTF8AndSize`-equivalent semantics; the caller owns the
/// buffer after a successful string result and must free it.
void sat_py_free_result_str(char *buf);

/// Shut down the embedded CPython interpreter.
///
/// Must be called exactly once, after all other Python bridge calls have
/// completed and after all object handles have been released. Calling
/// this while handles are still outstanding is undefined behaviour.
void sat_py_shutdown(void);