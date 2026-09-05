//! Saturnite Python runtime shim — implementation.
//!
//! A thin C wrapper around the CPython C API. This is NOT a copy of CPython
//! source and does not reimplement the Python interpreter. It provides a
//! small, explicit, single-threaded surface used by the Saturnite runtime.

#include "pyrt.h"

#include <Python.h>

#include <wchar.h>

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/// The single embedded CPython interpreter. Initialized once by
/// `sat_py_init` and torn down by `sat_py_shutdown`.
static int g_interpreter_initialized = 0;

/// Storage for the most recently fetched exception instance. Held between
/// `PyErr_Fetch` and `record_exception` so the exception details can be
/// rendered into the static buffers.
static PyObject *g_stored_exception = NULL;

/// Static buffers for the most recent exception details. Valid until the
/// next bridge call that raises. Kept here so the Saturnite runtime can
/// read them without allocating.
static char g_error_class[256];
static char g_error_message[1024];

/// Next handle id to hand out for opaque Python objects.
static uint64_t g_next_handle_id = 1;

sat_py_object_handle *sat_py_handle_new(void *object) {
    sat_py_object_handle *h = (sat_py_object_handle *)malloc(sizeof(sat_py_object_handle));
    if (h == NULL) {
        return NULL;
    }
    h->id = g_next_handle_id++;
    PyObject *obj = (PyObject *)object;
    if (obj != NULL) {
        Py_INCREF(obj);
    }
    h->object = obj;
    return h;
}

static void record_exception(void) {
    // Fetch the exception instance first (clears the thread state), then
    // render it into the static buffers. We must NOT call PyErr_Fetch
    // twice: the second call would segfault because the thread state has
    // already been cleared.
    //
    // NOTE: this Python build segfaults when PyErr_Fetch is called with
    // NULL for the value/traceback out-parameters, so we pass real
    // out-parameter addresses and discard the value/traceback.
    PyObject *exc = NULL, *val = NULL, *tb = NULL;
    PyErr_Fetch(&exc, &val, &tb);
    Py_XDECREF(val);
    Py_XDECREF(tb);
    g_stored_exception = NULL;
    g_error_class[0] = '\0';
    g_error_message[0] = '\0';
    if (exc == NULL) {
        return;
    }
    PyObject *type_obj = PyObject_Type(exc);
    if (type_obj != NULL) {
        PyObject *name = PyObject_GetAttrString(type_obj, "__name__");
        if (name != NULL) {
            if (PyUnicode_Check(name)) {
                const char *s = PyUnicode_AsUTF8(name);
                if (s != NULL) {
                    strncpy(g_error_class, s, sizeof(g_error_class) - 1);
                }
            }
            Py_DECREF(name);
        }
        Py_DECREF(type_obj);
    }
    PyObject *msg = PyObject_Str(exc);
    if (msg != NULL) {
        if (PyUnicode_Check(msg)) {
            const char *s = PyUnicode_AsUTF8(msg);
            if (s != NULL) {
                strncpy(g_error_message, s, sizeof(g_error_message) - 1);
            }
        }
        Py_DECREF(msg);
    }
    Py_DECREF(exc);
}

static PyObject *build_arg(const sat_py_value *v) {
    switch (v->kind) {
    case SAT_PY_NONE:
        Py_RETURN_NONE;
    case SAT_PY_BOOL:
        if (v->as.bool_val) {
            Py_RETURN_TRUE;
        }
        Py_RETURN_FALSE;
    case SAT_PY_I64:
        return PyLong_FromLongLong(v->as.i64_val);
    case SAT_PY_F64:
        return PyFloat_FromDouble(v->as.f64_val);
    case SAT_PY_STR:
        if (v->as.str_val == NULL) {
            Py_RETURN_NONE;
        }
        return PyUnicode_FromString(v->as.str_val);
    }
    Py_RETURN_NONE;
}

static bool fill_result(sat_py_result *out, PyObject *result) {
    memset(out, 0, sizeof(*out));
    out->ok = true;
    if (result == NULL) {
        // No result value: treat as None.
        out->kind = SAT_PY_NONE;
        return true;
    }
    if (result == Py_None) {
        out->kind = SAT_PY_NONE;
        Py_DECREF(result);
        return true;
    }
    if (PyBool_Check(result)) {
        out->kind = SAT_PY_BOOL;
        out->as.bool_val = (result == Py_True);
        Py_DECREF(result);
        return true;
    }
    if (PyLong_Check(result)) {
        long long v = PyLong_AsLongLong(result);
        if (v == -1 && PyErr_Occurred()) {
            Py_DECREF(result);
            return false;
        }
        out->kind = SAT_PY_I64;
        out->as.i64_val = (int64_t)v;
        Py_DECREF(result);
        return true;
    }
    if (PyFloat_Check(result)) {
        out->kind = SAT_PY_F64;
        out->as.f64_val = PyFloat_AsDouble(result);
        Py_DECREF(result);
        return true;
    }
    if (PyUnicode_Check(result)) {
        Py_ssize_t len = 0;
        const char *s = PyUnicode_AsUTF8AndSize(result, &len);
        if (s == NULL) {
            Py_DECREF(result);
            return false;
        }
        out->kind = SAT_PY_STR;
        out->str_len = (size_t)len;
        out->as.str_buf = (char *)malloc((size_t)len + 1);
        if (out->as.str_buf == NULL) {
            Py_DECREF(result);
            return false;
        }
        memcpy(out->as.str_buf, s, (size_t)len);
        out->as.str_buf[len] = '\0';
        Py_DECREF(result);
        return true;
    }
    // Unsupported / dynamic value: hand back an opaque handle. The handle
    // takes ownership of the reference returned by the call.
    out->kind = SAT_PY_NONE;
    out->handle = sat_py_handle_new(result);
    Py_DECREF(result);
    return out->handle != NULL;
}

bool sat_py_init(const char *program_name) {
    if (g_interpreter_initialized) {
        // Already initialized; do not re-initialize.
        return true;
    }
    if (program_name == NULL || program_name[0] == '\0') {
        program_name = "saturnite";
    }
    // `Py_SetProgramName` takes a wide string in Python 3.13. Convert the
    // UTF-8 program name to the host wide encoding. The buffer is static:
    // CPython 3.13 stores the pointer directly (no copy), so a stack
    // buffer would dangle after this function returns.
    static wchar_t wide_name[256];
    size_t converted = mbstowcs(wide_name, program_name, sizeof(wide_name) / sizeof(wide_name[0]) - 1);
    if (converted == (size_t)-1) {
        converted = 0;
    }
    wide_name[converted] = L'\0';
    Py_SetProgramName(wide_name);
    Py_Initialize();
    if (!Py_IsInitialized()) {
        fprintf(stderr, "Saturnite: failed to initialize the Python interpreter.\n");
        return false;
    }
    g_interpreter_initialized = 1;
    return true;
}

sat_py_object_handle *sat_py_import_module(const char *name, const char *search_path) {
    if (name == NULL) {
        return NULL;
    }
    PyObject *sys_path = PySys_GetObject("path");
    if (sys_path == NULL) {
        return NULL;
    }
    if (search_path != NULL && search_path[0] != '\0') {
        char *copy = strdup(search_path);
        if (copy == NULL) {
            return NULL;
        }
        char *saveptr = NULL;
        char *dir = strtok_r(copy, ":", &saveptr);
        while (dir != NULL) {
            PyObject *py_dir = PyUnicode_FromString(dir);
            if (py_dir != NULL) {
                PyList_Append(sys_path, py_dir);
                Py_DECREF(py_dir);
            }
            dir = strtok_r(NULL, ":", &saveptr);
        }
        free(copy);
    }
    PyObject *mod_name = PyUnicode_FromString(name);
    if (mod_name == NULL) {
        return NULL;
    }
    PyObject *mod = PyImport_Import(mod_name);
    Py_DECREF(mod_name);
    if (mod == NULL) {
        record_exception();
        return NULL;
    }
    return sat_py_handle_new(mod);
}

sat_py_object_handle *sat_py_get_callable(sat_py_object_handle *object, const char *name) {
    if (object == NULL || name == NULL) {
        return NULL;
    }
    PyObject *obj = (PyObject *)object->object;
    if (obj == NULL) {
        return NULL;
    }
    PyObject *attr = PyObject_GetAttrString(obj, name);
    if (attr == NULL) {
        record_exception();
        return NULL;
    }
    if (!PyCallable_Check(attr)) {
        Py_DECREF(attr);
        PyErr_Format(PyExc_TypeError, "'%s' is not callable", name);
        record_exception();
        return NULL;
    }
    return sat_py_handle_new(attr);
}

/// Core call implementation shared by `sat_py_call` and
/// `sat_py_call_flat`. `callable` must be a valid owned handle; on success
/// the caller still owns the handle and must release it.
static bool call_with_values(sat_py_object_handle *callable,
                             const sat_py_value *args,
                             size_t arg_count,
                             sat_py_result *out) {
    if (out == NULL) {
        return false;
    }
    memset(out, 0, sizeof(*out));
    if (callable == NULL) {
        out->ok = false;
        snprintf(g_error_class, sizeof(g_error_class), "RuntimeError");
        snprintf(g_error_message, sizeof(g_error_message), "no callable provided");
        out->error_class = g_error_class;
        out->error_message = g_error_message;
        return true;
    }
    PyObject *callable_obj = (PyObject *)callable->object;
    if (callable_obj == NULL || !PyCallable_Check(callable_obj)) {
        out->ok = false;
        snprintf(g_error_class, sizeof(g_error_class), "RuntimeError");
        snprintf(g_error_message, sizeof(g_error_message), "object is not callable");
        out->error_class = g_error_class;
        out->error_message = g_error_message;
        return true;
    }
    // Always pass a real tuple to PyObject_CallObject, even when the call
    // takes no arguments. Passing NULL for the args tuple is undefined
    // behaviour in CPython: when the callable raises, the interpreter's
    // error handling dereferences the NULL tuple and segfaults.
    PyObject *py_args = PyTuple_New((Py_ssize_t)arg_count);
    if (py_args == NULL) {
        return false;
    }
    for (size_t i = 0; i < arg_count; i++) {
        PyObject *arg = build_arg(&args[i]);
        if (arg == NULL) {
            Py_DECREF(py_args);
            return false;
        }
        PyTuple_SET_ITEM(py_args, (Py_ssize_t)i, arg);
    }
    PyObject *result = PyObject_CallObject(callable_obj, py_args);
    Py_DECREF(py_args);
    if (result == NULL) {
        record_exception();
        out->ok = false;
        out->error_class = g_error_class;
        out->error_message = g_error_message;
        return true;
    }
    return fill_result(out, result);
}

bool sat_py_call(sat_py_object_handle *callable, const sat_py_value *args, size_t arg_count, sat_py_result *out) {
    return call_with_values(callable, args, arg_count, out);
}

bool sat_py_call_flat(const char *spec, const char *search_path,
                      const int32_t *kinds, const int64_t *values,
                      size_t arg_count, sat_py_result *out) {
    if (out == NULL) {
        return false;
    }
    memset(out, 0, sizeof(*out));
    if (spec == NULL) {
        out->ok = false;
        snprintf(g_error_class, sizeof(g_error_class), "RuntimeError");
        snprintf(g_error_message, sizeof(g_error_message), "no Python spec provided");
        out->error_class = g_error_class;
        out->error_message = g_error_message;
        return true;
    }
    if (!sat_py_init(NULL)) {
        out->ok = false;
        snprintf(g_error_class, sizeof(g_error_class), "RuntimeError");
        snprintf(g_error_message, sizeof(g_error_message), "failed to initialize Python interpreter");
        out->error_class = g_error_class;
        out->error_message = g_error_message;
        return true;
    }

    // Split "module::func" on the first "::".
    char *spec_copy = strdup(spec);
    if (spec_copy == NULL) {
        return false;
    }
    char *module = spec_copy;
    char *func = strstr(spec_copy, "::");
    if (func == NULL) {
        free(spec_copy);
        out->ok = false;
        snprintf(g_error_class, sizeof(g_error_class), "RuntimeError");
        snprintf(g_error_message, sizeof(g_error_message),
                 "invalid Python spec '%s': expected 'module::func'", spec);
        out->error_class = g_error_class;
        out->error_message = g_error_message;
        return true;
    }
    *func = '\0';
    func += 2;

    sat_py_object_handle *mod = sat_py_import_module(module, search_path);
    if (mod == NULL) {
        free(spec_copy);
        record_exception();
        out->ok = false;
        out->error_class = g_error_class;
        out->error_message = g_error_message;
        return true;
    }
    sat_py_object_handle *callable = sat_py_get_callable(mod, func);
    sat_py_release_handle(mod);
    if (callable == NULL) {
        free(spec_copy);
        record_exception();
        out->ok = false;
        out->error_class = g_error_class;
        out->error_message = g_error_message;
        return true;
    }

    // Rebuild sat_py_value array from the flat kinds/values ABI.
    sat_py_value *sv = (sat_py_value *)malloc(sizeof(sat_py_value) * (arg_count > 0 ? arg_count : 1));
    if (sv == NULL) {
        sat_py_release_handle(callable);
        free(spec_copy);
        return false;
    }
    for (size_t i = 0; i < arg_count; i++) {
        sv[i].kind = (sat_py_value_kind)kinds[i];
        switch (sv[i].kind) {
        case SAT_PY_NONE:
            sv[i].as.bool_val = false;
            break;
        case SAT_PY_BOOL:
            sv[i].as.bool_val = values[i] != 0;
            break;
        case SAT_PY_I64:
            sv[i].as.i64_val = (int64_t)values[i];
            break;
        case SAT_PY_F64:
            memcpy(&sv[i].as.f64_val, &values[i], sizeof(double));
            break;
        case SAT_PY_STR:
            sv[i].as.str_val = (const char *)(intptr_t)values[i];
            break;
        default:
            sv[i].kind = SAT_PY_NONE;
            sv[i].as.bool_val = false;
            break;
        }
    }
    bool ok = call_with_values(callable, sv, arg_count, out);
    free(sv);
    sat_py_release_handle(callable);
    free(spec_copy);
    return ok;
}

void sat_py_release_handle(void *handle) {
    if (handle == NULL) {
        return;
    }
    sat_py_object_handle *h = (sat_py_object_handle *)handle;
    if (h->object != NULL) {
        Py_DECREF(h->object);
        h->object = NULL;
    }
    free(h);
}

void sat_py_free_result_str(char *buf) {
    free(buf);
}

void sat_py_shutdown(void) {
    if (g_interpreter_initialized) {
        Py_FinalizeEx();
        g_interpreter_initialized = 0;
    }
}