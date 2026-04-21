//! Pure launcher and search-session state.
//!
//! This module deliberately contains no Tao/Wry or macOS side effects, which
//! makes it the best place to understand and test how searches reset, how stale
//! provider results are ignored, and how frontend view state is assembled.

use std::collections::HashMap;

use crate::{
    config::RankingConfig,
    scoring::sort_and_trim,
    types::{Action, SearchItem, ViewItem, ViewState},
};

/// Top-level visibility and search-session state for the launcher.
#[derive(Default)]
pub struct AppState {
    visible: bool,
    focused_since_show: bool,
    session: SearchSession,
}

impl AppState {
    /// Creates a new hidden launcher state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Marks the launcher as visible and resets the current session.
    pub fn show(&mut self) {
        self.visible = true;
        self.focused_since_show = false;
        self.session.reset();
    }

    /// Marks the launcher as hidden and clears any current session state.
    pub fn hide(&mut self) {
        self.visible = false;
        self.focused_since_show = false;
        self.session.reset();
    }

    /// Returns whether the launcher is currently visible.
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Returns whether the window has focused since the last `show()`.
    pub fn focused_since_show(&self) -> bool {
        self.focused_since_show
    }

    /// Notes a native focus event for the launcher window.
    pub fn note_window_focused(&mut self) {
        if self.visible {
            self.focused_since_show = true;
        }
    }

    /// Borrows the current search session.
    pub fn session(&self) -> &SearchSession {
        &self.session
    }

    /// Mutably borrows the current search session.
    pub fn session_mut(&mut self) -> &mut SearchSession {
        &mut self.session
    }
}

/// Search-specific state for one launcher session.
#[derive(Default)]
pub struct SearchSession {
    query: String,
    generation: u64,
    search_token: u64,
    provider_items: HashMap<String, Vec<SearchItem>>,
    rendered_items: Vec<SearchItem>,
    pending_providers: usize,
}

impl SearchSession {
    /// Clears query, status, and result state, invalidating older generations.
    pub fn reset(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.search_token = self.search_token.wrapping_add(1);
        self.query.clear();
        self.provider_items.clear();
        self.rendered_items.clear();
        self.pending_providers = 0;
    }

    /// Returns the user-visible query string.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Replaces the current query and returns the new debounce token.
    pub fn set_query(&mut self, query: String) -> u64 {
        self.query = query;
        self.search_token = self.search_token.wrapping_add(1);
        self.search_token
    }

    /// Returns the token for the most recently scheduled search.
    pub fn search_token(&self) -> u64 {
        self.search_token
    }

    /// Starts a new provider search generation.
    pub fn begin_search(&mut self, provider_count: usize) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.pending_providers = provider_count;
        self.generation
    }

    /// Applies provider results if they still belong to the active generation.
    pub fn apply_provider_items(
        &mut self,
        generation: u64,
        provider: String,
        items: Vec<SearchItem>,
    ) -> bool {
        if generation != self.generation {
            return false;
        }

        self.pending_providers = self.pending_providers.saturating_sub(1);
        self.provider_items.insert(provider, items);
        true
    }

    /// Applies a provider error if it still belongs to the active generation.
    pub fn apply_provider_error(
        &mut self,
        generation: u64,
        provider: &str,
        message: String,
    ) -> bool {
        if generation != self.generation {
            return false;
        }

        self.pending_providers = self.pending_providers.saturating_sub(1);
        self.provider_items.insert(
            provider.to_owned(),
            vec![provider_error_item(provider, message, generation)],
        );
        true
    }

    /// Returns the rendered item at the given visible list index.
    pub fn rendered_item(&self, index: usize) -> Option<&SearchItem> {
        self.rendered_items.get(index)
    }

    /// Rebuilds the rendered item list from current provider results.
    pub fn refresh_rendered_items(&mut self, ranking: &RankingConfig) {
        let all_items = self
            .provider_items
            .values()
            .flatten()
            .cloned()
            .collect::<Vec<_>>();
        let next_items = sort_and_trim(all_items, ranking);
        let keep_previous_items =
            self.pending_providers > 0 && next_items.is_empty() && !self.rendered_items.is_empty();
        if !keep_previous_items {
            self.rendered_items = next_items;
        }
    }

    /// Converts the session into the frontend-facing serialized view model.
    pub fn view_state(&self) -> ViewState {
        let items = self
            .rendered_items
            .iter()
            .enumerate()
            .map(|(index, item)| ViewItem {
                title: item.title.clone(),
                subtitle: item.subtitle.clone(),
                badge: item.badge.clone(),
                icon: item.icon.clone(),
                accelerator: (index < 9).then(|| format!("⌥{}", index + 1)),
                compact: item.compact,
            })
            .collect();

        ViewState {
            query: self.query.clone(),
            items,
        }
    }
}

fn provider_error_item(provider: &str, message: String, generation: u64) -> SearchItem {
    SearchItem {
        id: format!("error:{provider}:{generation}"),
        provider: "system".to_owned(),
        badge: "ERR".to_owned(),
        icon: None,
        title: provider_error_title(provider),
        subtitle: message,
        compact: false,
        raw_score: 1_000_000,
        action: Action::Noop,
    }
}

fn provider_error_title(provider: &str) -> String {
    match provider {
        "plugins" => "Plugin query error".to_owned(),
        "spotlight" => "Spotlight query error".to_owned(),
        "settings" => "Settings query error".to_owned(),
        "apps" => "App query error".to_owned(),
        "windows" => "Window query error".to_owned(),
        other => format!("{other} query error"),
    }
}

#[cfg(test)]
mod tests {
    use super::AppState;
    use crate::{
        config::RankingConfig,
        types::{Action, SearchItem},
    };

    #[test]
    fn hide_clears_session_state() {
        let mut state = AppState::new();
        state.show();
        populate(&mut state, "alpha");

        state.hide();

        assert!(!state.is_visible());
        assert_eq!(state.session().query(), "");
        assert!(state.session().rendered_item(0).is_none());
    }

    #[test]
    fn activate_then_show_reopens_clean() {
        let mut state = AppState::new();
        state.show();
        populate(&mut state, "alpha");

        state.hide();
        state.show();

        assert!(state.is_visible());
        assert_eq!(state.session().query(), "");
        assert!(state.session().rendered_item(0).is_none());
    }

    #[test]
    fn stale_provider_results_are_ignored_after_reset() {
        let mut state = AppState::new();
        state.show();
        let generation = state.session_mut().begin_search(1);

        state.hide();

        assert!(!state.session_mut().apply_provider_items(
            generation,
            "windows".to_owned(),
            vec![item("stale", 10)],
        ));
        state
            .session_mut()
            .refresh_rendered_items(&default_ranking());
        assert!(state.session().rendered_item(0).is_none());
    }

    #[test]
    fn stale_provider_results_are_ignored_after_new_search_generation() {
        let mut state = AppState::new();
        state.show();
        let old_generation = state.session_mut().begin_search(1);
        populate(&mut state, "alpha");

        state.session_mut().set_query("beta".to_owned());
        let new_generation = state.session_mut().begin_search(1);

        assert_ne!(old_generation, new_generation);
        assert!(!state.session_mut().apply_provider_items(
            old_generation,
            "windows".to_owned(),
            vec![item("stale", 100)],
        ));
    }

    #[test]
    fn provider_error_replaces_previous_results_with_error_item() {
        let mut state = AppState::new();
        state.show();
        state.session_mut().set_query("pass 'tg".to_owned());
        let generation = state.session_mut().begin_search(1);
        assert!(state.session_mut().apply_provider_items(
            generation,
            "plugins".to_owned(),
            vec![plugin_item("old result", 100)],
        ));

        assert!(state.session_mut().apply_provider_error(
            generation,
            "plugins",
            "unterminated single-quoted string in command arguments".to_owned(),
        ));

        state
            .session_mut()
            .refresh_rendered_items(&default_ranking());

        let rendered = state
            .session()
            .rendered_item(0)
            .expect("error row should render");
        assert_eq!(rendered.title, "Plugin query error");
        assert!(
            rendered
                .subtitle
                .contains("unterminated single-quoted string")
        );
    }

    fn populate(state: &mut AppState, query: &str) {
        state.session_mut().set_query(query.to_owned());
        let generation = state.session_mut().begin_search(1);
        assert!(state.session_mut().apply_provider_items(
            generation,
            "windows".to_owned(),
            vec![item(query, 100)],
        ));
        state
            .session_mut()
            .refresh_rendered_items(&default_ranking());
    }

    fn default_ranking() -> RankingConfig {
        RankingConfig {
            tie_threshold: 120,
            provider_order: vec![
                "windows".to_owned(),
                "apps".to_owned(),
                "settings".to_owned(),
                "plugins".to_owned(),
                "spotlight".to_owned(),
            ],
            empty_query_providers: vec!["windows".to_owned()],
            result_limit: 24,
        }
    }

    fn item(id: &str, raw_score: i64) -> SearchItem {
        SearchItem {
            id: id.to_owned(),
            provider: "windows".to_owned(),
            badge: "WIN".to_owned(),
            icon: None,
            title: id.to_owned(),
            subtitle: "test".to_owned(),
            compact: false,
            raw_score,
            action: Action::OpenPath {
                path: format!("/tmp/{id}"),
            },
        }
    }

    fn plugin_item(id: &str, raw_score: i64) -> SearchItem {
        SearchItem {
            id: id.to_owned(),
            provider: "plugins".to_owned(),
            badge: "PLG".to_owned(),
            icon: None,
            title: id.to_owned(),
            subtitle: "test".to_owned(),
            compact: true,
            raw_score,
            action: Action::Noop,
        }
    }
}
