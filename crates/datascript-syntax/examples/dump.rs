//! Prints the CST and diagnostics of a program: `cargo run -p datascript-syntax --example dump -- 'text'`.

use datascript_diagnostics::{Renderer, SourceMap};

fn main() {
    let text = std::env::args().nth(1).unwrap_or_default();
    let mut sources = SourceMap::new();
    let file = sources.add("query", text.as_str());
    let parse = datascript_syntax::parse(&text, file);
    print!("{}", parse.debug_tree());
    if !parse.errors.is_empty() {
        println!();
        print!("{}", Renderer::plain().render_all(&parse.errors, &sources));
    }
}
