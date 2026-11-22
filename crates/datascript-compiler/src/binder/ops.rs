//! Operator schemas (M1-16): the columns after each operator, and the column naming rules of
//! LANGUAGE.md §3.3. `join` is in [`super::join`].

use std::collections::{BTreeSet, HashMap, HashSet};

use datascript_diagnostics::codes::{DS1001, DS1005, DS1032, DS2003, DS2030};
use datascript_diagnostics::{Diagnostic, Span, Suggestion, did_you_mean};
use datascript_syntax::ast::{self, AstNode, SortDirection};

use super::{Agg, Binder, Cols, Cx};
use crate::Type;
use crate::hir::{Column, Expr, ExprKind, Op, ProjectItem, SortKey, Source};

/// How an output column got its name.
#[derive(Debug, Clone)]
pub(super) enum Naming {
    /// Written by the user: `name = expr`, or a bare column `a`. Two of these with the same name
    /// are `DS1005`.
    Explicit {
        name: String,
        span: Span,
        bare: bool,
    },
    /// Derived from the expression (`sum_amount`, a `by` key's column). On a collision it gets
    /// `1`, `2`, … appended.
    Derived(String),
    /// Nothing to derive it from: `Column1`, `Column2`, ….
    Generated,
}

/// The smallest `base`, `base1`, `base2`, … not in `taken`.
pub(crate) fn unique_name(base: &str, taken: &HashSet<String>) -> String {
    if !taken.contains(base) {
        return base.to_string();
    }
    (1..)
        .map(|n| format!("{base}{n}"))
        .find(|name| !taken.contains(name))
        .expect("an unused name exists")
}

/// The columns an expression reads, not counting those inside `let` values.
fn columns_read(expr: &Expr) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    fn walk(e: &Expr, out: &mut BTreeSet<String>) {
        match &e.kind {
            ExprKind::Column(name) => {
                out.insert(name.clone());
            }
            ExprKind::Let { .. } => {}
            _ => e.children().into_iter().for_each(|c| walk(c, out)),
        }
    }
    walk(expr, &mut out);
    out
}

impl Binder<'_> {
    /// Names a list of output columns. `reserved` are names generated names must avoid (an
    /// `extend`'s input columns). Explicit duplicates are `DS1005` and give `None`: the item is
    /// dropped, so later operators still see one column of that name.
    pub(super) fn assign_names(
        &mut self,
        namings: &[Naming],
        reserved: &HashSet<String>,
    ) -> Vec<Option<String>> {
        let mut names = vec![None; namings.len()];
        let mut taken = reserved.clone();
        let mut first: HashMap<String, Span> = HashMap::new();
        for (i, naming) in namings.iter().enumerate() {
            let Naming::Explicit { name, span, bare } = naming else {
                continue;
            };
            if let Some(first_span) = first.get(name) {
                self.duplicate_name(name, *span, *first_span, *bare);
                continue;
            }
            first.insert(name.clone(), *span);
            taken.insert(name.clone());
            names[i] = Some(name.clone());
        }
        let mut generated = 0;
        for (i, naming) in namings.iter().enumerate() {
            let name = match naming {
                Naming::Explicit { .. } => continue,
                Naming::Derived(base) => unique_name(base, &taken),
                Naming::Generated => loop {
                    generated += 1;
                    let name = format!("Column{generated}");
                    if !taken.contains(&name) {
                        break name;
                    }
                },
            };
            taken.insert(name.clone());
            names[i] = Some(name);
        }
        names
    }

    fn duplicate_name(&mut self, name: &str, span: Span, first: Span, bare: bool) {
        let d = Diagnostic::error(DS1005, format!("duplicate column name `{name}`"))
            .primary(span, format!("a second column named `{name}`"))
            .secondary(first, "another column with this name");
        let d = if bare {
            d.help(format!("remove the repeated `{name}`"))
        } else {
            d.help(format!(
                "output column names must be unique; give this one another name, such as \
                 `{name}2`"
            ))
        };
        self.diagnostics.push(d);
    }

    /// How a `project`, `extend` or `by` item is named, before binding it.
    fn naming_of(&self, item: &ast::NamedExpr) -> Option<Naming> {
        if let Some(name) = item.name() {
            return Some(Naming::Explicit {
                name: name.text()?,
                span: self.span(&name),
                bare: false,
            });
        }
        match item.expr()? {
            ast::Expr::NameRef(name_ref) => Some(Naming::Explicit {
                name: name_ref.text()?,
                span: self.span(&name_ref),
                bare: true,
            }),
            _ => Some(Naming::Generated),
        }
    }

    /// The input column `name`, for where its values were read from.
    fn read_from(columns: &[Column], expr: &Expr) -> Option<String> {
        match &expr.kind {
            ExprKind::Column(name) => columns
                .iter()
                .find(|c| &c.name == name)
                .and_then(|c| c.read_from.clone()),
            _ => None,
        }
    }

    // --- project, project-away, project-rename ---

    pub(super) fn bind_project(
        &mut self,
        op: &ast::ProjectOp,
        columns: &Option<Vec<Column>>,
    ) -> (Op, Option<Vec<Column>>) {
        let cx = Cx {
            cols: Cols::of(columns),
            agg: Agg::Row("`project`"),
        };
        let mut namings = Vec::new();
        let mut exprs = Vec::new();
        let mut arrow_onlys = Vec::new();
        for item in op.items() {
            let (Some(naming), Some(expr)) = (self.naming_of(&item), item.expr()) else {
                continue;
            };
            // A column with no DataScript type can be projected as it is (DS2008 otherwise).
            let arrow_only = match (&expr, columns) {
                (ast::Expr::NameRef(name_ref), Some(cs)) => name_ref.text().and_then(|n| {
                    cs.iter()
                        .find(|c| c.name == n)
                        .and_then(|c| c.arrow_only.clone())
                }),
                _ => None,
            };
            let bound = match (&arrow_only, &expr) {
                (Some(_), ast::Expr::NameRef(name_ref)) => Expr {
                    kind: ExprKind::Column(name_ref.text().unwrap_or_default()),
                    ty: Type::Error,
                    span: self.span(&expr),
                },
                _ => {
                    let bound = self.expr(&expr, &cx);
                    self.value_sink(bound)
                }
            };
            exprs.push(bound);
            arrow_onlys.push(arrow_only);
            namings.push(naming);
        }
        let names = self.assign_names(&namings, &HashSet::new());
        let mut items = Vec::new();
        let mut out = Vec::new();
        for ((name, expr), arrow_only) in names.into_iter().zip(exprs).zip(arrow_onlys) {
            let Some(name) = name else { continue };
            let read_from = columns.as_deref().and_then(|cs| Self::read_from(cs, &expr));
            out.push(Column {
                name: name.clone(),
                ty: expr.ty,
                read_from,
                arrow_only,
            });
            items.push(ProjectItem { name, expr });
        }
        let out = columns.as_ref().map(|_| out);
        (Op::Project(items), out)
    }

    pub(super) fn bind_project_away(
        &mut self,
        op: &ast::ProjectAwayOp,
        columns: &Option<Vec<Column>>,
    ) -> (Op, Option<Vec<Column>>) {
        let span = self.span(op);
        let Some(columns) = columns else {
            return (Op::Error, None);
        };
        let mut removed = HashSet::new();
        for pattern in op.patterns() {
            let text = pattern.text();
            let matching: Vec<&Column> = columns
                .iter()
                .filter(|c| wildcard_match(&text, &c.name))
                .collect();
            if matching.is_empty() && !pattern.has_wildcard() {
                let pattern_span = self.span(&pattern);
                self.unknown_column(pattern_span, &text, columns);
            }
            removed.extend(matching.into_iter().map(|c| c.name.clone()));
        }
        let kept: Vec<Column> = columns
            .iter()
            .filter(|c| !removed.contains(&c.name))
            .cloned()
            .collect();
        let items = kept.iter().map(|c| column_item(c, span)).collect();
        (Op::Project(items), Some(kept))
    }

    pub(super) fn bind_project_rename(
        &mut self,
        op: &ast::ProjectRenameOp,
        columns: &Option<Vec<Column>>,
    ) -> (Op, Option<Vec<Column>>) {
        let span = self.span(op);
        let Some(columns) = columns else {
            return (Op::Error, None);
        };
        // (output name, input column), applied one rename at a time.
        let mut current: Vec<(String, Column)> = columns
            .iter()
            .map(|c| (c.name.clone(), c.clone()))
            .collect();
        for item in op.items() {
            let (Some(new), Some(old)) = (item.new_name(), item.old_name()) else {
                continue;
            };
            let (Some(new_name), Some(old_name)) = (new.text(), old.text()) else {
                continue;
            };
            let old_span = self.span(&old);
            let Some(index) = current.iter().position(|(n, _)| *n == old_name) else {
                let now: Vec<Column> = current
                    .iter()
                    .map(|(n, c)| Column {
                        name: n.clone(),
                        ..c.clone()
                    })
                    .collect();
                self.unknown_column(old_span, &old_name, &now);
                continue;
            };
            if new_name != old_name && current.iter().any(|(n, _)| *n == new_name) {
                let new_span = self.span(&new);
                self.diagnostics.push(
                    Diagnostic::error(DS1005, format!("duplicate column name `{new_name}`"))
                        .primary(new_span, format!("there is already a column `{new_name}`"))
                        .help(format!(
                            "pick a name no other column has, or remove `{new_name}` first with \
                             `project-away {new_name}`"
                        )),
                );
                continue;
            }
            current[index].0 = new_name;
        }
        let mut items = Vec::new();
        let mut out = Vec::new();
        for (name, column) in current {
            let mut item = column_item(&column, span);
            item.name = name.clone();
            items.push(item);
            out.push(Column { name, ..column });
        }
        (Op::Project(items), Some(out))
    }

    // --- extend ---

    pub(super) fn bind_extend(
        &mut self,
        op: &ast::ExtendOp,
        columns: &Option<Vec<Column>>,
    ) -> (Op, Option<Vec<Column>>) {
        let ast_items: Vec<(Naming, ast::Expr)> = op
            .items()
            .filter_map(|item| Some((self.naming_of(&item)?, item.expr()?)))
            .collect();
        let namings: Vec<Naming> = ast_items.iter().map(|(n, _)| n.clone()).collect();
        let reserved: HashSet<String> = columns.iter().flatten().map(|c| c.name.clone()).collect();
        let names = self.assign_names(&namings, &reserved);
        let mut current = columns.clone();
        let mut items = Vec::new();
        for ((_, expr), name) in ast_items.iter().zip(names) {
            let cx = Cx {
                cols: Cols::of(&current),
                agg: Agg::Row("`extend`"),
            };
            let bound = self.expr(expr, &cx);
            let bound = self.value_sink(bound);
            let Some(name) = name else { continue };
            if let Some(cols) = &mut current {
                let column = Column {
                    name: name.clone(),
                    ty: bound.ty,
                    read_from: Self::read_from(cols, &bound),
                    arrow_only: None,
                };
                match cols.iter_mut().find(|c| c.name == name) {
                    Some(existing) => *existing = column,
                    None => cols.push(column),
                }
            }
            items.push(ProjectItem { name, expr: bound });
        }
        (Op::Extend(items), current)
    }

    // --- summarize ---

    pub(super) fn bind_summarize(
        &mut self,
        op: &ast::SummarizeOp,
        columns: &Option<Vec<Column>>,
    ) -> (Op, Option<Vec<Column>>) {
        let row = Cx {
            cols: Cols::of(columns),
            agg: Agg::Row("`by`"),
        };
        let mut namings = Vec::new();
        let mut exprs = Vec::new();
        let mut keys = 0;
        for key in op.by().into_iter().flat_map(|by| by.keys()) {
            let (Some(naming), Some(expr)) = (self.naming_of(&key), key.expr()) else {
                continue;
            };
            let bound = self.expr(&expr, &row);
            let bound = self.value_sink(bound);
            let naming = match naming {
                Naming::Generated => match columns_read(&bound).into_iter().collect::<Vec<_>>()[..]
                {
                    [ref one] => Naming::Derived(one.clone()),
                    _ => Naming::Generated,
                },
                other => other,
            };
            namings.push(naming);
            exprs.push(bound);
            keys += 1;
        }
        for item in op.aggregates() {
            let Some(expr) = item.expr() else { continue };
            let bound = self.aggregate_item(&expr, Cols::of(columns));
            let naming = match item.name() {
                Some(name) => match name.text() {
                    Some(text) => Naming::Explicit {
                        name: text,
                        span: self.span(&name),
                        bare: false,
                    },
                    None => continue,
                },
                None => self.aggregate_naming(&bound),
            };
            namings.push(naming);
            exprs.push(bound);
        }
        if namings.is_empty() {
            if !self.broken.contains(op.syntax()) {
                let span = self.span(op);
                self.diagnostics.push(
                    Diagnostic::error(DS2030, "`summarize` has nothing to compute")
                        .primary(span, "no aggregate and no `by` key")
                        .help("add an aggregate, a key, or both: `summarize count() by key`"),
                );
            }
            return (Op::Error, None);
        }
        let names = self.assign_names(&namings, &HashSet::new());
        let mut key_items = Vec::new();
        let mut aggregates = Vec::new();
        let mut out = Vec::new();
        for (i, (name, expr)) in names.into_iter().zip(exprs).enumerate() {
            let Some(name) = name else { continue };
            let read_from = match (columns, i < keys) {
                (Some(cs), true) => Self::read_from(cs, &expr),
                _ => None,
            };
            out.push(Column {
                name: name.clone(),
                ty: expr.ty,
                read_from,
                arrow_only: None,
            });
            let item = ProjectItem { name, expr };
            if i < keys {
                key_items.push(item);
            } else {
                aggregates.push(item);
            }
        }
        let out = columns.as_ref().map(|_| out);
        (
            Op::Summarize {
                keys: key_items,
                aggregates,
            },
            out,
        )
    }

    /// The name of an unnamed aggregate item (LANGUAGE.md §3.3): `sum(amount)` → `sum_amount`,
    /// any other aggregate call → `sum_`, `count()` → `count_`; anything else `Column1`, ….
    fn aggregate_naming(&self, bound: &Expr) -> Naming {
        let mut expr = bound;
        while let ExprKind::Cast(inner) = &expr.kind {
            expr = inner;
        }
        match &expr.kind {
            ExprKind::Call { function, args }
                if self
                    .catalog
                    .lookup(function)
                    .is_ok_and(|f| f.kind == crate::FunctionKind::Aggregate) =>
            {
                let column = match args.as_slice() {
                    [arg] => {
                        let mut arg = arg;
                        while let ExprKind::Cast(inner) = &arg.kind {
                            arg = inner;
                        }
                        match &arg.kind {
                            ExprKind::Column(name) => name.as_str(),
                            _ => "",
                        }
                    }
                    _ => "",
                };
                Naming::Derived(format!("{function}_{column}"))
            }
            _ => Naming::Generated,
        }
    }

    // --- sort, top ---

    pub(super) fn bind_sort_key(
        &mut self,
        key: &ast::SortKey,
        columns: &Option<Vec<Column>>,
        place: &'static str,
        default_descending: bool,
    ) -> Option<SortKey> {
        let expr = key.expr()?;
        let cx = Cx {
            cols: Cols::of(columns),
            agg: Agg::Row(place),
        };
        let bound = self.expr(&expr, &cx);
        let bound = self.value_sink(bound);
        let direction = key.direction();
        let descending = match direction {
            Some(SortDirection::Desc) => true,
            Some(SortDirection::Asc) => false,
            None => default_descending,
        };
        let nulls_first = match key.nulls() {
            Some(ast::NullsPlacement::First) => true,
            Some(ast::NullsPlacement::Last) => false,
            None => !descending,
        };
        Some(SortKey {
            expr: bound,
            descending,
            nulls_first,
            explicit_direction: direction.is_some(),
        })
    }

    // --- distinct ---

    pub(super) fn bind_distinct(
        &mut self,
        op: &ast::DistinctOp,
        columns: &Option<Vec<Column>>,
    ) -> (Op, Option<Vec<Column>>) {
        if op.star_token().is_some() {
            let names = columns.iter().flatten().map(|c| c.name.clone()).collect();
            return (Op::Distinct(names), columns.clone());
        }
        let cx = Cx {
            cols: Cols::of(columns),
            agg: Agg::Row("`distinct`"),
        };
        let mut names: Vec<String> = Vec::new();
        let mut out = Vec::new();
        let mut first: HashMap<String, Span> = HashMap::new();
        let mut known = true;
        for expr in op.columns() {
            let span = self.span(&expr);
            let bound = self.expr(&expr, &cx);
            let ExprKind::Column(name) = &bound.kind else {
                // An unknown column was reported, and an expression is reported here. Either
                // way the output is not known, so later operators report nothing more.
                known = false;
                if !bound.ty.is_error() {
                    let text = &self.text[span.range()];
                    let label = match &bound.kind {
                        ExprKind::Let { .. } => "this is a `let`, not a column",
                        _ => "this is an expression, not a column",
                    };
                    self.diagnostics.push(
                        Diagnostic::error(DS1032, "`distinct` takes column names")
                            .primary(span, label)
                            .help(format!(
                                "compute it first and name it: `extend value = {text} | distinct \
                                 value`"
                            )),
                    );
                }
                continue;
            };
            if let Some(first_span) = first.get(name) {
                self.duplicate_name(name, span, *first_span, true);
                continue;
            }
            first.insert(name.clone(), span);
            if let Some(column) = columns.iter().flatten().find(|c| &c.name == name) {
                out.push(column.clone());
            }
            names.push(name.clone());
        }
        let out = columns.as_ref().filter(|_| known).map(|_| out);
        (Op::Distinct(names), out)
    }

    // --- union ---

    pub(super) fn bind_union(
        &mut self,
        op: &ast::UnionOp,
        columns: &Option<Vec<Column>>,
    ) -> (Op, Option<Vec<Column>>) {
        let mut merged = columns.clone();
        let mut sources = Vec::new();
        for table in op.tables() {
            let table_span = self.span(&table);
            let (source, table_columns) = self.bind_source(&table);
            sources.push(source);
            let (Some(out), Some(table_columns)) = (&mut merged, table_columns) else {
                merged = None;
                continue;
            };
            for column in table_columns {
                let Some(existing) = out.iter_mut().find(|c| c.name == column.name) else {
                    out.push(column);
                    continue;
                };
                match existing.ty.common(column.ty) {
                    Some(ty) => existing.ty = ty,
                    None => {
                        let d = self.union_conflict(table_span, existing, &column);
                        self.diagnostics.push(d);
                        existing.ty = Type::Error;
                    }
                }
                if existing.read_from != column.read_from {
                    existing.read_from = None;
                }
            }
        }
        if sources.iter().any(|s| matches!(s, Source::Error { .. })) {
            return (Op::Error, None);
        }
        (Op::Union(sources), merged)
    }

    fn union_conflict(&self, span: Span, before: &Column, column: &Column) -> Diagnostic {
        let name = &column.name;
        let (was, is) = (before.ty, column.ty);
        let mut d = Diagnostic::error(
            DS2003,
            format!("`union` cannot combine column `{name}` of types `{was}` and `{is}`"),
        )
        .primary(span, format!("here `{name}` is `{is}`"));
        let before_from = match &before.read_from {
            Some(source) => format!(" (read from {source})"),
            None => String::new(),
        };
        d = d.note(format!(
            "before this table, `{name}` is `{was}`{before_from}"
        ));
        let convert = format!("to{was}");
        if self.catalog.lookup(&convert).is_ok() {
            let table = &self.text[span.range()];
            let inner = paren_pipeline(table, &format!("extend {name} = {convert}({name})"));
            d = d.help(format!(
                "convert it to one type before the `union`: `union {inner}`"
            ));
        }
        d
    }

    // --- count, getschema ---

    pub(super) fn count_columns() -> Vec<Column> {
        vec![Column {
            name: "Count".to_string(),
            ty: Type::Long,
            read_from: None,
            arrow_only: None,
        }]
    }

    pub(super) fn getschema_columns() -> Vec<Column> {
        [
            ("ColumnName", Type::String),
            ("ColumnOrdinal", Type::Long),
            ("DataType", Type::String),
        ]
        .into_iter()
        .map(|(name, ty)| Column {
            name: name.to_string(),
            ty,
            read_from: None,
            arrow_only: None,
        })
        .collect()
    }

    /// `DS1001` for `name`, which is not one of `columns`, with did-you-mean over them and a note
    /// that lists them. `side` names the table when there are two (`join`).
    pub(super) fn missing_column(
        &mut self,
        span: Span,
        name: &str,
        columns: &[Column],
        side: &str,
    ) {
        let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
        let mut d = Diagnostic::error(DS1001, format!("the {side} table has no column `{name}`"))
            .primary(span, format!("not a column of the {side} table"));
        let suggestions = did_you_mean(name, names.iter().copied());
        if let Some(best) = suggestions.single() {
            d = d.help_with(
                format!("a column with a similar name exists: `{best}`"),
                Suggestion::replace(span, best),
            );
        }
        let note = if names.is_empty() {
            format!("the {side} table has no columns")
        } else {
            format!("columns of the {side} table: {}", names.join(", "))
        };
        self.diagnostics.push(d.note(note));
    }
}

/// The text of a right table (`customers`, `'c.csv'`, `(from c | where x)`) with `stage`
/// appended, as a parenthesized pipeline.
pub(super) fn paren_pipeline(table: &str, stage: &str) -> String {
    let table = table.trim();
    if let Some(inner) = table.strip_prefix('(').and_then(|t| t.strip_suffix(')')) {
        format!("({} | {stage})", inner.trim())
    } else {
        format!("(from {table} | {stage})")
    }
}

/// A `project` item that keeps an input column, named `column.name`.
fn column_item(column: &Column, span: Span) -> ProjectItem {
    ProjectItem {
        name: column.name.clone(),
        expr: Expr {
            kind: ExprKind::Column(column.name.clone()),
            ty: column.ty,
            span,
        },
    }
}

/// Whether `name` matches `pattern`, where `*` matches any run of characters.
fn wildcard_match(pattern: &str, name: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == name;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !name.starts_with(first) || name.len() < first.len() + last.len() {
        return false;
    }
    let mut rest = &name[first.len()..name.len() - last.len()];
    if !name.ends_with(last) {
        return false;
    }
    for part in &parts[1..parts.len() - 1] {
        match rest.find(part) {
            Some(i) => rest = &rest[i + part.len()..],
            None => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards() {
        assert!(wildcard_match("a*", "abc"));
        assert!(wildcard_match("*_id", "customer_id"));
        assert!(wildcard_match("*", "x"));
        assert!(wildcard_match("a*c*e", "abcde"));
        assert!(!wildcard_match("a*c*e", "abde"));
        assert!(!wildcard_match("ab*ba", "aba"));
        assert!(wildcard_match("abc", "abc"));
        assert!(!wildcard_match("abc", "abcd"));
    }

    #[test]
    fn unique_names() {
        let taken: HashSet<String> = ["a", "a1"].into_iter().map(String::from).collect();
        assert_eq!(unique_name("a", &taken), "a2");
        assert_eq!(unique_name("b", &taken), "b");
    }
}
