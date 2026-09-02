#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

// ---------------------------------------------------------------------------
// Interpolated-string arena (0.5.1).
//
// Literal strings in Saturnite are NUL-terminated byte globals owned by the
// executable for the whole process. Interpolated strings built at runtime
// (from `{expr}` segments) and numeric-to-string conversions are heap
// allocated here. To avoid per-interpolation leaks the runtime keeps every
// heap string it produces in a process-wide arena and frees the whole arena
// once at process exit (via atexit). This mirrors the existing "strings live
// for the process" model and removes the need to tag heap- vs static-owned
// `char*` at every call site.
//
// Note: a program that interpolates inside a long-running loop accumulates
// bytes until exit (no per-iteration reclaim). Saturnite 0.5.1 has no string
// mutation/destruction semantics; a real free/ownership model for strings is
// deferred to a later phase.
// ---------------------------------------------------------------------------

static char **sat_arena_bufs = NULL;
static size_t sat_arena_count = 0;
static size_t sat_arena_cap = 0;

static void sat_arena_free_all(void) {
    for (size_t i = 0; i < sat_arena_count; i++) free(sat_arena_bufs[i]);
    free(sat_arena_bufs);
    sat_arena_bufs = NULL;
    sat_arena_count = 0;
    sat_arena_cap = 0;
}

static void sat_oom(void) {
    fputs("Saturnite runtime: out of memory\n", stderr);
    exit(1);
}

static char *sat_arena_own(char *p) {
    if (p == NULL) sat_oom();
    if (sat_arena_bufs == NULL) {
        if (atexit(sat_arena_free_all) != 0) sat_oom();
    }
    if (sat_arena_count == sat_arena_cap) {
        size_t ncap = sat_arena_cap ? sat_arena_cap * 2 : 32;
        char **nbufs = realloc(sat_arena_bufs, ncap * sizeof(*nbufs));
        if (nbufs == NULL) sat_oom();
        sat_arena_bufs = nbufs;
        sat_arena_cap = ncap;
    }
    sat_arena_bufs[sat_arena_count++] = p;
    return p;
}

static char *sat_strdup_len(const char *s, size_t len) {
    char *out = malloc(len + 1);
    if (out == NULL) sat_oom();
    memcpy(out, s, len);
    out[len] = '\0';
    return out;
}

long long println_i64(long long value) {
    printf("%lld\n", (long long)value);
    return 0;
}

long long println_str(const char *value) {
    printf("%s\n", value);
    return 0;
}

// Concatenate two NUL-terminated strings into a new, arena-owned string.
// `a` and `b` may be static literals or previous arena strings; neither is
// freed here (the arena owns all heap strings and releases them at exit).
char *concat_str(const char *a, const char *b) {
    size_t la = a ? strlen(a) : 0;
    size_t lb = b ? strlen(b) : 0;
    char *out = malloc(la + lb + 1);
    if (out == NULL) sat_oom();
    if (la) memcpy(out, a, la);
    if (lb) memcpy(out + la, b, lb);
    out[la + lb] = '\0';
    return sat_arena_own(out);
}

// Decimal rendering of an i64 as an arena-owned NUL-terminated string.
char *str_i64(long long value) {
    char tmp[64];
    int len = snprintf(tmp, sizeof(tmp), "%lld", (long long)value);
    if (len < 0) sat_oom();
    return sat_arena_own(sat_strdup_len(tmp, (size_t)len));
}
