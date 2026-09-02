//! Module system for Saturnite -- module graph and project loading infrastructure.
//!
//! This module provides the data structures and discovery logic for Saturnite's
//! multi-file module system. It implements the file-mapping rules described in
//! the Phase 3 design documents:
//!
//! - `mod foo;` -> `<dir>/foo.stnx` or `<dir>/foo/mod.stnx`
//! - `mod foo::bar;` -> `<dir>/foo/bar.stnx` or `<dir>/foo/bar/mod.stnx`
//!
//! ## Design principles
//!
//! * [`ModuleId`] is a stable `u32` identity space, **separate** from [`DefId`].
//!   Modules get their own identity space; definitions keep their flat `DefId` space.
//! * [`ModulePath`] is a `Vec<SymbolId>` -- each segment is an interned string from
//!   the shared [`SymbolInterner`], so path comparisons are cheap numeric equality
//!   checks. This reuses the existing interning architecture instead of introducing
//!   a separate string `HashMap`.
//! * [`ModuleGraph`] collects all discovered modules and tracks import/edge
//!   relationships for future name resolution and incremental compilation.
//! * [`Project`] locates the project root by walking upward for `saturn.toml`,
//!   loads the config, and discovers the module graph from a root source file.

use crate::ast::{ItemKind, Program};
use crate::config::SaturnConfig;
use crate::error::{CompilerError, CompilerResult};
use crate::hir::symbol::{DefId, SymbolId, SymbolInterner};
use crate::parser;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// ModuleId
// ---------------------------------------------------------------------------

/// Stable module identifier -- separate from [`DefId`].
///
/// Module IDs are assigned sequentially as modules are discovered, starting at 0
/// for the root module. They serve as array indices into [`ModuleGraph::modules`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ModuleId(pub u32);

impl ModuleId {
    /// The root module identifier (the crate root).
    pub const ROOT: ModuleId = ModuleId(0);

    /// Create a new `ModuleId` from a raw `u32`.
    pub const fn new(id: u32) -> Self {
        ModuleId(id)
    }
}

impl From<u32> for ModuleId {
    fn from(id: u32) -> Self {
        ModuleId(id)
    }
}

impl From<ModuleId> for u32 {
    fn from(id: ModuleId) -> u32 {
        id.0
    }
}

// ---------------------------------------------------------------------------
// ModulePath
// ---------------------------------------------------------------------------

/// A module path -- a sequence of interned [`SymbolId`] segments.
///
/// Examples:
/// - Root module: `[]` (empty path)
/// - `crate::math`: `[SymbolId("math")]`
/// - `crate::graphics::math`: `[SymbolId("graphics"), SymbolId("math")]`
///
/// The segments are interned strings, so equality and hashing are cheap.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModulePath {
    segments: Vec<SymbolId>,
}

impl PartialOrd for ModulePath {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ModulePath {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Compare by length first, then by segments (using SymbolId's u32 value).
        self.segments
            .len()
            .cmp(&other.segments.len())
            .then_with(|| {
                self.segments
                    .iter()
                    .map(|s| s.0)
                    .cmp(other.segments.iter().map(|s| s.0))
            })
    }
}

impl ModulePath {
    /// Create an empty (root) module path.
    pub fn new() -> Self {
        ModulePath {
            segments: Vec::new(),
        }
    }

    /// Create a module path from a vector of `SymbolId` segments.
    pub fn from_segments(segments: Vec<SymbolId>) -> Self {
        ModulePath { segments }
    }

    /// Create a module path from string segments, interning each one in the
    /// provided [`SymbolInterner`].
    pub fn from_strings(interner: &mut SymbolInterner, segments: &[&str]) -> Self {
        let segs: Vec<SymbolId> = segments.iter().map(|s| interner.intern(s)).collect();
        ModulePath { segments: segs }
    }

    /// Returns the interned name of the last path segment (the module's own name).
    ///
    /// Returns `None` for the root module (empty path).
    pub fn name<'a>(&self, interner: &'a SymbolInterner) -> Option<&'a str> {
        self.segments.last().and_then(|sid| interner.lookup(*sid))
    }

    /// Returns the path segments as a slice of `SymbolId`.
    pub fn segments(&self) -> &[SymbolId] {
        &self.segments
    }

    /// Returns an iterator over the path segments.
    pub fn iter(&self) -> impl Iterator<Item = &SymbolId> {
        self.segments.iter()
    }

    /// Returns the number of segments in the path.
    pub fn len(&self) -> usize {
        self.segments.len()
    }

    /// Returns `true` if this is the root module path (no segments).
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// Returns the parent path (all segments except the last).
    /// For the root module, returns `None`.
    pub fn parent(&self) -> Option<ModulePath> {
        if self.segments.is_empty() {
            None
        } else {
            Some(ModulePath {
                segments: self.segments[..self.segments.len() - 1].to_vec(),
            })
        }
    }

    /// Returns `true` if `self` is a descendant of (or equal to) `ancestor`.
    ///
    /// For example, `[foo, bar]` is a descendant of `[]` and `[foo]`, but
    /// `[foo, bar]` is not a descendant of `[baz]`.
    pub fn is_descendant_of(&self, ancestor: &ModulePath) -> bool {
        if ancestor.segments.len() > self.segments.len() {
            return false;
        }
        ancestor.segments == self.segments[..ancestor.segments.len()]
    }

    /// Build a child path by appending a new segment.
    pub fn child(&self, segment: SymbolId) -> ModulePath {
        let mut segs = self.segments.clone();
        segs.push(segment);
        ModulePath { segments: segs }
    }
}

impl Default for ModulePath {
    fn default() -> Self {
        ModulePath::new()
    }
}

impl std::fmt::Display for ModulePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Note: this Display does not have access to the SymbolInterner,
        // so it falls back to showing the raw SymbolId values.
        // Use ModuleGraph::format_path for human-readable names.
        if self.segments.is_empty() {
            write!(f, "crate")
        } else {
            write!(f, "crate::")?;
            for (i, seg) in self.segments.iter().enumerate() {
                if i > 0 {
                    write!(f, "::")?;
                }
                write!(f, "{}", seg.0)?;
            }
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Module
// ---------------------------------------------------------------------------

/// A single module in the module graph.
///
/// Each module has a stable [`ModuleId`], a [`ModulePath`] (the qualified path
/// within the crate), a `file_path` pointing to its source file, and optionally
/// a parsed [`Program`] AST (loaded lazily).
#[derive(Debug, Clone)]
pub struct Module {
    /// Unique module identifier.
    pub id: ModuleId,
    /// Qualified path (e.g., `crate::math`, `crate::graphics::math`).
    pub path: ModulePath,
    /// Path to the source file on disk.
    pub file_path: PathBuf,
    /// Parsed AST -- loaded lazily on demand.
    pub ast: Option<Program>,
    /// Parent module ID, if any (None for the root module).
    pub parent: Option<ModuleId>,
    /// Names of modules declared with `mod` in this module's source.
    /// Used to discover child modules during recursive loading.
    pub mod_declarations: Vec<String>,
}

impl Module {
    /// Create a new module with the given id, path, and file path.
    pub fn new(id: ModuleId, path: ModulePath, file_path: PathBuf) -> Self {
        Module {
            id,
            path,
            file_path,
            ast: None,
            parent: None,
            mod_declarations: Vec::new(),
        }
    }

    /// The name of this module (the last path segment) as a string.
    ///
    /// Returns `"crate"` for the root module.
    pub fn name(&self, interner: &SymbolInterner) -> String {
        self.path.name(interner).unwrap_or("crate").to_string()
    }

    /// The directory containing this module's source file.
    ///
    /// Module file resolution for child `mod` declarations is relative to
    /// this directory.
    pub fn dir(&self) -> &Path {
        match self.file_path.parent() {
            Some(dir) => dir,
            None => Path::new("."),
        }
    }

    /// Returns `true` if this is the root (crate) module.
    pub fn is_root(&self) -> bool {
        self.id == ModuleId::ROOT
    }
}

// ---------------------------------------------------------------------------
// ModuleScope — per-module name → DefId resolution table
// ---------------------------------------------------------------------------

/// Per-module namespace that maps interned names to `DefId`s.
///
/// A `ModuleScope` is the module-level counterpart of HIR's `LowerScope`
/// (which handles lexical/variable scoping). Each scope holds:
///
/// - **items**: top-level definitions declared directly in this module
///   (`fn`, `struct`, `enum`, `mod`), keyed by their interned `SymbolId`.
/// - **imports**: names brought into scope via `use` declarations, keyed by
///   their alias `SymbolId` and valued with the target `DefId`.
/// - **parent**: the parent module's `ModuleId`, enabling Rust-style
///   parent-chain visibility and path resolution ("Rust 2018 model").
///
/// Module scopes use `SymbolId` keys (via the shared `SymbolInterner`)
/// rather than `String` keys, so lookups are numeric equality checks.
/// We deliberately do NOT collapse namespaces into a global string HashMap.
#[derive(Debug, Clone, Default)]
pub struct ModuleScope {
    /// Items declared in this module (interned name → DefId).
    pub items: HashMap<SymbolId, DefId>,
    /// Items imported via `use` (alias → DefId).
    pub imports: HashMap<SymbolId, DefId>,
    /// Parent module (for visibility/path resolution).
    /// `None` for the root module.
    pub parent: Option<ModuleId>,
}

impl ModuleScope {
    /// Create a new empty module scope with no parent.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new module scope with the given parent.
    pub fn with_parent(parent: ModuleId) -> Self {
        Self {
            items: HashMap::new(),
            imports: HashMap::new(),
            parent: Some(parent),
        }
    }

    /// Register a top-level item in this module's namespace.
    pub fn define_item(&mut self, name: SymbolId, def_id: DefId) {
        self.items.insert(name, def_id);
    }

    /// Register an import (alias → target DefId) in this module's namespace.
    pub fn define_import(&mut self, alias: SymbolId, target: DefId) {
        self.imports.insert(alias, target);
    }

    /// Look up a name in this scope: first checks items, then imports.
    ///
    /// This does NOT walk the parent chain — callers that need parent-chain
    /// resolution should use [`lookup_with_parent`].
    pub fn lookup(&self, name: &SymbolId) -> Option<DefId> {
        self.items
            .get(name)
            .copied()
            .or_else(|| self.imports.get(name).copied())
    }

    /// Look up a name in this scope, walking the parent chain recursively.
    ///
    /// This mirrors the Rust 2018 name-resolution model: a bare name reference
    /// searches the current module's scope first, then the parent module's
    /// scope, and so on up to the root.
    pub fn lookup_with_parent(&self, name: &SymbolId, scopes: &[ModuleScope]) -> Option<DefId> {
        if let Some(def_id) = self.lookup(name) {
            return Some(def_id);
        }
        if let Some(parent) = self.parent {
            if let Some(parent_scope) = scopes.get(parent.0 as usize) {
                return parent_scope.lookup_with_parent(name, scopes);
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// ModuleGraph
// ---------------------------------------------------------------------------

/// Module discovery and graph management.
///
/// The `ModuleGraph` holds all discovered modules, a shared [`SymbolInterner`],
/// a path-to-module index for fast lookups, and import edges for future
/// dependency tracking.
#[derive(Debug)]
pub struct ModuleGraph {
    /// All discovered modules, indexed by `ModuleId.0`.
    pub modules: Vec<Module>,
    /// The root (crate) module.
    pub root: ModuleId,
    /// Shared symbol table for interning module path segments.
    pub symbol_interner: SymbolInterner,
    /// Map from `ModulePath` to `ModuleId` for fast lookups.
    module_index: HashMap<ModulePath, ModuleId>,
    /// Import edges: `ModuleId` -> list of imported `ModuleId`s.
    /// Populated as `use` declarations are parsed (Phase 5+).
    pub imports: HashMap<ModuleId, Vec<ModuleId>>,
}

impl ModuleGraph {
    /// Create an empty module graph.
    pub fn new() -> Self {
        ModuleGraph {
            modules: Vec::new(),
            root: ModuleId::ROOT,
            symbol_interner: SymbolInterner::default(),
            module_index: HashMap::new(),
            imports: HashMap::new(),
        }
    }

    /// Create a module graph with a pre-populated symbol interner.
    pub fn with_interner(interner: SymbolInterner) -> Self {
        ModuleGraph {
            modules: Vec::new(),
            root: ModuleId::ROOT,
            symbol_interner: interner,
            module_index: HashMap::new(),
            imports: HashMap::new(),
        }
    }

    /// Add a module to the graph and return its assigned `ModuleId`.
    ///
    /// The `ModuleId` is assigned sequentially based on the current length
    /// of the `modules` vector. The module is also indexed by its path in
    /// `module_index` for fast lookups via [`find_module`].
    pub fn add_module(&mut self, module: Module) -> ModuleId {
        let id = ModuleId(self.modules.len() as u32);
        let path = module.path.clone();
        self.module_index.insert(path, id);
        self.modules.push(module);
        id
    }

    /// Find a module by its [`ModulePath`].
    ///
    /// Returns the `ModuleId` if the module exists in the graph, `None` otherwise.
    pub fn find_module(&self, path: &ModulePath) -> Option<ModuleId> {
        self.module_index.get(path).copied()
    }

    /// Look up a module by its `ModuleId`.
    pub fn get_module(&self, id: ModuleId) -> Option<&Module> {
        self.modules.get(id.0 as usize)
    }

    /// Look up a module by its `ModuleId` (mutable reference).
    pub fn get_module_mut(&mut self, id: ModuleId) -> Option<&mut Module> {
        self.modules.get_mut(id.0 as usize)
    }

    /// Returns the root (crate) module.
    pub fn root_module(&self) -> &Module {
        &self.modules[0]
    }

    /// Returns the root (crate) module ID.
    pub fn root_id(&self) -> ModuleId {
        self.root
    }

    /// Returns the total number of modules in the graph.
    pub fn len(&self) -> usize {
        self.modules.len()
    }

    /// Returns `true` if the graph contains no modules.
    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }

    /// Returns an iterator over all modules.
    pub fn modules(&self) -> impl Iterator<Item = &Module> {
        self.modules.iter()
    }

    /// Returns an iterator over all import edges.
    pub fn imports(&self) -> &HashMap<ModuleId, Vec<ModuleId>> {
        &self.imports
    }

    /// Format a module path as a human-readable string (e.g., `crate::math::utils`).
    pub fn format_path(&self, path: &ModulePath) -> String {
        if path.is_empty() {
            return "crate".to_string();
        }
        let segments: Vec<&str> = path
            .segments()
            .iter()
            .map(|sid| self.symbol_interner.lookup(*sid).unwrap_or("<unknown>"))
            .collect();
        format!("crate::{}", segments.join("::"))
    }

    /// Detect a simple cycle in the module import/dependency graph.
    ///
    /// Defensive guard only; visibility enforcement remains deferred.
    pub fn detect_cycle(&self) -> Option<Vec<ModuleId>> {
        let mut visited = std::collections::HashSet::new();
        let mut stack = Vec::new();
        fn dfs(
            graph: &ModuleGraph,
            node: ModuleId,
            visited: &mut std::collections::HashSet<ModuleId>,
            stack: &mut Vec<ModuleId>,
            cycle: &mut Option<Vec<ModuleId>>,
        ) {
            if cycle.is_some() {
                return;
            }
            if visited.contains(&node) {
                if let Some(pos) = stack.iter().position(|&n| n == node) {
                    *cycle = Some(stack[pos..].to_vec());
                }
                return;
            }
            visited.insert(node);
            stack.push(node);
            if let Some(deps) = graph.imports.get(&node) {
                for &child in deps {
                    dfs(graph, child, visited, stack, cycle);
                }
            }
            stack.pop();
        }
        for id in 0..self.modules.len() as u32 {
            let mid = ModuleId(id);
            if !visited.contains(&mid) {
                let mut cycle_opt: Option<Vec<ModuleId>> = None;
                dfs(self, mid, &mut visited, &mut stack, &mut cycle_opt);
                if cycle_opt.is_some() {
                    return cycle_opt;
                }
            }
        }
        None
    }

    /// Resolve a module path relative to a current module's path.
    pub fn resolve_path(&self, from: ModuleId, segments: &[SymbolId]) -> Option<ModuleId> {
        let base_module = self.get_module(from)?;
        let mut path = base_module.path.clone();
        for seg in segments {
            path = path.child(*seg);
        }
        self.find_module(&path)
    }

    /// Discover all modules starting from a root source file.
    ///
    /// This is the entry point for module discovery: it lexes and parses the
    /// root file into an AST, walks the AST for `mod` declarations, and
    /// recursively discovers child module files using the file-mapping rules:
    ///
    /// 1. `<dir>/<name>.stnx` (single file)
    /// 2. `<dir>/<name>/mod.stnx` (directory module)
    ///
    /// The AST (`ast::ItemKind::ModDecl`) is the authoritative source of
    /// module names. If AST parsing fails, the text-based fallback
    /// [`extract_mod_declarations`] is used so that discovery remains robust
    /// even for partially-valid or malformed source.
    pub fn discover_modules(root_file: PathBuf) -> CompilerResult<ModuleGraph> {
        let mut graph = ModuleGraph::new();

        // Create the root module.
        let root_path = ModulePath::new();
        let mut root_module = Module::new(ModuleId::ROOT, root_path, root_file.clone());

        // Read the root file and parse it.
        let source = std::fs::read_to_string(&root_file).map_err(|e| {
            CompilerError::config(format!(
                "failed to read root module {}: {}",
                root_file.display(),
                e
            ))
        })?;
        let root_ast = parse_source(&source).ok();
        root_module.mod_declarations =
            extract_mod_declarations_from_ast(root_ast.as_ref(), &source);
        root_module.ast = root_ast;

        let root_id = graph.add_module(root_module);

        // Recursively discover child modules.
        let mut to_visit: Vec<(ModuleId, PathBuf)> = vec![(root_id, root_file)];
        while let Some((module_id, _file_path)) = to_visit.pop() {
            let module = graph
                .get_module(module_id)
                .ok_or_else(|| {
                    CompilerError::config("internal error: module disappeared during discovery")
                })?
                .clone();

            let module_dir = module.dir().to_path_buf();
            let current_path = module.path.clone();

            for mod_name in &module.mod_declarations {
                // Build the child module path by appending the name segment.
                let segment = graph.symbol_interner.intern(mod_name);
                let child_path = current_path.child(segment);

                // Try to find the module file.
                if let Some(child_file) = resolve_module_file(&module_dir, mod_name) {
                    let child_source = std::fs::read_to_string(&child_file).map_err(|e| {
                        CompilerError::config(format!(
                            "failed to read module {}: {}",
                            child_file.display(),
                            e
                        ))
                    })?;

                    let child_ast = parse_source(&child_source).ok();
                    let child_mods =
                        extract_mod_declarations_from_ast(child_ast.as_ref(), &child_source);

                    let mut child_module = Module::new(
                        ModuleId(graph.modules.len() as u32),
                        child_path.clone(),
                        child_file,
                    );
                    child_module.parent = Some(module_id);
                    child_module.mod_declarations = child_mods;
                    child_module.ast = child_ast;

                    let child_id = graph.add_module(child_module);
                    let child_file_path = graph.get_module(child_id).unwrap().file_path.clone();
                    to_visit.push((child_id, child_file_path));
                } else {
                    // Could not find the module file -- return an error.
                    return Err(CompilerError::config(format!(
                        "unresolved module '{}': file not found in {}",
                        mod_name,
                        module_dir.display()
                    )));
                }
            }
        }

        Ok(graph)
    }
}

impl Default for ModuleGraph {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// File resolution helpers
// ---------------------------------------------------------------------------

/// Resolve a `mod foo;` declaration to a file on disk.
///
/// Given the directory of the current module's source file and a module name,
/// try the following files in order:
///
/// 1. `<dir>/<name>.stnx` (single file)
/// 2. `<dir>/<name>/mod.stnx` (directory module)
///
/// Returns the first existing file, or `None` if neither exists.
fn resolve_module_file(dir: &Path, name: &str) -> Option<PathBuf> {
    // Rule 1: `<dir>/<name>.stnx`
    let single_file = dir.join(format!("{}.stnx", name));
    if single_file.is_file() {
        return Some(single_file);
    }

    // Rule 2: `<dir>/<name>/mod.stnx`
    let mod_file = dir.join(name).join("mod.stnx");
    if mod_file.is_file() {
        return Some(mod_file);
    }

    None
}

/// Extract `mod` declarations from a parsed AST.
///
/// Walks `Program::items`, filtering for `ItemKind::ModDecl` items, and
/// collects each item's `name` (the module name). This is the AST-based
/// primary path used by [`ModuleGraph::discover_modules`].
///
/// If the AST is `None` (parse failure), falls back to the text-based
/// [`extract_mod_declarations`] scanner on the raw source so that discovery
/// remains robust for partially-valid or malformed source files.
fn extract_mod_declarations_from_ast(ast: Option<&Program>, source: &str) -> Vec<String> {
    match ast {
        Some(program) => program
            .items
            .iter()
            .filter(|item| matches!(item.kind, ItemKind::ModDecl))
            .map(|item| item.name.clone())
            .collect(),
        None => extract_mod_declarations(source),
    }
}

/// Extract `mod` declarations from source text.
///
/// This is a lightweight, text-based fallback scanner used when AST parsing
/// fails (e.g. for partially-valid or malformed source files). It looks for
/// lines matching `mod <ident>` or `pub mod <ident>` at the start of a line
/// (ignoring leading whitespace) and extracts the identifier.
fn extract_mod_declarations(source: &str) -> Vec<String> {
    let mut mods = Vec::new();

    for line in source.lines() {
        let trimmed = line.trim_start();

        // Skip comments.
        if trimmed.starts_with("//") {
            continue;
        }

        // Check for `mod <ident>` or `pub mod <ident>` pattern.
        if let Some(rest) = stripped_mod_prefix(trimmed) {
            if let Some(name) = extract_ident(rest) {
                if !name.is_empty() {
                    mods.push(name);
                }
            }
        }
    }

    mods
}

/// Strip a `mod` or `pub mod` prefix from a line, returning the rest.
fn stripped_mod_prefix(trimmed: &str) -> Option<&str> {
    if let Some(rest) = trimmed.strip_prefix("mod ") {
        Some(rest)
    } else if let Some(rest) = trimmed.strip_prefix("pub mod ") {
        Some(rest)
    } else {
        None
    }
}

/// Extract an identifier from the start of a string.
///
/// An identifier is `[a-zA-Z_][a-zA-Z0-9_]*`. Returns the longest matching
/// prefix that is a valid identifier.
fn extract_ident(s: &str) -> Option<String> {
    let mut chars = s.chars().peekable();
    let first = chars.peek()?;
    if !first.is_alphabetic() && *first != '_' {
        return None;
    }

    let mut result = String::new();
    result.push(*first);
    chars.next();

    while let Some(&c) = chars.peek() {
        if c.is_alphanumeric() || c == '_' {
            result.push(c);
            chars.next();
        } else {
            break;
        }
    }

    Some(result)
}

/// Lex and parse source text into an AST [`Program`].
///
/// Tokens go through the 0.5 preparation pass (`lexer::prepare`), which
/// runs the indent pre-pass and desugars native colon-blocks into brace
/// blocks before parsing.
fn parse_source(source: &str) -> CompilerResult<Program> {
    let tokens = crate::lexer::prepare(source)?;
    parser::parse(source, tokens)
}

// ---------------------------------------------------------------------------
// Project
// ---------------------------------------------------------------------------

/// Project: project root discovery, config loading, and module discovery.
///
/// A `Project` bundles together the parsed [`SaturnConfig`], the project root
/// directory, the source root directory, and the fully-discovered [`ModuleGraph`].
pub struct Project {
    /// The parsed `saturn.toml` configuration.
    pub config: SaturnConfig,
    /// The project root directory (contains `saturn.toml`).
    pub root: PathBuf,
    /// The source root directory (typically `<root>/src/`).
    pub source_root: PathBuf,
    /// The discovered module graph.
    pub graph: ModuleGraph,
}

impl Project {
    /// Discover a project starting from a given path.
    ///
    /// The discovery algorithm walks upward from `start` looking for a
    /// `saturn.toml` file:
    ///
    /// 1. If `start` is a file, begin from its parent directory.
    /// 2. Check each directory (and its ancestors) for `saturn.toml`.
    /// 3. The first directory containing `saturn.toml` is the **project root**.
    /// 4. The source root is `<root>/src/`.
    /// 5. If no `saturn.toml` is found, synthesize a config from the starting
    ///    directory name and use the starting directory as the project root.
    ///
    /// This mirrors Cargo's `Cargo.toml` root-finding behavior.
    pub fn discover(start: &Path) -> CompilerResult<Project> {
        // Determine the starting directory.
        let start_dir = if start.is_file() {
            start.parent().unwrap_or_else(|| Path::new("."))
        } else {
            start
        };

        // Walk upward to find saturn.toml.
        let mut current: &Path = start_dir;
        loop {
            let config_path = current.join("saturn.toml");
            if config_path.is_file() {
                // Found the project root.
                let root = current.to_path_buf();
                let config = SaturnConfig::from_dir(&root)?;
                let source_root = root.join("src");
                let graph = ModuleGraph::new();

                return Ok(Project {
                    config,
                    root,
                    source_root,
                    graph,
                });
            }

            // Move up one directory.
            match current.parent() {
                Some(parent) => current = parent,
                None => {
                    // Reached the filesystem root without finding saturn.toml.
                    // Synthesize a config from the starting directory name.
                    let name = start_dir
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("project")
                        .to_string();
                    let config = SaturnConfig::from_name(&name)?;
                    let root = start_dir.to_path_buf();
                    let source_root = root.join("src");
                    let graph = ModuleGraph::new();

                    return Ok(Project {
                        config,
                        root,
                        source_root,
                        graph,
                    });
                }
            }
        }
    }

    /// Load all modules for this project.
    ///
    /// Determines the entry point (defaults to `<source_root>/main.stnx` if it
    /// exists), then runs module discovery to build the complete [`ModuleGraph`].
    pub fn load(&mut self) -> CompilerResult<Program> {
        // Determine the entry point file.
        let entry = self.source_root.join("main.stnx");
        if !entry.is_file() {
            return Err(CompilerError::config(format!(
                "no entry point found: expected {} (create a saturn.toml project with src/main.stnx or pass a file explicitly)",
                entry.display()
            )));
        }

        // Discover all modules from the entry point.
        self.graph = ModuleGraph::discover_modules(entry)?;

        // Return the root module's AST as the combined program.
        // (Phase 4: return the root AST; cross-module merging happens in Phase 5.)
        let root = self.graph.root_module();
        Ok(root.ast.clone().unwrap_or_else(|| Program {
            functions: Vec::new(),
            items: Vec::new(),
        }))
    }

    /// Load the project from a specific file path (not necessarily the default entry).
    ///
    /// This is useful when the caller has an explicit file path (e.g., from the CLI).
    pub fn load_from(&mut self, file: &Path) -> CompilerResult<Program> {
        self.graph = ModuleGraph::discover_modules(file.to_path_buf())?;

        let root = self.graph.root_module();
        Ok(root.ast.clone().unwrap_or_else(|| Program {
            functions: Vec::new(),
            items: Vec::new(),
        }))
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::tempdir;

    // --- ModuleId tests ---

    #[test]
    fn test_module_id_creation() {
        let id = ModuleId::new(42);
        assert_eq!(id.0, 42);
    }

    #[test]
    fn test_module_id_from_u32() {
        let id = ModuleId::from(7u32);
        assert_eq!(id.0, 7);
    }

    #[test]
    fn test_module_id_to_u32() {
        let id = ModuleId(5);
        let val: u32 = id.into();
        assert_eq!(val, 5);
    }

    #[test]
    fn test_module_id_equality() {
        assert_eq!(ModuleId(3), ModuleId(3));
        assert_ne!(ModuleId(3), ModuleId(4));
    }

    #[test]
    fn test_module_id_root_constant() {
        assert_eq!(ModuleId::ROOT, ModuleId(0));
    }

    #[test]
    fn test_module_id_ordering() {
        assert!(ModuleId(1) < ModuleId(2));
        assert!(ModuleId(0) < ModuleId(1));
        assert!(ModuleId(2) > ModuleId::ROOT);
    }

    #[test]
    fn test_module_id_hash() {
        let mut map: HashMap<ModuleId, &str> = HashMap::new();
        let id1 = ModuleId(1);
        let id2 = ModuleId(2);
        map.insert(id1, "a");
        map.insert(id2, "b");
        assert_eq!(map.get(&id1), Some(&"a"));
        assert_eq!(map.get(&id2), Some(&"b"));
    }

    // --- ModulePath tests ---

    #[test]
    fn test_module_path_empty() {
        let path = ModulePath::new();
        assert!(path.is_empty());
        assert_eq!(path.len(), 0);
        assert_eq!(path.segments().len(), 0);
    }

    #[test]
    fn test_module_path_from_strings() {
        let mut interner = SymbolInterner::default();
        let path = ModulePath::from_strings(&mut interner, &["math", "utils"]);

        assert!(!path.is_empty());
        assert_eq!(path.len(), 2);
        assert_eq!(path.name(&interner), Some("utils"));
    }

    #[test]
    fn test_module_path_name_root() {
        let path = ModulePath::new();
        let interner = SymbolInterner::default();
        assert_eq!(path.name(&interner), None);
    }

    #[test]
    fn test_module_path_parent() {
        let mut interner = SymbolInterner::default();
        let path = ModulePath::from_strings(&mut interner, &["graphics", "math"]);
        let parent = path.parent().unwrap();

        assert_eq!(parent.len(), 1);
        assert_eq!(parent.name(&interner), Some("graphics"));

        // Root's parent is None.
        let root = ModulePath::new();
        assert!(root.parent().is_none());
    }

    #[test]
    fn test_module_path_child() {
        let mut interner = SymbolInterner::default();
        let root = ModulePath::new();
        let segment = interner.intern("foo");
        let child = root.child(segment);

        assert_eq!(child.len(), 1);
        assert_eq!(child.name(&interner), Some("foo"));

        // Verify root is unaffected.
        assert!(root.is_empty());
    }

    #[test]
    fn test_module_path_is_descendant_of() {
        let mut interner = SymbolInterner::default();
        let root = ModulePath::new();
        let foo = root.child(interner.intern("foo"));
        let foo_bar = foo.child(interner.intern("bar"));
        let baz = root.child(interner.intern("baz"));

        // root is descendant of itself.
        assert!(root.is_descendant_of(&root));
        // foo is descendant of root.
        assert!(foo.is_descendant_of(&root));
        // foo::bar is descendant of root and foo.
        assert!(foo_bar.is_descendant_of(&root));
        assert!(foo_bar.is_descendant_of(&foo));
        // foo is NOT descendant of foo::bar.
        assert!(!foo.is_descendant_of(&foo_bar));
        // baz is not descendant of foo.
        assert!(!baz.is_descendant_of(&foo));
    }

    #[test]
    fn test_module_path_equality() {
        let mut interner1 = SymbolInterner::default();
        let mut interner2 = SymbolInterner::default();

        let p1 = ModulePath::from_strings(&mut interner1, &["a", "b"]);
        let p2 = ModulePath::from_strings(&mut interner2, &["a", "b"]);

        // Same string segments produce the same path structure.
        assert_eq!(p1.segments(), p2.segments());
        assert_eq!(p1, p2);
    }

    #[test]
    fn test_module_path_default() {
        let path = ModulePath::default();
        assert!(path.is_empty());
    }

    #[test]
    fn test_module_path_display_root() {
        let path = ModulePath::new();
        assert_eq!(path.to_string(), "crate");
    }

    // --- Module struct tests ---

    #[test]
    fn test_module_new() {
        let mut interner = SymbolInterner::default();
        let path = ModulePath::from_strings(&mut interner, &["foo"]);
        let file_path = PathBuf::from("src/foo.stnx");
        let module = Module::new(ModuleId::new(1), path, file_path.clone());

        assert_eq!(module.id, ModuleId(1));
        assert!(!module.is_root());
        assert!(module.ast.is_none());
        assert_eq!(module.parent, None);
    }

    #[test]
    fn test_module_new_root() {
        let path = ModulePath::new();
        let file_path = PathBuf::from("src/main.stnx");
        let module = Module::new(ModuleId::ROOT, path, file_path);

        assert!(module.is_root());
        assert_eq!(module.name(&SymbolInterner::default()), "crate");
    }

    #[test]
    fn test_module_dir() {
        let path = ModulePath::new();
        let file_path = PathBuf::from("src/main.stnx");
        let module = Module::new(ModuleId::ROOT, path, file_path);

        assert_eq!(module.dir(), Path::new("src"));
    }

    // --- File resolution tests ---

    #[test]
    fn test_resolve_module_file_single_file() {
        let dir = tempdir().unwrap();
        let name = "math";
        let file_path = dir.path().join(format!("{}.stnx", name));
        std::fs::write(&file_path, "fn main() -> i64 {}").unwrap();

        let result = resolve_module_file(dir.path(), name);
        assert_eq!(result, Some(file_path));
    }

    #[test]
    fn test_resolve_module_file_directory_module() {
        let dir = tempdir().unwrap();
        let name = "math";
        let mod_dir = dir.path().join(name);
        std::fs::create_dir_all(&mod_dir).unwrap();
        let file_path = mod_dir.join("mod.stnx");
        std::fs::write(&file_path, "fn main() -> i64 {}").unwrap();

        let result = resolve_module_file(dir.path(), name);
        assert_eq!(result, Some(file_path));
    }

    #[test]
    fn test_resolve_module_file_single_file_takes_precedence() {
        let dir = tempdir().unwrap();
        let name = "math";

        // Create both forms.
        let single_file = dir.path().join(format!("{}.stnx", name));
        std::fs::write(&single_file, "fn main() -> i64 {}").unwrap();

        let mod_dir = dir.path().join(name);
        std::fs::create_dir_all(&mod_dir).unwrap();
        let mod_file = mod_dir.join("mod.stnx");
        std::fs::write(&mod_file, "fn main() -> i64 {}").unwrap();

        // The single-file form should be preferred.
        let result = resolve_module_file(dir.path(), name);
        assert_eq!(result, Some(single_file));
    }

    #[test]
    fn test_resolve_module_file_not_found() {
        let dir = tempdir().unwrap();
        let result = resolve_module_file(dir.path(), "nonexistent");
        assert!(result.is_none());
    }

    // --- mod declaration extraction tests ---

    #[test]
    fn test_extract_mod_declarations_basic() {
        let source = "mod io\nmod math\nfn main() -> i64 {}\n";
        let mods = extract_mod_declarations(source);
        assert_eq!(mods, vec!["io", "math"]);
    }

    #[test]
    fn test_extract_mod_declarations_indented() {
        let source = "    mod io\n  \nmod math\n";
        let mods = extract_mod_declarations(source);
        assert_eq!(mods, vec!["io", "math"]);
    }

    #[test]
    fn test_extract_mod_declarations_pub_mod() {
        let source = "pub mod io\nmod math\n";
        let mods = extract_mod_declarations(source);
        assert_eq!(mods, vec!["io", "math"]);
    }

    #[test]
    fn test_extract_mod_declarations_with_comments() {
        let source = "// mod ignored\nmod real_io\n";
        let mods = extract_mod_declarations(source);
        assert_eq!(mods, vec!["real_io"]);
    }

    #[test]
    fn test_extract_mod_declarations_none() {
        let source = "fn main() -> i64 {}\nlet x = 42\n";
        let mods = extract_mod_declarations(source);
        assert!(mods.is_empty());
    }

    #[test]
    fn test_extract_mod_declarations_nested() {
        // mod declarations with longer names
        let source = "mod utils\nmod graphics_math\n";
        let mods = extract_mod_declarations(source);
        assert_eq!(mods, vec!["utils", "graphics_math"]);
    }

    // --- ModuleGraph tests ---

    #[test]
    fn test_module_graph_new() {
        let graph = ModuleGraph::new();
        assert!(graph.is_empty());
        assert_eq!(graph.len(), 0);
        assert_eq!(graph.root_id(), ModuleId::ROOT);
    }

    #[test]
    fn test_module_graph_add_module() {
        let mut graph = ModuleGraph::new();
        let mut interner = SymbolInterner::default();
        let path = ModulePath::from_strings(&mut interner, &["foo"]);
        // add_module assigns the ID, so we pass a placeholder; the real ID
        // is returned by add_module and should match the position in the vec.
        let module = Module::new(
            ModuleId::new(0),
            path.clone(),
            PathBuf::from("src/foo.stnx"),
        );

        let id = graph.add_module(module);
        assert_eq!(id, ModuleId(0));
        assert_eq!(graph.len(), 1);
        assert_eq!(graph.get_module(id).unwrap().path, path);
    }

    #[test]
    fn test_module_graph_find_module() {
        let mut graph = ModuleGraph::new();
        // Use the graph's own interner so SymbolIds are consistent.
        let path = ModulePath::from_strings(&mut graph.symbol_interner, &["foo", "bar"]);
        let module = Module::new(
            ModuleId::new(0),
            path.clone(),
            PathBuf::from("src/foo/bar.stnx"),
        );

        let id = graph.add_module(module);
        assert_eq!(graph.find_module(&path), Some(id));
        assert_eq!(graph.find_module(&ModulePath::new()), None);
    }

    #[test]
    fn test_module_graph_format_path() {
        // Use the graph's own interner so SymbolIds are consistent for formatting.
        let mut graph = ModuleGraph::new();
        let path = ModulePath::from_strings(&mut graph.symbol_interner, &["graphics", "math"]);
        assert_eq!(graph.format_path(&path), "crate::graphics::math");
    }

    #[test]
    fn test_module_graph_format_path_root() {
        let graph = ModuleGraph::new();
        let path = ModulePath::new();
        let formatted = graph.format_path(&path);
        assert_eq!(formatted, "crate");
    }

    #[test]
    fn test_module_graph_resolve_path() {
        let mut graph = ModuleGraph::new();
        let root_path = ModulePath::new();
        let root_module = Module::new(
            ModuleId::ROOT,
            root_path.clone(),
            PathBuf::from("src/main.stnx"),
        );
        let root_id = graph.add_module(root_module);

        // Register a child module path manually in the index.
        let foo_seg = graph.symbol_interner.intern("foo");
        let foo_path = root_path.child(foo_seg);
        let foo_module = Module::new(
            ModuleId::new(1),
            foo_path.clone(),
            PathBuf::from("src/foo.stnx"),
        );
        let foo_id = graph.add_module(foo_module);
        assert_eq!(foo_id, ModuleId(1));

        // Resolve from root to foo.
        let result = graph.resolve_path(root_id, &[foo_seg]);
        assert_eq!(result, Some(foo_id));
    }

    // --- Project discovery tests ---

    #[test]
    fn test_project_discover_finds_saturn_toml() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        // Write a saturn.toml.
        std::fs::write(root.join("saturn.toml"), "[package]\nname = \"testproj\"\n").unwrap();

        let project = Project::discover(root).unwrap();
        assert_eq!(project.config.package.name, "testproj");
        assert_eq!(project.root, root);
        assert_eq!(project.source_root, root.join("src"));
    }

    #[test]
    fn test_project_discover_walks_upward() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        // Create a nested directory structure.
        let nested = root.join("a").join("b").join("c");
        std::fs::create_dir_all(&nested).unwrap();

        // saturn.toml at the top level.
        std::fs::write(
            root.join("saturn.toml"),
            "[package]\nname = \"nested_test\"\n",
        )
        .unwrap();

        // Discover from the nested directory.
        let project = Project::discover(&nested).unwrap();
        assert_eq!(project.config.package.name, "nested_test");
        assert_eq!(project.root, root);
    }

    #[test]
    fn test_project_discover_no_saturn_toml_synthesized() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        // No saturn.toml anywhere; should synthesize a config.
        let project = Project::discover(root).unwrap();
        // The name should come from the directory name.
        assert_eq!(
            project.config.package.name,
            root.file_name().unwrap().to_str().unwrap()
        );
        assert_eq!(project.root, root);
    }

    #[test]
    fn test_project_discover_from_file() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        std::fs::write(root.join("saturn.toml"), "[package]\nname = \"fromfile\"\n").unwrap();
        let src_dir = root.join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        let main_file = src_dir.join("main.stnx");
        std::fs::write(&main_file, "fn main() -> i64 {}").unwrap();

        // Discover from a file path.
        let project = Project::discover(&main_file).unwrap();
        assert_eq!(project.config.package.name, "fromfile");
        assert_eq!(project.root, root);
    }

    // --- ModuleGraph::discover_modules tests ---

    #[test]
    fn test_discover_modules_single_file() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        // Create main.stnx with no mod declarations.
        let main_file = root.join("main.stnx");
        std::fs::write(&main_file, "fn main() -> i64 { return 0 }").unwrap();

        let graph = ModuleGraph::discover_modules(main_file.clone()).unwrap();

        assert_eq!(graph.len(), 1);
        assert_eq!(graph.root_id(), ModuleId::ROOT);
        assert!(graph.root_module().is_root());
        assert_eq!(
            graph.get_module(ModuleId::ROOT).unwrap().file_path,
            main_file
        );
    }

    #[test]
    fn test_discover_modules_with_child_module() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        // Create main.stnx with a mod declaration.
        let main_file = root.join("main.stnx");
        std::fs::write(&main_file, "mod io\nfn main() -> i64 { return 0 }").unwrap();

        // Create the child module as a single file.
        let io_file = root.join("io.stnx");
        std::fs::write(&io_file, "fn println(n: i64) -> unit {}").unwrap();

        let graph = ModuleGraph::discover_modules(main_file).unwrap();

        assert_eq!(graph.len(), 2);

        // The child module should be findable by its path.
        let mut interner = SymbolInterner::default();
        let child_path = ModulePath::from_strings(&mut interner, &["io"]);
        let child_id = graph
            .find_module(&child_path)
            .expect("child module should be in graph");
        let child = graph.get_module(child_id).unwrap();
        assert_eq!(child.file_path, io_file);
        assert_eq!(child.parent, Some(ModuleId::ROOT));
    }

    #[test]
    fn test_discover_modules_directory_module() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        // Create main.stnx.
        let main_file = root.join("main.stnx");
        std::fs::write(&main_file, "mod io\nfn main() -> i64 { return 0 }").unwrap();

        // Create io/mod.stnx.
        let io_dir = root.join("io");
        std::fs::create_dir_all(&io_dir).unwrap();
        let io_mod_file = io_dir.join("mod.stnx");
        std::fs::write(&io_mod_file, "fn debug(n: i64) -> unit {}").unwrap();

        let graph = ModuleGraph::discover_modules(main_file).unwrap();

        assert_eq!(graph.len(), 2);

        let mut interner = SymbolInterner::default();
        let child_path = ModulePath::from_strings(&mut interner, &["io"]);
        let child_id = graph
            .find_module(&child_path)
            .expect("child module should be in graph");
        assert_eq!(graph.get_module(child_id).unwrap().file_path, io_mod_file);
    }

    #[test]
    fn test_discover_modules_nested_modules() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        // main.stnx declares mod utils
        let main_file = root.join("main.stnx");
        std::fs::write(&main_file, "mod utils\nfn main() -> i64 { return 0 }").unwrap();

        // utils/mod.stnx declares mod math
        let utils_dir = root.join("utils");
        std::fs::create_dir_all(&utils_dir).unwrap();
        let utils_mod_file = utils_dir.join("mod.stnx");
        std::fs::write(&utils_mod_file, "mod math\n").unwrap();

        // utils/math.stnx
        let math_file = utils_dir.join("math.stnx");
        std::fs::write(&math_file, "fn add(a: i64, b: i64) -> i64 {}").unwrap();

        let graph = ModuleGraph::discover_modules(main_file).unwrap();

        // Should have: root, utils, utils::math = 3 modules.
        assert_eq!(graph.len(), 3);

        // Check that utils::math is discoverable.
        let mut interner = SymbolInterner::default();
        let math_path = ModulePath::from_strings(&mut interner, &["utils", "math"]);
        let math_id = graph
            .find_module(&math_path)
            .expect("math module should be in graph");
        let math_module = graph.get_module(math_id).unwrap();
        assert_eq!(math_module.file_path, math_file);

        // Check that utils is also present.
        let utils_path = ModulePath::from_strings(&mut interner, &["utils"]);
        let utils_id = graph
            .find_module(&utils_path)
            .expect("utils module should be in graph");
        assert_eq!(
            graph.get_module(utils_id).unwrap().parent,
            Some(ModuleId::ROOT)
        );
        assert_eq!(math_module.parent, Some(utils_id));
    }

    #[test]
    fn test_discover_modules_unresolved_error() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        // main.stnx declares a mod that doesn't exist on disk.
        let main_file = root.join("main.stnx");
        std::fs::write(&main_file, "mod nonexistent\nfn main() -> i64 { return 0 }").unwrap();

        let result = ModuleGraph::discover_modules(main_file);
        assert!(result.is_err());
        // The error message should mention the unresolved module.
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("nonexistent"),
            "error should mention the module name: {}",
            err
        );
    }

    #[test]
    fn test_discover_modules_single_file_preferred_over_directory() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        let main_file = root.join("main.stnx");
        std::fs::write(&main_file, "mod foo\nfn main() -> i64 { return 0 }").unwrap();

        // Create both foo.stnx and foo/mod.stnx.
        let single_file = root.join("foo.stnx");
        std::fs::write(&single_file, "fn foo_fn() -> i64 {}").unwrap();

        let foo_dir = root.join("foo");
        std::fs::create_dir_all(&foo_dir).unwrap();
        let mod_file = foo_dir.join("mod.stnx");
        std::fs::write(&mod_file, "fn foo_mod_fn() -> i64 {}").unwrap();

        let graph = ModuleGraph::discover_modules(main_file).unwrap();

        // Should find foo module.
        let mut interner = SymbolInterner::default();
        let foo_path = ModulePath::from_strings(&mut interner, &["foo"]);
        let foo_id = graph
            .find_module(&foo_path)
            .expect("foo module should be in graph");

        // The single-file form should be preferred.
        assert_eq!(graph.get_module(foo_id).unwrap().file_path, single_file);
    }

    // --- Project load tests ---

    #[test]
    fn test_project_load_default_entry() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        std::fs::write(root.join("saturn.toml"), "[package]\nname = \"loadtest\"\n").unwrap();
        let src_dir = root.join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        let main_file = src_dir.join("main.stnx");
        std::fs::write(&main_file, "fn main() -> i64 { return 0 }").unwrap();

        let mut project = Project::discover(root).unwrap();
        let program = project.load().unwrap();

        // Should have loaded the main module.
        assert_eq!(project.graph.len(), 1);
        // The root module's AST should contain the `main` function.
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_project_load_from_explicit_file() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        std::fs::write(root.join("saturn.toml"), "[package]\nname = \"loadfile\"\n").unwrap();
        let custom_file = root.join("custom.stnx");
        std::fs::write(&custom_file, "fn main() -> i64 { return 0 }").unwrap();

        let mut project = Project::discover(root).unwrap();
        let program = project.load_from(&custom_file).unwrap();

        assert_eq!(project.graph.len(), 1);
        let root_module = project.graph.root_module();
        assert_eq!(root_module.file_path, custom_file);
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_project_load_no_entry_point_error() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        std::fs::write(root.join("saturn.toml"), "[package]\nname = \"noentry\"\n").unwrap();
        // Create src/ but no main.stnx.
        std::fs::create_dir_all(root.join("src")).unwrap();

        let mut project = Project::discover(root).unwrap();
        let result = project.load();
        assert!(result.is_err());
    }
}
