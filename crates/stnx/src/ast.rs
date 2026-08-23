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
        fields: Vec<(String, Type)>,
        span: Range<usize>,
    },
    EnumDef {
        name: String,
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
}

#[derive(Clone, Debug)]
pub struct Function {
    pub name: String,
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
    /// A struct definition: `struct Point { x: i64, y: i64 }`
    StructDef {
        name: String,
        fields: Vec<(String, Type)>,
        span: Range<usize>,
    },
    /// An enum definition: `enum Result { Ok, Error }`
    EnumDef {
        name: String,
        variants: Vec<String>,
        span: Range<usize>,
    },
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
        args: Vec<Expr>,
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
        span: Range<usize>,
    },
    /// Field access: `p.x`
    FieldAccess {
        expr: Box<Expr>,
        field: String,
        span: Range<usize>,
    },
    /// Enum variant construction: `Result::Ok`
    EnumConstructor {
        name: String,
        variant: String,
        span: Range<usize>,
    },
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
