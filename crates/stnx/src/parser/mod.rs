pub use crate::ast::*;

use crate::error::{CompilerError, CompilerResult, ParseError};
use crate::lexer::{Token, TokenKind};
use chumsky::error::Simple;
use chumsky::prelude::*;
use chumsky::recursive::Direct;
use chumsky::span::SimpleSpan;
use std::ops::Range;

/// Error type alias for parsers that track spans via `SimpleSpan<usize>`.
pub type ParserExtra<'a> = extra::Err<Simple<'a, Token, SimpleSpan<usize>>>;

/// Convert a `Simple` error to a human-readable message.
fn format_simple_error(err: &Simple<'_, Token, SimpleSpan<usize>>) -> String {
    let found = err
        .found()
        .map(|t| format!("{:?}", t.kind))
        .unwrap_or_else(|| "end of input".to_string());
    format!("unexpected token: {}", found)
}

/// Convert a token-index span to a byte-offset span using the token array.
fn token_span_to_byte_span(tokens: &[Token], token_span: &Range<usize>) -> Range<usize> {
    let start = tokens
        .get(token_span.start)
        .map(|t| t.span.start)
        .unwrap_or_default();
    let end = token_span
        .end
        .checked_sub(1)
        .and_then(|idx| tokens.get(idx))
        .map(|t| t.span.end)
        .unwrap_or(start);
    start..end
}

pub fn parse(src: &str, tokens: Vec<Token>) -> CompilerResult<Program> {
    let parser = program();
    let result = parser.parse(&tokens);

    let (output, errors) = result.into_output_errors();

    if errors.is_empty() {
        Ok(output.unwrap())
    } else {
        let parse_errors: Vec<ParseError> = errors
            .iter()
            .map(|e| {
                let span_range: Range<usize> = e.span().into_range();
                let byte_span = token_span_to_byte_span(&tokens, &span_range);
                ParseError {
                    src: src.to_string(),
                    span: (
                        byte_span.start,
                        byte_span.end.saturating_sub(byte_span.start),
                    )
                        .into(),
                    message: format_simple_error(e),
                }
            })
            .collect();

        // Always return the first parse error with the richest span info.
        // If there are additional errors, include them in the message.
        let first = parse_errors.into_iter().next().unwrap();
        if errors.len() > 1 {
            let extra = errors.len() - 1;
            Err(CompilerError::Parse(ParseError {
                src: first.src,
                span: first.span,
                message: format!("{} (plus {} more error(s))", first.message, extra),
            }))
        } else {
            Err(CompilerError::Parse(first))
        }
    }
}

fn program<'a>() -> impl Parser<'a, &'a [Token], Program, ParserExtra<'a>> {
    item()
        .repeated()
        .collect::<Vec<_>>()
        .map(Program::from_items)
}

fn func<'a>() -> impl Parser<'a, &'a [Token], Function, ParserExtra<'a>> {
    kw("fn")
        .ignore_then(t_ident())
        .then(generic_params())
        .then(params())
        .then(ret_type())
        .then(block(recursive_expr()))
        .map(
            |(((((name, name_span), generic_params), params), ret_type), body)| Function {
                name,
                generic_params,
                params,
                return_type: ret_type,
                body,
                span: name_span,
            },
        )
}

/// Parse an optional `<T1, T2, ...>` generic parameter list after a function
/// or type name. Returns an empty `Vec` when no angle brackets are present.
fn generic_params<'a>() -> impl Parser<'a, &'a [Token], Vec<String>, ParserExtra<'a>> {
    lt().ignore_then(
        t_ident()
            .map(|(name, _span)| name)
            .separated_by(comma())
            .collect::<Vec<_>>(),
    )
    .then_ignore(gt())
    .or_not()
    .map(|opt: Option<Vec<String>>| opt.unwrap_or_default())
}

/// Parse an optional `pub` visibility prefix.
/// Returns `Visibility::Public` if `pub` is present, `Visibility::Private` otherwise.
fn visibility<'a>() -> impl Parser<'a, &'a [Token], Visibility, ParserExtra<'a>> {
    kw("pub")
        .to(Visibility::Public)
        .or_not()
        .map(|opt| opt.unwrap_or(Visibility::Private))
}

/// Parse a top-level item: `fn`, `struct`, `enum`, `mod`, or `use`,
/// optionally preceded by `pub`.
///
/// All top-level constructs are items. `mod` and `use` use no semicolons
/// (Saturnite's no-semicolon style — items terminate at the next newline/item).
fn item<'a>() -> impl Parser<'a, &'a [Token], Item, ParserExtra<'a>> {
    visibility()
        .then(
            // function: fn name(params) -> ret { body }
            func()
                .map(|f| {
                    let span = f.span.clone();
                    let name = f.name.clone();
                    (name, ItemKind::Function(f), span)
                })
                // struct definition at top level: struct Name { fields }
                .or(struct_item().map(|(name, generic_params, fields, span)| {
                    (
                        name.clone(),
                        ItemKind::StructDef {
                            name,
                            generic_params,
                            fields,
                            span: span.clone(),
                        },
                        span,
                    )
                }))
                // enum definition at top level: enum Name { variants }
                .or(enum_item().map(|(name, generic_params, variants, span)| {
                    (
                        name.clone(),
                        ItemKind::EnumDef {
                            name,
                            generic_params,
                            variants,
                            span: span.clone(),
                        },
                        span,
                    )
                }))
                // mod declaration: mod <ident>
                .or(mod_decl().map(|(name, span)| (name.clone(), ItemKind::ModDecl, span)))
                // use declaration: use <path> [as <alias>]
                .or(use_decl().map(|(name, kind, span)| (name, kind, span))),
        )
        .map(|(vis, (name, kind, span))| Item {
            name,
            visibility: vis,
            kind,
            span,
        })
}

/// Parse a top-level struct definition: `struct Name { field1: type1, field2: type2 }`
#[allow(clippy::type_complexity)]
fn struct_item<'a>() -> impl Parser<
    'a,
    &'a [Token],
    (String, Vec<String>, Vec<(String, Type)>, Range<usize>),
    ParserExtra<'a>,
> {
    kw_span("struct")
        .ignore_then(t_ident())
        .then(generic_params())
        .then(
            lbrace()
                .ignore_then(
                    t_ident()
                        .map(|(name, _)| name)
                        .then(type_ann())
                        .separated_by(comma())
                        .collect::<Vec<_>>(),
                )
                .then_ignore(rbrace()),
        )
        .map(|(((name, name_span), generic_params), fields)| {
            (name, generic_params, fields, name_span)
        })
}

/// Parse a top-level enum definition: `enum Name { Variant1, Variant2 }`
fn enum_item<'a>(
) -> impl Parser<'a, &'a [Token], (String, Vec<String>, Vec<String>, Range<usize>), ParserExtra<'a>>
{
    kw_span("enum")
        .ignore_then(t_ident())
        .then(generic_params())
        .then(
            lbrace()
                .ignore_then(
                    t_ident()
                        .map(|(name, _)| name)
                        .separated_by(comma())
                        .collect::<Vec<_>>(),
                )
                .then_ignore(rbrace()),
        )
        .map(|(((name, name_span), generic_params), variants)| {
            (name, generic_params, variants, name_span)
        })
}

/// Parse a `mod <ident>` declaration (no semicolon).
/// Returns the module name and its byte span.
fn mod_decl<'a>() -> impl Parser<'a, &'a [Token], (String, Range<usize>), ParserExtra<'a>> {
    kw_span("mod")
        .ignore_then(t_ident())
        .map(|(name, span)| (name, span))
}

/// Parse a `use foo::bar::baz` declaration (no semicolon, with optional `as alias`).
/// Returns (name, ItemKind, span).
fn use_decl<'a>() -> impl Parser<'a, &'a [Token], (String, ItemKind, Range<usize>), ParserExtra<'a>>
{
    kw_span("use")
        .then(path_with_span())
        .then(kw("as").ignore_then(t_ident().map(|(n, _)| n)).or_not())
        .map(|((use_span, (parts, last_span)), alias)| {
            let name = parts.last().cloned().unwrap_or_default();
            let full_span = use_span.start..last_span.end;
            (name, ItemKind::UseDecl { path: parts, alias }, full_span)
        })
}

/// Parse a path segment — accepts an identifier OR a keyword token (e.g. `println`
/// used in `use io::println`).  Returns (name_string, span).
fn path_segment<'a>() -> impl Parser<'a, &'a [Token], (String, Range<usize>), ParserExtra<'a>> {
    any::<&[Token], _>()
        .filter(|t: &Token| {
            matches!(&t.kind, TokenKind::Ident(s) if !is_keyword(s))
                || matches!(
                    &t.kind,
                    TokenKind::Println
                        | TokenKind::True
                        | TokenKind::False
                        | TokenKind::I64
                        | TokenKind::F64
                        | TokenKind::Bool
                        | TokenKind::Str
                        | TokenKind::Unit
                )
        })
        .map(|t| {
            let name = match &t.kind {
                TokenKind::Ident(s) => s.clone(),
                TokenKind::Println => "println".to_string(),
                TokenKind::True => "true".to_string(),
                TokenKind::False => "false".to_string(),
                TokenKind::I64 => "i64".to_string(),
                TokenKind::F64 => "f64".to_string(),
                TokenKind::Bool => "bool".to_string(),
                TokenKind::Str => "str".to_string(),
                TokenKind::Unit => "unit".to_string(),
                _ => unreachable!(),
            };
            (name, t.span.clone())
        })
}

/// Parse a path with span: `ident (:: ident)*` returning (Vec<String>, span).
fn path_with_span<'a>() -> impl Parser<'a, &'a [Token], (Vec<String>, Range<usize>), ParserExtra<'a>>
{
    path_segment()
        .map(|(n, s)| (n, s))
        .then(
            double_colon()
                .ignore_then(path_segment().map(|(n, s)| (n, s)))
                .repeated()
                .collect::<Vec<_>>(),
        )
        .map(|((first, first_span), rest)| {
            let last_end = rest.last().map(|(_, s)| s.end).unwrap_or(first_span.end);
            let mut parts = vec![first];
            for (n, _) in rest {
                parts.push(n);
            }
            (parts, first_span.start..last_end)
        })
}

fn params<'a>() -> impl Parser<'a, &'a [Token], Vec<(String, Type)>, ParserExtra<'a>> {
    let param = t_ident().map(|(name, _span)| name).then(type_ann());

    lparen()
        .ignore_then(param.separated_by(comma()).collect::<Vec<_>>())
        .then_ignore(rparen())
        .or(rparen().to(Vec::new()))
}

fn type_ann<'a>() -> impl Parser<'a, &'a [Token], Type, ParserExtra<'a>> {
    colon().ignore_then(
        kw("i64")
            .to(Type::I64)
            .or(kw("f64").to(Type::F64))
            .or(kw("bool").to(Type::Bool))
            .or(kw("str").to(Type::Str))
            .or(kw("unit").to(Type::Unit))
            .or(t_ident().map(|(name, _)| Type::Struct(name))),
    )
}

fn ret_type<'a>() -> impl Parser<'a, &'a [Token], Type, ParserExtra<'a>> {
    rarrow()
        .ignore_then(
            kw("i64")
                .to(Type::I64)
                .or(kw("f64").to(Type::F64))
                .or(kw("bool").to(Type::Bool))
                .or(kw("str").to(Type::Str))
                .or(kw("unit").to(Type::Unit))
                .or(t_ident().map(|(name, _)| Type::Struct(name))),
        )
        .or_not()
        .map(|opt: Option<Type>| opt.unwrap_or(Type::Unit))
}

fn block<'a>(
    expr: Recursive<Direct<'a, 'a, &'a [Token], Expr, ParserExtra<'a>>>,
) -> impl Parser<'a, &'a [Token], Vec<Stmt>, ParserExtra<'a>> + 'a {
    lbrace()
        .ignore_then(stmt(expr.clone()).repeated().collect::<Vec<_>>())
        .then_ignore(rbrace())
}

fn stmt<'a>(
    expr: Recursive<Direct<'a, 'a, &'a [Token], Expr, ParserExtra<'a>>>,
) -> Boxed<'a, 'a, &'a [Token], Stmt, ParserExtra<'a>> {
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

    let return_stmt = kw_span("return")
        .then(expr.clone().or_not())
        .map(|(ret_span, e)| {
            let span = e
                .as_ref()
                .map(|e| {
                    let es = stmt_span(e);
                    ret_span.start..es.end
                })
                .unwrap_or(ret_span);
            Stmt::Return(e, span)
        });

    let println_stmt = kw_span("println")
        .ignore_then(lparen())
        .ignore_then(expr.clone())
        .then_ignore(rparen())
        .map(|e| {
            let span = stmt_span(&e);
            Stmt::Println(e, span)
        });

    // Struct definition: `struct Name { field1: type1, field2: type2 }`
    let struct_def = kw("struct")
        .ignore_then(t_ident())
        .then(generic_params())
        .then(
            lbrace()
                .ignore_then(
                    t_ident()
                        .map(|(name, _)| name)
                        .then(type_ann())
                        .separated_by(comma())
                        .collect::<Vec<_>>(),
                )
                .then_ignore(rbrace()),
        )
        .map(
            |(((name, name_span), generic_params), fields)| Stmt::StructDef {
                name,
                generic_params,
                fields,
                span: name_span,
            },
        );

    // Enum definition: `enum Name { Variant1, Variant2 }`
    let enum_def = kw("enum")
        .ignore_then(t_ident())
        .then(generic_params())
        .then(
            lbrace()
                .ignore_then(
                    t_ident()
                        .map(|(name, _)| name)
                        .separated_by(comma())
                        .collect::<Vec<_>>(),
                )
                .then_ignore(rbrace()),
        )
        .map(
            |(((name, name_span), generic_params), variants)| Stmt::EnumDef {
                name,
                generic_params,
                variants,
                span: name_span,
            },
        );

    let expr_stmt = expr.map(|e| {
        let span = stmt_span(&e);
        Stmt::Expr(e, span)
    });

    let_stmt
        .or(return_stmt)
        .or(println_stmt)
        .or(struct_def)
        .or(enum_def)
        .or(expr_stmt)
        .boxed()
}

fn recursive_expr<'a>() -> Recursive<Direct<'a, 'a, &'a [Token], Expr, ParserExtra<'a>>> {
    recursive(|expr| {
        // Precedence chain (left-associative, left-recursive):
        // Logical -> Comparison -> Additive -> Multiplicative -> Unary -> Primary
        //
        // To avoid stack overflow from deep recursion, we:
        // 1. Use .memoized() on the recursive expr to cache results and detect cycles
        // 2. Structure the chain so primary is tried once, not re-entered

        // Primary: literals, identifiers, calls, parenthesized expressions
        let primary: Boxed<'a, 'a, &'a [Token], Expr, ParserExtra<'a>> = any::<&[Token], _>()
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
            .or(kw_span("true")
                .map(|s| Expr::Bool(true, s))
                .or(kw_span("false").map(|s| Expr::Bool(false, s))))
            .or(lbrace_span().then_ignore(rbrace()).map(Expr::Unit))
            .or(t_ident()
                // Optional turbofish for struct literals: `Box::<i64> { ... }`.
                .then(
                    double_colon()
                        .ignore_then(lt())
                        .ignore_then(
                            kw("i64")
                                .to(Type::I64)
                                .or(kw("f64").to(Type::F64))
                                .or(kw("bool").to(Type::Bool))
                                .or(kw("str").to(Type::Str))
                                .or(kw("unit").to(Type::Unit))
                                .or(t_ident().map(|(name, _)| Type::Struct(name)))
                                .separated_by(comma())
                                .collect::<Vec<_>>(),
                        )
                        .then_ignore(gt())
                        .or_not(),
                )
                .then(
                    lbrace()
                        .ignore_then(
                            t_ident()
                                .map(|(name, _)| name)
                                .then(colon().ignore_then(expr.clone()))
                                .separated_by(comma())
                                .collect::<Vec<_>>(),
                        )
                        .then_ignore(rbrace()),
                )
                .map(
                    |(((name, name_span), type_args), fields)| Expr::StructLiteral {
                        name,
                        fields,
                        type_args: type_args.unwrap_or_default(),
                        span: name_span,
                    },
                ))
            .or(t_ident().then(double_colon().ignore_then(t_ident())).map(
                |((name, name_span), (variant, _))| Expr::EnumConstructor {
                    name,
                    variant,
                    span: name_span,
                },
            ))
            // Call or variable: `f` or `f(args)` or `f::<T>(args)`.
            //
            // The turbofish `::<T1, T2>` is consumed before the optional
            // `(args)` group. We parse it as an optional segment after the
            // identifier; if absent, `type_args` is an empty Vec.
            .or(t_ident()
                .then(
                    double_colon()
                        .ignore_then(lt())
                        .ignore_then(
                            // A type-arg list: one or more `Type`s separated by commas.
                            // We reuse `type_ann`-shaped identifiers via the same
                            // inner Type productions used elsewhere.
                            kw("i64")
                                .to(Type::I64)
                                .or(kw("f64").to(Type::F64))
                                .or(kw("bool").to(Type::Bool))
                                .or(kw("str").to(Type::Str))
                                .or(kw("unit").to(Type::Unit))
                                .or(t_ident().map(|(name, _)| Type::Struct(name)))
                                .separated_by(comma())
                                .collect::<Vec<_>>(),
                        )
                        .then_ignore(gt())
                        .or_not(),
                )
                .then(
                    lparen()
                        .ignore_then(expr.clone().separated_by(comma()).collect::<Vec<_>>())
                        .then_ignore(rparen())
                        .or_not(),
                )
                .map(|(((name, name_span), type_args), args)| {
                    if let Some(args) = args {
                        Expr::Call {
                            func: name,
                            args,
                            type_args: type_args.unwrap_or_default(),
                            span: name_span,
                        }
                    } else if type_args.is_some() {
                        // `f::<T>` without `(args)` — surface as a parse error
                        // (a turbofish without a call is malformed).
                        Expr::Var(name, name_span)
                    } else {
                        Expr::Var(name, name_span)
                    }
                }))
            .boxed();

        // Unary: -expr, !expr
        let unary = minus()
            .ignore_then(expr.clone())
            .map(|e| {
                let s = stmt_span(&e);
                Expr::Unary {
                    op: UnOp::Neg,
                    expr: Box::new(e),
                    span: s,
                }
            })
            .or(bang().ignore_then(expr.clone()).map(|e| {
                let s = stmt_span(&e);
                Expr::Unary {
                    op: UnOp::Not,
                    expr: Box::new(e),
                    span: s,
                }
            }))
            .or(primary.clone())
            .boxed();

        // Multiplicative: * / %
        let mul_chain = unary
            .clone()
            .foldl(mul_op().then(unary.clone()).repeated(), |lhs, (op, rhs)| {
                let span = combine_spans(&lhs, &rhs);
                Expr::Binary {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                    span,
                }
            })
            .boxed();

        // Additive: + -
        let add_chain = mul_chain
            .clone()
            .foldl(
                add_op().then(mul_chain.clone()).repeated(),
                |lhs, (op, rhs)| {
                    let span = combine_spans(&lhs, &rhs);
                    Expr::Binary {
                        op,
                        lhs: Box::new(lhs),
                        rhs: Box::new(rhs),
                        span,
                    }
                },
            )
            .boxed();

        // Comparison: == != < > <= >=
        let cmp_chain = add_chain
            .clone()
            .foldl(
                cmp_op().then(add_chain.clone()).repeated(),
                |lhs, (op, rhs)| {
                    let span = combine_spans(&lhs, &rhs);
                    Expr::Binary {
                        op,
                        lhs: Box::new(lhs),
                        rhs: Box::new(rhs),
                        span,
                    }
                },
            )
            .boxed();

        // Logical AND: &&
        let and_chain = cmp_chain
            .clone()
            .foldl(
                and_op().then(cmp_chain.clone()).repeated(),
                |lhs, (op, rhs)| {
                    let span = combine_spans(&lhs, &rhs);
                    Expr::Binary {
                        op,
                        lhs: Box::new(lhs),
                        rhs: Box::new(rhs),
                        span,
                    }
                },
            )
            .boxed();

        // Logical OR: ||
        let or_chain = and_chain
            .clone()
            .foldl(
                or_op().then(and_chain.clone()).repeated(),
                |lhs, (op, rhs)| {
                    let span = combine_spans(&lhs, &rhs);
                    Expr::Binary {
                        op,
                        lhs: Box::new(lhs),
                        rhs: Box::new(rhs),
                        span,
                    }
                },
            )
            .boxed();

        // Range: start..end (exclusive), start...end (inclusive)
        // Lower precedence than logical operators
        let range_expr = or_chain
            .clone()
            .then(
                dot_dot()
                    .to(false)
                    .or(dot_dot_dot().to(true))
                    .then(or_chain.clone())
                    .repeated()
                    .collect::<Vec<_>>(),
            )
            .map(|(first, rest)| {
                if rest.is_empty() {
                    first
                } else {
                    // Chain ranges: a..b..c => Range(a..b, c, false) etc.
                    // For simplicity, only handle single range
                    let (is_inclusive, last) = rest.into_iter().next().unwrap();
                    let span = combine_spans(&first, &last);
                    Expr::Range {
                        start: Box::new(first),
                        end: Box::new(last),
                        is_inclusive,
                        span,
                    }
                }
            })
            .boxed();

        // Control flow expressions - these start with keywords, so they don't
        // cause recursion when the first token doesn't match
        let if_span = kw_span("if");
        let if_expr = if_span
            .clone()
            .then(expr.clone())
            .then(block_from_expr(expr.clone()))
            .then(
                kw("elif")
                    .ignore_then(expr.clone())
                    .then(block_from_expr(expr.clone()))
                    .repeated()
                    .collect::<Vec<_>>(),
            )
            .then(
                kw("else")
                    .ignore_then(block_from_expr(expr.clone()))
                    .or_not(),
            )
            .map(|((((if_start, cond), then_b), elifs), else_b)| {
                let mut end = stmt_span(&cond);
                let then_last = then_b.last();
                if let Some(t) = then_last {
                    let ts = stmt_span_expr(t);
                    end.end = end.end.max(ts.end);
                }
                Expr::If {
                    condition: Box::new(cond),
                    then_branch: then_b,
                    elif_branches: elifs,
                    else_branch: else_b,
                    span: if_start.start..end.end,
                }
            })
            .boxed();

        let for_expr = kw_span("for")
            .then(t_ident())
            .then_ignore(kw("in"))
            .then(expr.clone())
            .then(block_from_expr(expr.clone()))
            .map(|(((for_span, (var, _var_span)), iter), body)| {
                let mut end = stmt_span(&iter);
                if let Some(t) = body.last() {
                    let ts = stmt_span_expr(t);
                    end.end = end.end.max(ts.end);
                }
                Expr::For {
                    var,
                    iter: Box::new(iter),
                    body,
                    span: for_span.start..end.end,
                }
            })
            .boxed();

        let while_expr = kw_span("while")
            .then(expr.clone())
            .then(block_from_expr(expr.clone()))
            .map(|((start_span, cond), body)| {
                let mut end = stmt_span(&cond);
                if let Some(t) = body.last() {
                    let ts = stmt_span_expr(t);
                    end.end = end.end.max(ts.end);
                }
                Expr::While {
                    condition: Box::new(cond),
                    body,
                    span: start_span.start..end.end,
                }
            })
            .boxed();

        // Assignment: ident = expr (and augmented assignment: ident += expr)
        let assign_expr = t_ident()
            .then(
                plus_assign()
                    .to(AugOp::Add)
                    .or(minus_assign().to(AugOp::Sub))
                    .or(star_assign().to(AugOp::Mul))
                    .or(slash_assign().to(AugOp::Div)),
            )
            .then(expr.clone())
            .map(|(((target, target_span), op), value)| {
                let span = combine_spans_with(target_span, &value);
                Expr::AugAssign {
                    target,
                    op,
                    value: Box::new(value),
                    span,
                }
            })
            .or(t_ident().then(assign()).then(expr.clone()).map(
                |(((target, target_span), _), value)| {
                    let span = combine_spans_with(target_span, &value);
                    Expr::Assign {
                        target,
                        value: Box::new(value),
                        span,
                    }
                },
            ))
            .boxed();

        // Control flow expressions are tried first (they start with keywords
        // that don't match primary), then assignment (starts with ident),
        // then the basic expression chain (logical OR is the outermost binary layer)
        // Postfix: field access (a.b, a.b.c, func().field, etc.)
        let base_expr = if_expr
            .or(for_expr.clone())
            .or(while_expr.clone())
            .or(assign_expr.clone())
            .or(range_expr.clone())
            .memoized()
            .boxed();

        base_expr
            .clone()
            .then(dot().ignore_then(t_ident()).repeated().collect::<Vec<_>>())
            .map(|(base, accesses)| {
                let mut expr = base;
                for (field_name, field_span) in accesses {
                    let expr_span = stmt_span(&expr);
                    expr = Expr::FieldAccess {
                        expr: Box::new(expr),
                        field: field_name,
                        span: expr_span.start..field_span.end,
                    };
                }
                expr
            })
            .memoized()
            .boxed()
    })
}

fn block_from_expr<'a>(
    expr: Recursive<Direct<'a, 'a, &'a [Token], Expr, ParserExtra<'a>>>,
) -> impl Parser<'a, &'a [Token], Vec<Stmt>, ParserExtra<'a>> + Clone + 'a {
    lbrace()
        .ignore_then(stmt(expr).repeated().collect::<Vec<_>>())
        .then_ignore(rbrace())
        .boxed()
}

// Debug function for testing
pub fn parse_debug(tokens: &[Token]) -> Result<(), Vec<String>> {
    let parser = recursive_expr();
    let result = parser.parse(tokens);
    let (_output, errors) = result.into_output_errors();
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
    let (_output, errors) = result.into_output_errors();
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
    let (_output, errors) = result.into_output_errors();
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
    let (_output, errors) = result.into_output_errors();
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
    let (_output, errors) = result.into_output_errors();
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
    let (_output, errors) = result.into_output_errors();
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
    let (_output, errors) = result.into_output_errors();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.iter().map(|e| format!("{:?}", e)).collect())
    }
}

fn mul_op<'a>() -> impl Parser<'a, &'a [Token], BinOp, ParserExtra<'a>> + Clone + 'a {
    star()
        .to(BinOp::Mul)
        .or(slash().to(BinOp::Div))
        .or(percent().to(BinOp::Mod))
        .boxed()
}

fn add_op<'a>() -> impl Parser<'a, &'a [Token], BinOp, ParserExtra<'a>> + Clone + 'a {
    plus().to(BinOp::Add).or(minus().to(BinOp::Sub)).boxed()
}

fn cmp_op<'a>() -> impl Parser<'a, &'a [Token], BinOp, ParserExtra<'a>> + Clone + 'a {
    eqeq()
        .to(BinOp::Eq)
        .or(not_eq().to(BinOp::Ne))
        .or(lt().to(BinOp::Lt))
        .or(gt().to(BinOp::Gt))
        .or(lte().to(BinOp::Le))
        .or(gte().to(BinOp::Ge))
        .boxed()
}

// --- Token helpers ---

fn kw<'a>(k: &'a str) -> impl Parser<'a, &'a [Token], (), ParserExtra<'a>> {
    kw_span(k).ignored()
}

/// Match a keyword and return its byte span.
fn kw_span<'a>(k: &'a str) -> Boxed<'a, 'a, &'a [Token], Range<usize>, ParserExtra<'a>> {
    any::<&[Token], _>()
        .filter(move |t: &Token| {
            matches!(
                (&t.kind, k),
                (TokenKind::Fn, "fn")
                    | (TokenKind::Let, "let")
                    | (TokenKind::Mut, "mut")
                    | (TokenKind::If, "if")
                    | (TokenKind::Elif, "elif")
                    | (TokenKind::Else, "else")
                    | (TokenKind::For, "for")
                    | (TokenKind::While, "while")
                    | (TokenKind::In, "in")
                    | (TokenKind::Return, "return")
                    | (TokenKind::I64, "i64")
                    | (TokenKind::F64, "f64")
                    | (TokenKind::Bool, "bool")
                    | (TokenKind::Str, "str")
                    | (TokenKind::Unit, "unit")
                    | (TokenKind::True, "true")
                    | (TokenKind::False, "false")
                    | (TokenKind::Println, "println")
                    | (TokenKind::Struct, "struct")
                    | (TokenKind::Enum, "enum")
                    | (TokenKind::Mod, "mod")
                    | (TokenKind::Use, "use")
                    | (TokenKind::Pub, "pub")
                    | (TokenKind::As, "as")
            )
        })
        .map(|t| t.span.clone())
        .boxed()
}

fn t_ident<'a>() -> impl Parser<'a, &'a [Token], (String, Range<usize>), ParserExtra<'a>> {
    any::<&[Token], _>()
        .filter(|t: &Token| matches!(&t.kind, TokenKind::Ident(s) if !is_keyword(s)))
        .map(|t| match &t.kind {
            TokenKind::Ident(s) => (s.clone(), t.span.clone()),
            _ => unreachable!(),
        })
}

fn is_keyword(s: &str) -> bool {
    matches!(
        s,
        "fn" | "let"
            | "mut"
            | "if"
            | "elif"
            | "else"
            | "for"
            | "while"
            | "in"
            | "return"
            | "i64"
            | "f64"
            | "bool"
            | "str"
            | "unit"
            | "true"
            | "false"
            | "println"
            | "struct"
            | "enum"
            | "mod"
            | "use"
            | "pub"
            | "as"
    )
}

fn lparen<'a>() -> impl Parser<'a, &'a [Token], (), ParserExtra<'a>> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::LParen)
        .ignored()
}

fn rparen<'a>() -> impl Parser<'a, &'a [Token], (), ParserExtra<'a>> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::RParen)
        .ignored()
}

fn lbrace<'a>() -> impl Parser<'a, &'a [Token], (), ParserExtra<'a>> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::LBrace)
        .ignored()
}

fn lbrace_span<'a>() -> impl Parser<'a, &'a [Token], Range<usize>, ParserExtra<'a>> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::LBrace)
        .map(|t| t.span.clone())
}

fn rbrace<'a>() -> impl Parser<'a, &'a [Token], (), ParserExtra<'a>> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::RBrace)
        .ignored()
}

fn comma<'a>() -> impl Parser<'a, &'a [Token], (), ParserExtra<'a>> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::Comma)
        .ignored()
}

fn assign<'a>() -> impl Parser<'a, &'a [Token], (), ParserExtra<'a>> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::Assign)
        .ignored()
}

fn rarrow<'a>() -> impl Parser<'a, &'a [Token], (), ParserExtra<'a>> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::RArrow)
        .ignored()
}

fn colon<'a>() -> impl Parser<'a, &'a [Token], (), ParserExtra<'a>> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::Colon)
        .ignored()
}

fn dot<'a>() -> impl Parser<'a, &'a [Token], (), ParserExtra<'a>> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::Dot)
        .ignored()
}

fn double_colon<'a>() -> impl Parser<'a, &'a [Token], (), ParserExtra<'a>> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::DoubleColon)
        .ignored()
}

fn plus<'a>() -> impl Parser<'a, &'a [Token], (), ParserExtra<'a>> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::Plus)
        .ignored()
}

fn minus<'a>() -> impl Parser<'a, &'a [Token], (), ParserExtra<'a>> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::Minus)
        .ignored()
}

fn star<'a>() -> impl Parser<'a, &'a [Token], (), ParserExtra<'a>> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::Star)
        .ignored()
}

fn slash<'a>() -> impl Parser<'a, &'a [Token], (), ParserExtra<'a>> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::Slash)
        .ignored()
}

fn percent<'a>() -> impl Parser<'a, &'a [Token], (), ParserExtra<'a>> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::Percent)
        .ignored()
}

fn eqeq<'a>() -> impl Parser<'a, &'a [Token], (), ParserExtra<'a>> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::EqEq)
        .ignored()
}

fn not_eq<'a>() -> impl Parser<'a, &'a [Token], (), ParserExtra<'a>> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::NotEq)
        .ignored()
}

fn lt<'a>() -> impl Parser<'a, &'a [Token], (), ParserExtra<'a>> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::Lt)
        .ignored()
}

fn gt<'a>() -> impl Parser<'a, &'a [Token], (), ParserExtra<'a>> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::Gt)
        .ignored()
}

fn lte<'a>() -> impl Parser<'a, &'a [Token], (), ParserExtra<'a>> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::LtEq)
        .ignored()
}

fn gte<'a>() -> impl Parser<'a, &'a [Token], (), ParserExtra<'a>> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::GtEq)
        .ignored()
}

fn and_op<'a>() -> impl Parser<'a, &'a [Token], BinOp, ParserExtra<'a>> + Clone + 'a {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::And)
        .to(BinOp::And)
        .boxed()
}

fn or_op<'a>() -> impl Parser<'a, &'a [Token], BinOp, ParserExtra<'a>> + Clone + 'a {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::Or)
        .to(BinOp::Or)
        .boxed()
}

fn bang<'a>() -> impl Parser<'a, &'a [Token], (), ParserExtra<'a>> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::Bang)
        .ignored()
}

fn plus_assign<'a>() -> impl Parser<'a, &'a [Token], (), ParserExtra<'a>> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::PlusAssign)
        .ignored()
}

fn minus_assign<'a>() -> impl Parser<'a, &'a [Token], (), ParserExtra<'a>> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::MinusAssign)
        .ignored()
}

fn star_assign<'a>() -> impl Parser<'a, &'a [Token], (), ParserExtra<'a>> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::StarAssign)
        .ignored()
}

fn slash_assign<'a>() -> impl Parser<'a, &'a [Token], (), ParserExtra<'a>> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::SlashAssign)
        .ignored()
}

#[allow(dead_code)]
fn range_op<'a>() -> impl Parser<'a, &'a [Token], (), ParserExtra<'a>> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::DotDot)
        .ignored()
}

fn dot_dot<'a>() -> impl Parser<'a, &'a [Token], (), ParserExtra<'a>> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::DotDot)
        .ignored()
}

fn dot_dot_dot<'a>() -> impl Parser<'a, &'a [Token], (), ParserExtra<'a>> {
    any::<&[Token], _>()
        .filter(|t: &Token| t.kind == TokenKind::DotDotEllipsis)
        .ignored()
}

/// Helper to extract the span from an Expr for use in Stmt::Expr
fn stmt_span(e: &Expr) -> Range<usize> {
    match e {
        Expr::Integer(_, s)
        | Expr::Float(_, s)
        | Expr::StrLit(_, s)
        | Expr::Bool(_, s)
        | Expr::Unit(s)
        | Expr::Var(_, s) => s.clone(),
        Expr::Assign { span, .. }
        | Expr::AugAssign { span, .. }
        | Expr::Binary { span, .. }
        | Expr::Unary { span, .. }
        | Expr::Call { span, .. }
        | Expr::If { span, .. }
        | Expr::For { span, .. }
        | Expr::While { span, .. }
        | Expr::Range { span, .. }
        | Expr::StructLiteral { span, .. }
        | Expr::FieldAccess { span, .. }
        | Expr::EnumConstructor { span, .. } => span.clone(),
    }
}

/// Extract the span from a Stmt (used for computing parent spans).
fn stmt_span_expr(s: &Stmt) -> Range<usize> {
    match s {
        Stmt::Let { span, .. }
        | Stmt::Expr(_, span)
        | Stmt::Return(_, span)
        | Stmt::Println(_, span)
        | Stmt::StructDef { span, .. }
        | Stmt::EnumDef { span, .. } => span.clone(),
    }
}

/// Combine the spans of two expressions (lhs start to rhs end).
fn combine_spans(lhs: &Expr, rhs: &Expr) -> Range<usize> {
    let ls = stmt_span(lhs);
    let rs = stmt_span(rhs);
    ls.start.min(rs.start)..ls.end.max(rs.end)
}

/// Combine a known span with an expression's span.
fn combine_spans_with(span: Range<usize>, expr: &Expr) -> Range<usize> {
    let es = stmt_span(expr);
    span.start.min(es.start)..span.end.max(es.end)
}

// ---------------------------------------------------------------------------
// Phase 5A: Parser tests for mod / use / pub syntax
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

    /// Helper: lex + parse a source string, returning the resulting `Program`.
    fn parse_src(src: &str) -> Program {
        let tokens: Vec<Token> = Lexer::new(src)
            .collect::<Result<Vec<_>, _>>()
            .expect("lexing should succeed");
        parse(src, tokens).expect("parsing should succeed")
    }

    /// Helper: lex + parse, returning the first error (or panicking if none).
    fn parse_fail(src: &str) -> CompilerError {
        let tokens: Vec<Token> = Lexer::new(src)
            .collect::<Result<Vec<_>, _>>()
            .expect("lexing should succeed");
        parse(src, tokens)
            .map(|_| panic!("expected parse error for:\n{}", src))
            .err()
            .unwrap()
    }

    // --- mod declarations ---

    #[test]
    fn test_parse_mod_decl() {
        let prog = parse_src("mod io\n");
        assert_eq!(prog.items.len(), 1);
        assert_eq!(prog.items[0].name, "io");
        assert_eq!(prog.items[0].visibility, Visibility::Private);
        assert!(matches!(prog.items[0].kind, ItemKind::ModDecl));
    }

    #[test]
    fn test_parse_pub_mod_decl() {
        let prog = parse_src("pub mod io\n");
        assert_eq!(prog.items.len(), 1);
        assert_eq!(prog.items[0].name, "io");
        assert_eq!(prog.items[0].visibility, Visibility::Public);
        assert!(matches!(prog.items[0].kind, ItemKind::ModDecl));
    }

    #[test]
    fn test_parse_mod_decl_preserves_function_backwards_compat() {
        // `functions` vec should still contain functions parsed at top level.
        let prog = parse_src("mod io\nfn main() -> i64 { 0 }\n");
        assert_eq!(prog.items.len(), 2);
        assert_eq!(prog.functions.len(), 1);
        assert_eq!(prog.functions[0].name, "main");
    }

    // --- use declarations ---

    #[test]
    fn test_parse_use_simple_path() {
        let prog = parse_src("use io::println\n");
        assert_eq!(prog.items.len(), 1);
        assert_eq!(prog.items[0].name, "println");
        assert_eq!(prog.items[0].visibility, Visibility::Private);
        match &prog.items[0].kind {
            ItemKind::UseDecl { path, alias } => {
                assert_eq!(path, &vec!["io".to_string(), "println".to_string()]);
                assert_eq!(*alias, None);
            }
            other => panic!("expected UseDecl, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_use_deep_path() {
        let prog = parse_src("use utils::math::add\n");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0].kind {
            ItemKind::UseDecl { path, alias } => {
                assert_eq!(
                    path,
                    &vec!["utils".to_string(), "math".to_string(), "add".to_string(),]
                );
                assert_eq!(*alias, None);
            }
            other => panic!("expected UseDecl, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_pub_use_decl() {
        let prog = parse_src("pub use io::writer\n");
        assert_eq!(prog.items.len(), 1);
        assert_eq!(prog.items[0].visibility, Visibility::Public);
        match &prog.items[0].kind {
            ItemKind::UseDecl { path, alias: _ } => {
                assert_eq!(path, &vec!["io".to_string(), "writer".to_string()]);
            }
            other => panic!("expected UseDecl, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_use_with_as_alias() {
        // `as` is reserved but the parser supports rename for forward compatibility.
        let prog = parse_src("use io::writer as w\n");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0].kind {
            ItemKind::UseDecl { path, alias } => {
                assert_eq!(path, &vec!["io".to_string(), "writer".to_string()]);
                assert_eq!(alias, &Some("w".to_string()));
                // The item name is the last path segment (the original name).
                assert_eq!(prog.items[0].name, "writer");
            }
            other => panic!("expected UseDecl, got {:?}", other),
        }
    }

    // --- pub on functions and types ---

    #[test]
    fn test_parse_pub_fn() {
        let prog = parse_src("pub fn greet(n: i64) -> i64 { return n }\n");
        assert_eq!(prog.items.len(), 1);
        assert_eq!(prog.items[0].name, "greet");
        assert_eq!(prog.items[0].visibility, Visibility::Public);
        assert!(matches!(prog.items[0].kind, ItemKind::Function(_)));
    }

    #[test]
    fn test_parse_pub_struct() {
        let prog = parse_src("pub struct Point { x: i64, y: i64 }\n");
        assert_eq!(prog.items.len(), 1);
        assert_eq!(prog.items[0].name, "Point");
        assert_eq!(prog.items[0].visibility, Visibility::Public);
        match &prog.items[0].kind {
            ItemKind::StructDef { name, fields, .. } => {
                assert_eq!(name, "Point");
                assert_eq!(fields.len(), 2);
            }
            other => panic!("expected StructDef, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_pub_enum() {
        let prog = parse_src("pub enum Color { Red, Green, Blue }\n");
        assert_eq!(prog.items.len(), 1);
        assert_eq!(prog.items[0].name, "Color");
        assert_eq!(prog.items[0].visibility, Visibility::Public);
        match &prog.items[0].kind {
            ItemKind::EnumDef { name, variants, .. } => {
                assert_eq!(name, "Color");
                assert_eq!(variants.len(), 3);
            }
            other => panic!("expected EnumDef, got {:?}", other),
        }
    }

    // --- mixed programs ---

    #[test]
    fn test_parse_mixed_program() {
        let src = "mod io\nuse io::println\npub fn greet(n: i64) -> i64 { return n }\nfn main() -> i64 { return 0 }\n";
        let prog = parse_src(src);
        assert_eq!(prog.items.len(), 4);
        assert_eq!(prog.functions.len(), 2); // only fn items in the backwards-compat vec

        assert_eq!(prog.items[0].name, "io");
        assert!(matches!(prog.items[0].kind, ItemKind::ModDecl));

        assert_eq!(prog.items[1].name, "println");
        assert!(matches!(prog.items[1].kind, ItemKind::UseDecl { .. }));

        assert_eq!(prog.items[2].name, "greet");
        assert_eq!(prog.items[2].visibility, Visibility::Public);
        assert!(matches!(prog.items[2].kind, ItemKind::Function(_)));

        assert_eq!(prog.items[3].name, "main");
        assert_eq!(prog.items[3].visibility, Visibility::Private);
        assert!(matches!(prog.items[3].kind, ItemKind::Function(_)));
    }

    #[test]
    fn test_parse_struct_and_enum_with_pub_and_private() {
        let src = "pub struct Point { x: i64, y: i64 }\nenum Color { Red, Green }\n";
        let prog = parse_src(src);
        assert_eq!(prog.items.len(), 2);

        assert_eq!(prog.items[0].name, "Point");
        assert_eq!(prog.items[0].visibility, Visibility::Public);
        assert!(matches!(prog.items[0].kind, ItemKind::StructDef { .. }));

        assert_eq!(prog.items[1].name, "Color");
        assert_eq!(prog.items[1].visibility, Visibility::Private);
        assert!(matches!(prog.items[1].kind, ItemKind::EnumDef { .. }));
    }

    // --- error cases ---

    #[test]
    fn test_parse_mod_without_name_errors() {
        let err = parse_fail("mod\n");
        assert!(
            err.to_string().contains("unexpected") || err.to_string().contains("expected"),
            "expected error for `mod` without name, got: {}",
            err
        );
    }

    #[test]
    fn test_parse_use_without_path_errors() {
        let err = parse_fail("use\n");
        assert!(
            err.to_string().contains("unexpected") || err.to_string().contains("expected"),
            "expected error for `use` without path, got: {}",
            err
        );
    }

    #[test]
    fn test_parse_pub_alone_errors() {
        // `pub` must be followed by an item keyword
        let err = parse_fail("pub\n");
        assert!(
            err.to_string().contains("unexpected") || err.to_string().contains("expected"),
            "expected error for `pub` without item, got: {}",
            err
        );
    }

    #[test]
    fn test_parse_mod_inside_function_is_error() {
        // `mod` is a top-level item only; it should NOT be valid inside a function body.
        let tokens: Vec<Token> = Lexer::new("fn main() -> i64 { mod foo }")
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let result = parse("fn main() -> i64 { mod foo }", tokens);
        assert!(
            result.is_err(),
            "mod inside a function body should be a parse error"
        );
    }
}
