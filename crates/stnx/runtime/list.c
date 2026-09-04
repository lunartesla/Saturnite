// ---------------------------------------------------------------------------
// Saturnite 0.5.3 runtime: List<i64> construction.
//
// Representation (ABI locked for 0.5.3):
//
//     typedef struct {
//         int64_t *data;   // heap-allocated element storage
//         size_t   len;    // logical element count
//         size_t   cap;    // allocated capacity (0 <= len <= cap)
//     } sat_list;
//
// Saturnite-visible values (indexes, lengths, elements) use `long long`
// (i64), matching the compiler's i64 representation. The struct uses
// `size_t` internally for allocation arithmetic only.
//
// Memory model: lists are allocated with malloc and live for the whole
// process (no free), mirroring the string arena model. Lists do NOT use the
// string arena — they are structured collections with separate allocation.
// Ownership/free semantics are deferred.
//
// This phase (0.5.3 construction) provides only list_new_from; indexing,
// length, mutation, and iteration are later phases.
// ---------------------------------------------------------------------------

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

typedef struct {
    int64_t *data;
    size_t len;
    size_t cap;
} sat_list;

static void sat_list_oom(void) {
    fputs("Saturnite runtime: out of memory (list allocation failed)\n", stderr);
    exit(1);
}

// Construct a list of `count` elements from `elems` (an array of `count`
// int64 values, evaluated left-to-right by the generated caller before this
// call). Returns a process-lifetime sat_list*.
//
// Allocation-failure behavior is deterministic: print a diagnostic to
// stderr and exit(1).
sat_list *list_new_from(long long *elems, long long count) {
    if (count < 0) {
        fputs("Saturnite runtime: negative list length\n", stderr);
        exit(1);
    }
    sat_list *list = malloc(sizeof(sat_list));
    if (list == NULL) sat_list_oom();
    list->len = (size_t)count;
    list->cap = (size_t)count;
    if (count == 0) {
        // Zero-length list: valid, data stays NULL.
        list->data = NULL;
        return list;
    }
    // Overflow guard: count elements * 8 bytes. count came from a
    // non-negative i64, so >SIZE_MAX/8 can only trigger on 32-bit size_t.
    if ((size_t)count > (size_t)-1 / sizeof(int64_t)) sat_list_oom();
    list->data = malloc((size_t)count * sizeof(int64_t));
    if (list->data == NULL) sat_list_oom();
    for (long long i = 0; i < count; i++) {
        list->data[i] = elems[i];
    }
    return list;
}

// Return the current logical length of `list` as an i64 (Saturnite-visible).
// 0 <= len <= cap invariant is maintained by list_new_from.
long long list_len(sat_list *list) {
    if (list == NULL) {
        fputs("Saturnite runtime: list_len on null list\n", stderr);
        exit(1);
    }
    return (long long)list->len;
}

// Bounds-checked element read. Returns the element at `index` (i64, matching
// the Saturnite i64 loop/index representation). Out-of-bounds prints a
// diagnostic to stderr and aborts — no exception propagation in 0.5.3.
long long list_get(sat_list *list, long long index) {
    if (list == NULL) {
        fputs("Saturnite runtime: list_get on null list\n", stderr);
        exit(1);
    }
    if (index < 0 || (size_t)index >= list->len) {
        fprintf(stderr,
                "Saturnite runtime: list index out of bounds: index=%lld, len=%zu\n",
                (long long)index, list->len);
        exit(1);
    }
    return list->data[(size_t)index];
}

// Bounds-checked element write. Out-of-bounds prints a diagnostic and aborts.
void list_set(sat_list *list, long long index, long long value) {
    if (list == NULL) {
        fputs("Saturnite runtime: list_set on null list\n", stderr);
        exit(1);
    }
    if (index < 0 || (size_t)index >= list->len) {
        fprintf(stderr,
                "Saturnite runtime: list index out of bounds: index=%lld, len=%zu\n",
                (long long)index, list->len);
        exit(1);
    }
    list->data[(size_t)index] = value;
}

// Return the current logical length of `list` as an i64 (Saturnite-visible).
// 0 <= len <= cap invariant is maintained by list_new_from.
