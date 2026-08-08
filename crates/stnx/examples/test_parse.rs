fn main() {
    let src = "fn main() -> i64 { let x = 42 println(x) 0 }";
    let mut lexer = stnx::lexer::Lexer::new(src);
    let tokens: Vec<_> = lexer.collect::<Result<Vec<_>, _>>().unwrap();
    println!("Tokens: {}", tokens.len());
    let program = stnx::parser::parse(src, tokens);
    match program {
        Ok(p) => println!("Parsed: {:?}", p),
        Err(e) => println!("Error: {}", e),
    }
}
