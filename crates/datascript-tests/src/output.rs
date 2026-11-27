//! Comparing driver output with expected sections.

use std::path::{MAIN_SEPARATOR, Path};

use similar::TextDiff;

use crate::case::{Order, normalize_body};

/// Rewrites absolute paths so output is the same on every machine: the test file's directory
/// becomes `<test>`, the repository root `<repo>` (TESTING.md § Determinism). Every `\` in a
/// rewritten `<test>/…` or `<repo>/…` path becomes `/`.
pub fn normalize_paths(output: &str, test_dir: &Path, repo_root: &Path) -> String {
    let mut prefixes: Vec<(String, &str)> = Vec::new();
    for (dir, token) in [(test_dir, "<test>"), (repo_root, "<repo>")] {
        let mut forms = vec![dir.to_path_buf()];
        if let Ok(canonical) = dir.canonicalize() {
            forms.push(canonical);
        }
        for form in forms {
            let s = form
                .to_string_lossy()
                .trim_end_matches(['/', '\\'])
                .to_string();
            if s.is_empty() {
                continue;
            }
            if MAIN_SEPARATOR == '\\' {
                prefixes.push((s.replace('\\', "/"), token));
                // `canonicalize` on Windows adds a verbatim prefix that output never shows.
                if let Some(stripped) = s.strip_prefix(r"\\?\") {
                    prefixes.push((stripped.to_string(), token));
                    prefixes.push((stripped.replace('\\', "/"), token));
                }
            }
            prefixes.push((s, token));
        }
    }
    // Longest first, so the test directory wins over the repository root that contains it.
    prefixes.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
    prefixes.dedup();

    let mut out = String::with_capacity(output.len());
    let mut rest = output;
    'outer: while !rest.is_empty() {
        for (prefix, token) in &prefixes {
            if let Some(after) = rest.strip_prefix(prefix.as_str()) {
                // Only whole path components: `/repo` must not match `/repository`.
                match after.chars().next() {
                    None => {}
                    Some('/' | '\\') => {}
                    Some(c) if !c.is_alphanumeric() && c != '_' && c != '-' && c != '.' => {}
                    Some(_) => continue,
                }
                let windows_path = prefix.contains('\\') || after.starts_with('\\');
                let prev_quote = out.chars().last().filter(|c| matches!(c, '`' | '\'' | '"'));
                out.push_str(token);
                rest = after;
                if windows_path && rest.starts_with(['/', '\\']) {
                    // Only treat a leading quote as delimiting spaces in the path when the quote
                    // is closed on the same line.
                    let closing_quote = prev_quote.filter(|&q| {
                        rest.split(['\n', '\r'])
                            .next()
                            .is_some_and(|line| line.contains(q))
                    });
                    // Turn every `\` in the rewritten Windows `<test>/...` or `<repo>/...` path
                    // into `/`. Unix paths never rewrite backslashes, so escapes like `\n` after
                    // a path are preserved.
                    let mut consumed = 0;
                    for (idx, c) in rest.char_indices() {
                        let stop = match closing_quote {
                            Some(q) => c == q || c == '\n' || c == '\r',
                            None => {
                                c.is_whitespace()
                                    || matches!(c, '`' | '\'' | '"' | ')' | ']' | '}' | ',' | ';')
                            }
                        };
                        if stop {
                            break;
                        }
                        out.push(if c == '\\' { '/' } else { c });
                        consumed = idx + c.len_utf8();
                    }
                    rest = &rest[consumed..];
                }
                continue 'outer;
            }
        }
        let c = rest.chars().next().expect("rest is not empty");
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// Prepares text for comparison: line endings and trailing blank lines are normalized, and with
/// `Order::Any` the data rows of every result table are sorted.
pub fn canonicalize(text: &str, order: Order) -> String {
    canonicalize_with_renders(text, order, None)
}

/// Like [`canonicalize`], using `table_renders` (one `bool` per query table, saying whether that
/// query ends with a `render` operator) to decide structurally whether a table's final `render:`
/// line is a chart footer rather than a data row.
pub fn canonicalize_with_renders(
    text: &str,
    order: Order,
    table_renders: Option<&[bool]>,
) -> String {
    let text = normalize_body(text);
    match order {
        Order::Strict => text,
        Order::Any => sort_table_rows(&text, table_renders),
    }
}

/// A table separator line: dashes, optionally split by `+` (`-----+------`).
fn is_table_separator(line: &str) -> bool {
    !line.is_empty()
        && line.starts_with('-')
        && line.ends_with('-')
        && line.chars().all(|c| c == '-' || c == '+')
        && !line.contains("++")
}

/// Whether `line` is a DataScript/Arrow type row matching a separator with `cols` columns.
fn is_type_row(line: &str, cols: usize) -> bool {
    let cells: Vec<&str> = line.split(" | ").collect();
    cells.len() == cols
        && cells.iter().all(|cell| {
            let ty = cell.trim();
            datascript_compiler::Type::from_name(ty).is_some() || ty.starts_with("arrow:")
        })
}

/// Whether three consecutive lines are the header, type row, and separator of a formatted table.
fn is_table_header(header: &str, types: &str, separator: &str) -> bool {
    !header.is_empty()
        && is_table_separator(separator)
        && is_type_row(types, separator.split('+').count())
}

/// Whether `rest` starts a new result table (after the blank line that separates tables).
fn starts_new_table(rest: &[&str]) -> bool {
    if rest.first() == Some(&"(no columns)") {
        return rest.len() == 1 || rest[1].is_empty() || is_render_summary(rest[1]);
    }
    rest.len() >= 3 && is_table_header(rest[0], rest[1], rest[2])
}

/// Whether `line` is a `RenderSpec::summary()` line (`render: <kind> ...`).
fn is_render_summary(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("render: ") else {
        return false;
    };
    let (kind, fields) = match rest.split_once(' ') {
        Some((k, f)) => (k, f),
        None => (rest, ""),
    };
    if datascript_compiler::ir::ChartKind::from_name(kind).is_none() {
        return false;
    }
    if fields.is_empty() {
        return true;
    }
    const KEYS: &[&str] = &[
        "x=",
        "y=[",
        "series=[",
        "title=\"",
        "xtitle=\"",
        "ytitle=\"",
        "legend=hidden",
        "ymin=",
        "ymax=",
        "stacking=stacked",
    ];
    KEYS.iter().any(|k| fields.starts_with(k))
}

/// Sorts the data rows that follow each table separator, up to the end of the table.
///
/// A table ends at EOF or at a blank line followed by another table; a final `render:` summary
/// line stays after the sorted rows when the query has a `render` operator (from `table_renders`,
/// or by shape when `table_renders` is `None`). Blank cells in 1-column tables and data cells
/// starting with `render:` stay part of the table's rows.
fn sort_table_rows(text: &str, table_renders: Option<&[bool]>) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out: Vec<&str> = Vec::with_capacity(lines.len());
    let mut table_index = 0usize;
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let at_block_start = i == 0 || lines[i - 1].is_empty();
        out.push(line);
        i += 1;
        if at_block_start
            && line == "(no columns)"
            && (i == lines.len() || lines[i].is_empty() || lines[i].starts_with("render: "))
        {
            table_index += 1;
            continue;
        }
        // The separator is the third line of a table: header, types, separator.
        if out.len() >= 3 && is_table_header(out[out.len() - 3], out[out.len() - 2], line) {
            let has_render = table_renders.and_then(|flags| flags.get(table_index).copied());
            table_index += 1;
            let start = i;
            while i < lines.len() && !(lines[i].is_empty() && starts_new_table(&lines[i + 1..])) {
                i += 1;
            }
            let is_footer = i > start
                && match has_render {
                    Some(true) => lines[i - 1].starts_with("render: "),
                    Some(false) => false,
                    None => is_render_summary(lines[i - 1]),
                };
            let (rows_end, render_line) = if is_footer {
                (i - 1, Some(lines[i - 1]))
            } else {
                (i, None)
            };
            let mut rows = lines[start..rows_end].to_vec();
            rows.sort_unstable();
            out.extend(rows);
            if let Some(render) = render_line {
                out.push(render);
            }
        }
    }
    out.join("\n")
}

/// A unified diff from `expected` to `actual`.
pub fn diff(expected: &str, actual: &str) -> String {
    let expected = with_final_newline(expected);
    let actual = with_final_newline(actual);
    TextDiff::from_lines(expected.as_str(), actual.as_str())
        .unified_diff()
        .context_radius(3)
        .header("expected", "actual")
        .to_string()
}

fn with_final_newline(s: &str) -> String {
    if s.is_empty() || s.ends_with('\n') {
        s.to_string()
    } else {
        format!("{s}\n")
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn paths_become_tokens() {
        let repo = PathBuf::from(format!("{MAIN_SEPARATOR}home{MAIN_SEPARATOR}repo"));
        let test = repo.join("tests").join("cases");
        let sep = MAIN_SEPARATOR;
        let out = format!(
            "a {t}{sep}x.csv b {r}{sep}README.md c {r} d {r}sitory{sep}x",
            t = test.display(),
            r = repo.display()
        );
        assert_eq!(
            normalize_paths(&out, &test, &repo),
            format!(
                "a <test>/x.csv b <repo>/README.md c <repo> d {}sitory{sep}x",
                repo.display()
            )
        );
    }

    #[test]
    fn windows_backslashes_in_rewritten_paths_all_become_slashes() {
        let repo = Path::new(r"C:\work\repo");
        let test = Path::new(r"C:\work\repo\tests\cases");
        let out = concat!(
            r"file `C:\work\repo\tests\cases\sub\deep\a.csv` not found; ",
            r"see `C:\work\repo\my dir\sub\b.csv` and C:\work\repo\x\y\z.csv ",
            r"plus C:\other\keep\slashes.csv",
        );
        assert_eq!(
            normalize_paths(out, test, repo),
            concat!(
                "file `<test>/sub/deep/a.csv` not found; ",
                "see `<repo>/my dir/sub/b.csv` and <repo>/x/y/z.csv ",
                r"plus C:\other\keep\slashes.csv",
            )
        );
    }

    #[test]
    fn unix_paths_preserve_escapes_after_path() {
        let repo = Path::new("/home/repo");
        let test = Path::new("/home/repo/tests/cases");
        let out = r#"cannot read /home/repo/tests/cases/a.csv\nhelp: check /home/repo/b.csv\tnow"#;
        assert_eq!(
            normalize_paths(out, test, repo),
            r#"cannot read <test>/a.csv\nhelp: check <repo>/b.csv\tnow"#
        );

        // An unclosed leading quote on Windows must not rewrite unrelated paths later on the line.
        let win_repo = Path::new(r"C:\work\repo");
        let win_test = Path::new(r"C:\work\repo\tests\cases");
        let unclosed = r"open 'C:\work\repo\sub\a.csv and C:\other\keep\b.csv";
        assert_eq!(
            normalize_paths(unclosed, win_test, win_repo),
            r"open '<repo>/sub/a.csv and C:\other\keep\b.csv"
        );
    }

    #[test]
    fn any_order_sorts_only_table_rows() {
        let text = "warning: w\n\na | b\nlong | long\n--+--\n3 | 1\n1 | 2\nrender: table\n\nx\nstring\n-\nz\ny\n";
        assert_eq!(
            canonicalize(text, Order::Any),
            "warning: w\n\na | b\nlong | long\n--+--\n1 | 2\n3 | 1\nrender: table\n\nx\nstring\n-\ny\nz"
        );
        assert_eq!(canonicalize("b\r\na\r\n\r\n", Order::Strict), "b\na");
    }

    #[test]
    fn any_order_keeps_blank_cells_in_single_column_tables() {
        // Blank cell in the middle of a 1-column table.
        let mid = "x\nstring\n------\nz\n\na\n";
        assert_eq!(canonicalize(mid, Order::Any), "x\nstring\n------\n\na\nz");

        // Blank cell at the end of table 1, followed by a blank separator line and table 2.
        let two_tables = "x\nstring\n------\nz\n\n\ny\nlong\n----\n2\n1\n";
        assert_eq!(
            canonicalize(two_tables, Order::Any),
            "x\nstring\n------\n\nz\n\ny\nlong\n----\n1\n2"
        );

        // Blank cell right before a real `render:` summary line.
        let with_render = "x\nstring\n------\nz\n\nrender: table\n";
        assert_eq!(
            canonicalize(with_render, Order::Any),
            "x\nstring\n------\n\nz\nrender: table"
        );
    }

    #[test]
    fn any_order_keeps_cells_starting_with_render() {
        // 1-column table with a data cell starting with `render:` (not a chart summary).
        let text = "msg\nstring\n-----------\nrender: foo\nalpha\n";
        assert_eq!(
            canonicalize(text, Order::Any),
            "msg\nstring\n-----------\nalpha\nrender: foo"
        );

        // 2-column table with a cell that starts with `render: table`.
        let two_col = "msg           | n\nstring        | long\n--------------+-----\nrender: table | 2\nalpha         | 1\n";
        assert_eq!(
            canonicalize(two_col, Order::Any),
            "msg           | n\nstring        | long\n--------------+-----\nalpha         | 1\nrender: table | 2"
        );

        // Data row `render: table` followed by another row and then a real `render: table` footer.
        let with_footer = "msg\nstring\n-------------\nrender: table\nalpha\nrender: table\n";
        assert_eq!(
            canonicalize(with_footer, Order::Any),
            "msg\nstring\n-------------\nalpha\nrender: table\nrender: table"
        );

        // When the query has no `render` operator (`Some(&[false])`), a data cell equal to
        // `render: table` is sorted identically regardless of whether it arrives first or last.
        let last = "msg\nstring\n-------------\nzeta\nrender: table\n";
        let first = "msg\nstring\n-------------\nrender: table\nzeta\n";
        assert_eq!(
            canonicalize_with_renders(last, Order::Any, Some(&[false])),
            canonicalize_with_renders(first, Order::Any, Some(&[false])),
        );
        assert_eq!(
            canonicalize_with_renders(last, Order::Any, Some(&[false])),
            "msg\nstring\n-------------\nrender: table\nzeta"
        );

        // When the query DOES end with `render` (`Some(&[true])`), the final `render:` line is
        // pinned as the footer while a data row `render: table` still sorts with `zeta`.
        let with_render_a = "msg\nstring\n-------------\nzeta\nrender: table\nrender: table\n";
        let with_render_b = "msg\nstring\n-------------\nrender: table\nzeta\nrender: table\n";
        assert_eq!(
            canonicalize_with_renders(with_render_a, Order::Any, Some(&[true])),
            canonicalize_with_renders(with_render_b, Order::Any, Some(&[true])),
        );
        assert_eq!(
            canonicalize_with_renders(with_render_a, Order::Any, Some(&[true])),
            "msg\nstring\n-------------\nrender: table\nzeta\nrender: table"
        );
    }

    #[test]
    fn diff_is_unified() {
        assert_eq!(
            diff("a\nb", "a\nc"),
            "--- expected\n+++ actual\n@@ -1,2 +1,2 @@\n a\n-b\n+c\n"
        );
    }
}
