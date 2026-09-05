use std::ops::Range;

// --- Types ---

#[derive(Clone, Debug, PartialEq)]
pub enum Type {
    I64,
    F64,
    Bool,
    Str,
    Unit,
    /// A named struct type, referenced by name. The name is resolved
    /// to a `SymbolId` during HIR lowering. For the AST, the name is
    /// an unresolved `String` (what the programmer wrote).
    Struct(String),
    /// A named enum type, referenced by name.
    Enum(String),
    /// `List<T>` — a homogeneous list type. For 0.5 this is parsed and
    /// lowered but the runtime support is deferred (desugars to a
    /// sequence of allocations). Tracked here so the parser can accept
    /// the syntax without changing semantics.
    List(Box<Type>),
}

// --- AST Nodes ---

#[derive(Clone, Debug)]
pub struct Program {
    /// Top-level items — functions, structs, enums, modules, and uses.
    /// This is the authoritative collection; `functions` is kept for backwards
    /// compatibility during the Phase 5 transition.
    pub items: Vec<Item>,
    /// Backwards-compatible view of top-level functions.
    /// Populated alongside `items` so existing HIR lowering continues to work
    /// until Phase 5B migrates to iterating `items` directly.
    pub functions: Vec<Function>,
}

impl Program {
    /// Convenience: build a `Program` from a list of items.
    pub fn from_items(items: Vec<Item>) -> Self {
        let functions: Vec<Function> = items
            .iter()
            .filter_map(|item| match &item.kind {
                ItemKind::Function(f) => Some(f.clone()),
                _ => None,
            })
            .collect();
        Program { functions, items }
    }
}

/// A top-level program item (function, struct, enum, module, or use).
#[derive(Clone, Debug)]
pub struct Item {
    /// The name of this item (the last path segment).
    /// Empty for `use` declarations.
    pub name: String,
    /// Visibility of this item.
    pub visibility: Visibility,
    /// What kind of item this is.
    pub kind: ItemKind,
    /// Byte-span of the item in the source.
    pub span: Range<usize>,
}

/// Visibility modifier for items: `pub` or private.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    Private,
    Public,
}

/// The kind of a top-level item.
#[derive(Clone, Debug)]
pub enum ItemKind {
    Function(Function),
    StructDef {
        name: String,
        generic_params: Vec<String>,
        fields: Vec<(String, Type)>,
        span: Range<usize>,
    },
    EnumDef {
        name: String,
        generic_params: Vec<String>,
        variants: Vec<String>,
        span: Range<usize>,
    },
    /// `mod foo` — declares a dependency on an external module file.
    /// The module name is the item's `name`; the file is loaded later by the
    /// module loader (Phase 4 infrastructure in `module.rs`).
    ModDecl,
    /// `use foo::bar` — imports `bar` into the current module's namespace.
    /// The last path segment becomes available as a local name.
    UseDecl {
        path: Vec<String>,
        alias: Option<String>,
    },
    /// `module name` — 0.5 advisory module declaration (no semantic effect).
    ModuleDecl,
    /// `main:` — 0.5 entry-point block. Lowered to a synthetic `Function`
    /// named `main` with empty parameters and `i64` return type.
    MainBlock(Vec<Stmt>, Range<usize>),
    /// `external <kind> "<ecosystem>" "<symbol>"(params) -> ret` — declares
    /// a foreign function call across an interoperability boundary.
    ///
    /// The declaration is explicit metadata: the compiler records the
    /// ecosystem name, the foreign symbol, the ABI-safe parameter types, and
    /// the return type. It does NOT parse arbitrary foreign source. The
    /// runtime bridge resolves the symbol at link/runtime time.
    ///
    /// `kind` is one of `rust`, `python`, or `native` (case-sensitive).
    ExternalFunction {
        kind: ExternalKind,
        /// The foreign ecosystem name: a Rust crate name, a Python module
        /// name, or a native library name.
        ecosystem: String,
        /// The foreign symbol to bind to. For Rust/Native this is the
        /// link-time symbol; for Python this is the module-qualified
        /// function name.
        symbol: String,
        /// Parameters in declaration order.
        params: Vec<(String, Type)>,
        /// Return type.
        return_type: Type,
        span: Range<usize>,
    },
}

/// The runtime kind of an `external` declaration.
///
/// Rust and Python are kept as separate bridges (per the campaign rules);
/// `Native` is the path for plain C-ABI shared libraries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExternalKind {
    Rust,
    Python,
    Native,
}

#[derive(Clone, Debug)]
pub struct Function {
    pub name: String,
    /// Generic parameter names (`fn id<T>(...)` → `["T"]`).
    /// Empty for non-generic functions.
    pub generic_params: Vec<String>,
    pub params: Vec<(String, Type)>,
    pub return_type: Type,
    pub body: Vec<Stmt>,
    pub span: Range<usize>,
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Let {
        name: String,
        mutable: bool,
        ty: Option<Type>,
        value: Expr,
        span: Range<usize>,
    },
    Expr(Expr, Range<usize>),
    Return(Option<Expr>, Range<usize>),
    Println(Expr, Range<usize>),
    /// `give expr` — 0.5 synonym for `return`.
    Give(Option<Expr>, Range<usize>),
    /// `say expr` — 0.5 synonym for `println`.
    Say(Expr, Range<usize>),
    /// `raise expr` — 0.5 error raise. Lowers to a stub that prints the
    /// expression and aborts the process; real error semantics are deferred.
    Raise(Expr, Range<usize>),
    /// A struct definition: `struct Point { x: i64, y: i64 }`
    StructDef {
        name: String,
        generic_params: Vec<String>,
        fields: Vec<(String, Type)>,
        span: Range<usize>,
    },
    /// An enum definition: `enum Result { Ok, Error }`
    EnumDef {
        name: String,
        generic_params: Vec<String>,
        variants: Vec<String>,
        span: Range<usize>,
    },
}

/// A single segment of an interpolated string.
#[derive(Clone, Debug)]
pub enum StrPart {
    /// A literal text segment, with no `{...}` expressions inside it.
    Literal(String),
    /// An interpolated expression (the contents of a `{...}` in the source).
    Expr(Expr),
}

/// A single argument in a call: either positional or named.
#[derive(Clone, Debug)]
pub enum CallArg {
    Positional(Expr),
    Named { name: String, value: Expr },
}

/// A single parameter of a closure: name with optional explicit annotation.
#[derive(Clone, Debug)]
pub struct ClosureParam {
    pub name: String,
    pub ty: Option<Type>,
}

#[derive(Clone, Debug)]
pub enum Expr {
    Integer(i64, Range<usize>),
    Float(f64, Range<usize>),
    StrLit(String, Range<usize>),
    Bool(bool, Range<usize>),
    Unit(Range<usize>),
    Var(String, Range<usize>),
    Assign {
        target: String,
        value: Box<Expr>,
        span: Range<usize>,
    },
    AugAssign {
        target: String,
        op: AugOp,
        value: Box<Expr>,
        span: Range<usize>,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        span: Range<usize>,
    },
    Unary {
        op: UnOp,
        expr: Box<Expr>,
        span: Range<usize>,
    },
    Call {
        func: String,
        /// Positional arguments (in source order). Named arguments live in
        /// `named_args` and are reordered against the callee's signature at
        /// AST→HIR lowering.
        args: Vec<Expr>,
        /// Named arguments: `f(name: value)`.
        named_args: Vec<(String, Expr)>,
        /// Explicit type arguments supplied at the call site via turbofish
        /// syntax (`f::<i64, bool>(x, y)`). Empty when no turbofish was used.
        type_args: Vec<Type>,
        span: Range<usize>,
    },
    If {
        condition: Box<Expr>,
        then_branch: Vec<Stmt>,
        elif_branches: Vec<(Expr, Vec<Stmt>)>,
        else_branch: Option<Vec<Stmt>>,
        span: Range<usize>,
    },
    For {
        var: String,
        iter: Box<Expr>,
        body: Vec<Stmt>,
        span: Range<usize>,
    },
    While {
        condition: Box<Expr>,
        body: Vec<Stmt>,
        span: Range<usize>,
    },
    Range {
        start: Box<Expr>,
        end: Box<Expr>,
        is_inclusive: bool,
        span: Range<usize>,
    },
    /// A struct literal: `Point { x: 10, y: 20 }`
    StructLiteral {
        name: String,
        fields: Vec<(String, Expr)>,
        /// Explicit type arguments supplied via turbofish syntax
        /// (`Box::<i64> { value: 21 }`). Empty when no turbofish was used.
        type_args: Vec<Type>,
        span: Range<usize>,
    },
    /// Field access: `p.x`
    FieldAccess {
        expr: Box<Expr>,
        field: String,
        span: Range<usize>,
    },
    /// List element access: `items[i]`. The container must be a List.
    Index {
        list: Box<Expr>,
        index: Box<Expr>,
        span: Range<usize>,
    },
    /// List length: `items.length`. The receiver must be a List.
    Length {
        expr: Box<Expr>,
        span: Range<usize>,
    },
    /// Enum variant construction: `Result::Ok`
    EnumConstructor {
        name: String,
        variant: String,
        span: Range<usize>,
    },
    /// 0.5 pipeline: `a |> f(x)` desugars to `f(a, x)` at AST→HIR lowering.
    Pipeline {
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        span: Range<usize>,
    },
    /// 0.5 closure: `x -> body` or `(x, y) -> body`. Lowered to a synthetic
    /// top-level `Function` whose body captures free variables (lambda
    /// lifting). At call sites the closure expression is replaced by a
    /// `Var` reference to the synthetic function.
    Closure {
        params: Vec<ClosureParam>,
        body: Box<Expr>,
        span: Range<usize>,
    },
    /// Real list literal: `[1, 2, 3]`. Element expressions stored directly.
    ListLiteral {
        items: Vec<Expr>,
        span: Range<usize>,
    },
    /// 0.5 string interpolation: `"hello {name}!"`. Lowered to a chain of
    /// `concat_str` calls at AST→HIR lowering.
    InterpolatedStr(Vec<StrPart>, Range<usize>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AugOp {
    Add,
    Sub,
    Mul,
    Div,
}
