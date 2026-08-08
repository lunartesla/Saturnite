fn main() {
    // Test with just integers
    let src = "42";
    let lexer = stnx::lexer::Lexer::new(src);
    let tokens: Vec<_> = lexer.collect::<Result<Vec<_>, _>>().unwrap();
    println!("Tokens: {:?}", tokens);

    // Try parsing as an expression
    let lexer2 = stnx::lexer::Lexer::new(src);
    let tokens2: Vec<_> = lexer2.collect::<Result<Vec<_>, _>>().unwrap();
    let program = stnx::parser::parse(src, tokens2);
    match program {
        Ok(p) => println!("Parsed: {:?}", p),
        Err(e) => println!("Error: {}", e),
    }
}
