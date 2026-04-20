//! Search scheduling, provider fan-out, and frontend rendering.
//!
//! `Launcher` delegates query updates and provider callbacks here so the
//! launcher module can stay focused on high-level event routing.

use std::time::Duration;

use anyhow::{Context, Result};
use tokio::runtime::Runtime;
use wry::WebView;

use crate::{
    config::RankingConfig,
    providers::ProviderSet,
    state::AppState,
    types::{AppEvent, SearchItem},
};

const RENDER_COALESCE: Duration = Duration::from_millis(16);
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(24);

/// Owns the transient scheduling state around searches and rerenders.
#[derive(Default)]
pub(crate) struct SearchController {
    render_scheduled: bool,
}

impl SearchController {
    /// Replaces the current query and schedules a debounced search start.
    pub(crate) fn handle_query_changed(
        &mut self,
        state: &mut AppState,
        query: String,
        runtime: &Runtime,
        proxy: tao::event_loop::EventLoopProxy<AppEvent>,
    ) {
        let token = state.session_mut().set_query(query);
        runtime.handle().spawn(async move {
            tokio::time::sleep(SEARCH_DEBOUNCE).await;
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

        let generation = state.session_mut().begin_search(providers.provider_count());
        providers.spawn_search(proxy, generation, state.session().query().to_owned());
    }

    /// Merges one provider response into the current session and schedules rerender.
    pub(crate) fn handle_provider_items(
        &mut self,
        state: &mut AppState,
        generation: u64,
        provider: String,
        items: Vec<SearchItem>,
        runtime: &Runtime,
        proxy: tao::event_loop::EventLoopProxy<AppEvent>,
    ) {
        if state
            .session_mut()
            .apply_provider_items(generation, provider, items)
        {
            self.request_render(runtime, proxy, RENDER_COALESCE);
        }
    }

    /// Records one provider error for the current generation and schedules rerender.
    pub(crate) fn handle_provider_error(
        &mut self,
        state: &mut AppState,
        generation: u64,
        provider: String,
        message: String,
        runtime: &Runtime,
        proxy: tao::event_loop::EventLoopProxy<AppEvent>,
    ) {
        if state
            .session_mut()
            .apply_provider_error(generation, &provider, message)
        {
            self.request_render(runtime, proxy, RENDER_COALESCE);
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

    /// Marks any pending delayed render as obsolete.
    pub(crate) fn cancel_pending_render(&mut self) {
        self.render_scheduled = false;
    }

    fn request_render(
        &mut self,
        runtime: &Runtime,
        proxy: tao::event_loop::EventLoopProxy<AppEvent>,
        delay: Duration,
    ) {
        if self.render_scheduled {
            return;
        }

        self.render_scheduled = true;
        runtime.handle().spawn(async move {
            tokio::time::sleep(delay).await;
            let _ = proxy.send_event(AppEvent::Render);
        });
    }

    /// Clears the render debounce latch and renders the current session immediately.
    pub(crate) fn flush_render(
        &mut self,
        state: &mut AppState,
        ranking: &RankingConfig,
        webview: &WebView,
    ) -> Result<()> {
        self.render_scheduled = false;
        self.render(state, ranking, webview)
    }
}
