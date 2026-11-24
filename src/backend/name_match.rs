//! Finding names that are close to what the user typed, for "did you mean ...?" hints.

/// Most suggestions returned by `similar_names`.
pub const MAX_SUGGESTIONS: usize = 5;

/// How a candidate relates to the query; lower is a better match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    /// One is the start of the other: `web` for `webserver`.
    Prefix,
    /// One appears inside the other: `server` for `webserver`.
    Substring,
    /// A few typos away: `wbeserver` for `webserver`.
    Typo,
}

/// The candidates that look like `query`, best match first, at most `MAX_SUGGESTIONS`.
/// Comparison ignores case. Candidates that are nothing like the query are left out.
pub fn similar_names<'a>(query: &str, candidates: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    let query_lower = query.to_lowercase();
    let max_typos = max_typos(&query_lower);

    let mut scored: Vec<(Kind, usize, &str)> = candidates
        .into_iter()
        .filter_map(|candidate| {
            let candidate_lower = candidate.to_lowercase();
            let distance = edit_distance(&query_lower, &candidate_lower);

            let kind = if candidate_lower.starts_with(&query_lower)
                || query_lower.starts_with(&candidate_lower)
            {
                Kind::Prefix
            } else if candidate_lower.contains(&query_lower) || query_lower.contains(&candidate_lower)
            {
                Kind::Substring
            } else if distance <= max_typos {
                Kind::Typo
            } else {
                return None;
            };
            Some((kind, distance, candidate))
        })
        .collect();

    scored.sort();
    scored.dedup_by(|a, b| a.2 == b.2);
    scored
        .into_iter()
        .take(MAX_SUGGESTIONS)
        .map(|(_, _, candidate)| candidate)
        .collect()
}

/// Typos allowed before a name stops counting as similar: roughly one per three characters,
/// so short names only match close misses and long names tolerate a few more.
fn max_typos(query: &str) -> usize {
    (query.chars().count() / 3).max(1)
}

/// Optimal string alignment distance: insertions, deletions, substitutions and swaps of two
/// neighbouring characters each count as one edit, so `wbe` is one edit away from `web`.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();

    // Three rolling rows: two rows back (for swaps), the previous row, and the current one.
    let mut before_previous: Vec<usize> = vec![0; b.len() + 1];
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current: Vec<usize> = vec![0; b.len() + 1];

    for i in 1..=a.len() {
        current[0] = i;
        for j in 1..=b.len() {
            let substitution = usize::from(a[i - 1] != b[j - 1]);
            current[j] = (previous[j] + 1)
                .min(current[j - 1] + 1)
                .min(previous[j - 1] + substitution);

            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                current[j] = current[j].min(before_previous[j - 2] + 1);
            }
        }
        std::mem::swap(&mut before_previous, &mut previous);
        std::mem::swap(&mut previous, &mut current);
    }

    previous[b.len()]
}
