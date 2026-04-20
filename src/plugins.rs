//! Lua plugin hosting, routing, and validation.
//!
//! This module loads plugin files, exposes the `runx.*` runtime helpers to Lua,
//! validates plugin-produced items and actions, and routes configured command
//! prefixes to plugin search handlers.

use std::{
    collections::{HashMap, HashSet},
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context, Result, anyhow, bail};
use mlua::{Function, Lua, LuaSerdeExt, Table};
use serde::Deserialize;
use serde_json::Value as JsonValue;

use crate::{
    macos::{FrontmostApp, type_text_into_previous_app},
    scoring::fuzzy_score,
    types::{Action, PluginActionPayload, SearchItem},
};

const PLUGIN_API_VERSION: u32 = 1;

/// In-memory plugin registry plus the routing/config needed to execute plugins.
#[derive(Clone, Default)]
pub struct PluginHost {
    plugins: Vec<LuaPlugin>,
    search_paths: Vec<PathBuf>,
    config: HashMap<String, JsonValue>,
    routes: Vec<PluginRoute>,
    routed_plugin_ids: HashSet<String>,
}

#[derive(Clone)]
struct LuaPlugin {
    id: String,
    name: String,
    badge: String,
    path: PathBuf,
    source: String,
}

#[derive(Debug, Clone)]
struct PluginRoute {
    plugin_id: String,
    command: String,
    handler: String,
}

#[derive(Debug, Deserialize)]
struct PluginMetadata {
    id: Option<String>,
    name: Option<String>,
    badge: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PluginItemWire {
    id: Option<String>,
    title: String,
    subtitle: Option<String>,
    score: Option<i64>,
    badge: Option<String>,
    action: PluginActionPayload,
}

#[derive(Debug)]
struct PluginItem {
    id: String,
    title: String,
    subtitle: String,
    score: i64,
    badge: String,
    action: PluginActionPayload,
}

/// Minimal Rust context passed into plugin action execution.
#[derive(Debug, Clone, Default)]
pub struct PluginExecutionContext {
    pub previous_app: Option<FrontmostApp>,
}

impl PluginHost {
    /// Loads all Lua plugins from the configured directories.
    pub fn load(
        directories: &[PathBuf],
        search_paths: &[PathBuf],
        config: HashMap<String, JsonValue>,
        route_config: HashMap<String, HashMap<String, String>>,
    ) -> Self {
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

                match load_plugin(&path, search_paths) {
                    Ok(plugin) => plugins.push(plugin),
                    Err(error) => eprintln!("Skipping plugin {}: {error:#}", path.display()),
                }
            }
        }

        plugins.sort_by(|left, right| left.name.cmp(&right.name));
        let mut routes = build_routes(route_config);
        routes.sort_by(|left, right| {
            right
                .command
                .len()
                .cmp(&left.command.len())
                .then_with(|| left.command.cmp(&right.command))
                .then_with(|| left.plugin_id.cmp(&right.plugin_id))
        });
        let routed_plugin_ids = routes
            .iter()
            .map(|route| route.plugin_id.clone())
            .collect::<HashSet<_>>();
        Self {
            plugins,
            search_paths: search_paths.to_vec(),
            config,
            routes,
            routed_plugin_ids,
        }
    }

    /// Returns search items contributed by plugins for the current query.
    pub fn search(&self, query: &str) -> Result<Vec<SearchItem>> {
        if let Some((route, args)) = self.match_route(query) {
            let plugin = self
                .plugins
                .iter()
                .find(|plugin| plugin.id == route.plugin_id)
                .with_context(|| format!("unknown plugin `{}`", route.plugin_id))?;

            return run_search_handler(
                plugin,
                &route.handler,
                args,
                self.plugin_config(&plugin.id),
                &self.search_paths,
            )
            .with_context(|| {
                format!(
                    "plugin `{}` handler `{}` search failed",
                    route.plugin_id, route.handler
                )
            });
        }

        let mut items = Vec::new();
        for plugin in &self.plugins {
            if self.routed_plugin_ids.contains(&plugin.id) {
                continue;
            }
            let plugin_items = run_search(
                plugin,
                query,
                self.plugin_config(&plugin.id),
                &self.search_paths,
            )
            .with_context(|| format!("plugin `{}` search failed", plugin.id))?;
            items.extend(plugin_items);
        }
        Ok(items)
    }

    /// Runs the selected plugin action.
    pub fn run(
        &self,
        plugin_id: &str,
        payload: &PluginActionPayload,
        context: &PluginExecutionContext,
    ) -> Result<Option<String>> {
        let plugin = self
            .plugins
            .iter()
            .find(|plugin| plugin.id == plugin_id)
            .with_context(|| format!("unknown plugin `{plugin_id}`"))?;
        run_action(
            plugin,
            payload,
            context,
            self.plugin_config(plugin_id),
            &self.search_paths,
        )
    }

    fn plugin_config(&self, plugin_id: &str) -> JsonValue {
        self.config
            .get(plugin_id)
            .cloned()
            .unwrap_or_else(empty_plugin_config)
    }

    fn match_route<'a>(&'a self, query: &'a str) -> Option<(&'a PluginRoute, &'a str)> {
        self.routes
            .iter()
            .find_map(|route| match_command(query, &route.command).map(|args| (route, args)))
    }
}

fn build_routes(route_config: HashMap<String, HashMap<String, String>>) -> Vec<PluginRoute> {
    let mut routes = Vec::new();

    for (plugin_id, commands) in route_config {
        for (command, handler) in commands {
            routes.push(PluginRoute {
                plugin_id: plugin_id.clone(),
                command,
                handler,
            });
        }
    }

    routes
}

fn match_command<'a>(query: &'a str, command: &str) -> Option<&'a str> {
    if query == command {
        return Some("");
    }

    query
        .strip_prefix(command)
        .and_then(|suffix| suffix.strip_prefix(' '))
}

fn load_plugin(path: &Path, search_paths: &[PathBuf]) -> Result<LuaPlugin> {
    let source = fs::read_to_string(path)
        .with_context(|| format!("failed to read plugin {}", path.display()))?;
    let (lua, table) = load_table(path, &source, &empty_plugin_config(), search_paths)?;
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

fn run_search(
    plugin: &LuaPlugin,
    query: &str,
    plugin_config: JsonValue,
    search_paths: &[PathBuf],
) -> Result<Vec<SearchItem>> {
    let (lua, table) = load_table(&plugin.path, &plugin.source, &plugin_config, search_paths)?;
    let search: Function = match table.get::<Option<Function>>("search")? {
        Some(function) => function,
        None => return Ok(Vec::new()),
    };
    let result = search.call::<mlua::Value>(query.to_owned())?;
    let raw_items: Vec<PluginItemWire> = lua.from_value(result)?;

    Ok(raw_items
        .into_iter()
        .enumerate()
        .map(|(index, item)| validate_plugin_item(plugin, index, item))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .map(|item| SearchItem {
            id: item.id,
            provider: "plugins".to_owned(),
            badge: item.badge,
            icon: None,
            title: item.title,
            subtitle: item.subtitle,
            raw_score: item.score,
            action: Action::Plugin {
                plugin_id: plugin.id.clone(),
                payload: item.action,
            },
        })
        .collect())
}

fn run_search_handler(
    plugin: &LuaPlugin,
    handler_name: &str,
    args: &str,
    plugin_config: JsonValue,
    search_paths: &[PathBuf],
) -> Result<Vec<SearchItem>> {
    let (lua, table) = load_table(&plugin.path, &plugin.source, &plugin_config, search_paths)?;
    let search: Function = table
        .get::<Option<Function>>(handler_name)?
        .ok_or_else(|| anyhow!("plugin `{}` does not export `{}`", plugin.id, handler_name))?;
    let result = search.call::<mlua::Value>(args.to_owned())?;
    let raw_items: Vec<PluginItemWire> = lua.from_value(result)?;

    Ok(raw_items
        .into_iter()
        .enumerate()
        .map(|(index, item)| validate_plugin_item(plugin, index, item))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .map(|item| SearchItem {
            id: item.id,
            provider: "plugins".to_owned(),
            badge: item.badge,
            icon: None,
            title: item.title,
            subtitle: item.subtitle,
            raw_score: item.score,
            action: Action::Plugin {
                plugin_id: plugin.id.clone(),
                payload: item.action,
            },
        })
        .collect())
}

fn validate_plugin_item(
    plugin: &LuaPlugin,
    index: usize,
    item: PluginItemWire,
) -> Result<PluginItem> {
    item.action
        .validate()
        .map_err(|error| anyhow!("plugin item {} action is invalid: {error}", index + 1))?;

    let title = item.title.trim();
    if title.is_empty() {
        bail!("plugin item {} must have a non-empty title", index + 1);
    }

    let id = match item.id {
        Some(id) => {
            let id = id.trim();
            if id.is_empty() {
                bail!("plugin item {} has an empty `id`", index + 1);
            }
            id.to_owned()
        }
        None => format!("plugin:{}:{}", plugin.id, title),
    };

    let badge = match item.badge {
        Some(badge) => {
            let badge = badge.trim();
            if badge.is_empty() {
                bail!("plugin item {} has an empty `badge`", index + 1);
            }
            badge.to_owned()
        }
        None => plugin.badge.clone(),
    };

    let subtitle = item
        .subtitle
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(plugin.name.as_str())
        .to_owned();

    Ok(PluginItem {
        id,
        title: title.to_owned(),
        subtitle,
        score: item.score.unwrap_or(0),
        badge,
        action: item.action,
    })
}

fn run_action(
    plugin: &LuaPlugin,
    payload: &PluginActionPayload,
    context: &PluginExecutionContext,
    plugin_config: JsonValue,
    search_paths: &[PathBuf],
) -> Result<Option<String>> {
    let (lua, table) = load_table_with_context(
        &plugin.path,
        &plugin.source,
        context,
        &plugin_config,
        search_paths,
    )?;
    let run: Function = table
        .get::<Option<Function>>("run")?
        .ok_or_else(|| anyhow!("plugin `{}` does not export a `run` function", plugin.id))?;
    let value = lua.to_value(&payload.as_json())?;
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

fn load_table(
    path: &Path,
    source: &str,
    plugin_config: &JsonValue,
    search_paths: &[PathBuf],
) -> Result<(Lua, Table)> {
    load_table_with_context(
        path,
        source,
        &PluginExecutionContext::default(),
        plugin_config,
        search_paths,
    )
}

fn load_table_with_context(
    path: &Path,
    source: &str,
    context: &PluginExecutionContext,
    plugin_config: &JsonValue,
    search_paths: &[PathBuf],
) -> Result<(Lua, Table)> {
    let lua = Lua::new();
    install_runtime(&lua, context, plugin_config, search_paths)?;
    let table: Table = lua
        .load(source)
        .set_name(path.to_string_lossy().as_ref())
        .eval()
        .with_context(|| format!("failed to evaluate {}", path.display()))?;
    Ok((lua, table))
}

fn install_runtime(
    lua: &Lua,
    context: &PluginExecutionContext,
    plugin_config: &JsonValue,
    search_paths: &[PathBuf],
) -> Result<()> {
    let runtime = lua.create_table()?;
    let search_paths = search_paths.to_vec();
    runtime.set("api_version", PLUGIN_API_VERSION)?;

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

    let exec_capture_paths = search_paths.clone();
    runtime.set(
        "exec_capture",
        lua.create_function(
            move |_, (program, args, first_line_only): (String, Vec<String>, Option<bool>)| {
                exec_capture(
                    &program,
                    &args,
                    first_line_only.unwrap_or(false),
                    &exec_capture_paths,
                )
                .map_err(mlua::Error::external)
            },
        )?,
    )?;

    let exec_status_paths = search_paths.clone();
    runtime.set(
        "exec_status",
        lua.create_function(
            move |_, (program, args, silence_stderr): (String, Vec<String>, Option<bool>)| {
                exec_status(
                    &program,
                    &args,
                    silence_stderr.unwrap_or(false),
                    &exec_status_paths,
                )
                .map_err(mlua::Error::external)?;
                Ok(true)
            },
        )?,
    )?;

    let exec_json_paths = search_paths.clone();
    runtime.set(
        "exec_json",
        lua.create_function(move |lua, (program, args): (String, Vec<String>)| {
            let output = exec_capture(&program, &args, false, &exec_json_paths)
                .map_err(mlua::Error::external)?;
            let json = serde_json::from_str::<JsonValue>(&output).map_err(mlua::Error::external)?;
            lua.to_value(&json)
        })?,
    )?;

    let copy_text_paths = search_paths.clone();
    runtime.set(
        "copy_text",
        lua.create_function(move |_, text: String| {
            let mut child = command_for_plugin("pbcopy", &copy_text_paths)
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
            type_text_into_previous_app(&text, previous_app.as_ref()).map_err(mlua::Error::external)
        })?
    })?;

    runtime.set(
        "home_dir",
        lua.create_function(|_, ()| {
            let home = BaseHome::resolve()?;
            Ok(home.display().to_string())
        })?,
    )?;

    runtime.set("plugin_config", lua.to_value(plugin_config)?)?;

    lua.globals().set("runx", runtime)?;
    Ok(())
}

fn empty_plugin_config() -> JsonValue {
    JsonValue::Object(Default::default())
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

fn exec_capture(
    program: &str,
    args: &[String],
    first_line_only: bool,
    search_paths: &[PathBuf],
) -> Result<String> {
    let output = command_for_plugin(program, search_paths)
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

fn exec_status(
    program: &str,
    args: &[String],
    silence_stderr: bool,
    search_paths: &[PathBuf],
) -> Result<()> {
    let mut command = command_for_plugin(program, search_paths);
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

fn command_for_plugin(program: &str, search_paths: &[PathBuf]) -> Command {
    let mut command = Command::new(program);
    command.env("PATH", plugin_search_path(search_paths));
    command
}

fn plugin_search_path(search_paths: &[PathBuf]) -> OsString {
    let mut paths = match env::var_os("PATH") {
        Some(value) => env::split_paths(&value).collect::<Vec<_>>(),
        None => Vec::new(),
    };

    for path in search_paths {
        if !paths.iter().any(|candidate| candidate == path) {
            paths.push(path.clone());
        }
    }

    env::join_paths(paths).unwrap_or_default()
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::env;
    use std::path::PathBuf;

    use serde_json::json;

    use super::{
        LuaPlugin, PluginItemWire, build_routes, match_command, plugin_search_path,
        validate_plugin_item,
    };

    #[test]
    fn plugin_search_path_includes_configured_paths() {
        let path = plugin_search_path(&[PathBuf::from("/opt/homebrew/bin")])
            .to_string_lossy()
            .into_owned();

        assert!(path.contains("/opt/homebrew/bin"));
    }

    #[test]
    fn plugin_search_path_keeps_existing_path_entries() {
        let path = plugin_search_path(&[PathBuf::from("/opt/homebrew/bin")])
            .to_string_lossy()
            .into_owned();

        if let Some(existing) = env::var_os("PATH") {
            let existing = existing.to_string_lossy();
            if !existing.is_empty() {
                assert!(path.contains(existing.as_ref()));
            }
        }
    }

    fn test_plugin() -> LuaPlugin {
        LuaPlugin {
            id: "test".to_owned(),
            name: "Test Plugin".to_owned(),
            badge: "TST".to_owned(),
            path: PathBuf::from("test.lua"),
            source: String::new(),
        }
    }

    #[test]
    fn validate_plugin_item_rejects_empty_title() {
        let item = serde_json::from_value::<PluginItemWire>(json!({
            "title": "   ",
            "action": { "kind": "copy" }
        }))
        .expect("valid wire item");

        let error = validate_plugin_item(&test_plugin(), 0, item)
            .expect_err("empty titles should fail")
            .to_string();

        assert!(error.contains("non-empty title"));
    }

    #[test]
    fn validate_plugin_item_rejects_invalid_action() {
        let item = serde_json::from_value::<PluginItemWire>(json!({
            "title": "Copy secret",
            "action": { "kind": "  " }
        }))
        .expect("valid wire item");

        let error = validate_plugin_item(&test_plugin(), 0, item)
            .expect_err("invalid action should fail")
            .to_string();

        assert!(error.contains("action is invalid"));
    }

    #[test]
    fn validate_plugin_item_falls_back_to_plugin_defaults() {
        let item = serde_json::from_value::<PluginItemWire>(json!({
            "title": "Copy secret",
            "action": { "kind": "copy_secret" }
        }))
        .expect("valid wire item");

        let item = validate_plugin_item(&test_plugin(), 0, item).expect("validated item");

        assert_eq!(item.id, "plugin:test:Copy secret");
        assert_eq!(item.badge, "TST");
        assert_eq!(item.subtitle, "Test Plugin");
    }

    #[test]
    fn match_command_supports_exact_and_spaced_forms() {
        assert_eq!(match_command("calc", "calc"), Some(""));
        assert_eq!(match_command("calc 2+2", "calc"), Some("2+2"));
        assert_eq!(match_command("calculator", "calc"), None);
    }

    #[test]
    fn build_routes_preserves_plugin_and_handler() {
        let routes = build_routes(HashMap::from([(
            "calc".to_owned(),
            HashMap::from([("calc".to_owned(), "search_calc".to_owned())]),
        )]));

        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].plugin_id, "calc");
        assert_eq!(routes[0].command, "calc");
        assert_eq!(routes[0].handler, "search_calc");
    }
}
