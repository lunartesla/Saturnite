use stnx::lexer::Lexer;
use stnx::lexer::Token;

fn main() {
    let src = std::fs::read_to_string("examples/hello.stn").unwrap();
    let mut lexer = Lexer::new(&src);
    let tokens: Vec<Token> = lexer.by_ref().collect::<Result<Vec<_>, _>>().unwrap();

    println!("Tokens:");
    for t in &tokens {
        println!("  {:?}", t.kind);
    }
    println!("Total: {} tokens", tokens.len());
}
