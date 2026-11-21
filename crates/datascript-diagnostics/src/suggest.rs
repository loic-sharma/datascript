//! "Did you mean" suggestions over the names in scope (DIAGNOSTICS.md, style rule 9).

/// The names close enough to an unknown name to suggest, best first.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Suggestions {
    names: Vec<String>,
}

impl Suggestions {
    /// At most this many names are suggested.
    pub const MAX: usize = 3;

    /// Whether nothing is close enough.
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// The single clear winner, if there is exactly one suggestion.
    pub fn single(&self) -> Option<&str> {
        match self.names.as_slice() {
            [name] => Some(name),
            _ => None,
        }
    }

    /// The suggested names, best first, at most [`Suggestions::MAX`].
    pub fn names(&self) -> &[String] {
        &self.names
    }
}

/// Finds names in `in_scope` that the user may have meant when writing `unknown`.
///
/// - A name equal to `unknown` ignoring case wins outright (several such names are all listed).
/// - Otherwise names within a Damerau-Levenshtein distance (optimal string alignment, compared
///   ignoring case) of a third of the length of `unknown`, at least 1, are candidates.
/// - If one candidate is strictly closer than all others, it alone is returned. Otherwise the
///   closest are listed, up to [`Suggestions::MAX`].
///
/// Ties are broken by name, so the result does not depend on the order of `in_scope`.
pub fn did_you_mean<'a>(unknown: &str, in_scope: impl IntoIterator<Item = &'a str>) -> Suggestions {
    let target: Vec<char> = unknown.chars().flat_map(char::to_lowercase).collect();
    let threshold = (target.len() / 3).max(1);

    let mut candidates: Vec<(usize, &str)> = in_scope
        .into_iter()
        .filter(|name| *name != unknown)
        .filter_map(|name| {
            let lowered: Vec<char> = name.chars().flat_map(char::to_lowercase).collect();
            let distance = osa_distance(&target, &lowered);
            (distance <= threshold).then_some((distance, name))
        })
        .collect();
    candidates.sort_unstable();
    candidates.dedup();

    let best = match candidates.first() {
        Some(&(distance, _)) => distance,
        None => return Suggestions::default(),
    };
    let take = if best == 0 {
        // Case-insensitive matches only.
        candidates.iter().take_while(|(d, _)| *d == 0).count()
    } else if candidates.get(1).is_none_or(|&(d, _)| d > best) {
        1
    } else {
        candidates.len()
    };
    Suggestions {
        names: candidates
            .into_iter()
            .take(take.min(Suggestions::MAX))
            .map(|(_, name)| name.to_string())
            .collect(),
    }
}

/// Restricted Damerau-Levenshtein distance: insertions, deletions, substitutions and swaps of
/// two adjacent characters each cost 1.
fn osa_distance(a: &[char], b: &[char]) -> usize {
    let width = b.len() + 1;
    let mut rows = vec![0usize; (a.len() + 1) * width];
    let at = |i: usize, j: usize| i * width + j;
    for i in 0..=a.len() {
        rows[at(i, 0)] = i;
    }
    for j in 0..=b.len() {
        rows[at(0, j)] = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (rows[at(i - 1, j)] + 1)
                .min(rows[at(i, j - 1)] + 1)
                .min(rows[at(i - 1, j - 1)] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(rows[at(i - 2, j - 2)] + 1);
            }
            rows[at(i, j)] = best;
        }
    }
    rows[at(a.len(), b.len())]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dist(a: &str, b: &str) -> usize {
        let a: Vec<char> = a.chars().collect();
        let b: Vec<char> = b.chars().collect();
        osa_distance(&a, &b)
    }

    #[test]
    fn distances() {
        assert_eq!(dist("", ""), 0);
        assert_eq!(dist("abc", ""), 3);
        assert_eq!(dist("amout", "amount"), 1);
        assert_eq!(dist("ab", "ba"), 1); // transposition
        assert_eq!(dist("kitten", "sitting"), 3);
        assert_eq!(dist("héllo", "hello"), 1); // characters, not bytes
    }

    const COLUMNS: [&str; 4] = ["orderId", "customerId", "amount", "timestamp"];

    #[test]
    fn single_clear_winner() {
        let s = did_you_mean("amout", COLUMNS);
        assert_eq!(s.single(), Some("amount"));
    }

    #[test]
    fn case_insensitive_match_wins() {
        let s = did_you_mean("CustomerID", COLUMNS);
        assert_eq!(s.single(), Some("customerId"));
        let s = did_you_mean("summarise", ["summarize", "Summarise"]);
        assert_eq!(s.single(), Some("Summarise"));
    }

    #[test]
    fn nothing_close() {
        assert!(did_you_mean("price", COLUMNS).is_empty());
        assert!(did_you_mean("x", ["y"; 0]).is_empty());
    }

    #[test]
    fn ties_are_listed_sorted_and_capped() {
        let s = did_you_mean("cat", ["bat", "rat", "hat", "mat", "car"]);
        assert_eq!(s.names(), ["bat", "car", "hat"]);
        // Order of the input does not matter.
        let s2 = did_you_mean("cat", ["mat", "car", "hat", "rat", "bat"]);
        assert_eq!(s, s2);
    }

    #[test]
    fn exact_name_and_duplicates_are_ignored() {
        let s = did_you_mean("take", ["take", "tale", "tale"]);
        assert_eq!(s.names(), ["tale"]);
    }
}
