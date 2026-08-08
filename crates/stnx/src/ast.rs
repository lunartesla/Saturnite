use std::ops::Range;

// --- Types ---

#[derive(Clone, Debug, PartialEq)]
pub enum Type {
    I64,
    F64,
    Bool,
    Str,
    Unit,
}

// --- AST Nodes ---

#[derive(Clone, Debug)]
pub struct Program {
    pub functions: Vec<Function>,
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
