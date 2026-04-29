//! Search scheduling, provider fan-out, and frontend rendering.
//!
//! `Launcher` delegates query updates and provider callbacks here so the
//! launcher module can stay focused on high-level event routing.

use std::time::Duration;

use anyhow::{Context, Result};
use tokio::runtime::Runtime;
use wry::WebView;

use crate::{
    config::{RankingConfig, TimingConfig},
    providers::ProviderSet,
    state::AppState,
    types::{AppEvent, SearchItem},
};

/// Owns the transient scheduling state around searches and rerenders.
pub(crate) struct SearchController {
    search_debounce: Duration,
}

impl SearchController {
    /// Creates a controller using the configured debounce/coalescing timings.
    pub(crate) fn new(timing: &TimingConfig) -> Self {
        Self {
            search_debounce: Duration::from_millis(timing.search_debounce_ms),
        }
    }

    /// Replaces the current query and schedules a debounced search start.
    pub(crate) fn handle_query_changed(
        &mut self,
        state: &mut AppState,
        query: String,
        runtime: &Runtime,
        proxy: tao::event_loop::EventLoopProxy<AppEvent>,
    ) {
        state.session_mut().clear_config_error();
        let token = state.session_mut().set_query(query);
        let search_debounce = self.search_debounce;
        runtime.handle().spawn(async move {
            tokio::time::sleep(search_debounce).await;
            let _ = proxy.send_event(AppEvent::StartSearch { token });
        });
    }

    /// Starts a new provider generation if the debounce token is still current.
    pub(crate) fn handle_start_search(
        &mut self,
        state: &mut AppState,
        token: u64,
        providers: &ProviderSet,
        proxy: tao::event_loop::EventLoopProxy<AppEvent>,
    ) {
        if token != state.session().search_token() {
            return;
        }

        let query = state.session().query().to_owned();
        let provider_count = providers.provider_count_for_query(&query);
        let generation = state.session_mut().begin_search(provider_count);
        if provider_count == 0 {
            let _ = proxy.send_event(AppEvent::Render);
            return;
        }
        providers.spawn_search(proxy, generation, query);
    }

    /// Merges one provider response into the current session and renders when all are done.
    pub(crate) fn handle_provider_items(
        &mut self,
        state: &mut AppState,
        generation: u64,
        provider: String,
        items: Vec<SearchItem>,
        proxy: tao::event_loop::EventLoopProxy<AppEvent>,
    ) {
        if state
            .session_mut()
            .apply_provider_items(generation, provider, items)
            && state.session().is_search_complete()
        {
            let _ = proxy.send_event(AppEvent::Render);
        }
    }

    /// Records one provider error for the current generation and renders when all are done.
    pub(crate) fn handle_provider_error(
        &mut self,
        state: &mut AppState,
        generation: u64,
        provider: String,
        message: String,
        proxy: tao::event_loop::EventLoopProxy<AppEvent>,
    ) {
        if state
            .session_mut()
            .apply_provider_error(generation, &provider, message)
            && state.session().is_search_complete()
        {
            let _ = proxy.send_event(AppEvent::Render);
        }
    }

    /// Recomputes the visible result list and pushes a new view state into the webview.
    pub(crate) fn render(
        &mut self,
        state: &mut AppState,
        ranking: &RankingConfig,
        webview: &WebView,
    ) -> Result<()> {
        state.session_mut().refresh_rendered_items(ranking);
        let script = format!(
            "window.__RUNX_RENDER({});",
            serde_json::to_string(&state.session().view_state())?
        );
        webview
            .evaluate_script(&script)
            .context("failed to render the launcher UI")?;
        Ok(())
    }

    /// Clears the render debounce latch and renders the current session immediately.
    pub(crate) fn flush_render(
        &mut self,
        state: &mut AppState,
        ranking: &RankingConfig,
        webview: &WebView,
    ) -> Result<()> {
        self.render(state, ranking, webview)
    }
}
