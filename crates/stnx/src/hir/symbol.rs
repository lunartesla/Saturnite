//! Symbol interning for HIR.
//!
//! Saturnite 0.3 replaces string-based name resolution (used throughout 0.2
//! in `semantic.rs` and `codegen/context.rs`) with stable numeric identifiers.
//! Every identifier, function name, and string literal is interned once and
//! referred to by a [`SymbolId`]. Top-level definitions (functions, later
//! structs) get a [`DefId`].
//!
//! ## Module integration (Phase 5B)
//!
//! [`DefId`] stays flat (a `u32` array index) and is globally unique across
//! all modules. Module identity lives in [`ModuleId`] (defined in
//! `crate::module`), which is a separate `u32` space. The [`DefTable`] bridges
//! the two: it maps each `DefId` back to the `ModuleId` that owns it, the
//! local index within that module, and the [`DefKind`].
//!
//! The `SymbolInterner` — the global string table — is unchanged. Module
//! path segments are interned `SymbolId`s, so namespace lookups are numeric
//! equality checks, not string comparisons. We deliberately do NOT collapse
//! module scopes into a global string `HashMap`.

use serde::{Deserialize, Serialize};

/// A stable identifier for an interned string (variable name, function name,
/// string literal, etc.).
///
/// `SymbolId`s are cheap to copy, hash, and compare — codegen uses them as
/// `HashMap` keys instead of `String` lookups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SymbolId(pub u32);

/// A stable identifier for a top-level definition (function, later struct).
///
/// `DefId`s are assigned during HIR lowering: each definition gets a
/// globally-unique `DefId` that is an index into the [`DefTable`].
/// The `DefTable` maps each `DefId` back to its owning [`crate::module::ModuleId`],
/// its local index, and its [`DefKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DefId(pub u32);

/// A simple, allocation-efficient string interner.
///
/// Strings are stored once; `intern()` returns a `SymbolId` that can be
/// used as a `HashMap` key or array index. This replaces the `HashMap<String, V>`
/// pattern used in 0.2's `Scope` and `FunctionScope`.
#[derive(Debug, Default, Clone)]
pub struct SymbolInterner {
    strings: Vec<String>,
    indices: std::collections::HashMap<String, SymbolId>,
}

impl SymbolInterner {
    /// Intern a string, returning its [`SymbolId`].
    ///
    /// If the string was already interned, the existing `SymbolId` is returned.
    pub fn intern(&mut self, s: &str) -> SymbolId {
        if let Some(&id) = self.indices.get(s) {
            return id;
        }
        let id = SymbolId(self.strings.len() as u32);
        self.indices.insert(s.to_string(), id);
        self.strings.push(s.to_string());
        id
    }

    /// Look up the string for a [`SymbolId`].
    ///
    /// Returns `None` if the id was not produced by this interner.
    pub fn lookup(&self, id: SymbolId) -> Option<&str> {
        self.strings.get(id.0 as usize).map(|s| s.as_str())
    }

    /// Returns the next available `SymbolId` that would be assigned by
    /// [`intern`](Self::intern). Useful for generating fresh identifiers
    /// for synthetic symbols (e.g. use/mod declaration DefIds).
    pub fn next_id(&self) -> SymbolId {
        SymbolId(self.strings.len() as u32)
    }
}

// ---------------------------------------------------------------------------
// DefTable — maps DefId → (ModuleId, local_index, DefKind)
// ---------------------------------------------------------------------------

use crate::module::ModuleId;

/// The kind of definition a [`DefId`] refers to.
///
/// Used by [`DefTable`] to disambiguate what a `DefId` indexes without
/// requiring a runtime type check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DefKind {
    Function,
    Struct,
    Enum,
    /// A module declaration (`mod foo;`).
    Module,
    /// An import declaration (`use foo::bar`).
    Use,
    /// An `external` declaration — a foreign function call across an
    /// interoperability boundary (Rust crate, Python module, or native
    /// C-ABI library).
    External,
}

/// A single entry in the [`DefTable`].
///
/// Each entry records which module owns the definition, its local index
/// within that module, and what kind of definition it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DefEntry {
    /// The module that contains this definition.
    pub module: ModuleId,
    /// The definition's index within `module`'s namespace (0-based).
    pub local_index: u32,
    /// What kind of definition this is.
    pub kind: DefKind,
}

/// Maps every globally-unique [`DefId`] to its owning
/// [`ModuleId`], local index, and [`DefKind`].
///
/// This is the bridge between the flat `DefId` space (which MIR and codegen
/// treat as an opaque array index) and the module system. `DefId`s are
/// assigned sequentially during lowering, so `DefEntry` at index `i`
/// corresponds to `DefId(i)`.
#[derive(Debug, Default)]
pub struct DefTable {
    entries: Vec<DefEntry>,
}

impl DefTable {
    /// Create an empty `DefTable`.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Register a definition and return its assigned [`DefId`].
    ///
    /// The `DefId` is the index at which the entry is stored, guaranteeing
    /// `lookup(def_id)` will return `Some(&entry)` for the returned id.
    pub fn register(&mut self, entry: DefEntry) -> DefId {
        let id = DefId(self.entries.len() as u32);
        self.entries.push(entry);
        id
    }

    /// Look up the [`DefEntry`] for a [`DefId`], if it exists.
    pub fn lookup(&self, id: DefId) -> Option<&DefEntry> {
        self.entries.get(id.0 as usize)
    }

    /// Returns the total number of registered definitions.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if no definitions are registered.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns an iterator over all entries.
    pub fn iter(&self) -> impl Iterator<Item = (DefId, &DefEntry)> {
        self.entries
            .iter()
            .enumerate()
            .map(|(i, e)| (DefId(i as u32), e))
    }
}

// ---------------------------------------------------------------------------
// Visibility
// ---------------------------------------------------------------------------

/// Visibility of a top-level definition, determined during lowering.
///
/// This mirrors the AST `Visibility` enum but lives in HIR so that later
/// stages (MIR, codegen) can make access decisions without re-parsing.
///
/// Resolution of visibility (e.g. cross-module access checks) is deferred
/// to Phase 6; for now we only track and propagate the label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Visibility {
    #[default]
    Private,
    Public,
}
