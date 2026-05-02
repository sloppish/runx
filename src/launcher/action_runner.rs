//! Background execution of activated search results.
//!
//! This module owns the plugin host handle used during activation and keeps the
//! "run this action off the UI thread" behavior out of `Launcher`.

use std::sync::Arc;

use tokio::runtime::Runtime;

use crate::{
    actions::execute_action,
    debug_log,
    plugins::{PluginExecutionContext, PluginHost},
    types::{AppEvent, SearchItem},
};

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
    pub(crate) fn spawn(
        &self,
        runtime: &Runtime,
        proxy: tao::event_loop::EventLoopProxy<AppEvent>,
        item: SearchItem,
        all_windows: bool,
        context: PluginExecutionContext,
    ) {
        let plugins = self.plugins.clone();
        runtime.handle().spawn_blocking(move || {
            let result = execute_action(&item.action, all_windows, &plugins, &context);
            let (message, is_error) = match result {
                Ok(Some(message)) => (message, false),
                Ok(None) => ("Action completed".to_owned(), false),
                Err(error) => (error.to_string(), true),
            };
            debug_log::append(format!(
                "activate outcome all_windows={} is_error={} message={:?}",
                all_windows, is_error, message
            ));
            let _ = proxy.send_event(AppEvent::ActionOutcome { message, is_error });
        });
    }
}
