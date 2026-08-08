use chumsky::extra::Default as ExtraDefault;
use chumsky::prelude::*;
use std::ops::Range;
use stnx::ast::{BinOp, Expr};
use stnx::lexer::{Lexer, Token, TokenKind};

fn expr<'a>() -> impl Parser<'a, &'a [Token], Expr> {
    comparison().boxed()
}

fn comparison<'a>() -> impl Parser<'a, &'a [Token], Expr> {
    additive().boxed()
}

fn additive<'a>() -> impl Parser<'a, &'a [Token], Expr> {
    multiplicative()
        .foldl(add_op().repeated(), |lhs, (op, rhs)| Expr::Binary {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
            span: Range::default(),
        })
        .boxed()
}

fn multiplicative<'a>() -> impl Parser<'a, &'a [Token], Expr> {
    unary().boxed()
}

fn unary<'a>() -> impl Parser<'a, &'a [Token], Expr> {
    primary().boxed()
}

fn primary<'a>() -> impl Parser<'a, &'a [Token], Expr> {
    let integer_lit = any::<&[Token], ExtraDefault>()
        .filter(|t: &Token| matches!(&t.kind, TokenKind::Integer(_)))
        .map(|t| match &t.kind {
            TokenKind::Integer(n) => Expr::Integer(*n, t.span.clone()),
            _ => unreachable!(),
        });

    integer_lit.boxed()
}

fn add_op<'a>() -> impl Parser<'a, &'a [Token], (BinOp, Expr)> {
    any::<&[Token], ExtraDefault>()
        .filter(|t: &Token| t.kind == TokenKind::Plus)
        .map(|_| BinOp::Add)
        .then(expr())
        .map(|(op, rhs)| (op, rhs))
        .boxed()
}

fn main() {
    let src = "42";
    let lexer = Lexer::new(src);
    let tokens: Vec<_> = lexer.collect::<Result<Vec<_>, _>>().unwrap();
    println!("Tokens: {:?}", tokens);

    let result = expr().parse(&tokens);
    println!("Result: {:?}", result);
}
