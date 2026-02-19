/// Result of a successful fuzzy match.
#[derive(Debug, Clone)]
pub struct FuzzyMatch {
    /// Higher = better match. Used for sorting results.
    pub score: i32,
    /// Character-based indices in the target string where query characters matched.
    /// These are CHARACTER indices (not byte indices) for correct highlighting.
    pub positions: Vec<usize>,
}

/// Perform a case-insensitive subsequence fuzzy match.
///
/// Returns `Some(FuzzyMatch)` if every character of `query` appears in `target`
/// in order. Returns `None` if the query does not match.
pub fn fuzzy_match(query: &str, target: &str) -> Option<FuzzyMatch> {
    if query.is_empty() {
        return None;
    }

    let query_lower: Vec<char> = query.to_lowercase().chars().collect();
    let target_lower: Vec<char> = target.to_lowercase().chars().collect();
    let target_chars: Vec<char> = target.chars().collect();

    let mut positions: Vec<usize> = Vec::with_capacity(query_lower.len());
    let mut target_idx = 0;

    for query_char in &query_lower {
        let mut found = false;
        while target_idx < target_lower.len() {
            if target_lower[target_idx] == *query_char {
                positions.push(target_idx);
                target_idx += 1;
                found = true;
                break;
            }
            target_idx += 1;
        }
        if !found {
            return None;
        }
    }

    // Calculate score
    let mut score: i32 = 0;

    for (i, &pos) in positions.iter().enumerate() {
        // Consecutive match bonus: +5 if current position is exactly 1 after previous
        if i > 0 && pos == positions[i - 1] + 1 {
            score += 5;
        }

        // Head match bonus: +10 if match is at position 0
        if pos == 0 {
            score += 10;
        }

        // Separator bonus: +5 if the character before match position is a separator
        if pos > 0 {
            let prev_char = target_chars[pos - 1];
            if prev_char == '/' || prev_char == '-' || prev_char == '_' || prev_char == ' ' {
                score += 5;
            }
        }

        // Position penalty: -1 for each position index
        score -= pos as i32;
    }

    Some(FuzzyMatch { score, positions })
}

/// Filter items by fuzzy matching and return sorted results.
///
/// Each item is a tuple of `(original_index, searchable_text)`.
/// Returns matching items sorted by score (descending).
/// If query is empty, returns all items in original order with score=0 and empty positions.
pub fn filter_and_sort(query: &str, items: &[(usize, String)]) -> Vec<(usize, FuzzyMatch)> {
    if query.is_empty() {
        return items
            .iter()
            .map(|(idx, _)| {
                (
                    *idx,
                    FuzzyMatch {
                        score: 0,
                        positions: Vec::new(),
                    },
                )
            })
            .collect();
    }

    let mut results: Vec<(usize, FuzzyMatch)> = items
        .iter()
        .filter_map(|(idx, text)| fuzzy_match(query, text).map(|m| (*idx, m)))
        .collect();

    // Sort by score descending
    results.sort_by(|a, b| b.1.score.cmp(&a.1.score));

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    // Basic match: "abc" matches "a_b_c"
    #[test]
    fn basic_subsequence_match() {
        let result = fuzzy_match("abc", "a_b_c");
        assert!(result.is_some());
        let m = result.unwrap();
        assert_eq!(m.positions, vec![0, 2, 4]);
    }

    // No match: "xyz" does not match "abc"
    #[test]
    fn no_match() {
        assert!(fuzzy_match("xyz", "abc").is_none());
    }

    // Case insensitive: "ABC" matches "abc"
    #[test]
    fn case_insensitive_match() {
        let result = fuzzy_match("ABC", "abc");
        assert!(result.is_some());
    }

    // Score ordering: consecutive match > non-consecutive
    #[test]
    fn consecutive_scores_higher_than_non_consecutive() {
        let consecutive = fuzzy_match("abc", "abcdef").unwrap();
        let non_consecutive = fuzzy_match("abc", "a_b_c").unwrap();
        assert!(consecutive.score > non_consecutive.score);
    }

    // Head match bonus: "cl" in "Claude" > "cl" in "include"
    #[test]
    fn head_match_scores_higher() {
        let head = fuzzy_match("cl", "Claude").unwrap();
        let mid = fuzzy_match("cl", "include").unwrap();
        assert!(head.score > mid.score);
    }

    // Empty query: returns None (individual fuzzy_match) but filter_and_sort returns all
    #[test]
    fn empty_query_no_match() {
        assert!(fuzzy_match("", "anything").is_none());
    }

    // Empty target: no match
    #[test]
    fn empty_target_no_match() {
        assert!(fuzzy_match("a", "").is_none());
    }

    // Japanese characters: should not crash
    #[test]
    fn japanese_chars_no_crash() {
        let result = fuzzy_match("\u{30c6}", "\u{30c6}\u{30b9}\u{30c8}");
        assert!(result.is_some());
    }

    // Positions are correct character indices
    #[test]
    fn positions_are_char_indices() {
        let result = fuzzy_match("ac", "abcd").unwrap();
        assert_eq!(result.positions, vec![0, 2]);
    }

    // filter_and_sort with empty query returns all items
    #[test]
    fn filter_and_sort_empty_query_returns_all() {
        let items = vec![
            (0, "alpha".to_string()),
            (1, "beta".to_string()),
            (2, "gamma".to_string()),
        ];
        let result = filter_and_sort("", &items);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].0, 0);
        assert_eq!(result[1].0, 1);
        assert_eq!(result[2].0, 2);
    }

    // filter_and_sort filters non-matching items
    #[test]
    fn filter_and_sort_filters_non_matching() {
        let items = vec![
            (0, "alpha".to_string()),
            (1, "beta".to_string()),
            (2, "gamma".to_string()),
        ];
        let result = filter_and_sort("alp", &items);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, 0);
    }

    // filter_and_sort sorts by score descending
    #[test]
    fn filter_and_sort_sorts_by_score() {
        let items = vec![
            (0, "a_b_c".to_string()),
            (1, "abcdef".to_string()),
        ];
        let result = filter_and_sort("abc", &items);
        assert_eq!(result.len(), 2);
        // "abcdef" (consecutive match) should be first (higher score)
        assert_eq!(result[0].0, 1);
        assert_eq!(result[1].0, 0);
    }

    // Separator bonus: match after '/' scores higher
    #[test]
    fn separator_bonus() {
        let with_sep = fuzzy_match("my", "/my-app").unwrap();
        let without_sep = fuzzy_match("my", "army").unwrap();
        assert!(with_sep.score > without_sep.score);
    }
}
