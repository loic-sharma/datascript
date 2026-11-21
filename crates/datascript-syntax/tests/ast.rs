//! The typed AST accessors on valid programs.

use datascript_diagnostics::FileId;
use datascript_syntax::ast::*;
use datascript_syntax::parse;

fn query(text: &str) -> Pipeline {
    let parse = parse(text, FileId(0));
    assert!(parse.errors.is_empty(), "{:#?}", parse.errors);
    match parse.program().statements().last() {
        Some(Stmt::Query(pipeline)) => pipeline,
        other => panic!("expected a query, got {other:?}"),
    }
}

fn text(node: &impl AstNode) -> String {
    node.syntax().text().to_string()
}

fn ops(text: &str) -> Vec<Operator> {
    query(text).operators().collect()
}

#[test]
fn statements_and_lets() {
    let parse = parse("let n = 1; let t = 'a.csv' | take n; t", FileId(0));
    assert!(parse.errors.is_empty());
    let stmts: Vec<Stmt> = parse.program().statements().collect();
    let [Stmt::Let(scalar), Stmt::Let(tabular), Stmt::Query(q)] = &stmts[..] else {
        panic!("{stmts:?}")
    };
    assert_eq!(scalar.name().unwrap().text().unwrap(), "n");
    assert!(matches!(scalar.value(), Some(Expr::Literal(_))));
    assert!(matches!(tabular.value(), Some(Expr::Pipeline(_))));
    assert!(matches!(q.source(), Some(Expr::NameRef(_))));
    assert_eq!(q.operators().count(), 0);
}

#[test]
fn operator_names() {
    let names: Vec<String> = ops("t | project-away a | project b | order by c asc | limit 1")
        .iter()
        .map(Operator::name)
        .collect();
    assert_eq!(names, ["project-away", "project", "order", "limit"]);
}

#[test]
fn project_and_extend_items() {
    let [Operator::Project(p)] = &ops("t | project a, total = a + b, ['x y'] = c")[..] else {
        panic!()
    };
    let items: Vec<NamedExpr> = p.items().collect();
    assert_eq!(items.len(), 3);
    assert!(items[0].name().is_none());
    assert_eq!(items[1].name().unwrap().text().unwrap(), "total");
    assert!(matches!(items[1].expr(), Some(Expr::Bin(_))));
    assert_eq!(items[2].name().unwrap().text().unwrap(), "x y");
}

#[test]
fn project_away_and_rename() {
    let ops = ops("t | project-away x, tmp* | project-rename id = orderId");
    let Operator::ProjectAway(away) = &ops[0] else {
        panic!()
    };
    let patterns: Vec<(String, bool)> = away
        .patterns()
        .map(|p| (p.text(), p.has_wildcard()))
        .collect();
    assert_eq!(patterns, [("x".into(), false), ("tmp*".into(), true)]);
    let Operator::ProjectRename(rename) = &ops[1] else {
        panic!()
    };
    let item = rename.items().next().unwrap();
    assert_eq!(item.new_name().unwrap().text().unwrap(), "id");
    assert_eq!(item.old_name().unwrap().text().unwrap(), "orderId");
}

#[test]
fn summarize_parts() {
    let [Operator::Summarize(s)] = &ops("t | summarize n = count(), sum(x) by k, bin(ts, 1h)")[..]
    else {
        panic!()
    };
    assert_eq!(s.aggregates().count(), 2);
    let keys: Vec<String> = s.by().unwrap().keys().map(|k| text(&k)).collect();
    assert_eq!(keys, ["k", "bin(ts, 1h)"]);

    let [Operator::Summarize(s)] = &ops("t | summarize by k")[..] else {
        panic!()
    };
    assert_eq!(s.aggregates().count(), 0);
    assert_eq!(s.by().unwrap().keys().count(), 1);
}

#[test]
fn sort_and_top_keys() {
    let ops = ops("t | sort by a desc nulls last, b | top 10 by c asc");
    let Operator::Sort(sort) = &ops[0] else {
        panic!()
    };
    assert!(sort.by_token().is_some());
    let keys: Vec<(Option<SortDirection>, Option<NullsPlacement>)> =
        sort.keys().map(|k| (k.direction(), k.nulls())).collect();
    assert_eq!(
        keys,
        [
            (Some(SortDirection::Desc), Some(NullsPlacement::Last)),
            (None, None)
        ]
    );
    let Operator::Top(top) = &ops[1] else {
        panic!()
    };
    assert_eq!(text(&top.count().unwrap()), "10");
    assert_eq!(top.key().unwrap().direction(), Some(SortDirection::Asc));
}

/// `DESC` in caps is reported (DS0105) but still sorts descending.
#[test]
fn sort_direction_in_caps() {
    let parse = parse("t | sort by a DESC, b Asc", FileId(0));
    assert!(parse.errors.iter().all(|d| d.code.to_string() == "DS0105"));
    let Some(Stmt::Query(q)) = parse.program().statements().next() else {
        panic!()
    };
    let Some(Operator::Sort(sort)) = q.operators().next() else {
        panic!()
    };
    let directions: Vec<Option<SortDirection>> = sort.keys().map(|k| k.direction()).collect();
    assert_eq!(
        directions,
        [Some(SortDirection::Desc), Some(SortDirection::Asc)]
    );
}

/// A method-style call is a call with no name, so its arguments can still be walked.
#[test]
fn method_call_is_a_call_without_callee() {
    let parse = parse("t | extend x = a.b(1, 2)", FileId(0));
    let codes: Vec<String> = parse.errors.iter().map(|d| d.code.to_string()).collect();
    assert_eq!(codes, ["DS0113"]);
    let Some(Stmt::Query(q)) = parse.program().statements().next() else {
        panic!()
    };
    let Some(Operator::Extend(extend)) = q.operators().next() else {
        panic!()
    };
    let Some(Expr::Call(call)) = extend.items().next().and_then(|i| i.expr()) else {
        panic!()
    };
    assert!(call.callee().is_none());
    assert_eq!(call.args().count(), 2);
}

#[test]
fn take_count_distinct() {
    let ops = ops("t | take 5 | count | distinct * | distinct a, b | getschema");
    let Operator::Take(take) = &ops[0] else {
        panic!()
    };
    assert_eq!(text(&take.count().unwrap()), "5");
    assert!(matches!(ops[1], Operator::Count(_)));
    let Operator::Distinct(star) = &ops[2] else {
        panic!()
    };
    assert!(star.star_token().is_some());
    let Operator::Distinct(cols) = &ops[3] else {
        panic!()
    };
    assert_eq!(cols.columns().count(), 2);
    assert!(matches!(ops[4], Operator::GetSchema(_)));
}

#[test]
fn join_parts() {
    let [Operator::Join(j)] =
        &ops("t | join kind = leftouter (u | take 1) on k, $left.a == $right.b")[..]
    else {
        panic!()
    };
    assert_eq!(j.kind().unwrap().value_token().unwrap().text(), "leftouter");
    let Some(Expr::Paren(right)) = j.right() else {
        panic!()
    };
    assert!(matches!(right.inner(), Some(Expr::Pipeline(_))));
    let conds: Vec<String> = j.conditions().map(|c| text(&c)).collect();
    assert_eq!(conds, ["k", "$left.a == $right.b"]);

    let [Operator::Join(j)] = &ops("t | join 'c.csv' on id")[..] else {
        panic!()
    };
    assert!(j.kind().is_none());
    assert!(matches!(j.right(), Some(Expr::Literal(_))));
}

#[test]
fn union_and_render() {
    let ops = ops("t | union u, 'v.csv' | render timechart with (title = 'x', ycolumns = a, b)");
    let Operator::Union(u) = &ops[0] else {
        panic!()
    };
    assert_eq!(u.tables().count(), 2);
    let Operator::Render(r) = &ops[1] else {
        panic!()
    };
    assert_eq!(r.kind_token().unwrap().text(), "timechart");
    let props: Vec<(String, usize)> = r
        .properties()
        .map(|p| (p.name().unwrap().text().unwrap(), p.values().count()))
        .collect();
    assert_eq!(props, [("title".into(), 1), ("ycolumns".into(), 2)]);
}

fn where_expr(text: &str) -> Expr {
    let [Operator::Where(w)] = &ops(&format!("t | where {text}"))[..] else {
        panic!()
    };
    w.predicate().unwrap()
}

#[test]
fn binary_and_prefix_expressions() {
    let Expr::Prefix(not) = where_expr("not a == b") else {
        panic!()
    };
    assert_eq!(not.op(), Some(PrefixOp::Not));
    let Some(Expr::Bin(eq)) = not.operand() else {
        panic!()
    };
    assert_eq!(eq.op(), Some(BinaryOp::Eq));
    assert_eq!(text(&eq.lhs().unwrap()), "a");
    assert_eq!(text(&eq.rhs().unwrap()), "b");

    let Expr::Bin(sub) = where_expr("a - -b") else {
        panic!()
    };
    assert_eq!(sub.op(), Some(BinaryOp::Sub));
    let Some(Expr::Prefix(neg)) = sub.rhs() else {
        panic!()
    };
    assert_eq!(neg.op(), Some(PrefixOp::Neg));

    let ops: Vec<Option<BinaryOp>> = [
        "a !has 'x'",
        "a contains_cs 'x'",
        "a matches regex 'x'",
        "a =~ 'x'",
    ]
    .iter()
    .map(|t| match where_expr(t) {
        Expr::Bin(b) => b.op(),
        _ => None,
    })
    .collect();
    assert_eq!(
        ops,
        [
            Some(BinaryOp::String {
                op: StringOp::Has,
                negated: true
            }),
            Some(BinaryOp::String {
                op: StringOp::ContainsCs,
                negated: false
            }),
            Some(BinaryOp::MatchesRegex),
            Some(BinaryOp::EqCi),
        ]
    );
}

#[test]
fn in_and_between() {
    let Expr::In(i) = where_expr("x !in~ ('a', 'b')") else {
        panic!()
    };
    assert!(i.negated() && i.case_insensitive());
    assert_eq!(text(&i.lhs().unwrap()), "x");
    assert_eq!(i.list().unwrap().args().count(), 2);

    let Expr::Between(b) = where_expr("x between (1 .. 10)") else {
        panic!()
    };
    assert!(!b.negated());
    assert_eq!(text(&b.low().unwrap()), "1");
    assert_eq!(text(&b.high().unwrap()), "10");
}

#[test]
fn literals_calls_fields() {
    let [Operator::Extend(e)] = &ops(
        "t | extend a = 1.5h, b = long(null), c = round(x, digits = 2), d = e.f, g = h[0], i = datetime(null)",
    )[..] else {
        panic!()
    };
    let exprs: Vec<Expr> = e.items().filter_map(|i| i.expr()).collect();
    let Expr::Literal(ts) = &exprs[0] else {
        panic!()
    };
    assert_eq!(ts.kind(), Some(LiteralKind::Timespan));
    let Expr::TypedNull(tn) = &exprs[1] else {
        panic!()
    };
    assert_eq!(tn.type_token().unwrap().text(), "long");
    let Expr::Call(call) = &exprs[2] else {
        panic!()
    };
    assert_eq!(call.callee().unwrap().text().unwrap(), "round");
    let args: Vec<Arg> = call.args().collect();
    assert!(matches!(&args[..], [Arg::Positional(_), Arg::Named(_)]));
    let Expr::Field(field) = &exprs[3] else {
        panic!()
    };
    assert_eq!(text(&field.base().unwrap()), "e");
    assert_eq!(field.field().unwrap().text().unwrap(), "f");
    let Expr::Index(index) = &exprs[4] else {
        panic!()
    };
    assert_eq!(text(&index.index().unwrap()), "0");
    let Expr::Literal(dt) = &exprs[5] else {
        panic!()
    };
    assert_eq!(dt.kind(), Some(LiteralKind::Datetime));
}

#[test]
fn spans_exclude_trivia() {
    let q = query("  t | where  x > 1  // done\n");
    let where_op = q.operators().next().unwrap();
    let span = where_op.span(FileId(3));
    assert_eq!((span.start, span.end), (6, 18));
    assert_eq!(span.file, FileId(3));
}

#[test]
fn from_token_and_source() {
    let q = query("from 'a.csv' | take 1");
    let from = q.from_token().expect("a `from` token");
    assert_eq!(from.text(), "from");
    assert_eq!(u32::from(from.text_range().start()), 0);
    assert!(matches!(q.source(), Some(Expr::Literal(_))));
    assert_eq!(q.operators().count(), 1);

    // `from from`: the second `from` is the source, a name.
    let q = query("from from");
    assert!(q.from_token().is_some());
    let Some(Expr::NameRef(name)) = q.source() else {
        panic!("{:?}", q.source())
    };
    assert_eq!(name.text().unwrap(), "from");
}

#[test]
fn let_kind_is_syntactic() {
    let parse = parse(
        "let a = from t; let b = (from t | where x); let c = 'p.csv'; let d = other; \
         let e = (from t | count) + 1; from a",
        FileId(0),
    );
    assert!(parse.errors.is_empty(), "{:#?}", parse.errors);
    let kinds: Vec<bool> = parse
        .program()
        .statements()
        .filter_map(|stmt| match stmt {
            Stmt::Let(stmt) => Some(stmt.is_tabular()),
            Stmt::Query(_) => None,
        })
        .collect();
    assert_eq!(kinds, [true, true, false, false, false]);
}
