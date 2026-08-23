//! Tests for the HIR → MIR lowering pass.

mod common;

use common::ir_only;
use stnx::ast::Program;
use stnx::hir::lower::lower;
use stnx::lexer::Lexer;
use stnx::mir::lower::lower_program;
use stnx::mir::MirBinOp;
use stnx::mir::MirOperand;
use stnx::mir::MirRvalue;
use stnx::mir::MirStmtKind;
use stnx::mir::MirTerminator;
use stnx::parser;

/// Full pipeline: lex → parse → HIR → MIR.
fn to_mir(src: &str) -> stnx::mir::MirProgram {
    let tokens: Vec<_> = Lexer::new(src)
        .collect::<Result<Vec<_>, _>>()
        .expect("lexing failed");
    let program: Program = parser::parse(src, tokens).expect("parsing failed");
    let hir = lower(&program).expect("HIR lowering failed");
    lower_program(&hir).expect("MIR lowering failed")
}

#[test]
fn test_simple_return() {
    let mir = to_mir("fn main() -> i64 { return 42 }");
    assert_eq!(mir.functions.len(), 1);
    let func = &mir.functions[0];
    assert_eq!(func.blocks.len(), 1);
    let block = &func.blocks[0];
    match &block.terminator {
        MirTerminator::Return(Some(operand)) => {
            assert_eq!(operand.ty(&func.locals), stnx::hir::HirType::I64);
        }
        other => panic!("expected Return, got {:?}", other),
    }
}

#[test]
fn test_implicit_return() {
    let mir = to_mir("fn main() -> i64 { 7 }");
    let func = &mir.functions[0];
    let block = &func.blocks[0];
    match &block.terminator {
        MirTerminator::Return(Some(_)) => {}
        other => panic!("expected Return, got {:?}", other),
    }
}

#[test]
fn test_return_without_value() {
    let mir = to_mir("fn main() { return }");
    let func = &mir.functions[0];
    let block = &func.blocks[0];
    match &block.terminator {
        MirTerminator::Return(None) => {}
        other => panic!("expected Return(None), got {:?}", other),
    }
}

#[test]
fn test_binary_op_in_block() {
    let mir = to_mir("fn main() -> i64 { 1 + 2 * 3 }");
    let func = &mir.functions[0];
    let block = &func.blocks[0];
    // Should have LocalDecls and Assigns for the binary ops, then Return
    assert!(
        block.stmts.len() >= 3,
        "expected at least 3 statements (2 LocalDecls + 2 Assigns), got {}",
        block.stmts.len()
    );
    // The terminator should be Return
    match &block.terminator {
        MirTerminator::Return(_) => {}
        other => panic!("expected Return, got {:?}", other),
    }
}

#[test]
fn test_printf_call_as_terminator() {
    let mir = to_mir("fn main() -> i64 { println(42) return 0 }");
    let func = &mir.functions[0];
    // The entry block should end with a Call terminator (println)
    // followed by a continuation block with Return
    let entry = &func.blocks[0];
    match &entry.terminator {
        MirTerminator::Call {
            func: _,
            args,
            destination: _,
            next,
        } => {
            assert_eq!(args.len(), 1);
            assert_eq!(*next, stnx::mir::BlockId(1)); // continuation
        }
        other => panic!("expected Call terminator in entry block, got {:?}", other),
    }
    // Continuation block should have the Return
    let cont = &func.blocks[1];
    match &cont.terminator {
        MirTerminator::Return(Some(_)) => {}
        other => panic!("expected Return in continuation, got {:?}", other),
    }
}

#[test]
fn test_if_else_cfg() {
    let mir = to_mir("fn main() -> i64 { if true { 1 } else { 2 } return 0 }");
    let func = &mir.functions[0];
    // Should have multiple blocks: entry, then, else, end, (ret)
    assert!(
        func.blocks.len() >= 4,
        "expected >= 4 blocks, got {}",
        func.blocks.len()
    );
    // Entry block should end with SwitchInt
    let entry = &func.blocks[0];
    match &entry.terminator {
        MirTerminator::SwitchInt { .. } => {}
        other => panic!("expected SwitchInt in entry, got {:?}", other),
    }
}

#[test]
fn test_while_loop_cfg() {
    let mir = to_mir("fn main() -> i64 { let mut i = 0 while i < 10 { i = i + 1 } return i }");
    let func = &mir.functions[0];
    // Should have: entry (with let + goto), cond, body, exit, ...
    assert!(
        func.blocks.len() >= 4,
        "expected >= 4 blocks, got {}",
        func.blocks.len()
    );
}

#[test]
fn test_for_loop_cfg() {
    let mir = to_mir("fn main() -> i64 { let mut s = 0 for i in 0..10 { s = s + i } return s }");
    let func = &mir.functions[0];
    assert!(
        func.blocks.len() >= 4,
        "expected >= 4 blocks, got {}",
        func.blocks.len()
    );
}

#[test]
fn test_elif_chain_cfg() {
    let mir = to_mir(
        "fn main() -> i64 { let x = 1 if x == 1 { 1 } elif x == 2 { 2 } else { 3 } return 0 }",
    );
    let func = &function(&mir, "main");
    // Should have entry, then, elif_cond0, elif_body0, else, end, ...
    assert!(
        func.blocks.len() >= 6,
        "expected >= 6 blocks, got {}",
        func.blocks.len()
    );
}

/// Helper: find a function by name in a MirProgram.
fn function<'a>(prog: &'a stnx::mir::MirProgram, name: &str) -> &'a stnx::mir::MirFunction {
    prog.functions
        .iter()
        .find(|f| prog.symbols.lookup(f.name) == Some(name))
        .expect("function not found")
}

#[test]
fn test_mir_ir_generation_still_works() {
    // Ensure the existing HIR-based codegen path still works alongside MIR.
    let ir = ir_only("fn main() -> i64 { return 42 }");
    assert!(ir.contains("define i64 @main"));
}

#[test]
fn test_smoke_test_mir_structure() {
    use common::to_mir;
    use stnx::hir::HirType;

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let path = std::path::Path::new(&manifest_dir).join("../../examples/smoke_test.stnx");
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("failed to read {}", path.display()));
    let mir = to_mir(&src);

    // --- 6 functions ---
    assert_eq!(mir.functions.len(), 6, "smoke test should have 6 functions");

    let names: std::collections::HashSet<&str> = mir
        .functions
        .iter()
        .filter_map(|f| mir.symbols.lookup(f.name))
        .collect();
    for expected in [
        "factorial",
        "is_even",
        "sum_even_squares",
        "sum_range",
        "classify",
        "main",
    ] {
        assert!(names.contains(expected), "missing function: {expected}");
    }

    // --- is_even returns bool ---
    let is_even = function(&mir, "is_even");
    assert_eq!(
        is_even.return_type,
        HirType::Bool,
        "is_even should return bool"
    );

    // --- factorial has recursive Call ---
    let factorial = function(&mir, "factorial");
    let has_call = factorial
        .blocks
        .iter()
        .any(|b| matches!(b.terminator, MirTerminator::Call { .. }));
    assert!(
        has_call,
        "factorial should have a Call terminator (recursion)"
    );

    // --- while loop in sum_even_squares ---
    let sum_even_squares = function(&mir, "sum_even_squares");
    assert!(
        sum_even_squares.blocks.len() >= 4,
        "sum_even_squares should have >= 4 blocks (while loop), got {}",
        sum_even_squares.blocks.len()
    );

    // --- for loop in sum_range ---
    let sum_range = function(&mir, "sum_range");
    assert!(
        sum_range.blocks.len() >= 4,
        "sum_range should have >= 4 blocks (for loop), got {}",
        sum_range.blocks.len()
    );

    // --- if/elif/else in classify ---
    let classify = function(&mir, "classify");
    assert!(
        classify.blocks.len() >= 4,
        "classify should have >= 4 blocks (if/elif/else), got {}",
        classify.blocks.len()
    );

    // --- main has Call terminators (function calls + printlns) ---
    let main_fn = function(&mir, "main");
    let call_count = main_fn
        .blocks
        .iter()
        .filter(|b| matches!(b.terminator, MirTerminator::Call { .. }))
        .count();
    assert!(
        call_count >= 5,
        "main should have >= 5 Call terminators, got {call_count}"
    );

    // --- arithmetic: Add, Mul, Mod ---
    let has_add = has_binop(&mir, MirBinOp::Add);
    let has_mul = has_binop(&mir, MirBinOp::Mul);
    let has_mod = has_binop(&mir, MirBinOp::Mod);
    assert!(has_add, "MIR should contain an Add binary op");
    assert!(has_mul, "MIR should contain a Mul binary op");
    assert!(has_mod, "MIR should contain a Mod binary op");

    // --- SwitchInt (conditional branch) ---
    let has_switch = mir.functions.iter().any(|f| {
        f.blocks
            .iter()
            .any(|b| matches!(b.terminator, MirTerminator::SwitchInt { .. }))
    });
    assert!(has_switch, "MIR should contain SwitchInt terminators");

    // --- Return terminators ---
    let has_return = mir.functions.iter().any(|f| {
        f.blocks
            .iter()
            .any(|b| matches!(b.terminator, MirTerminator::Return(_)))
    });
    assert!(has_return, "MIR should contain Return terminators");

    // --- locals exist in every function ---
    for f in &mir.functions {
        assert!(
            !f.locals.is_empty(),
            "function {:?} should have at least one local",
            mir.symbols.lookup(f.name)
        );
    }
}

/// Check whether any block in any function of `prog` contains a binary
/// operation with the given operator.
fn has_binop(prog: &stnx::mir::MirProgram, op: MirBinOp) -> bool {
    prog.functions.iter().any(|f| {
        f.blocks.iter().any(|b| {
            b.stmts.iter().any(|s| {
                matches!(
                    &s.kind,
                    MirStmtKind::Assign {
                        rvalue: MirRvalue::Binary { op: o, .. },
                        ..
                    } if *o == op
                )
            })
        })
    })
}

/// Check whether a MirConst with the given value exists as a folded constant
/// in any rvalue (Binary operand or Use(Const)) in the program.
fn has_const_value(prog: &stnx::mir::MirProgram, expected: &stnx::mir::MirConst) -> bool {
    prog.functions.iter().any(|f| {
        f.blocks.iter().any(|b| {
            b.stmts.iter().any(|s| {
                if let MirStmtKind::Assign { rvalue, .. } = &s.kind {
                    match rvalue {
                        MirRvalue::Binary { lhs, rhs, .. } => {
                            matches!(lhs, MirOperand::Const(c) if c == expected)
                                || matches!(rhs, MirOperand::Const(c) if c == expected)
                        }
                        MirRvalue::Use(MirOperand::Const(c)) => *c == *expected,
                        _ => false,
                    }
                } else {
                    false
                }
            })
        })
    })
}

// --- Constant folding tests ---

#[test]
fn test_constant_folding_i64_add() {
    let tokens: Vec<_> = Lexer::new("fn main() -> i64 { 2 + 3 }")
        .collect::<Result<_, _>>()
        .unwrap();
    let program = parser::parse("fn main() -> i64 { 2 + 3 }", tokens).unwrap();
    let hir = lower(&program).unwrap();
    let mut mir = lower_program(&hir).unwrap();
    stnx::mir::opt::optimize(&mut mir);

    // After folding, the binary op should be replaced by a Use(Const(I64(5)))
    let has_binary_add = has_binop(&mir, MirBinOp::Add);
    assert!(
        !has_binary_add,
        "2 + 3 should be folded, no Add binary op should remain"
    );
    assert!(
        has_const_value(&mir, &stnx::mir::MirConst::I64(5)),
        "folded result 5 should appear as a constant operand"
    );
}

#[test]
fn test_constant_folding_i64_sub() {
    let tokens: Vec<_> = Lexer::new("fn main() -> i64 { 10 - 4 }")
        .collect::<Result<_, _>>()
        .unwrap();
    let program = parser::parse("fn main() -> i64 { 10 - 4 }", tokens).unwrap();
    let hir = lower(&program).unwrap();
    let mut mir = lower_program(&hir).unwrap();
    stnx::mir::opt::optimize(&mut mir);

    assert!(!has_binop(&mir, MirBinOp::Sub));
    assert!(has_const_value(&mir, &stnx::mir::MirConst::I64(6)));
}

#[test]
fn test_constant_folding_i64_mul() {
    let tokens: Vec<_> = Lexer::new("fn main() -> i64 { 3 * 4 }")
        .collect::<Result<_, _>>()
        .unwrap();
    let program = parser::parse("fn main() -> i64 { 3 * 4 }", tokens).unwrap();
    let hir = lower(&program).unwrap();
    let mut mir = lower_program(&hir).unwrap();
    stnx::mir::opt::optimize(&mut mir);

    assert!(!has_binop(&mir, MirBinOp::Mul));
    assert!(has_const_value(&mir, &stnx::mir::MirConst::I64(12)));
}

#[test]
fn test_constant_folding_i64_div() {
    let tokens: Vec<_> = Lexer::new("fn main() -> i64 { 20 / 5 }")
        .collect::<Result<_, _>>()
        .unwrap();
    let program = parser::parse("fn main() -> i64 { 20 / 5 }", tokens).unwrap();
    let hir = lower(&program).unwrap();
    let mut mir = lower_program(&hir).unwrap();
    stnx::mir::opt::optimize(&mut mir);

    assert!(!has_binop(&mir, MirBinOp::Div));
    assert!(has_const_value(&mir, &stnx::mir::MirConst::I64(4)));
}

#[test]
fn test_constant_folding_i64_mod() {
    let tokens: Vec<_> = Lexer::new("fn main() -> i64 { 20 % 6 }")
        .collect::<Result<_, _>>()
        .unwrap();
    let program = parser::parse("fn main() -> i64 { 20 % 6 }", tokens).unwrap();
    let hir = lower(&program).unwrap();
    let mut mir = lower_program(&hir).unwrap();
    stnx::mir::opt::optimize(&mut mir);

    assert!(!has_binop(&mir, MirBinOp::Mod));
    assert!(has_const_value(&mir, &stnx::mir::MirConst::I64(2)));
}

#[test]
fn test_constant_folding_f64_add() {
    let tokens: Vec<_> = Lexer::new("fn main() -> f64 { 1.5 + 2.5 }")
        .collect::<Result<_, _>>()
        .unwrap();
    let program = parser::parse("fn main() -> f64 { 1.5 + 2.5 }", tokens).unwrap();
    let hir = lower(&program).unwrap();
    let mut mir = lower_program(&hir).unwrap();
    stnx::mir::opt::optimize(&mut mir);

    assert!(!has_binop(&mir, MirBinOp::Add));
    assert!(has_const_value(&mir, &stnx::mir::MirConst::F64(4.0)));
}

#[test]
fn test_constant_folding_f64_sub() {
    let tokens: Vec<_> = Lexer::new("fn main() -> f64 { 10.0 - 4.0 }")
        .collect::<Result<_, _>>()
        .unwrap();
    let program = parser::parse("fn main() -> f64 { 10.0 - 4.0 }", tokens).unwrap();
    let hir = lower(&program).unwrap();
    let mut mir = lower_program(&hir).unwrap();
    stnx::mir::opt::optimize(&mut mir);

    assert!(!has_binop(&mir, MirBinOp::Sub));
    assert!(has_const_value(&mir, &stnx::mir::MirConst::F64(6.0)));
}

#[test]
fn test_constant_folding_f64_mul() {
    let tokens: Vec<_> = Lexer::new("fn main() -> f64 { 4.0 * 2.0 }")
        .collect::<Result<_, _>>()
        .unwrap();
    let program = parser::parse("fn main() -> f64 { 4.0 * 2.0 }", tokens).unwrap();
    let hir = lower(&program).unwrap();
    let mut mir = lower_program(&hir).unwrap();
    stnx::mir::opt::optimize(&mut mir);

    assert!(!has_binop(&mir, MirBinOp::Mul));
    assert!(has_const_value(&mir, &stnx::mir::MirConst::F64(8.0)));
}

#[test]
fn test_constant_folding_f64_div() {
    let tokens: Vec<_> = Lexer::new("fn main() -> f64 { 10.0 / 2.0 }")
        .collect::<Result<_, _>>()
        .unwrap();
    let program = parser::parse("fn main() -> f64 { 10.0 / 2.0 }", tokens).unwrap();
    let hir = lower(&program).unwrap();
    let mut mir = lower_program(&hir).unwrap();
    stnx::mir::opt::optimize(&mut mir);

    assert!(!has_binop(&mir, MirBinOp::Div));
    assert!(has_const_value(&mir, &stnx::mir::MirConst::F64(5.0)));
}

#[test]
fn test_constant_folding_i64_eq() {
    let tokens: Vec<_> = Lexer::new("fn main() -> i64 { 3 == 4 }")
        .collect::<Result<_, _>>()
        .unwrap();
    let program = parser::parse("fn main() -> i64 { 3 == 4 }", tokens).unwrap();
    let hir = lower(&program).unwrap();
    let mut mir = lower_program(&hir).unwrap();
    stnx::mir::opt::optimize(&mut mir);

    assert!(!has_binop(&mir, MirBinOp::Eq));
    assert!(has_const_value(&mir, &stnx::mir::MirConst::Bool(false)));
}

#[test]
fn test_constant_folding_bool_and() {
    let tokens: Vec<_> = Lexer::new("fn main() -> i64 { true && false }")
        .collect::<Result<_, _>>()
        .unwrap();
    let program = parser::parse("fn main() -> i64 { true && false }", tokens).unwrap();
    let hir = lower(&program).unwrap();
    let mut mir = lower_program(&hir).unwrap();
    stnx::mir::opt::optimize(&mut mir);

    assert!(!has_binop(&mir, MirBinOp::And));
    assert!(has_const_value(&mir, &stnx::mir::MirConst::Bool(false)));
}

#[test]
fn test_constant_folding_bool_or() {
    let tokens: Vec<_> = Lexer::new("fn main() -> i64 { true || false }")
        .collect::<Result<_, _>>()
        .unwrap();
    let program = parser::parse("fn main() -> i64 { true || false }", tokens).unwrap();
    let hir = lower(&program).unwrap();
    let mut mir = lower_program(&hir).unwrap();
    stnx::mir::opt::optimize(&mut mir);

    assert!(!has_binop(&mir, MirBinOp::Or));
    assert!(has_const_value(&mir, &stnx::mir::MirConst::Bool(true)));
}

#[test]
fn test_constant_folding_not_bool() {
    let tokens: Vec<_> = Lexer::new("fn main() -> i64 { !true }")
        .collect::<Result<_, _>>()
        .unwrap();
    let program = parser::parse("fn main() -> i64 { !true }", tokens).unwrap();
    let hir = lower(&program).unwrap();
    let mut mir = lower_program(&hir).unwrap();
    stnx::mir::opt::optimize(&mut mir);

    // After folding, !true should become Const(Bool(false))
    assert!(has_const_value(&mir, &stnx::mir::MirConst::Bool(false)));
}

#[test]
fn test_constant_folding_neg_i64() {
    let tokens: Vec<_> = Lexer::new("fn main() -> i64 { -42 }")
        .collect::<Result<_, _>>()
        .unwrap();
    let program = parser::parse("fn main() -> i64 { -42 }", tokens).unwrap();
    let hir = lower(&program).unwrap();
    let mut mir = lower_program(&hir).unwrap();
    stnx::mir::opt::optimize(&mut mir);

    assert!(has_const_value(&mir, &stnx::mir::MirConst::I64(-42)));
}

#[test]
fn test_constant_folding_not_applied_to_non_constants() {
    // Variables should NOT be folded
    let tokens: Vec<_> = Lexer::new("fn main() -> i64 { let a = 2 let b = 3 a + b }")
        .collect::<Result<_, _>>()
        .unwrap();
    let program = parser::parse("fn main() -> i64 { let a = 2 let b = 3 a + b }", tokens).unwrap();
    let hir = lower(&program).unwrap();
    let mut mir = lower_program(&hir).unwrap();
    stnx::mir::opt::optimize(&mut mir);

    // The binary op with variable operands should remain
    assert!(
        has_binop(&mir, MirBinOp::Add),
        "a + b with variables should not be folded"
    );
}

#[test]
fn test_constant_folding_i64_lt() {
    let tokens: Vec<_> = Lexer::new("fn main() -> i64 { 3 < 4 }")
        .collect::<Result<_, _>>()
        .unwrap();
    let program = parser::parse("fn main() -> i64 { 3 < 4 }", tokens).unwrap();
    let hir = lower(&program).unwrap();
    let mut mir = lower_program(&hir).unwrap();
    stnx::mir::opt::optimize(&mut mir);

    assert!(!has_binop(&mir, MirBinOp::Lt));
    assert!(has_const_value(&mir, &stnx::mir::MirConst::Bool(true)));
}
