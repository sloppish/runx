//! Fuzzy scoring and final result ordering.
//!
//! Providers produce raw result sets; this module turns them into a stable
//! ordered list that respects fuzzy score, provider priority, and result limits.

mod arinae;

use std::{collections::HashSet, sync::OnceLock};

use self::arinae::{ArinaeMatcher, FuzzyMatcher};
use crate::{
    config::{RankingConfig, RankingScoreRule, RankingScoreRuleField, RankingScoreRuleMatchKind},
    types::SearchItem,
};

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

    for item in &mut unique {
        item.raw_score = item
            .raw_score
            .saturating_add(ranking.provider_score_boost(&item.provider))
            .saturating_add(score_rule_boost(item, ranking));
    }

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

fn score_rule_boost(item: &SearchItem, ranking: &RankingConfig) -> i64 {
    ranking
        .score_rules
        .iter()
        .filter(|rule| rule_matches_item(rule, item))
        .fold(0_i64, |total, rule| total.saturating_add(rule.boost))
}

fn rule_matches_item(rule: &RankingScoreRule, item: &SearchItem) -> bool {
    if rule.pattern.trim().is_empty() {
        return false;
    }

    if !rule.providers.is_empty()
        && !rule
            .providers
            .iter()
            .any(|provider| provider == &item.provider)
    {
        return false;
    }

    let candidate = match rule.field {
        RankingScoreRuleField::Title => &item.title,
        RankingScoreRuleField::Subtitle => &item.subtitle,
        RankingScoreRuleField::Badge => &item.badge,
        RankingScoreRuleField::Id => &item.id,
    }
    .to_lowercase();
    let pattern = rule.pattern.to_lowercase();

    match rule.match_kind {
        RankingScoreRuleMatchKind::Exact => candidate == pattern,
        RankingScoreRuleMatchKind::Prefix => candidate.starts_with(&pattern),
        RankingScoreRuleMatchKind::Contains => candidate.contains(&pattern),
    }
}

fn matcher() -> &'static ArinaeMatcher {
    static MATCHER: OnceLock<ArinaeMatcher> = OnceLock::new();
    MATCHER.get_or_init(ArinaeMatcher::default)
}

#[cfg(test)]
mod tests {
    use super::{fuzzy_score, sort_and_trim};
    use crate::{
        config::{
            RankingConfig, RankingScoreRule, RankingScoreRuleField, RankingScoreRuleMatchKind,
        },
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
            provider_score_boosts: Default::default(),
            score_rules: vec![],
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

    #[test]
    fn provider_score_boosts_can_override_close_scores() {
        let ranking = RankingConfig {
            tie_threshold: 0,
            provider_order: vec!["settings".to_owned(), "apps".to_owned()],
            provider_score_boosts: [("apps".to_owned(), 20)].into_iter().collect(),
            score_rules: vec![],
            result_limit: 10,
        };

        let items = vec![
            item("settings", "settings", "Settings Hit", 100),
            item("app", "apps", "App Hit", 90),
        ];

        let ordered = sort_and_trim(items, &ranking);
        let ids = ordered
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["app", "settings"]);
    }

    #[test]
    fn item_score_rules_can_prefer_specific_titles_within_one_provider() {
        let ranking = RankingConfig {
            tie_threshold: 0,
            provider_order: vec!["apps".to_owned()],
            provider_score_boosts: Default::default(),
            score_rules: vec![RankingScoreRule {
                providers: vec!["apps".to_owned(), "windows".to_owned()],
                field: RankingScoreRuleField::Title,
                match_kind: RankingScoreRuleMatchKind::Contains,
                pattern: "spotify".to_owned(),
                boost: 20,
            }],
            result_limit: 10,
        };

        let items = vec![
            item("settings", "apps", "Settings", 100),
            item("spotify", "apps", "Spotify", 90),
        ];

        let ordered = sort_and_trim(items, &ranking);
        let ids = ordered
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["spotify", "settings"]);
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
            action: Action::OpenApplication {
                path: format!("/Applications/{id}.app"),
            },
        }
    }
}
