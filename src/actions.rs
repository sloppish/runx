use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::plugins::PluginHost;
use crate::types::Action;

pub fn execute_action(action: &Action, plugins: &PluginHost) -> Result<Option<String>> {
    match action {
        Action::OpenApplication { path } => {
            run_command("open", &[path.as_str()])?;
            Ok(Some(format!("Opened {}", display_name(path))))
        }
        Action::OpenPath { path } => {
            run_command("open", &[path.as_str()])?;
            Ok(Some(format!("Opened {}", display_name(path))))
        }
        Action::OpenSettings { url, title } => {
            run_command("open", &[url.as_str()])?;
            Ok(Some(format!("Opened {}", title)))
        }
        Action::FocusWindow {
            app_name,
            window_title,
        } => focus_window(app_name, window_title),
        Action::Plugin { plugin_id, payload } => plugins.run(plugin_id, payload),
    }
}

fn focus_window(app_name: &str, window_title: &str) -> Result<Option<String>> {
    let script = r#"
on run argv
    set targetApp to item 1 of argv
    set targetWindow to item 2 of argv
    tell application "System Events"
        tell application process targetApp
            set frontmost to true
            try
                perform action "AXRaise" of first window whose name is targetWindow
            end try
        end tell
    end tell
end run
"#;

    let output = Command::new("osascript")
        .args(["-e", script, "--", app_name, window_title])
        .output()
        .with_context(|| format!("failed to raise window {window_title}"))?;

    if output.status.success() {
        return Ok(Some(format!("Focused {}", window_title)));
    }

    run_command("open", &["-a", app_name]).with_context(|| {
        format!("failed to focus window {window_title} and also failed to activate {app_name}")
    })?;

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if stderr.is_empty() {
        Ok(Some(format!(
            "Activated {} (window focus requires Accessibility permission)",
            app_name
        )))
    } else {
        Ok(Some(format!(
            "Activated {} (direct window focus unavailable: {})",
            app_name, stderr
        )))
    }
}

fn run_command(program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("failed to run `{program}`"))?;

    if !status.success() {
        bail!("`{program}` exited with status {status}");
    }

    Ok(())
}

fn display_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(path)
        .to_owned()
}
