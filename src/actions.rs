use anyhow::Result;

use crate::macos;
use crate::plugins::{PluginExecutionContext, PluginHost};
use crate::types::Action;

pub fn execute_action(
    action: &Action,
    plugins: &PluginHost,
    context: &PluginExecutionContext,
) -> Result<Option<String>> {
    match action {
        Action::OpenApplication { path } => {
            macos::open_application(path)?;
            Ok(Some(format!("Opened {}", display_name(path))))
        }
        Action::OpenPath { path } => {
            macos::open_path(path)?;
            Ok(Some(format!("Opened {}", display_name(path))))
        }
        Action::OpenSettings { url, title } => {
            macos::open_settings(url)?;
            Ok(Some(format!("Opened {}", title)))
        }
        Action::FocusWindow {
            app_name,
            window_title,
        } => macos::focus_window(app_name, window_title),
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
