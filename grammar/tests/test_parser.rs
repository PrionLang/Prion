use grammar::parser::Parser;

#[test]
fn test_parser() {
    let mut p = Parser::from_str(r##" "\name \u200b \x50 \0 \"" "##);
    let expr = p.consume_expr().unwrap();
    println!("{0:?} => {0}", expr);
}
