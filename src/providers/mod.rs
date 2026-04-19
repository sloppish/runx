pub mod apps;
pub mod settings;
pub mod spotlight;
pub mod windows;

use std::sync::Arc;

use tokio::runtime::Handle;

use tao::event_loop::EventLoopProxy;

use crate::{
    config::Config,
    plugins::PluginHost,
    types::{AppEvent, SearchItem},
};

use self::{
    apps::AppProvider, settings::SettingsProvider, spotlight::SpotlightProvider,
    windows::WindowsProvider,
};

#[derive(Clone)]
pub struct ProviderSet {
    apps: Arc<AppProvider>,
    settings: Arc<SettingsProvider>,
    spotlight: Arc<SpotlightProvider>,
    windows: Arc<WindowsProvider>,
    plugins: Arc<PluginHost>,
    config: Arc<Config>,
}

impl ProviderSet {
    pub fn new(config: Arc<Config>, plugins: Arc<PluginHost>) -> anyhow::Result<Self> {
        Ok(Self {
            apps: Arc::new(AppProvider::new()?),
            settings: Arc::new(SettingsProvider::new()?),
            spotlight: Arc::new(SpotlightProvider::default()),
            windows: Arc::new(WindowsProvider::default()),
            plugins,
            config,
        })
    }

    pub fn spawn_search(
        &self,
        runtime: &Handle,
        proxy: EventLoopProxy<AppEvent>,
        generation: u64,
        query: String,
    ) {
        self.spawn_provider(
            runtime,
            proxy.clone(),
            generation,
            "windows",
            query.clone(),
            {
                let provider = self.windows.clone();
                let limit = self.config.ranking.result_limit;
                move |query| provider.search(&query, limit)
            },
        );

        self.spawn_provider(runtime, proxy.clone(), generation, "apps", query.clone(), {
            let provider = self.apps.clone();
            let limit = self.config.ranking.result_limit;
            move |query| provider.search(&query, limit)
        });

        self.spawn_provider(
            runtime,
            proxy.clone(),
            generation,
            "settings",
            query.clone(),
            {
                let provider = self.settings.clone();
                let limit = self.config.ranking.result_limit;
                move |query| provider.search(&query, limit)
            },
        );

        self.spawn_provider(
            runtime,
            proxy.clone(),
            generation,
            "plugins",
            query.clone(),
            {
                let plugins = self.plugins.clone();
                move |query| plugins.search(&query)
            },
        );

        self.spawn_provider(runtime, proxy, generation, "spotlight", query, {
            let provider = self.spotlight.clone();
            let limit = self.config.ranking.result_limit;
            move |query| provider.search(&query, limit)
        });
    }

    fn spawn_provider<F>(
        &self,
        runtime: &Handle,
        proxy: EventLoopProxy<AppEvent>,
        generation: u64,
        name: &'static str,
        query: String,
        search: F,
    ) where
        F: FnOnce(String) -> anyhow::Result<Vec<SearchItem>> + Send + 'static,
    {
        runtime.spawn_blocking(move || match search(query) {
            Ok(items) => {
                let _ = proxy.send_event(AppEvent::ProviderItems {
                    generation,
                    provider: name.to_owned(),
                    items,
                });
            }
            Err(error) => {
                let _ = proxy.send_event(AppEvent::ProviderError {
                    generation,
                    provider: name.to_owned(),
                    message: error.to_string(),
                });
            }
        });
    }
}
