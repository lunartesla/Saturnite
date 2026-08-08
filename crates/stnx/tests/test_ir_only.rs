use stnx::lexer::Lexer;
use stnx::parser;
use stnx::semantic::analyze;
use stnx::codegen::generate_ir;

#[test]
fn test_ir_generation_only() {
    let src = "fn main() -> i64 { return 42 }";
    let tokens: Vec<_> = Lexer::new(src).collect::<Result<Vec<_>, _>>().unwrap();
    let program = parser::parse(src, tokens).unwrap();
    analyze(&program).unwrap();
    let ir = generate_ir(&program).expect("IR generation should succeed");
    println!("IR: {}", ir);
}
