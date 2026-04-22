//! Search-provider fan-out and worker management.
//!
//! Each provider encapsulates one search source. [`ProviderSet`] owns the
//! workers and starts a new search generation across all of them.

pub mod apps;
pub mod settings;
pub mod spotlight;
pub mod windows;

use std::{
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

use tao::event_loop::EventLoopProxy;

use crate::{
    config::Config,
    icons::IconCache,
    plugins::PluginHost,
    types::{AppEvent, SearchItem},
};

use self::{
    apps::AppProvider, settings::SettingsProvider, spotlight::SpotlightProvider,
    windows::WindowsProvider,
};

#[derive(Clone)]
pub struct ProviderSet {
    plugins_host: Arc<PluginHost>,
    windows_provider: Arc<WindowsProvider>,
    windows: ProviderWorker,
    apps: ProviderWorker,
    settings: ProviderWorker,
    plugins: ProviderWorker,
    spotlight: ProviderWorker,
}

impl ProviderSet {
    /// Builds all provider workers from shared config, plugins, and icon state.
    pub fn new(
        config: Arc<Config>,
        plugins: Arc<PluginHost>,
        icons: Arc<IconCache>,
    ) -> anyhow::Result<Self> {
        let windows = Arc::new(WindowsProvider::new(
            icons.clone(),
            config.providers.windows.include_other_desktops,
        ));
        let apps = Arc::new(AppProvider::new(icons.clone())?);
        let settings = Arc::new(SettingsProvider::new(icons.clone())?);
        let spotlight = Arc::new(SpotlightProvider::new(icons));

        Ok(Self {
            plugins_host: plugins.clone(),
            windows_provider: windows.clone(),
            windows: ProviderWorker::new("windows", {
                let provider = windows;
                let limit = config.ranking.result_limit;
                move |query| provider.search(&query, limit)
            })?,
            apps: ProviderWorker::new("apps", {
                let provider = apps;
                let limit = config.ranking.result_limit;
                move |query| provider.search(&query, limit)
            })?,
            settings: ProviderWorker::new("settings", {
                let provider = settings;
                let limit = config.ranking.result_limit;
                move |query| provider.search(&query, limit)
            })?,
            plugins: ProviderWorker::new("plugins", move |query| plugins.search(&query))?,
            spotlight: ProviderWorker::new("spotlight", {
                let provider = spotlight;
                let limit = config.ranking.result_limit;
                move |query| provider.search(&query, limit)
            })?,
        })
    }

    /// Starts a new launcher-visible session for providers that cache per-open state.
    pub fn begin_session(&self) {
        self.windows_provider.begin_session();
    }

    /// Ends the current launcher-visible session and clears any per-open provider state.
    pub fn end_session(&self) {
        self.windows_provider.end_session();
    }

    /// Returns how many provider responses a session should wait for.
    pub fn provider_count_for_query(&self, query: &str, empty_query_providers: &[String]) -> usize {
        let enabled = enabled_providers_for_query(
            query,
            empty_query_providers,
            self.plugins_host.is_routed_query(query),
        );
        ["windows", "apps", "settings", "plugins", "spotlight"]
            .into_iter()
            .filter(|provider| enabled.contains(provider))
            .count()
    }

    /// Starts a new search request across all providers.
    pub fn spawn_search(
        &self,
        proxy: EventLoopProxy<AppEvent>,
        generation: u64,
        query: String,
        empty_query_providers: &[String],
    ) {
        let enabled = enabled_providers_for_query(
            &query,
            empty_query_providers,
            self.plugins_host.is_routed_query(&query),
        );

        if enabled.contains("windows") {
            self.windows
                .search(proxy.clone(), generation, query.clone());
        }
        if enabled.contains("apps") {
            self.apps.search(proxy.clone(), generation, query.clone());
        }
        if enabled.contains("settings") {
            self.settings
                .search(proxy.clone(), generation, query.clone());
        }
        if enabled.contains("plugins") {
            self.plugins
                .search(proxy.clone(), generation, query.clone());
        }
        if enabled.contains("spotlight") {
            self.spotlight.search(proxy, generation, query);
        }
    }
}

fn enabled_providers_for_query<'a>(
    query: &str,
    empty_query_providers: &'a [String],
    routed_plugin_query: bool,
) -> std::collections::HashSet<&'a str> {
    if !query.trim().is_empty() {
        if routed_plugin_query {
            return ["plugins"].into_iter().collect();
        }
        return ["windows", "apps", "settings", "plugins", "spotlight"]
            .into_iter()
            .collect();
    }

    empty_query_providers.iter().map(String::as_str).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::enabled_providers_for_query;

    #[test]
    fn routed_plugin_queries_only_run_plugin_provider() {
        let enabled = enabled_providers_for_query("pass secret", &[], true);

        assert_eq!(enabled, HashSet::from(["plugins"]));
    }

    #[test]
    fn normal_non_empty_queries_keep_full_provider_fanout() {
        let enabled = enabled_providers_for_query("safari", &[], false);

        assert_eq!(
            enabled,
            HashSet::from(["windows", "apps", "settings", "plugins", "spotlight"])
        );
    }

    #[test]
    fn empty_queries_still_use_configured_provider_list() {
        let configured = ["windows".to_owned()];
        let enabled = enabled_providers_for_query("", &configured, true);

        assert_eq!(enabled, HashSet::from(["windows"]));
    }
}

#[derive(Clone)]
struct ProviderWorker {
    sender: Sender<SearchRequest>,
}

struct SearchRequest {
    generation: u64,
    query: String,
    proxy: EventLoopProxy<AppEvent>,
}

impl ProviderWorker {
    fn new<F>(name: &'static str, search: F) -> anyhow::Result<Self>
    where
        F: Fn(String) -> anyhow::Result<Vec<SearchItem>> + Send + 'static,
    {
        let (sender, receiver) = mpsc::channel();
        thread::Builder::new()
            .name(format!("runx-provider-{name}"))
            .spawn(move || worker_loop(name, receiver, search))
            .map_err(anyhow::Error::from)?;
        Ok(Self { sender })
    }

    fn search(&self, proxy: EventLoopProxy<AppEvent>, generation: u64, query: String) {
        let _ = self.sender.send(SearchRequest {
            generation,
            query,
            proxy,
        });
    }
}

fn worker_loop<F>(name: &'static str, receiver: Receiver<SearchRequest>, search: F)
where
    F: Fn(String) -> anyhow::Result<Vec<SearchItem>>,
{
    while let Ok(mut request) = receiver.recv() {
        while let Ok(next) = receiver.try_recv() {
            request = next;
        }

        match search(request.query) {
            Ok(items) => {
                let _ = request.proxy.send_event(AppEvent::ProviderItems {
                    generation: request.generation,
                    provider: name.to_owned(),
                    items,
                });
            }
            Err(error) => {
                let _ = request.proxy.send_event(AppEvent::ProviderError {
                    generation: request.generation,
                    provider: name.to_owned(),
                    message: error.to_string(),
                });
            }
        }
    }
}
