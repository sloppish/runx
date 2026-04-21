//! Fuzzy scoring and final result ordering.
//!
//! Providers produce raw result sets; this module turns them into a stable
//! ordered list that respects fuzzy score, provider priority, and result limits.

mod arinae;

use std::{collections::HashSet, sync::OnceLock};

use self::arinae::{ArinaeMatcher, FuzzyMatcher};
use crate::{config::RankingConfig, types::SearchItem};

/// Returns an Arinae fuzzy score for `candidate` against `query`.
pub fn fuzzy_score(candidate: &str, query: &str) -> i64 {
    let query = query.trim();
    if query.is_empty() {
        return 1;
    }

    let candidate_lower = candidate.to_ascii_lowercase();
    let query_lower = query.to_ascii_lowercase();
    let Some(mut score) = matcher().fuzzy_match(&candidate_lower, &query_lower) else {
        return 0;
    };

    if candidate_lower.contains(&query_lower) {
        score *= 2;
    }

    score
}

/// Deduplicates, orders, and truncates the final merged result list.
pub fn sort_and_trim(items: Vec<SearchItem>, ranking: &RankingConfig) -> Vec<SearchItem> {
    let mut seen = HashSet::new();
    let mut unique = items
        .into_iter()
        .filter(|item| seen.insert(item.id.clone()))
        .collect::<Vec<_>>();

    unique.sort_by(base_compare);
    unique = regroup_by_threshold(unique, ranking);
    unique.truncate(ranking.result_limit);
    unique
}

fn regroup_by_threshold(items: Vec<SearchItem>, ranking: &RankingConfig) -> Vec<SearchItem> {
    if items.is_empty() {
        return items;
    }

    let mut grouped = Vec::with_capacity(items.len());
    let mut start = 0;

    while start < items.len() {
        let top_score = items[start].raw_score;
        let mut end = start + 1;

        while end < items.len() && (top_score - items[end].raw_score).abs() <= ranking.tie_threshold
        {
            end += 1;
        }

        let mut chunk = items[start..end].to_vec();
        chunk.sort_by(|left, right| {
            ranking
                .provider_rank(&left.provider)
                .cmp(&ranking.provider_rank(&right.provider))
                .then_with(|| base_compare(left, right))
        });
        grouped.extend(chunk);
        start = end;
    }

    grouped
}

fn base_compare(left: &SearchItem, right: &SearchItem) -> std::cmp::Ordering {
    right
        .raw_score
        .cmp(&left.raw_score)
        .then_with(|| left.title.len().cmp(&right.title.len()))
        .then_with(|| left.title.cmp(&right.title))
        .then_with(|| left.subtitle.cmp(&right.subtitle))
        .then_with(|| left.provider.cmp(&right.provider))
        .then_with(|| left.id.cmp(&right.id))
}

fn matcher() -> &'static ArinaeMatcher {
    static MATCHER: OnceLock<ArinaeMatcher> = OnceLock::new();
    MATCHER.get_or_init(ArinaeMatcher::default)
}

#[cfg(test)]
mod tests {
    use super::{fuzzy_score, sort_and_trim};
    use crate::{
        config::RankingConfig,
        types::{Action, SearchItem},
    };

    #[test]
    fn sorts_without_non_transitive_comparator_panics() {
        let ranking = RankingConfig {
            tie_threshold: 120,
            provider_order: vec![
                "windows".to_owned(),
                "apps".to_owned(),
                "settings".to_owned(),
            ],
            empty_query_providers: vec!["windows".to_owned()],
            result_limit: 10,
        };

        let items = vec![
            item("a", "apps", "Alpha", 300),
            item("b", "windows", "Beta", 210),
            item("c", "settings", "Gamma", 120),
        ];

        let ordered = sort_and_trim(items, &ranking);
        let ids = ordered
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["b", "a", "c"]);
    }

    #[test]
    fn prefers_contiguous_substring_over_scattered_subsequence() {
        let app_score = fuzzy_score("Google Chrome", "chrom");
        let window_score = fuzzy_score("Change retention order message", "chrom");

        assert!(app_score > window_score);
    }

    fn item(id: &str, provider: &str, title: &str, raw_score: i64) -> SearchItem {
        SearchItem {
            id: id.to_owned(),
            provider: provider.to_owned(),
            badge: provider.to_ascii_uppercase(),
            icon: None,
            title: title.to_owned(),
            subtitle: provider.to_owned(),
            compact: false,
            raw_score,
            action: Action::OpenPath {
                path: format!("/tmp/{id}"),
            },
        }
    }
}
