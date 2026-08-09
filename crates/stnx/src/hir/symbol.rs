//! Symbol interning for HIR.
//!
//! Saturnite 0.3 replaces string-based name resolution (used throughout 0.2
//! in `semantic.rs` and `codegen/context.rs`) with stable numeric identifiers.
//! Every identifier, function name, and string literal is interned once and
//! referred to by a [`SymbolId`]. Top-level definitions (functions, later
//! structs) get a [`DefId`].

/// A stable identifier for an interned string (variable name, function name,
/// string literal, etc.).
///
/// `SymbolId`s are cheap to copy, hash, and compare — codegen uses them as
/// `HashMap` keys instead of `String` lookups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SymbolId(pub u32);

/// A stable identifier for a top-level definition (function, later struct).
///
/// `DefId`s are assigned during HIR lowering: each function in
/// [`HirProgram::functions`] corresponds to one `DefId` (its array index).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DefId(pub u32);

/// A simple, allocation-efficient string interner.
///
/// Strings are stored once; `intern()` returns a `SymbolId` that can be
/// used as a `HashMap` key or array index. This replaces the `HashMap<String, V>`
/// pattern used in 0.2's `Scope` and `FunctionScope`.
#[derive(Debug, Default)]
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
}
