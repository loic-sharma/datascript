//! Comparing driver output with expected sections.

use std::path::{MAIN_SEPARATOR, Path};

use similar::TextDiff;

use crate::case::{Order, normalize_body};

/// Rewrites absolute paths so output is the same on every machine: the test file's directory
/// becomes `<test>`, the repository root `<repo>` (TESTING.md § Determinism). Separators right
/// after either prefix become `/`.
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
                out.push_str(token);
                rest = after;
                if let Some(after_sep) = rest.strip_prefix(['/', '\\']) {
                    out.push('/');
                    rest = after_sep;
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
    let text = normalize_body(text);
    match order {
        Order::Strict => text,
        Order::Any => sort_table_rows(&text),
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

/// Sorts the data rows that follow each table separator, up to a blank line or a `render:` line.
fn sort_table_rows(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out: Vec<&str> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        out.push(line);
        i += 1;
        // The separator is the third line of a table: header, types, separator.
        if is_table_separator(line) && out.len() >= 3 {
            let start = i;
            while i < lines.len() && !lines[i].is_empty() && !lines[i].starts_with("render:") {
                i += 1;
            }
            let mut rows = lines[start..i].to_vec();
            rows.sort_unstable();
            out.extend(rows);
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
    fn any_order_sorts_only_table_rows() {
        let text = "warning: w\n\na | b\nlong | long\n--+--\n3 | 1\n1 | 2\nrender: table\n\nx\nstring\n-\nz\ny\n";
        assert_eq!(
            canonicalize(text, Order::Any),
            "warning: w\n\na | b\nlong | long\n--+--\n1 | 2\n3 | 1\nrender: table\n\nx\nstring\n-\ny\nz"
        );
        assert_eq!(canonicalize("b\r\na\r\n\r\n", Order::Strict), "b\na");
    }

    #[test]
    fn diff_is_unified() {
        assert_eq!(
            diff("a\nb", "a\nc"),
            "--- expected\n+++ actual\n@@ -1,2 +1,2 @@\n a\n-b\n+c\n"
        );
    }
}
