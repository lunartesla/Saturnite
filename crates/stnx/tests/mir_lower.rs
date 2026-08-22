//! Tests for the HIR → MIR lowering pass.

mod common;

use common::ir_only;
use stnx::ast::Program;
use stnx::hir::lower::lower;
use stnx::lexer::Lexer;
use stnx::mir::lower::lower_program;
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
