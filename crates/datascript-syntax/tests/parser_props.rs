//! Property tests: the parser never panics, its tree reproduces the input exactly, and the typed
//! AST can be walked on any tree.

use datascript_diagnostics::FileId;
use datascript_syntax::ast::{Arg, AstNode, Expr, Operator, Pipeline, Program, Stmt};
use datascript_syntax::{SyntaxKind, parse};
use proptest::prelude::*;

fn check(text: &str) -> Result<(), TestCaseError> {
    let parse = parse(text, FileId(0));
    let root = parse.syntax();
    prop_assert_eq!(root.kind(), SyntaxKind::PROGRAM);
    prop_assert_eq!(root.text().to_string(), text);
    for node in root.descendants().skip(1) {
        prop_assert!(
            !node.text_range().is_empty(),
            "empty {:?} node in {:?}",
            node.kind(),
            text
        );
    }
    for d in &parse.errors {
        let primary = d.primary.as_ref().expect("parse errors have a span");
        prop_assert!(primary.span.start <= primary.span.end);
        prop_assert!(primary.span.end as usize <= text.len());
        prop_assert!(text.is_char_boundary(primary.span.start as usize));
        prop_assert!(text.is_char_boundary(primary.span.end as usize));
    }
    walk_program(&parse.program());
    let _ = parse.debug_tree();
    Ok(())
}

/// Calls every accessor, recursively, to check none of them panics on a broken tree.
fn walk_program(program: &Program) {
    for stmt in program.statements() {
        match stmt {
            Stmt::Let(let_stmt) => {
                let _ = let_stmt.name().and_then(|n| n.text());
                if let Some(value) = let_stmt.value() {
                    walk_expr(&value);
                }
            }
            Stmt::Query(pipeline) => walk_pipeline(&pipeline),
        }
    }
}

fn walk_pipeline(pipeline: &Pipeline) {
    if let Some(source) = pipeline.source() {
        walk_expr(&source);
    }
    for op in pipeline.operators() {
        let _ = op.name();
        let exprs: Vec<Expr> = match &op {
            Operator::Where(op) => op.predicate().into_iter().collect(),
            Operator::Project(op) => op.items().filter_map(|i| i.expr()).collect(),
            Operator::Extend(op) => op.items().filter_map(|i| i.expr()).collect(),
            Operator::ProjectAway(op) => {
                for p in op.patterns() {
                    let _ = (p.text(), p.has_wildcard());
                }
                Vec::new()
            }
            Operator::ProjectRename(op) => {
                for item in op.items() {
                    let _ = item.new_name().and_then(|n| n.text());
                    let _ = item.old_name().and_then(|n| n.text());
                }
                Vec::new()
            }
            Operator::Summarize(op) => op
                .aggregates()
                .chain(op.by().into_iter().flat_map(|b| b.keys()))
                .filter_map(|i| i.expr())
                .collect(),
            Operator::Sort(op) => {
                let _ = op.by_token();
                op.keys()
                    .inspect(|k| {
                        let _ = (k.direction(), k.nulls());
                    })
                    .filter_map(|k| k.expr())
                    .collect()
            }
            Operator::Top(op) => op
                .count()
                .into_iter()
                .chain(op.key().and_then(|k| k.expr()))
                .collect(),
            Operator::Take(op) => op.count().into_iter().collect(),
            Operator::Count(_) | Operator::GetSchema(_) => Vec::new(),
            Operator::Distinct(op) => {
                let _ = op.star_token();
                op.columns().collect()
            }
            Operator::Join(op) => {
                let _ = op.kind().and_then(|k| k.value_token());
                op.right().into_iter().chain(op.conditions()).collect()
            }
            Operator::Union(op) => op.tables().collect(),
            Operator::Render(op) => {
                let _ = op.kind_token();
                op.properties()
                    .inspect(|p| {
                        let _ = p.name();
                    })
                    .flat_map(|p| p.values())
                    .collect()
            }
        };
        for e in exprs {
            walk_expr(&e);
        }
    }
}

fn walk_expr(expr: &Expr) {
    let _ = expr.syntax().text_range();
    let children: Vec<Expr> = match expr {
        Expr::Literal(l) => {
            let _ = l.kind();
            Vec::new()
        }
        Expr::TypedNull(t) => {
            let _ = t.type_token();
            Vec::new()
        }
        Expr::NameRef(n) => {
            let _ = n.text();
            Vec::new()
        }
        Expr::Paren(p) => p.inner().into_iter().collect(),
        Expr::Call(c) => {
            let _ = c.callee().and_then(|n| n.text());
            c.args()
                .filter_map(|a| match a {
                    Arg::Positional(e) => Some(e),
                    Arg::Named(n) => n.expr(),
                })
                .collect()
        }
        Expr::Field(f) => {
            let _ = f.field();
            f.base().into_iter().collect()
        }
        Expr::Index(i) => i.base().into_iter().chain(i.index()).collect(),
        Expr::Prefix(p) => {
            let _ = p.op();
            p.operand().into_iter().collect()
        }
        Expr::Bin(b) => {
            let _ = b.op();
            b.lhs().into_iter().chain(b.rhs()).collect()
        }
        Expr::In(i) => {
            let _ = (i.negated(), i.case_insensitive());
            i.lhs()
                .into_iter()
                .chain(i.list().into_iter().flat_map(|l| {
                    l.args().filter_map(|a| match a {
                        Arg::Positional(e) => Some(e),
                        Arg::Named(n) => n.expr(),
                    })
                }))
                .collect()
        }
        Expr::Between(b) => {
            let _ = b.negated();
            b.lhs().into_iter().chain(b.low()).chain(b.high()).collect()
        }
        Expr::Pipeline(p) => {
            walk_pipeline(p);
            Vec::new()
        }
    };
    for child in children {
        walk_expr(&child);
    }
}

/// Pieces of DataScript, glued together at random, so that the parser sees nearly-valid input.
fn fragment() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("|".to_string()),
        Just(";".to_string()),
        Just(",".to_string()),
        Just("(".to_string()),
        Just(")".to_string()),
        Just("[".to_string()),
        Just("]".to_string()),
        Just("=".to_string()),
        Just("==".to_string()),
        Just("<".to_string()),
        Just("-".to_string()),
        Just("*".to_string()),
        Just(".".to_string()),
        Just("..".to_string()),
        Just("\n".to_string()),
        Just(" ".to_string()),
        Just("let".to_string()),
        Just("not".to_string()),
        Just("and".to_string()),
        Just("null".to_string()),
        Just("where".to_string()),
        Just("project".to_string()),
        Just("project-away".to_string()),
        Just("project-rename".to_string()),
        Just("mv-expand".to_string()),
        Just("extend".to_string()),
        Just("summarize".to_string()),
        Just("by".to_string()),
        Just("sort".to_string()),
        Just("order".to_string()),
        Just("asc".to_string()),
        Just("nulls".to_string()),
        Just("first".to_string()),
        Just("top".to_string()),
        Just("take".to_string()),
        Just("count".to_string()),
        Just("distinct".to_string()),
        Just("join".to_string()),
        Just("kind".to_string()),
        Just("on".to_string()),
        Just("$left".to_string()),
        Just("union".to_string()),
        Just("render".to_string()),
        Just("with".to_string()),
        Just("getschema".to_string()),
        Just("in".to_string()),
        Just("!in~".to_string()),
        Just("between".to_string()),
        Just("has".to_string()),
        Just("matches".to_string()),
        Just("regex".to_string()),
        Just("long(null)".to_string()),
        Just("datetime(2024-01-01)".to_string()),
        Just("'a.csv'".to_string()),
        Just("['x y']".to_string()),
        Just("1.5h".to_string()),
        Just("'unterminated".to_string()),
        Just("#".to_string()),
        Just("/* c */".to_string()),
        // Habits from other languages, which have targeted diagnostics (M1-03).
        Just("&&".to_string()),
        Just("||".to_string()),
        Just("<>".to_string()),
        Just("!".to_string()),
        Just("SELECT".to_string()),
        Just("FROM".to_string()),
        Just("WHERE".to_string()),
        Just("GROUP BY".to_string()),
        Just("ORDER BY".to_string()),
        Just("DESC".to_string()),
        Just("using".to_string()),
        Just("innerunique".to_string()),
        Just("iff".to_string()),
        Just("filter".to_string()),
        Just("project - away".to_string()),
        "[a-z]{1,3}",
        "[0-9]{1,3}",
        any::<char>().prop_map(String::from),
    ]
}

/// Fragments joined with a space or nothing in between.
fn fragments() -> impl Strategy<Value = String> {
    prop::collection::vec((fragment(), prop::bool::ANY), 0..40).prop_map(|parts| {
        let mut out = String::new();
        for (part, space) in parts {
            out.push_str(&part);
            if space {
                out.push(' ');
            }
        }
        out
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn arbitrary_text_parses(text in any::<String>()) {
        check(&text)?;
    }

    #[test]
    fn fragments_parse(text in fragments()) {
        check(&text)?;
    }
}

#[test]
fn realistic_nesting_parses() {
    let text = format!("t | where {}x{}", "(".repeat(200), ")".repeat(200));
    let parse = parse(&text, FileId(0));
    assert!(parse.errors.is_empty(), "{:?}", parse.errors);
    assert_eq!(parse.syntax().text().to_string(), text);
}

#[test]
fn pathological_nesting_gives_one_error() {
    for text in [
        format!(
            "t | where {}x{} | take 1; u | count",
            "(".repeat(100_000),
            ")".repeat(100_000)
        ),
        format!("t | where {}x | take 1; u | count", "(".repeat(100_000)),
        format!("t | where {}x | take 1; u | count", "- ".repeat(100_000)),
        format!("t | where {}x | take 1; u | count", "not ".repeat(100_000)),
        format!(
            "t | where {}x{} | take 1; u | count",
            "f(".repeat(100_000),
            ")".repeat(100_000)
        ),
    ] {
        let parse = parse(&text, FileId(0));
        assert_eq!(parse.syntax().text().to_string(), text);
        let codes: Vec<String> = parse.errors.iter().map(|d| d.code.to_string()).collect();
        assert_eq!(codes, ["DS0100"], "{}", &text[..20]);
        // The rest of the statement is skipped; the next statement parses.
        let Some(Stmt::Query(q)) = parse.program().statements().nth(1) else {
            panic!()
        };
        assert!(matches!(q.operators().last(), Some(Operator::Count(_))));
    }
}

/// Long flat chains: `a + a + …` and `a.b.b…` are built by folding, one tree level per operator.
fn chains(terms: usize) -> Vec<String> {
    vec![
        format!("t | where a{}", "+a".repeat(terms - 1)),
        format!("t | where a{}", " + a".repeat(terms - 1)),
        format!("t | where a{}", ".b".repeat(terms - 1)),
        format!("t | where a{}", "[0]".repeat(terms - 1)),
        format!("t | where a{}", " or a".repeat(terms - 1)),
        format!("t | where a{}", ".b + a".repeat(terms - 1)),
    ]
}

#[test]
fn realistic_chains_parse() {
    for text in chains(200) {
        let parse = parse(&text, FileId(0));
        assert!(parse.errors.is_empty(), "{:?}", parse.errors);
        check(&text).unwrap();
    }
}

#[test]
fn pathological_chains_give_one_error() {
    for chain in chains(100_000) {
        let text = format!("{chain} | take 1; u | count");
        // `check` walks the typed AST recursively and drops the tree.
        check(&text).unwrap();
        let parse = parse(&text, FileId(0));
        let codes: Vec<String> = parse.errors.iter().map(|d| d.code.to_string()).collect();
        assert_eq!(codes, ["DS0100"], "{}", &text[..20]);
        assert_eq!(
            parse.errors[0].message,
            "expression is too long",
            "{}",
            &text[..20]
        );
        let Some(Stmt::Query(q)) = parse.program().statements().nth(1) else {
            panic!()
        };
        assert!(matches!(q.operators().last(), Some(Operator::Count(_))));
    }
}

/// A chain inside brackets shares the budget with the brackets around it.
#[test]
fn chains_inside_nesting_give_one_error() {
    let text = format!(
        "t | where {}a{}{}; u | count",
        "(".repeat(200),
        " + a".repeat(100),
        ")".repeat(200)
    );
    check(&text).unwrap();
    let parse = parse(&text, FileId(0));
    let codes: Vec<String> = parse.errors.iter().map(|d| d.code.to_string()).collect();
    assert_eq!(codes, ["DS0100"]);
}

fn foreign_operator_lines(lines: usize) -> String {
    "T | where a && b;\n".repeat(lines)
}

#[test]
fn many_foreign_operator_lines_parse() {
    let text = foreign_operator_lines(8_000);
    check(&text).unwrap();
    let parse = parse(&text, FileId(0));
    assert_eq!(parse.errors.len(), 8_000);
    assert!(parse.errors.iter().all(|d| d.code.to_string() == "DS0111"));
}

/// Parsing time grows linearly with the input. Timing is noisy in debug builds and under load,
/// so this is opt-in: `cargo test --release -p datascript-syntax --test parser_props -- --ignored
/// --nocapture`. Numbers are recorded in the M1-18 task file.
#[test]
#[ignore = "timing; run in release with --ignored"]
fn long_input_parses_in_linear_time() {
    use std::time::{Duration, Instant};

    fn time(text: &str) -> Duration {
        (0..3)
            .map(|_| {
                let start = Instant::now();
                drop(parse(text, FileId(0)));
                start.elapsed()
            })
            .min()
            .unwrap()
    }

    type Make = Box<dyn Fn(usize) -> String>;
    let cases: Vec<(&str, Make)> = vec![
        (
            "a+a+…",
            Box::new(|n| format!("t | where a{}", "+a".repeat(n - 1))),
        ),
        (
            "a + a + …",
            Box::new(|n| format!("t | where a{}", " + a".repeat(n - 1))),
        ),
        (
            "a.b.b…",
            Box::new(|n| format!("t | where a{}", ".b".repeat(n - 1))),
        ),
        ("&& lines", Box::new(|n| foreign_operator_lines(n * 2 / 5))),
    ];
    for (name, make) in cases {
        let small = time(&make(5_000));
        let large = time(&make(20_000));
        let ratio = large.as_secs_f64() / small.as_secs_f64().max(1e-6);
        println!("{name}: 5k {small:?}, 20k {large:?}, ratio {ratio:.1}");
        // Linear is 4x; quadratic would be 16x.
        assert!(ratio < 8.0, "{name} looks superlinear: ratio {ratio:.1}");
    }
}

/// A long comparison chain is one error, not one per link.
#[test]
fn long_comparison_chain_gives_one_error() {
    let text = format!("t | where a{}", " < a".repeat(200));
    check(&text).unwrap();
    let parse = parse(&text, FileId(0));
    let codes: Vec<String> = parse.errors.iter().map(|d| d.code.to_string()).collect();
    assert_eq!(codes, ["DS0101"]);
    assert!(parse.errors[0].message.contains("cannot be chained"));
}
