use stnx::parser;

#[test]
fn external_rust_parses() {
    let src = "external rust \"x\" \"y\"(a: i64) -> i64\nfn main() -> i64 {\n    return 42\n}\n";
    let tokens = stnx::lexer::prepare(src).unwrap();
    let prog = parser::parse(src, tokens).unwrap();
    assert_eq!(prog.items.len(), 2);
}

#[test]
fn external_python_symbol_parses() {
    let src = "external python \"test_math\" \"test_math::add\"(a: i64, b: i64) -> i64\nfn main() -> i64 {\n    return 0\n}\n";
    let tokens = stnx::lexer::prepare(src).unwrap();
    match parser::parse(src, tokens) {
        Ok(prog) => assert_eq!(prog.items.len(), 2),
        Err(e) => panic!("parse failed: {:?}", e),
    }
}
