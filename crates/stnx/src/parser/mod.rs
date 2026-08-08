pub use crate::ast::*;

use crate::error::CompilerResult;
use crate::lexer::{Token, TokenKind};
use chumsky::prelude::*;
use chumsky::recursive::Direct;
use std::ops::Range;

pub fn parse(src: &str, tokens: Vec<Token>) -> CompilerResult<Program> {
    let parser = program();
    let result = parser.parse(&tokens);

    let (output, errors) = result.into_output_errors();

    if errors.is_empty() {
        Ok(output.unwrap())
    } else {
        eprintln!("Parse errors ({}):", errors.len());
        eprintln!("Tokens: {:?}", tokens.iter().map(|t| &t.kind).collect::<Vec<_>>());
        for e in &errors {
            // EmptyErr has no span info, just print it
            eprintln!("  {:?}", e);
        }
        let error_msgs: Vec<String> = errors.iter().map(|e| format!("{:?}", e)).collect();
        Err(crate::error::CompilerError::Parse(error_msgs.join("; ")))
    }
}

fn program<'a>() -> impl Parser<'a, &'a [Token], Program> {
    func()
        .repeated()
        .collect::<Vec<_>>()
        .map(|fns| Program { functions: fns })
}

fn func<'a>() -> impl Parser<'a, &'a [Token], Function> {
    kw("fn")
        .ignore_then(t_ident())
        .then(params())
        .then(ret_type())
        .then(block(recursive_expr()))
        .map(|((((name, name_span), params), ret_type), body)| Function {
            name,
            params,
            return_type: ret_type,
            body,
            span: name_span,
        })
}

fn params<'a>() -> impl Parser<'a, &'a [Token], Vec<(String, Type)>> {
    let param = t_ident().map(|(name, _span)| name).then(type_ann());

    lparen()
        .ignore_then(param.separated_by(comma()).collect::<Vec<_>>())
        .then_ignore(rparen())
        .or(rparen().to(Vec::new()))
}

fn type_ann<'a>() -> impl Parser<'a, &'a [Token], Type> {
    colon().ignore_then(
        kw("i64").to(Type::I64)
            .or(kw("f64").to(Type::F64))
            .or(kw("bool").to(Type::Bool))
            .or(kw("str").to(Type::Str))
            .or(kw("unit").to(Type::Unit)),
    )
}

fn ret_type<'a>() -> impl Parser<'a, &'a [Token], Type> {
    rarrow()
        .ignore_then(
            kw("i64").to(Type::I64)
                .or(kw("f64").to(Type::F64))
                .or(kw("bool").to(Type::Bool))
                .or(kw("str").to(Type::Str))
                .or(kw("unit").to(Type::Unit)),
        )
        .or_not()
        .map(|opt: Option<Type>| opt.unwrap_or(Type::Unit))
}

fn block<'a>(
    expr: Recursive<Direct<'a, 'a, &'a [Token], Expr, extra::Default>>,
) -> impl Parser<'a, &'a [Token], Vec<Stmt>> + 'a {
    lbrace()
        .ignore_then(stmt(expr.clone()).repeated().collect::<Vec<_>>())
        .then_ignore(rbrace())
}

fn stmt<'a>(
    expr: Recursive<Direct<'a, 'a, &'a [Token], Expr, extra::Default>>,
) -> Boxed<'a, 'a, &'a [Token], Stmt> {
    let let_stmt = kw("let")
        .ignore_then(
            kw("mut")
                .to(true)
                .or_not()
                .map(|opt: Option<bool>| opt.unwrap_or(false)),
        )
        .then(t_ident())
        .then(type_ann().or_not())
        .then(assign().ignore_then(expr.clone()))
        .map(|(((mutable, (name, name_span)), ty), value)| {
            // Use the union of the name span and value span as the statement span
            let value_span = stmt_span(&value);
            let span = name_span.start.min(value_span.start)..value_span.end.max(name_span.end);
            Stmt::Let {
                name,
                mutable,
                ty,
                value,
                span,
            }
        });

    let return_stmt = kw("return")
        .ignore_then(expr.clone().or_not())
        .map(|e| {
            let span = e.as_ref().map(stmt_span).unwrap_or_default();
            Stmt::Return(e, span)
        });

    let println_stmt = kw("println")
        .ignore_then(lparen())
        .ignore_then(expr.clone())
        .then_ignore(rparen())
        .map(|e| {
            let span = stmt_span(&e);
            Stmt::Println(e, span)
        });

    let expr_stmt = expr.map(|e| {
        let span = stmt_span(&e);
        Stmt::Expr(e, span)
    });

    let_stmt
        .or(return_stmt)
        .or(println_stmt)
        .or(expr_stmt)
        .boxed()
}

fn recursive_expr<'a>() -> Recursive<Direct<'a, 'a, &'a [Token], Expr, extra::Default>> {
    recursive(|expr| {
        // Precedence chain (left-associative, left-recursive):
        // Logical -> Comparison -> Additive -> Multiplicative -> Unary -> Primary
        //
        // To avoid stack overflow from deep recursion, we:
        // 1. Use .memoized() on the recursive expr to cache results and detect cycles
        // 2. Structure the chain so primary is tried once, not re-entered

        // Primary: literals, identifiers, calls, parenthesized expressions
        let primary: Boxed<'a, 'a, &'a [Token], Expr> = any::<&[Token], _>()
            .filter(|t: &Token| matches!(&t.kind, TokenKind::Integer(_)))
            .map(|t| match &t.kind {
                TokenKind::Integer(n) => Expr::Integer(*n, t.span.clone()),
                _ => unreachable!(),
            })
            .or(any::<&[Token], _>()
                .filter(|t: &Token| matches!(&t.kind, TokenKind::Float(_)))
                .map(|t| match &t.kind {
                    TokenKind::Float(f) => Expr::Float(*f, t.span.clone()),
                    _ => unreachable!(),
                }))
            .or(any::<&[Token], _>()
                .filter(|t: &Token| matches!(&t.kind, TokenKind::StrLit(_)))
                .map(|t| match &t.kind {
                    TokenKind::StrLit(s) => Expr::StrLit(s.clone(), t.span.clone()),
                    _ => unreachable!(),
                }))
            .or(kw("true").to(Expr::Bool(true, Range::default()))
                .or(kw("false").to(Expr::Bool(false, Range::default()))))
            .or(lbrace().ignore_then(rbrace()).to(Expr::Unit(Range::default())))
            .or(t_ident().then(
                lparen()
                    .ignore_then(expr.clone().separated_by(comma()).collect::<Vec<_>>())
                    .then_ignore(rparen())
                    .or_not(),
            ).map(|((name, name_span), args)| {
                if let Some(args) = args {
                    Expr::Call { func: name, args, span: name_span }
                } else {
                    Expr::Var(name, name_span)
                }
            }))
            .boxed();

        // Unary: -expr, !expr
        let unary = minus().ignore_then(expr.clone()).map(|e| Expr::Unary {
            op: UnOp::Neg,
            expr: Box::new(e),
            span: Range::default(),
        })
        .or(bang().ignore_then(expr.clone()).map(|e| Expr::Unary {
            op: UnOp::Not,
            expr: Box::new(e),
            span: Range::default(),
        }))
        .or(primary.clone())
        .boxed();

        // Multiplicative: * / %
        let mul_chain = unary.clone()
            .foldl(
                mul_op().then(unary.clone()).repeated(),
                |lhs, (op, rhs)| Expr::Binary {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                    span: Range::default(),
                },
            ).boxed();

        // Additive: + -
        let add_chain = mul_chain.clone()
            .foldl(
                add_op().then(mul_chain.clone()).repeated(),
                |lhs, (op, rhs)| Expr::Binary {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                    span: Range::default(),
                },
            ).boxed();

        // Comparison: == != < > <= >=
        let cmp_chain = add_chain.clone()
            .foldl(
                cmp_op().then(add_chain.clone()).repeated(),
                |lhs, (op, rhs)| Expr::Binary {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                    span: Range::default(),
                },
            ).boxed();

        // Logical AND: &&
        let and_chain = cmp_chain.clone()
            .foldl(
                and_op().then(cmp_chain.clone()).repeated(),
                |lhs, (op, rhs)| Expr::Binary {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                    span: Range::default(),
                },
            ).boxed();

        // Logical OR: ||
        let or_chain = and_chain.clone()
            .foldl(
                or_op().then(and_chain.clone()).repeated(),
                |lhs, (op, rhs)| Expr::Binary {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                    span: Range::default(),
                },
            ).boxed();

        // Range: start..end (exclusive), start...end (inclusive)
        // Lower precedence than logical operators
        let range_expr = or_chain.clone()
            .then(dot_dot().to(false).or(dot_dot_dot().to(true)).then(or_chain.clone()).repeated().collect::<Vec<_>>())
            .map(|(first, rest)| {
                if rest.is_empty() {
                    first
                } else {
                    // Chain ranges: a..b..c => Range(a..b, c, false) etc.
                    // For simplicity, only handle single range
                    let (is_inclusive, last) = rest.into_iter().next().unwrap();
                    Expr::Range {
                        start: Box::new(first),
                        end: Box::new(last),
                        is_inclusive,
                        span: Range::default(),
                    }
                }
            })
            .boxed();

        // Control flow expressions - these start with keywords, so they don't
        // cause recursion when the first token doesn't match
        let if_expr = kw("if")
            .ignore_then(expr.clone())
            .then(block_from_expr(expr.clone()))
            .then(
                kw("elif")
                    .ignore_then(expr.clone())
                    .then(block_from_expr(expr.clone()))
                    .repeated()
                    .collect::<Vec<_>>(),
            )
            .then(kw("else").ignore_then(block_from_expr(expr.clone())).or_not())
            .map(|(((cond, then_b), elifs), else_b)| Expr::If {
                condition: Box::new(cond),
                then_branch: then_b,
                elif_branches: elifs,
                else_branch: else_b,
                span: Range::default(),
            }).boxed();

        let for_expr = kw("for")
            .ignore_then(t_ident())
            .then_ignore(kw("in"))
            .then(expr.clone())
            .then(block_from_expr(expr.clone()))
            .map(|(((var, var_span), iter), body)| Expr::For {
                var,
                iter: Box::new(iter),
                body,
                span: var_span,
            }).boxed();

        let while_expr = kw("while")
            .ignore_then(expr.clone())
            .then(block_from_expr(expr.clone()))
            .map(|(cond, body)| Expr::While {
                condition: Box::new(cond),
                body,
                span: Range::default(),
            }).boxed();

        // Assignment: ident = expr (and augmented assignment: ident += expr)
        let assign_expr = t_ident()
            .then(
                plus_assign().to(AugOp::Add)
                    .or(minus_assign().to(AugOp::Sub))
                    .or(star_assign().to(AugOp::Mul))
                    .or(slash_assign().to(AugOp::Div)),
            )
            .then(expr.clone())
            .map(|(((target, _target_span), op), value)| Expr::AugAssign {
                target,
                op,
                value: Box::new(value),
                span: Range::default(),
            })
            .or(t_ident().then(assign()).then(expr.clone())
                .map(|(((target, _target_span), _), value)| Expr::Assign {
                    target,
                    value: Box::new(value),
                    span: Range::default(),
                }))
            .boxed();

        // Control flow expressions are tried first (they start with keywords
        // that don't match primary), then assignment (starts with ident),
        // then the basic expression chain (logical OR is the outermost binary layer)
        if_expr
            .or(for_expr.clone())
            .or(while_expr.clone())
            .or(assign_expr.clone())
            .or(range_expr.clone())
            .memoized()
            .boxed()
    })
}

fn block_from_expr<'a>(
    expr: Recursive<Direct<'a, 'a, &'a [Token], Expr, extra::Default>>,
) -> impl Parser<'a, &'a [Token], Vec<Stmt>> + Clone + 'a {
    lbrace()
        .ignore_then(
            stmt(expr).repeated().collect::<Vec<_>>()
        )
        .then_ignore(rbrace())
        .boxed()
}

fn expr<'a>() -> impl Parser<'a, &'a [Token], Expr> {
    recursive_expr()
}

// Debug function for testing
pub fn parse_debug(tokens: &[Token]) -> Result<(), Vec<String>> {
    let parser = recursive_expr();
    let result = parser.parse(tokens);
    let (output, errors) = result.into_output_errors();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.iter().map(|e| format!("{:?}", e)).collect())
    }
}

// Debug function to test stmt parsing
pub fn stmt_debug(tokens: &[Token]) -> Result<(), Vec<String>> {
    let expr = recursive_expr();
    let parser = stmt(expr);
    let result = parser.parse(tokens);
    let (output, errors) = result.into_output_errors();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.iter().map(|e| format!("{:?}", e)).collect())
    }
}

// Debug function to test block parsing
pub fn block_debug(tokens: &[Token]) -> Result<(), Vec<String>> {
    let expr = recursive_expr();
    let parser = block(expr);
    let result = parser.parse(tokens);
    let (output, errors) = result.into_output_errors();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.iter().map(|e| format!("{:?}", e)).collect())
    }
}

// Debug function to test func parsing
pub fn func_debug(tokens: &[Token]) -> Result<(), Vec<String>> {
    let parser = func();
    let result = parser.parse(tokens);
    let (output, errors) = result.into_output_errors();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.iter().map(|e| format!("{:?}", e)).collect())
    }
}

// Debug function to test params parsing
pub fn params_debug(tokens: &[Token]) -> Result<(), Vec<String>> {
    let parser = params();
    let result = parser.parse(tokens);
    let (output, errors) = result.into_output_errors();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.iter().map(|e| format!("{:?}", e)).collect())
    }
}

// Debug function to test ret_type parsing
pub fn ret_type_debug(tokens: &[Token]) -> Result<(), Vec<String>> {
    let parser = ret_type();
    let result = parser.parse(tokens);
    let (output, errors) = result.into_output_errors();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.iter().map(|e| format!("{:?}", e)).collect())
    }
}

// Debug function to test program parsing
pub fn program_debug(tokens: &[Token]) -> Result<(), Vec<String>> {
    let parser = program();
    let result = parser.parse(tokens);
    let (output, errors) = result.into_output_errors();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.iter().map(|e| format!("{:?}", e)).collect())
    }
}

fn mul_op<'a>() -> impl Parser<'a, &'a [Token], BinOp> + Clone + 'a {
    star().to(BinOp::Mul)
        .or(slash().to(BinOp::Div))
        .or(percent().to(BinOp::Mod))
        .boxed()
}

fn add_op<'a>() -> impl Parser<'a, &'a [Token], BinOp> + Clone + 'a {
    plus().to(BinOp::Add)
        .or(minus().to(BinOp::Sub))
        .boxed()
}

fn cmp_op<'a>() -> impl Parser<'a, &'a [Token], BinOp> + Clone + 'a {
    eqeq().to(BinOp::Eq)
        .or(not_eq().to(BinOp::Ne))
        .or(lt().to(BinOp::Lt))
        .or(gt().to(BinOp::Gt))
        .or(lte().to(BinOp::Le))
        .or(gte().to(BinOp::Ge))
        .boxed()
}

// --- Token helpers ---

fn kw<'a>(k: &'a str) -> impl Parser<'a, &'a [Token], ()> {
    any::<&[Token], _>()
        .filter(move |t: &Token| match (&t.kind, k) {
            (TokenKind::Fn, "fn") |
            (TokenKind::Let, "let") |
            (TokenKind::Mut, "mut") |
            (TokenKind::If, "if") |
            (TokenKind::Elif, "elif") |
            (TokenKind::Else, "else") |
            (TokenKind::For, "for") |
            (TokenKind::While, "while") |
            (TokenKind::In, "in") |
            (TokenKind::Return, "return") |
            (TokenKind::I64, "i64") |
            (TokenKind::F64, "f64") |
            (TokenKind::Bool, "bool") |
            (TokenKind::Str, "str") |
            (TokenKind::Unit, "unit") |
            (TokenKind::True, "true") |
            (TokenKind::False, "false") |
            (TokenKind::Println, "println") => true,
            _ => false,
        })
        .ignored()
}

fn t_ident<'a>() -> impl Parser<'a, &'a [Token], (String, Range<usize>)> {
    any::<&[Token], _>()
        .filter(|t: &Token| {
            matches!(&t.kind, TokenKind::Ident(s) if !is_keyword(s))
        })
        .map(|t| match &t.kind {
            TokenKind::Ident(s) => (s.clone(), t.span.clone()),
            _ => unreachable!(),
        })
}

fn is_keyword(s: &str) -> bool {
    matches!(
        s,
        "fn" | "let" | "mut" | "if" | "elif" | "else" | "for" | "while" | "in"
            | "return" | "i64" | "f64" | "bool" | "str" | "unit" | "true"
            | "false" | "println"
    )
}

fn lparen<'a>() -> impl Parser<'a, &'a [Token], ()> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::LParen)
        .ignored()
}

fn rparen<'a>() -> impl Parser<'a, &'a [Token], ()> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::RParen)
        .ignored()
}

fn lbrace<'a>() -> impl Parser<'a, &'a [Token], ()> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::LBrace)
        .ignored()
}

fn rbrace<'a>() -> impl Parser<'a, &'a [Token], ()> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::RBrace)
        .ignored()
}

fn comma<'a>() -> impl Parser<'a, &'a [Token], ()> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::Comma)
        .ignored()
}

fn assign<'a>() -> impl Parser<'a, &'a [Token], ()> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::Assign)
        .ignored()
}

fn rarrow<'a>() -> impl Parser<'a, &'a [Token], ()> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::RArrow)
        .ignored()
}

fn colon<'a>() -> impl Parser<'a, &'a [Token], ()> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::Colon)
        .ignored()
}

fn plus<'a>() -> impl Parser<'a, &'a [Token], ()> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::Plus)
        .ignored()
}

fn minus<'a>() -> impl Parser<'a, &'a [Token], ()> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::Minus)
        .ignored()
}

fn star<'a>() -> impl Parser<'a, &'a [Token], ()> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::Star)
        .ignored()
}

fn slash<'a>() -> impl Parser<'a, &'a [Token], ()> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::Slash)
        .ignored()
}

fn percent<'a>() -> impl Parser<'a, &'a [Token], ()> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::Percent)
        .ignored()
}

fn eqeq<'a>() -> impl Parser<'a, &'a [Token], ()> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::EqEq)
        .ignored()
}

fn not_eq<'a>() -> impl Parser<'a, &'a [Token], ()> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::NotEq)
        .ignored()
}

fn lt<'a>() -> impl Parser<'a, &'a [Token], ()> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::Lt)
        .ignored()
}

fn gt<'a>() -> impl Parser<'a, &'a [Token], ()> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::Gt)
        .ignored()
}

fn lte<'a>() -> impl Parser<'a, &'a [Token], ()> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::LtEq)
        .ignored()
}

fn gte<'a>() -> impl Parser<'a, &'a [Token], ()> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::GtEq)
        .ignored()
}

fn and_op<'a>() -> impl Parser<'a, &'a [Token], BinOp> + Clone + 'a {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::And)
        .to(BinOp::And)
        .boxed()
}

fn or_op<'a>() -> impl Parser<'a, &'a [Token], BinOp> + Clone + 'a {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::Or)
        .to(BinOp::Or)
        .boxed()
}

fn bang<'a>() -> impl Parser<'a, &'a [Token], ()> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::Bang)
        .ignored()
}

fn plus_assign<'a>() -> impl Parser<'a, &'a [Token], ()> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::PlusAssign)
        .ignored()
}

fn minus_assign<'a>() -> impl Parser<'a, &'a [Token], ()> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::MinusAssign)
        .ignored()
}

fn star_assign<'a>() -> impl Parser<'a, &'a [Token], ()> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::StarAssign)
        .ignored()
}

fn slash_assign<'a>() -> impl Parser<'a, &'a [Token], ()> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::SlashAssign)
        .ignored()
}

#[allow(dead_code)]
fn range_op<'a>() -> impl Parser<'a, &'a [Token], ()> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::DotDot)
        .ignored()
}

fn dot_dot<'a>() -> impl Parser<'a, &'a [Token], ()> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::DotDot)
        .ignored()
}

fn dot_dot_dot<'a>() -> impl Parser<'a, &'a [Token], ()> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::DotDotEllipsis)
        .ignored()
}

/// Helper to extract the span from an Expr for use in Stmt::Expr
fn stmt_span(e: &Expr) -> Range<usize> {
    match e {
        Expr::Integer(_, s) | Expr::Float(_, s) | Expr::StrLit(_, s) |
        Expr::Bool(_, s) | Expr::Unit(s) | Expr::Var(_, s) => s.clone(),
        Expr::Assign { span, .. } | Expr::AugAssign { span, .. } |
        Expr::Binary { span, .. } | Expr::Unary { span, .. } |
        Expr::Call { span, .. } | Expr::If { span, .. } |
        Expr::For { span, .. } | Expr::While { span, .. } |
        Expr::Range { span, .. } => span.clone(),
    }
}
