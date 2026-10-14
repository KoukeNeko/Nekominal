//! Subsequence matching for the command palette.

/// Returns `None` when `query` does not match `text`; a higher score is a better match.
pub fn fuzzy_score(query: &str, text: &str) -> Option<f64> {
    let needle: Vec<char> = query.trim().to_lowercase().chars().filter(|c| *c != ' ').collect();
    if needle.is_empty() {
        return Some(0.0);
    }
    let haystack: Vec<char> = text.to_lowercase().chars().collect();

    let mut score = 0.0;
    let mut position = 0usize;
    let mut previous_match: Option<usize> = None;
    for ch in needle {
        let found = haystack[position..].iter().position(|c| *c == ch)? + position;
        score += 1.0;
        if previous_match.is_some_and(|p| found == p + 1) {
            score += 2.0; // consecutive characters
        }
        let starts_word = found == 0
            || haystack
                .get(found - 1)
                .is_some_and(|c| c.is_whitespace() || matches!(c, ':' | '：' | '-' | '_' | '/'));
        if starts_word {
            score += 3.0;
        }
        previous_match = Some(found);
        position = found + 1;
    }
    // Shorter candidates win ties.
    Some(score - haystack.len() as f64 * 0.01)
}

/// Filters and orders `items` by match quality; an empty query keeps the original order.
pub fn rank<T>(items: Vec<T>, query: &str, label: impl Fn(&T) -> &str) -> Vec<T> {
    if query.trim().is_empty() {
        return items;
    }
    let mut scored: Vec<(f64, usize, T)> = items
        .into_iter()
        .enumerate()
        .filter_map(|(index, item)| fuzzy_score(query, label(&item)).map(|score| (score, index, item)))
        .collect();
    // Stable for equal scores: earlier items stay first.
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, _, item)| item).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_subsequences_and_rejects_other_text() {
        assert!(fuzzy_score("nt", "New Tab").is_some());
        assert!(fuzzy_score("xyz", "New Tab").is_none());
    }

    #[test]
    fn an_empty_query_matches_everything_equally() {
        assert_eq!(fuzzy_score("", "anything"), Some(0.0));
        assert_eq!(fuzzy_score("   ", "anything"), Some(0.0));
    }

    #[test]
    fn prefers_consecutive_and_word_start_matches() {
        assert!(fuzzy_score("tab", "New Tab").unwrap() > fuzzy_score("tab", "Translate abc").unwrap());
    }

    #[test]
    fn works_with_chinese_text() {
        assert!(fuzzy_score("分頁", "新增分頁").is_some());
        assert!(fuzzy_score("頁分", "新增分頁").is_none());
    }

    #[test]
    fn is_case_insensitive_and_ignores_spaces_in_the_query() {
        assert!(fuzzy_score("NEW TAB", "new tab").is_some());
    }

    #[test]
    fn rank_keeps_the_order_for_an_empty_query() {
        let items = vec!["關閉分頁", "新增分頁", "新增窗格"];
        assert_eq!(rank(items.clone(), "", |s| s), items);
    }

    #[test]
    fn rank_filters_and_orders_by_score() {
        let items = vec!["關閉分頁", "新增分頁", "新增窗格"];
        assert_eq!(rank(items.clone(), "新增", |s| s), vec!["新增分頁", "新增窗格"]);
        assert_eq!(rank(items, "窗", |s| s), vec!["新增窗格"]);
    }
}
