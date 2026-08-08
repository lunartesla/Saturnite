fn main() {
    let src = "fn main() -> i64 { let x = 42 println(x) 0 }";
    let lexer = stnx::lexer::Lexer::new(src);
    let tokens: Vec<_> = lexer.collect::<Result<Vec<_>, _>>().unwrap();
    for t in &tokens {
        println!("{:?}", t);
    }
}
