//! Parse-only validator: `check <schema1> <schema2> ...`.
//! Prints OK / the parse error for each schema file. Temporary dev tool for
//! authoring registry packs — grammar validation mirrors what plugin install
//! runs (`schema_parser::parse`).
fn main() {
    let mut bad = 0;
    for path in std::env::args().skip(1) {
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                println!("READ-ERR {path}: {e}");
                bad += 1;
                continue;
            }
        };
        match schema_parser::parse(&text) {
            Ok(s) => println!("OK       {path}  ({} structs)", s.structs.len()),
            Err(e) => {
                println!("PARSE-ERR {path}: {e}");
                bad += 1;
            }
        }
    }
    if bad > 0 {
        std::process::exit(1);
    }
}
