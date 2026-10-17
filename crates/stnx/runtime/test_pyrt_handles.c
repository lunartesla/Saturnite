/*
 * Tests for the Python object handle and method-call API declared in pyrt.h.
 *
 * Exercises:
 *   - sat_py_init / sat_py_shutdown lifecycle
 *   - sat_py_handle_new / sat_py_release_handle
 *   - sat_py_import_and_cache / sat_py_get_cached_module
 *   - sat_py_get_attr
 *   - sat_py_call_method
 *
 * These tests require an embedded CPython interpreter. They are compiled and
 * linked into a standalone executable by the test harness and run as part of
 * `cargo test` when Python development files are available.
 */

#include "pyrt.h"
#include <stdio.h>
#include <string.h>
#include <stdlib.h>

static int test_count = 0;
static int fail_count = 0;

#define TEST(fn)                          \
    do {                                  \
        test_count++;                     \
        printf("  [run] %s\n", #fn);     \
        if (fn() != 0) {                  \
            fail_count++;                 \
            printf("  [FAIL] %s\n", #fn); \
        } else {                          \
            printf("  [PASS] %s\n", #fn); \
        }                                 \
    } while (0)

#define ASSERT(cond, msg)                                    \
    do {                                                    \
        if (!(cond)) {                                      \
            fprintf(stderr, "    assert failed: %s\n", msg); \
            return 1;                                       \
        }                                                   \
    } while (0)

/* Test: initialize and shut down the interpreter cleanly. */
static int test_init_shutdown(void) {
    ASSERT(sat_py_init("test_pyrt_handles") == true, "init should succeed");
    sat_py_shutdown();
    return 0;
}

/* Test: handle_new with NULL returns a handle with NULL object. */
static int test_handle_new_null(void) {
    sat_py_object_handle *h = sat_py_handle_new(NULL);
    /* sat_py_handle_new with NULL object: the handle is allocated but
       object is NULL. It should be released without crashing. */
    ASSERT(h != NULL, "handle should be allocated even for NULL object");
    ASSERT(h->object == NULL, "handle->object should be NULL");
    sat_py_release_handle(h);
    /* Releasing NULL should be a no-op. */
    sat_py_release_handle(NULL);
    return 0;
}

/* Test: import a built-in module and get an attribute from it. */
static int test_import_and_get_attr(void) {
    ASSERT(sat_py_init("test_pyrt_handles") == true, "init should succeed");

    sat_py_object_handle *sys = sat_py_import_module("sys", NULL);
    ASSERT(sys != NULL, "should import sys module");

    /* sys.stdout is a valid attribute. */
    sat_py_object_handle *stdout_handle = sat_py_get_attr(sys, "stdout");
    ASSERT(stdout_handle != NULL, "should get sys.stdout attribute");
    ASSERT(stdout_handle->object != NULL, "stdout object should not be NULL");
    sat_py_release_handle(stdout_handle);

    sat_py_release_handle(sys);
    sat_py_shutdown();
    return 0;
}

/* Test: sat_py_import_and_cache + sat_py_get_cached_module. */
static int test_import_and_cache(void) {
    ASSERT(sat_py_init("test_pyrt_handles") == true, "init should succeed");

    sat_py_object_handle *h1 = sat_py_import_and_cache("sys", NULL);
    ASSERT(h1 != NULL, "import_and_cache should return a handle");
    sat_py_release_handle(h1);

    /* The cache should now have 'sys'. get_cached_module returns an owned
       handle (a new reference). */
    sat_py_object_handle *cached = sat_py_get_cached_module("sys");
    ASSERT(cached != NULL, "get_cached_module should find 'sys'");
    sat_py_release_handle(cached);

    /* A non-cached module should return NULL. */
    sat_py_object_handle *none = sat_py_get_cached_module("nonexistent_module_xyz");
    ASSERT(none == NULL, "get_cached_module for unknown module should return NULL");

    sat_py_shutdown();
    return 0;
}

/* Test: sat_py_get_attr on a missing attribute returns NULL. */
static int test_get_attr_missing(void) {
    ASSERT(sat_py_init("test_pyrt_handles") == true, "init should succeed");

    sat_py_object_handle *sys = sat_py_import_module("sys", NULL);
    ASSERT(sys != NULL, "should import sys");

    /* 'nonexistent_attr_xyz' does not exist on sys. */
    sat_py_object_handle *bad = sat_py_get_attr(sys, "nonexistent_attr_xyz_abc");
    ASSERT(bad == NULL, "get_attr for missing attr should return NULL");

    sat_py_release_handle(sys);
    sat_py_shutdown();
    return 0;
}

/* Test: sat_py_call_method — call a method on a Python object via handle. */
static int test_call_method_on_string(void) {
    ASSERT(sat_py_init("test_pyrt_handles") == true, "init should succeed");

    sat_py_object_handle *builtins = sat_py_import_module("builtins", NULL);
    ASSERT(builtins != NULL, "should import builtins");

    /* Create a Python string via builtins.str(42) → call the str callable
       with an i64 arg. */
    sat_py_result out;
    memset(&out, 0, sizeof(out));

    sat_py_object_handle *str_callable = sat_py_get_callable(builtins, "str");
    ASSERT(str_callable != NULL, "should get builtins.str callable");

    /* Call str(42) — pass a sat_py_value with kind SAT_PY_I64. */
    sat_py_value args[1];
    args[0].kind = SAT_PY_I64;
    args[0].as.i64_val = 42;

    bool ok = sat_py_call(str_callable, args, 1, &out);
    ASSERT(ok == true, "sat_py_call should return true (bridge success)");
    ASSERT(out.ok == true, "result ok should be true");
    /* str(42) produces the string "42". */
    ASSERT(out.kind == SAT_PY_STR, "str(42) result should be a string");

    sat_py_free_result_str(out.as.str_buf);
    sat_py_release_handle(str_callable);
    sat_py_release_handle(builtins);
    sat_py_shutdown();
    return 0;
}

/* Test: sat_py_call_method — call 'upper' on a Python string object. */
static int test_call_method_upper(void) {
    ASSERT(sat_py_init("test_pyrt_handles") == true, "init should succeed");

    sat_py_object_handle *builtins = sat_py_import_module("builtins", NULL);
    ASSERT(builtins != NULL, "should import builtins");

    sat_py_object_handle *str_callable = sat_py_get_callable(builtins, "str");
    ASSERT(str_callable != NULL, "should get builtins.str");

    /* str("hello") → a Python str object. */
    const char *hello = "hello";
    sat_py_result out;
    memset(&out, 0, sizeof(out));

    sat_py_value args[1];
    args[0].kind = SAT_PY_STR;
    args[0].as.str_val = hello;

    bool ok = sat_py_call(str_callable, args, 1, &out);
    ASSERT(ok == true, "sat_py_call should return true");
    ASSERT(out.ok == true, "result ok should be true");
    ASSERT(out.kind == SAT_PY_STR, "result should be a string");
    /* The result is a handle (opaque object). */
    ASSERT(out.handle != NULL, "str result should have a handle");

    sat_py_object_handle *str_obj = out.handle;
    sat_py_release_handle(str_callable);

    /* Now call str_obj.upper() via sat_py_call_method. */
    sat_py_result out2;
    memset(&out2, 0, sizeof(out2));
    ok = sat_py_call_method(str_obj, "upper", NULL, NULL, 0, &out2);
    ASSERT(ok == true, "sat_py_call_method should return true");
    ASSERT(out2.ok == true, "method call result ok should be true");
    ASSERT(out2.kind == SAT_PY_STR, "upper() result should be a string");

    sat_py_free_result_str(out2.as.str_buf);
    sat_py_release_handle(str_obj);
    sat_py_release_handle(builtins);
    sat_py_shutdown();
    return 0;
}

/* Test: sat_py_call_method error handling — missing method. */
static int test_call_method_missing(void) {
    ASSERT(sat_py_init("test_pyrt_handles") == true, "init should succeed");

    sat_py_object_handle *builtins = sat_py_import_module("builtins", NULL);
    ASSERT(builtins != NULL, "should import builtins");

    sat_py_object_handle *str_callable = sat_py_get_callable(builtins, "str");
    ASSERT(str_callable != NULL, "should get builtins.str");

    /* str("hello") */
    const char *hello = "hello";
    int32_t kinds[] = { SAT_PY_STR };
    int64_t values[] = { (int64_t)(intptr_t)hello };
    sat_py_result out;
    memset(&out, 0, sizeof(out));

    bool ok = sat_py_call(str_callable, kinds, 1, &out);
    ASSERT(ok == true, "sat_py_call should succeed");
    ASSERT(out.ok == true, "result ok should be true");
    ASSERT(out.handle != NULL, "result should have a handle");

    sat_py_object_handle *str_obj = out.handle;
    sat_py_release_handle(str_callable);

    /* Call a non-existent method. */
    sat_py_result out2;
    memset(&out2, 0, sizeof(out2));
    ok = sat_py_call_method(str_obj, "nonexistent_method_abc", NULL, NULL, 0, &out2);
    ASSERT(ok == true, "sat_py_call_method should return true even on Python error");
    ASSERT(out2.ok == false, "result ok should be false for missing method");
    ASSERT(out2.error_class != NULL, "error_class should be set");
    ASSERT(out2.error_message != NULL, "error_message should be set");

    sat_py_release_handle(str_obj);
    sat_py_release_handle(builtins);
    sat_py_shutdown();
    return 0;
}

/* Test: sat_py_call_method with a NULL handle returns an error gracefully. */
static int test_call_method_null_handle(void) {
    ASSERT(sat_py_init("test_pyrt_handles") == true, "init should succeed");

    sat_py_result out;
    memset(&out, 0, sizeof(out));
    bool ok = sat_py_call_method(NULL, "upper", NULL, NULL, 0, &out);
    ASSERT(ok == true, "sat_py_call_method should return true (bridge handles NULL)");
    ASSERT(out.ok == false, "result ok should be false for NULL handle");
    ASSERT(out.error_class != NULL, "error_class should be set for NULL handle");

    sat_py_shutdown();
    return 0;
}

int main(void) {
    printf("=== Python runtime handle/method-call tests ===\n");
    TEST(test_init_shutdown);
    TEST(test_handle_new_null);
    TEST(test_import_and_get_attr);
    TEST(test_import_and_cache);
    TEST(test_get_attr_missing);
    TEST(test_call_method_on_string);
    TEST(test_call_method_upper);
    TEST(test_call_method_missing);
    TEST(test_call_method_null_handle);

    printf("\n=== Results: %d tests, %d failures ===\n", test_count, fail_count);
    return fail_count > 0 ? 1 : 0;
}
