//! Saturnite Rust runtime shim — implementation.
//!
//! A small, deterministic registration table for Rust wrapper artifacts.
//! The actual symbol resolution is performed by the linker; this module
//! only records which wrapper crates were linked and validates their ABI
//! version so a mismatch produces a clear diagnostic instead of an opaque
//! link failure.

#include "rsrt.h"

#include <stdio.h>
#include <string.h>

#define SAT_RUST_MAX_ARTIFACTS 64

static sat_rust_artifact SAT_RUST_ARTIFACTS[SAT_RUST_MAX_ARTIFACTS];
static size_t SAT_RUST_ARTIFACT_COUNT = 0;

bool sat_rust_register_artifact(const sat_rust_artifact *artifact) {
    if (artifact == NULL) {
        return false;
    }
    if (artifact->abi_version != SAT_RUST_ABI_VERSION) {
        fprintf(
            stderr,
            "Saturnite: Rust wrapper artifact '%s' built against ABI version %u, "
            "but the runtime expects version %u. Rebuild the wrapper.\n",
            artifact->crate_name ? artifact->crate_name : "(unnamed)",
            artifact->abi_version,
            SAT_RUST_ABI_VERSION);
        return false;
    }
    if (SAT_RUST_ARTIFACT_COUNT >= SAT_RUST_MAX_ARTIFACTS) {
        fprintf(
            stderr,
            "Saturnite: too many Rust wrapper artifacts registered (max %d).\n",
            SAT_RUST_MAX_ARTIFACTS);
        return false;
    }
    SAT_RUST_ARTIFACTS[SAT_RUST_ARTIFACT_COUNT] = *artifact;
    SAT_RUST_ARTIFACT_COUNT += 1;
    return true;
}

const sat_rust_artifact *sat_rust_find_artifact(const char *crate_name) {
    if (crate_name == NULL) {
        return NULL;
    }
    for (size_t i = 0; i < SAT_RUST_ARTIFACT_COUNT; i++) {
        const sat_rust_artifact *a = &SAT_RUST_ARTIFACTS[i];
        if (a->crate_name != NULL && strcmp(a->crate_name, crate_name) == 0) {
            return a;
        }
    }
    return NULL;
}

bool sat_rust_has_symbol(const char *crate_name, const char *symbol) {
    if (crate_name == NULL || symbol == NULL) {
        return false;
    }
    const sat_rust_artifact *a = sat_rust_find_artifact(crate_name);
    if (a == NULL) {
        return false;
    }
    // The symbol is resolved at link time against the static library.
    // This check verifies the runtime registry only; the linker provides
    // the real binding. A missing symbol surfaces as a link failure with
    // the symbol name preserved.
    (void)a;
    (void)symbol;
    return true;
}