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
    windows: ProviderWorker,
    apps: ProviderWorker,
    settings: ProviderWorker,
    plugins: ProviderWorker,
    spotlight: ProviderWorker,
}

impl ProviderSet {
    pub const PROVIDER_COUNT: usize = 5;

    pub fn new(
        config: Arc<Config>,
        plugins: Arc<PluginHost>,
        icons: Arc<IconCache>,
    ) -> anyhow::Result<Self> {
        let windows = Arc::new(WindowsProvider::new(icons.clone()));
        let apps = Arc::new(AppProvider::new(icons.clone())?);
        let settings = Arc::new(SettingsProvider::new(icons.clone())?);
        let spotlight = Arc::new(SpotlightProvider::new(icons));

        Ok(Self {
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
            plugins: ProviderWorker::new("plugins", {
                let plugins = plugins;
                move |query| plugins.search(&query)
            })?,
            spotlight: ProviderWorker::new("spotlight", {
                let provider = spotlight;
                let limit = config.ranking.result_limit;
                move |query| provider.search(&query, limit)
            })?,
        })
    }

    pub fn provider_count(&self) -> usize {
        Self::PROVIDER_COUNT
    }

    pub fn spawn_search(&self, proxy: EventLoopProxy<AppEvent>, generation: u64, query: String) {
        self.windows
            .search(proxy.clone(), generation, query.clone());
        self.apps.search(proxy.clone(), generation, query.clone());
        self.settings
            .search(proxy.clone(), generation, query.clone());
        self.plugins
            .search(proxy.clone(), generation, query.clone());
        self.spotlight.search(proxy, generation, query);
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
