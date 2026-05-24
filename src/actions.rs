//! Thin adapter from selected result actions to concrete side effects.
//!
//! This module stays intentionally small: it pattern-matches on [`Action`] and
//! delegates the actual work to [`crate::macos`] or the plugin runtime.

use anyhow::Result;

use crate::macos;
use crate::plugins::{PluginActionOutcome, PluginExecutionContext, PluginHost};
use crate::types::Action;

/// Executes the action associated with the currently selected search result.
pub fn execute_action(
    action: &Action,
    all_windows: bool,
    plugins: &PluginHost,
    context: &PluginExecutionContext,
) -> Result<PluginActionOutcome> {
    match action {
        Action::Noop => Ok(PluginActionOutcome::Silent(None)),
        Action::OpenApplication { path } => {
            macos::open_application(path)?;
            Ok(PluginActionOutcome::Silent(Some(format!(
                "Opened {}",
                display_name(path)
            ))))
        }
        Action::OpenSettings { url, title } => {
            macos::open_settings(url)?;
            Ok(PluginActionOutcome::Silent(Some(format!(
                "Opened {}",
                title
            ))))
        }
        Action::FocusWindow {
            app_name,
            window_title,
            window_id,
            pid,
        } => {
            let result = if all_windows {
                macos::focus_window_and_activate_all_windows(
                    app_name,
                    window_title,
                    *window_id,
                    *pid,
                )
            } else {
                macos::focus_window(app_name, window_title, *window_id, *pid)
            };
            Ok(PluginActionOutcome::Silent(result?))
        }
        Action::Plugin { plugin_id, payload } => plugins.run(plugin_id, payload, context),
    }
}

fn display_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(path)
        .to_owned()
}
