//! Background execution of activated search results.
//!
//! This module owns the plugin host handle used during activation and keeps the
//! "run this action off the UI thread" behavior out of `Launcher`.

use std::sync::Arc;

use tokio::runtime::Runtime;

use crate::{
    actions::execute_action,
    plugins::{PluginExecutionContext, PluginHost},
    types::{AppEvent, SearchItem},
};
use tracing::{error, info, warn};

/// Executes selected actions and reports outcomes back into the event loop.
#[derive(Clone)]
pub(crate) struct ActionRunner {
    plugins: Arc<PluginHost>,
}

impl ActionRunner {
    /// Creates a runner that shares the loaded plugin registry.
    pub(crate) fn new(plugins: Arc<PluginHost>) -> Self {
        Self { plugins }
    }

    /// Runs the selected item on a blocking worker and emits an `ActionOutcome`.
    ///
    /// When `stale_query` is provided, the rendered results were outdated at
    /// activation time. The runner re-executes the plugin search with the live
    /// query on the worker thread and activates the first result.
    pub(crate) fn spawn(
        &self,
        runtime: &Runtime,
        proxy: tao::event_loop::EventLoopProxy<AppEvent>,
        item: Option<SearchItem>,
        stale_query: Option<String>,
        all_windows: bool,
        context: PluginExecutionContext,
    ) {
        let plugins = self.plugins.clone();
        runtime.handle().spawn_blocking(move || {
            let resolved = match (&item, &stale_query) {
                (Some(_), _) => item,
                (None, Some(query)) => match plugins.search(query) {
                    Ok(mut items) if !items.is_empty() => Some(items.swap_remove(0)),
                    Ok(_) => None,
                    Err(e) => {
                        error!(
                            all_windows,
                            query = %query,
                            error = %format!("{e:#}"),
                            "plugin search failed during stale-query activation"
                        );
                        None
                    }
                },
                (None, None) => None,
            };
            let Some(item) = resolved else {
                warn!(
                    all_windows,
                    query = ?stale_query,
                    "activate found no results for current query"
                );
                let _ = proxy.send_event(AppEvent::ActionOutcome {
                    message: "No results for current query".to_owned(),
                    is_error: true,
                });
                return;
            };
            let result = execute_action(&item.action, all_windows, &plugins, &context);
            let (message, is_error) = match result {
                Ok(Some(message)) => (message, false),
                Ok(None) => ("Action completed".to_owned(), false),
                Err(error) => {
                    error!(all_windows, error = %format!("{error:#}"), "activate action failed");
                    (error.to_string(), true)
                }
            };
            if !is_error {
                info!(all_windows, %message, "activate action completed");
            }
            let _ = proxy.send_event(AppEvent::ActionOutcome { message, is_error });
        });
    }
}
