//! Search-provider fan-out and worker management.
//!
//! Each provider encapsulates one search source. [`ProviderSet`] owns the
//! workers and starts a new search generation across all of them.

pub mod apps;
pub mod settings;
pub mod windows;

use std::{
    collections::HashSet,
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

use self::{apps::AppProvider, settings::SettingsProvider, windows::WindowsProvider};

#[derive(Clone)]
pub struct ProviderSet {
    plugins_host: Arc<PluginHost>,
    disabled_providers: HashSet<String>,
    windows_show_on_empty_query: bool,
    windows_provider: Arc<WindowsProvider>,
    windows: ProviderWorker,
    apps: ProviderWorker,
    settings: ProviderWorker,
    plugins: ProviderWorker,
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
        let apps = Arc::new(AppProvider::new(
            icons.clone(),
            config.providers.apps.clone(),
        )?);
        let settings = Arc::new(SettingsProvider::new(icons)?);
        let disabled_providers = config.providers.disabled.iter().cloned().collect();

        Ok(Self {
            plugins_host: plugins.clone(),
            disabled_providers,
            windows_show_on_empty_query: config.providers.windows.show_on_empty_query,
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
        })
    }

    /// Starts a new launcher-visible session for providers that cache per-open state.
    pub fn begin_session(&self) {
        if !self.disabled_providers.contains("windows") {
            self.windows_provider.begin_session();
        }
    }

    /// Ends the current launcher-visible session and clears any per-open provider state.
    pub fn end_session(&self) {
        self.windows_provider.end_session();
    }

    /// Returns how many provider responses a session should wait for.
    pub fn provider_count_for_query(&self, query: &str) -> usize {
        let enabled = enabled_providers_for_query(
            query,
            &self.disabled_providers,
            self.windows_show_on_empty_query,
            self.plugins_host.is_routed_query(query),
        );
        ["windows", "apps", "settings", "plugins"]
            .into_iter()
            .filter(|provider| enabled.contains(provider))
            .count()
    }

    /// Starts a new search request across all providers.
    pub fn spawn_search(&self, proxy: EventLoopProxy<AppEvent>, generation: u64, query: String) {
        let enabled = enabled_providers_for_query(
            &query,
            &self.disabled_providers,
            self.windows_show_on_empty_query,
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
            self.plugins.search(proxy, generation, query);
        }
    }
}

fn enabled_providers_for_query(
    query: &str,
    disabled_providers: &HashSet<String>,
    windows_show_on_empty_query: bool,
    routed_plugin_query: bool,
) -> std::collections::HashSet<&'static str> {
    if !query.trim().is_empty() {
        if routed_plugin_query {
            return (!disabled_providers.contains("plugins"))
                .then_some("plugins")
                .into_iter()
                .collect();
        } else {
            return ["windows", "apps", "settings", "plugins"]
                .into_iter()
                .filter(|provider| !disabled_providers.contains(*provider))
                .collect();
        }
    }

    (windows_show_on_empty_query && !disabled_providers.contains("windows"))
        .then_some("windows")
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::enabled_providers_for_query;

    #[test]
    fn routed_plugin_queries_only_run_plugin_provider() {
        let disabled = HashSet::new();
        let enabled = enabled_providers_for_query("pass secret", &disabled, true, true);

        assert_eq!(enabled, HashSet::from(["plugins"]));
    }

    #[test]
    fn normal_non_empty_queries_keep_full_provider_fanout() {
        let disabled = HashSet::new();
        let enabled = enabled_providers_for_query("safari", &disabled, false, false);

        assert_eq!(
            enabled,
            HashSet::from(["windows", "apps", "settings", "plugins"])
        );
    }

    #[test]
    fn empty_queries_show_windows_when_enabled() {
        let disabled_providers = HashSet::new();
        let enabled = enabled_providers_for_query("", &disabled_providers, true, true);

        assert_eq!(enabled, HashSet::from(["windows"]));
    }

    #[test]
    fn empty_queries_run_no_providers_when_windows_disabled_for_empty_query() {
        let disabled_providers = HashSet::new();
        let enabled = enabled_providers_for_query("", &disabled_providers, false, false);

        assert_eq!(enabled, HashSet::new());
    }

    #[test]
    fn disabled_provider_never_runs_for_non_empty_query() {
        let disabled = HashSet::from(["windows".to_owned()]);
        let enabled = enabled_providers_for_query("safari", &disabled, true, false);

        assert_eq!(enabled, HashSet::from(["apps", "settings", "plugins"]));
    }

    #[test]
    fn empty_query_filters_disabled_provider() {
        let disabled_providers = HashSet::from(["windows".to_owned()]);
        let enabled = enabled_providers_for_query("", &disabled_providers, true, false);

        assert_eq!(enabled, HashSet::new());
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
