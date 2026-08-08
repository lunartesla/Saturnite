use stnx::lexer::{Lexer, Token};
use stnx::parser::{block_debug, func_debug, params_debug, program_debug, ret_type_debug};

fn main() {
    let src = "fn main() -> i64 {
    0
}";
    let tokens: Vec<Token> = Lexer::new(src).collect::<Result<Vec<_>, _>>().unwrap();
    println!(
        "Tokens: {:?}",
        tokens.iter().map(|t| &t.kind).collect::<Vec<_>>()
    );

    // Test params parser (parses just LParen RParen)
    let result = params_debug(&tokens[2..4]); // LParen, RParen
    println!("Params parse (): {:?}", result);

    // Test ret_type parser (parses just RArrow I64)
    let result = ret_type_debug(&tokens[5..7]); // RArrow, I64
    println!("Ret type parse (-> i64): {:?}", result);

    // Test block parser
    let result = block_debug(&tokens[6..]);
    println!("Block parse (LBrace Integer(0) RBrace): {:?}", result);

    // Test func parser
    let result = func_debug(&tokens);
    println!("Func parse (fn main() -> i64 {{ 0 }}): {:?}", result);

    // Test program parser
    let result = program_debug(&tokens);
    println!("Program parse: {:?}", result);
}
