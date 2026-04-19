use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context, Result, anyhow, bail};
use mlua::{Function, Lua, LuaSerdeExt, Table};
use serde::Deserialize;
use serde_json::Value as JsonValue;

use crate::{
    macos::{
        FrontmostApp, accessibility_denied, automation_denied, ensure_accessibility_trusted,
        open_accessibility_settings, open_automation_settings, reactivate_previous_app,
    },
    scoring::fuzzy_score,
    types::{Action, SearchItem},
};

#[derive(Clone, Default)]
pub struct PluginHost {
    plugins: Vec<LuaPlugin>,
}

#[derive(Clone)]
struct LuaPlugin {
    id: String,
    name: String,
    badge: String,
    path: PathBuf,
    source: String,
}

#[derive(Debug, Deserialize)]
struct PluginMetadata {
    id: Option<String>,
    name: Option<String>,
    badge: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PluginItem {
    id: Option<String>,
    title: String,
    subtitle: Option<String>,
    score: Option<i64>,
    badge: Option<String>,
    action: JsonValue,
}

#[derive(Debug, Clone, Default)]
pub struct PluginExecutionContext {
    pub previous_app: Option<FrontmostApp>,
}

impl PluginHost {
    pub fn load(directories: &[PathBuf]) -> Self {
        let mut plugins = Vec::new();

        for directory in directories {
            let Ok(entries) = fs::read_dir(directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|value| value.to_str()) != Some("lua") {
                    continue;
                }

                match load_plugin(&path) {
                    Ok(plugin) => plugins.push(plugin),
                    Err(error) => eprintln!("Skipping plugin {}: {error:#}", path.display()),
                }
            }
        }

        plugins.sort_by(|left, right| left.name.cmp(&right.name));
        Self { plugins }
    }

    pub fn search(&self, query: &str) -> Result<Vec<SearchItem>> {
        let mut items = Vec::new();
        for plugin in &self.plugins {
            let plugin_items = run_search(plugin, query)
                .with_context(|| format!("plugin `{}` search failed", plugin.id))?;
            items.extend(plugin_items);
        }
        Ok(items)
    }

    pub fn run(
        &self,
        plugin_id: &str,
        payload: &JsonValue,
        context: &PluginExecutionContext,
    ) -> Result<Option<String>> {
        let plugin = self
            .plugins
            .iter()
            .find(|plugin| plugin.id == plugin_id)
            .with_context(|| format!("unknown plugin `{plugin_id}`"))?;
        run_action(plugin, payload, context)
    }
}

fn load_plugin(path: &Path) -> Result<LuaPlugin> {
    let source = fs::read_to_string(path)
        .with_context(|| format!("failed to read plugin {}", path.display()))?;
    let (lua, table) = load_table(path, &source)?;
    let metadata = extract_metadata(&lua, &table)?;

    let fallback_id = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("plugin")
        .to_owned();

    Ok(LuaPlugin {
        id: metadata.id.unwrap_or_else(|| fallback_id.clone()),
        name: metadata.name.unwrap_or_else(|| fallback_id.clone()),
        badge: metadata.badge.unwrap_or_else(|| "PLG".to_owned()),
        path: path.to_path_buf(),
        source,
    })
}

fn extract_metadata(lua: &Lua, table: &Table) -> Result<PluginMetadata> {
    let _ = lua;
    Ok(PluginMetadata {
        id: table.get("id")?,
        name: table.get("name")?,
        badge: table.get("badge")?,
    })
}

fn run_search(plugin: &LuaPlugin, query: &str) -> Result<Vec<SearchItem>> {
    let (lua, table) = load_table(&plugin.path, &plugin.source)?;
    let search: Function = match table.get::<Option<Function>>("search")? {
        Some(function) => function,
        None => return Ok(Vec::new()),
    };
    let result = search.call::<mlua::Value>(query.to_owned())?;
    let items: Vec<PluginItem> = lua.from_value(result)?;

    Ok(items
        .into_iter()
        .map(|item| SearchItem {
            id: item
                .id
                .unwrap_or_else(|| format!("plugin:{}:{}", plugin.id, item.title)),
            provider: "plugins".to_owned(),
            badge: item.badge.unwrap_or_else(|| plugin.badge.clone()),
            title: item.title,
            subtitle: item.subtitle.unwrap_or_else(|| plugin.name.clone()),
            raw_score: item.score.unwrap_or(0),
            action: Action::Plugin {
                plugin_id: plugin.id.clone(),
                payload: item.action,
            },
        })
        .collect())
}

fn run_action(
    plugin: &LuaPlugin,
    payload: &JsonValue,
    context: &PluginExecutionContext,
) -> Result<Option<String>> {
    let (lua, table) = load_table_with_context(&plugin.path, &plugin.source, context)?;
    let run: Function = table
        .get::<Option<Function>>("run")?
        .ok_or_else(|| anyhow!("plugin `{}` does not export a `run` function", plugin.id))?;
    let value = lua.to_value(payload)?;
    let result = run.call::<mlua::Value>(value)?;

    if matches!(result, mlua::Value::Nil) {
        return Ok(Some(format!("Ran {}", plugin.name)));
    }

    if let Ok(message) = lua.from_value::<String>(result.clone()) {
        if message.trim().is_empty() {
            return Ok(None);
        }
        return Ok(Some(message));
    }

    #[derive(Deserialize)]
    struct Outcome {
        message: Option<String>,
    }

    let outcome: Outcome = lua.from_value(result)?;
    Ok(outcome.message)
}

fn load_table(path: &Path, source: &str) -> Result<(Lua, Table)> {
    load_table_with_context(path, source, &PluginExecutionContext::default())
}

fn load_table_with_context(
    path: &Path,
    source: &str,
    context: &PluginExecutionContext,
) -> Result<(Lua, Table)> {
    let lua = Lua::new();
    install_runtime(&lua, context)?;
    let table: Table = lua
        .load(source)
        .set_name(path.to_string_lossy().as_ref())
        .eval()
        .with_context(|| format!("failed to evaluate {}", path.display()))?;
    Ok((lua, table))
}

fn install_runtime(lua: &Lua, context: &PluginExecutionContext) -> Result<()> {
    let runtime = lua.create_table()?;

    runtime.set(
        "fuzzy_score",
        lua.create_function(|_, (candidate, query): (String, String)| {
            Ok(fuzzy_score(&candidate, &query))
        })?,
    )?;

    runtime.set(
        "getenv",
        lua.create_function(|_, name: String| Ok(env::var(name).ok()))?,
    )?;

    runtime.set(
        "walk_files",
        lua.create_function(|_, root: String| {
            walk_files(Path::new(&root)).map_err(mlua::Error::external)
        })?,
    )?;

    runtime.set(
        "exec_capture",
        lua.create_function(
            |_, (program, args, first_line_only): (String, Vec<String>, Option<bool>)| {
                exec_capture(&program, &args, first_line_only.unwrap_or(false))
                    .map_err(mlua::Error::external)
            },
        )?,
    )?;

    runtime.set(
        "exec_status",
        lua.create_function(
            |_, (program, args, silence_stderr): (String, Vec<String>, Option<bool>)| {
                exec_status(&program, &args, silence_stderr.unwrap_or(false))
                    .map_err(mlua::Error::external)?;
                Ok(true)
            },
        )?,
    )?;

    runtime.set(
        "copy_text",
        lua.create_function(|_, text: String| {
            let mut child = Command::new("pbcopy")
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(mlua::Error::external)?;

            if let Some(stdin) = child.stdin.as_mut() {
                use std::io::Write;
                stdin
                    .write_all(text.as_bytes())
                    .map_err(mlua::Error::external)?;
            }

            let status = child.wait().map_err(mlua::Error::external)?;
            if !status.success() {
                return Err(mlua::Error::external("pbcopy failed"));
            }

            Ok("Copied to clipboard".to_owned())
        })?,
    )?;

    runtime.set("type_text", {
        let previous_app = context.previous_app.clone();
        lua.create_function(move |_, text: String| {
            type_text(&text, previous_app.as_ref()).map_err(mlua::Error::external)
        })?
    })?;

    runtime.set(
        "home_dir",
        lua.create_function(|_, ()| {
            let home = BaseHome::resolve()?;
            Ok(home.display().to_string())
        })?,
    )?;

    lua.globals().set("runx", runtime)?;
    Ok(())
}

fn type_text(text: &str, previous_app: Option<&FrontmostApp>) -> Result<String> {
    if !ensure_accessibility_trusted(true) {
        open_accessibility_settings();
        bail!(
            "Runx needs Accessibility permission to type into other apps. Approve the system prompt or enable your terminal/runx in System Settings > Privacy & Security > Accessibility, then retry."
        );
    }

    reactivate_previous_app(previous_app)?;

    let script = r#"
on run argv
    tell application "System Events"
        keystroke item 1 of argv
    end tell
end run
"#;
    let output = Command::new("osascript")
        .args(["-e", script, "--", text])
        .output()
        .context("failed to launch osascript for text typing")?;

    if output.status.success() {
        return Ok("Typed into the previous app".to_owned());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if automation_denied(&stderr) {
        open_automation_settings();
        bail!(
            "macOS blocked Apple Events to System Events. Allow your terminal/runx under System Settings > Privacy & Security > Automation, then retry."
        );
    }

    if accessibility_denied(&stderr) {
        open_accessibility_settings();
        bail!(
            "macOS blocked assistive access while typing. Enable your terminal/runx in Privacy & Security > Accessibility, then retry."
        );
    }

    if stderr.is_empty() {
        bail!("typing failed for an unknown macOS reason");
    }

    bail!("{stderr}");
}

fn walk_files(root: &Path) -> Result<Vec<String>> {
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    walk_directory(root, root, &mut files)?;
    files.sort();
    Ok(files)
}

fn walk_directory(root: &Path, current: &Path, files: &mut Vec<String>) -> Result<()> {
    for entry in
        fs::read_dir(current).with_context(|| format!("failed to read {}", current.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk_directory(root, &path, files)?;
            continue;
        }

        let relative = path
            .strip_prefix(root)
            .with_context(|| format!("failed to relativize {}", path.display()))?;
        files.push(relative.to_string_lossy().to_string());
    }

    Ok(())
}

fn exec_capture(program: &str, args: &[String], first_line_only: bool) -> Result<String> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("failed to run `{program}`"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        if stderr.is_empty() {
            bail!("`{program}` exited with status {}", output.status);
        }
        bail!("{stderr}");
    }

    let stdout = String::from_utf8(output.stdout).context("command output was not UTF-8")?;
    let text = if first_line_only {
        stdout.lines().next().unwrap_or_default().trim().to_owned()
    } else {
        stdout.trim().to_owned()
    };

    if text.is_empty() {
        bail!("`{program}` returned an empty result");
    }

    Ok(text)
}

fn exec_status(program: &str, args: &[String], silence_stderr: bool) -> Result<()> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    if silence_stderr {
        command.stderr(Stdio::null());
    }

    let output = command
        .output()
        .with_context(|| format!("failed to run `{program}`"))?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if stderr.is_empty() {
        bail!("`{program}` exited with status {}", output.status);
    }
    bail!("{stderr}");
}

struct BaseHome;

impl BaseHome {
    fn resolve() -> mlua::Result<PathBuf> {
        let home = directories::BaseDirs::new().ok_or_else(|| {
            mlua::Error::external("could not resolve the current user's home directory")
        })?;
        Ok(home.home_dir().to_path_buf())
    }
}
