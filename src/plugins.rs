//! Lua plugin hosting, routing, and validation.
//!
//! This module loads plugin files, exposes the `runx.*` runtime helpers to Lua,
//! validates plugin-produced items and actions, and routes configured command
//! prefixes to plugin search handlers.

mod commands;
mod item_validation;
pub mod manager;
mod routing;
mod runtime_api;

use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result, anyhow};
use mlua::{Function, LuaSerdeExt, Table};
use serde::Deserialize;
use serde_json::Value as JsonValue;
use tracing::warn;

use crate::{
    icons::IconCache,
    macos::FrontmostApp,
    types::{Action, PluginActionPayload, SearchItem},
};

use self::{
    commands::parse_shell_args,
    item_validation::{PluginItemWire, validate_plugin_item},
    routing::{PluginRoute, build_routes, match_command},
    runtime_api::{
        PluginRuntimeSession, empty_plugin_config, load_table, load_table_with_context_and_session,
        load_table_with_session,
    },
};

const PLUGIN_SESSION_KEY_CAP: usize = 1024;

/// In-memory plugin registry plus the routing/config needed to execute plugins.
#[derive(Clone, Default)]
pub struct PluginHost {
    plugins: Vec<LuaPlugin>,
    search_paths: Vec<PathBuf>,
    config: HashMap<String, JsonValue>,
    routes: Vec<PluginRoute>,
    routed_plugin_ids: HashSet<String>,
    session_store: Arc<Mutex<PluginSessionStore>>,
    icons: Option<Arc<IconCache>>,
}

/// Shared in-memory session values scoped by plugin id and cleared per launcher invocation.
#[derive(Debug, Default)]
pub(super) struct PluginSessionStore {
    plugins: HashMap<String, PluginSession>,
}

#[derive(Debug, Default)]
struct PluginSession {
    values: HashMap<String, JsonValue>,
    order: VecDeque<String>,
}

impl PluginSessionStore {
    pub(super) fn set(&mut self, plugin_id: &str, key: String, value: JsonValue) {
        let session = self.plugins.entry(plugin_id.to_owned()).or_default();
        if !session.values.contains_key(&key) {
            session.order.push_back(key.clone());
        }
        session.values.insert(key, value);

        while session.values.len() > PLUGIN_SESSION_KEY_CAP {
            let Some(oldest) = session.order.pop_front() else {
                break;
            };
            session.values.remove(&oldest);
        }
    }

    pub(super) fn get(&self, plugin_id: &str, key: &str) -> Option<JsonValue> {
        self.plugins
            .get(plugin_id)
            .and_then(|session| session.values.get(key))
            .cloned()
    }

    fn clear(&mut self) {
        self.plugins.clear();
    }
}

#[derive(Clone)]
struct LuaPlugin {
    id: String,
    name: String,
    badge: String,
    path: PathBuf,
    source: String,
    default_commands: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct PluginMetadata {
    id: Option<String>,
    name: Option<String>,
    badge: Option<String>,
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
        icons: Arc<IconCache>,
    ) -> Self {
        let mut plugins = Vec::new();

        for directory in directories {
            let Ok(entries) = fs::read_dir(directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let init = path.join("init.lua");
                if !init.exists() {
                    continue;
                }

                match load_plugin(&init, search_paths, Some(icons.clone())) {
                    Ok(plugin) => plugins.push(plugin),
                    Err(error) => {
                        warn!(
                            path = %path.display(),
                            error = %format!("{error:#}"),
                            "skipping plugin"
                        );
                        eprintln!("Skipping plugin {}: {error:#}", path.display());
                    }
                }
            }
        }

        plugins.sort_by(|left, right| left.name.cmp(&right.name));

        let mut merged_route_config = route_config;
        for plugin in &plugins {
            if !plugin.default_commands.is_empty() && !merged_route_config.contains_key(&plugin.id)
            {
                merged_route_config.insert(plugin.id.clone(), plugin.default_commands.clone());
            }
        }

        let mut routes = build_routes(merged_route_config);
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
            session_store: Arc::new(Mutex::new(PluginSessionStore::default())),
            icons: Some(icons),
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
                self.session_store.clone(),
                self.icons.clone(),
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
                self.session_store.clone(),
                self.icons.clone(),
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
            self.session_store.clone(),
            self.icons.clone(),
        )
    }

    /// Clears per-invocation plugin session data.
    pub fn clear_session_store(&self) {
        match self.session_store.lock() {
            Ok(mut store) => store.clear(),
            Err(poisoned) => poisoned.into_inner().clear(),
        }
    }

    fn plugin_config(&self, plugin_id: &str) -> JsonValue {
        self.config
            .get(plugin_id)
            .cloned()
            .unwrap_or_else(empty_plugin_config)
    }

    /// Returns whether the query matches any configured command route.
    pub fn is_routed_query(&self, query: &str) -> bool {
        self.match_route(query).is_some()
    }

    fn match_route<'a>(&'a self, query: &'a str) -> Option<(&'a PluginRoute, &'a str)> {
        self.routes
            .iter()
            .find_map(|route| match_command(query, &route.command).map(|args| (route, args)))
    }
}

fn load_plugin(
    path: &Path,
    search_paths: &[PathBuf],
    icons: Option<Arc<IconCache>>,
) -> Result<LuaPlugin> {
    let source = fs::read_to_string(path)
        .with_context(|| format!("failed to read plugin {}", path.display()))?;
    let (_lua, table) = load_table(path, &source, &empty_plugin_config(), search_paths, icons)?;
    let metadata = extract_metadata(&table)?;
    let default_commands = extract_default_commands(&table);

    let fallback_id = path
        .parent()
        .and_then(|dir| dir.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("plugin")
        .to_owned();

    Ok(LuaPlugin {
        id: metadata.id.unwrap_or_else(|| fallback_id.clone()),
        name: metadata.name.unwrap_or_else(|| fallback_id.clone()),
        badge: metadata.badge.unwrap_or_else(|| "PLG".to_owned()),
        path: path.to_path_buf(),
        source,
        default_commands,
    })
}

fn extract_metadata(table: &Table) -> Result<PluginMetadata> {
    Ok(PluginMetadata {
        id: table.get("id")?,
        name: table.get("name")?,
        badge: table.get("badge")?,
    })
}

fn extract_default_commands(table: &Table) -> HashMap<String, String> {
    let mut commands = HashMap::new();
    let Ok(Some(commands_table)) = table.get::<Option<Table>>("commands") else {
        return commands;
    };
    for pair in commands_table.pairs::<String, String>() {
        let Ok((key, value)) = pair else {
            continue;
        };
        let key = key.trim().to_owned();
        let value = value.trim().to_owned();
        if !key.is_empty() && !value.is_empty() {
            commands.insert(key, value);
        }
    }
    commands
}

fn items_from_wire(plugin: &LuaPlugin, raw_items: Vec<PluginItemWire>) -> Result<Vec<SearchItem>> {
    raw_items
        .into_iter()
        .enumerate()
        .map(|(index, item)| validate_plugin_item(plugin, index, item))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .map(|item| {
            Ok(SearchItem {
                id: item.id,
                provider: "plugins".to_owned(),
                badge: item.badge,
                icon: item.icon,
                title: item.title,
                subtitle: item.subtitle,
                compact: item.compact,
                raw_score: item.score,
                action: Action::Plugin {
                    plugin_id: plugin.id.clone(),
                    payload: item.payload,
                },
            })
        })
        .collect()
}

fn run_search(
    plugin: &LuaPlugin,
    query: &str,
    plugin_config: JsonValue,
    search_paths: &[PathBuf],
    session_store: Arc<Mutex<PluginSessionStore>>,
    icons: Option<Arc<IconCache>>,
) -> Result<Vec<SearchItem>> {
    let (lua, table) = load_table_with_session(
        &plugin.path,
        &plugin.source,
        &plugin.id,
        &plugin_config,
        search_paths,
        session_store,
        icons,
    )?;
    let search: Function = match table.get::<Option<Function>>("search")? {
        Some(function) => function,
        None => return Ok(Vec::new()),
    };
    let result = search.call::<mlua::Value>(query.to_owned())?;
    let raw_items: Vec<PluginItemWire> = lua.from_value(result)?;
    items_from_wire(plugin, raw_items)
}

fn run_search_handler(
    plugin: &LuaPlugin,
    handler_name: &str,
    args: &str,
    plugin_config: JsonValue,
    search_paths: &[PathBuf],
    session_store: Arc<Mutex<PluginSessionStore>>,
    icons: Option<Arc<IconCache>>,
) -> Result<Vec<SearchItem>> {
    let (lua, table) = load_table_with_session(
        &plugin.path,
        &plugin.source,
        &plugin.id,
        &plugin_config,
        search_paths,
        session_store,
        icons,
    )?;
    let search: Function = table
        .get::<Option<Function>>(handler_name)?
        .ok_or_else(|| anyhow!("plugin `{}` does not export `{}`", plugin.id, handler_name))?;
    let argv = lua.create_sequence_from(parse_shell_args(args)?)?;
    let result = search.call::<mlua::Value>((args.to_owned(), argv))?;
    let raw_items: Vec<PluginItemWire> = lua.from_value(result)?;
    items_from_wire(plugin, raw_items)
}

fn run_action(
    plugin: &LuaPlugin,
    payload: &PluginActionPayload,
    context: &PluginExecutionContext,
    plugin_config: JsonValue,
    search_paths: &[PathBuf],
    session_store: Arc<Mutex<PluginSessionStore>>,
    icons: Option<Arc<IconCache>>,
) -> Result<Option<String>> {
    let (lua, table) = load_table_with_context_and_session(
        &plugin.path,
        &plugin.source,
        context,
        &plugin_config,
        search_paths,
        PluginRuntimeSession::new(&plugin.id, session_store),
        icons,
    )?;
    let run: Function = match table.get::<Option<Function>>("run")? {
        Some(function) => function,
        None => return Ok(None),
    };
    let value = lua.to_value(&payload.0)?;
    let result = run.call::<mlua::Value>(value)?;

    if matches!(result, mlua::Value::Nil) {
        return Ok(Some(format!("Ran {}", plugin.name)));
    }

    let message: String = lua.from_value(result)?;
    if message.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(message))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use super::{
        LuaPlugin, PluginHost, PluginSessionStore, run_search, run_search_handler,
        runtime_api::empty_plugin_config,
    };

    fn session_store() -> Arc<Mutex<PluginSessionStore>> {
        Arc::new(Mutex::new(PluginSessionStore::default()))
    }

    #[test]
    fn routed_handlers_receive_raw_and_parsed_args() {
        let plugin = LuaPlugin {
            id: "args".to_owned(),
            name: "Args Plugin".to_owned(),
            badge: "ARG".to_owned(),
            path: PathBuf::from("args/init.lua"),
            source: r#"
                return {
                  search_echo = function(raw, argv)
                    return {
                      {
                        title = raw,
                        subtitle = table.concat(argv, "|"),
                        payload = { kind = "noop" },
                      },
                    }
                  end,
                }
            "#
            .to_owned(),
            default_commands: HashMap::new(),
        };

        let items = run_search_handler(
            &plugin,
            "search_echo",
            r#"1 "2 3" '4 "5"'"#,
            empty_plugin_config(),
            &[],
            session_store(),
            None,
        )
        .expect("handler search should succeed");

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, r#"1 "2 3" '4 "5"'"#);
        assert_eq!(items[0].subtitle, r#"1|2 3|4 "5""#);
        assert!(items[0].compact);
    }

    #[test]
    fn routed_handlers_can_request_full_style() {
        let plugin = LuaPlugin {
            id: "args".to_owned(),
            name: "Args Plugin".to_owned(),
            badge: "ARG".to_owned(),
            path: PathBuf::from("args/init.lua"),
            source: r#"
                return {
                  search_echo = function()
                    return {
                      {
                        title = "Echo",
                        style = "full",
                        payload = { kind = "noop" },
                      },
                    }
                  end,
                }
            "#
            .to_owned(),
            default_commands: HashMap::new(),
        };

        let items = run_search_handler(
            &plugin,
            "search_echo",
            "",
            empty_plugin_config(),
            &[],
            session_store(),
            None,
        )
        .expect("handler search should succeed");

        assert_eq!(items.len(), 1);
        assert!(!items[0].compact);
    }

    #[test]
    fn session_value_survives_separate_lua_evaluations() {
        let plugin = LuaPlugin {
            id: "tabs".to_owned(),
            name: "Tabs Plugin".to_owned(),
            badge: "TAB".to_owned(),
            path: PathBuf::from("tabs/init.lua"),
            source: r#"
                return {
                  search = function(query)
                    if query == "set" then
                      runx.session_set("tabs:snapshot", {
                        { title = "One", url = "https://example.com" },
                        { title = "Two", url = "https://example.org" },
                      })
                      return { { title = "stored", payload = {} } }
                    end

                    local tabs = runx.session_get("tabs:snapshot")
                    return {
                      {
                        title = tabs[2].title,
                        subtitle = tabs[2].url,
                        payload = {},
                      },
                    }
                  end,
                }
            "#
            .to_owned(),
            default_commands: HashMap::new(),
        };
        let store = session_store();

        run_search(
            &plugin,
            "set",
            empty_plugin_config(),
            &[],
            store.clone(),
            None,
        )
        .expect("first evaluation should store the snapshot");
        let items = run_search(&plugin, "get", empty_plugin_config(), &[], store, None)
            .expect("search works");

        assert_eq!(items[0].title, "Two");
        assert_eq!(items[0].subtitle, "https://example.org");
    }

    #[test]
    fn session_keys_are_scoped_by_plugin_id() {
        let source = r#"
            return {
              search = function(query)
                if query == "set" then
                  runx.session_set("shared", "owned")
                  return { { title = "stored", payload = {} } }
                end

                local value = runx.session_get("shared")
                return { { title = value or "missing", payload = {} } }
              end,
            }
        "#;
        let plugin_a = LuaPlugin {
            id: "a".to_owned(),
            name: "A".to_owned(),
            badge: "A".to_owned(),
            path: PathBuf::from("a/init.lua"),
            source: source.to_owned(),
            default_commands: HashMap::new(),
        };
        let plugin_b = LuaPlugin {
            id: "b".to_owned(),
            name: "B".to_owned(),
            badge: "B".to_owned(),
            path: PathBuf::from("b/init.lua"),
            source: source.to_owned(),
            default_commands: HashMap::new(),
        };
        let store = session_store();

        run_search(
            &plugin_a,
            "set",
            empty_plugin_config(),
            &[],
            store.clone(),
            None,
        )
        .expect("plugin A should store its own value");
        let items = run_search(&plugin_b, "get", empty_plugin_config(), &[], store, None)
            .expect("plugin B search works");

        assert_eq!(items[0].title, "missing");
    }

    #[test]
    fn session_set_rejects_unsupported_values() {
        let plugin = LuaPlugin {
            id: "bad".to_owned(),
            name: "Bad Plugin".to_owned(),
            badge: "BAD".to_owned(),
            path: PathBuf::from("bad/init.lua"),
            source: r#"
                return {
                  search = function()
                    runx.session_set("callback", function() end)
                    return {}
                  end,
                }
            "#
            .to_owned(),
            default_commands: HashMap::new(),
        };

        let error = run_search(
            &plugin,
            "",
            empty_plugin_config(),
            &[],
            session_store(),
            None,
        )
        .expect_err("functions are not JSON-serializable");

        assert!(format!("{error:#}").contains("function"));
    }

    #[test]
    fn session_store_evicts_oldest_key_for_plugin() {
        let mut store = PluginSessionStore::default();
        for index in 0..=super::PLUGIN_SESSION_KEY_CAP {
            store.set(
                "cache",
                format!("key-{index}"),
                serde_json::json!({ "index": index }),
            );
        }

        assert!(store.get("cache", "key-0").is_none());
        assert_eq!(
            store.get("cache", "key-1"),
            Some(serde_json::json!({ "index": 1 }))
        );
        assert_eq!(
            store.get("cache", "key-1024"),
            Some(serde_json::json!({ "index": 1024 }))
        );
    }

    #[test]
    fn clear_session_store_removes_cached_values() {
        let host = PluginHost::default();
        {
            let mut store = host.session_store.lock().expect("lock session store");
            store.set("plugin", "key".to_owned(), serde_json::json!("value"));
        }

        host.clear_session_store();

        let store = host.session_store.lock().expect("lock session store");
        assert!(store.get("plugin", "key").is_none());
    }

    #[test]
    fn runtime_exposes_clipboard_text_helper() {
        let plugin = LuaPlugin {
            id: "clipboard".to_owned(),
            name: "Clipboard Plugin".to_owned(),
            badge: "CLP".to_owned(),
            path: PathBuf::from("clipboard/init.lua"),
            source: r#"
                return {
                  search = function()
                    return {
                      {
                        title = type(runx.clipboard_text),
                        payload = { kind = "noop" },
                      },
                    }
                  end,
                }
            "#
            .to_owned(),
            default_commands: HashMap::new(),
        };

        let items = run_search(
            &plugin,
            "",
            empty_plugin_config(),
            &[],
            session_store(),
            None,
        )
        .expect("search should succeed");

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "function");
    }

    #[test]
    fn runtime_exposes_window_helpers() {
        let plugin = LuaPlugin {
            id: "windows".to_owned(),
            name: "Windows Plugin".to_owned(),
            badge: "WIN".to_owned(),
            path: PathBuf::from("windows/init.lua"),
            source: r#"
                return {
                  search = function()
                    return {
                      {
                        title = tostring(runx.api_version),
                        subtitle = table.concat({
                          type(runx.running_apps),
                          type(runx.icon_for_bundle_id),
                          type(runx.windows_for_pid),
                          type(runx.focus_window),
                        }, "|"),
                        payload = { kind = "noop" },
                      },
                    }
                  end,
                }
            "#
            .to_owned(),
            default_commands: HashMap::new(),
        };

        let items = run_search(
            &plugin,
            "",
            empty_plugin_config(),
            &[],
            session_store(),
            None,
        )
        .expect("search should succeed");

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "2");
        assert_eq!(items[0].subtitle, "function|function|function|function");
    }

    #[test]
    fn icon_for_bundle_id_returns_nil_without_icon_cache() {
        let plugin = LuaPlugin {
            id: "icons".to_owned(),
            name: "Icons Plugin".to_owned(),
            badge: "ICO".to_owned(),
            path: PathBuf::from("icons/init.lua"),
            source: r#"
                return {
                  search = function()
                    return {
                      {
                        title = tostring(runx.icon_for_bundle_id("com.example.Missing") == nil),
                        payload = { kind = "noop" },
                      },
                    }
                  end,
                }
            "#
            .to_owned(),
            default_commands: HashMap::new(),
        };

        let items = run_search(
            &plugin,
            "",
            empty_plugin_config(),
            &[],
            session_store(),
            None,
        )
        .expect("search should succeed");

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "true");
    }

    #[test]
    fn default_commands_merged_into_routes_when_no_user_config() {
        use super::routing::build_routes;

        let default_commands = HashMap::from([("greet".to_owned(), "search_greet".to_owned())]);
        let user_routes: HashMap<String, HashMap<String, String>> = HashMap::new();

        let mut merged = user_routes;
        let plugin_id = "greeter";
        if !default_commands.is_empty() && !merged.contains_key(plugin_id) {
            merged.insert(plugin_id.to_owned(), default_commands);
        }

        let routes = build_routes(merged);
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].command, "greet");
        assert_eq!(routes[0].handler, "search_greet");
        assert_eq!(routes[0].plugin_id, "greeter");
    }

    #[test]
    fn default_commands_are_overridden_by_user_config() {
        use super::routing::build_routes;

        let default_commands = HashMap::from([("greet".to_owned(), "search_greet".to_owned())]);
        let user_routes = HashMap::from([(
            "greeter".to_owned(),
            HashMap::from([("hi".to_owned(), "search_greet".to_owned())]),
        )]);

        // User config present — default_commands should NOT be merged
        let mut merged = user_routes.clone();
        let plugin_id = "greeter";
        if !default_commands.is_empty() && !merged.contains_key(plugin_id) {
            merged.insert(plugin_id.to_owned(), default_commands);
        }

        let routes = build_routes(merged);
        // Should only have user's "hi" route, not plugin's default "greet"
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].command, "hi");
    }

    #[test]
    fn extract_default_commands_from_lua_table() {
        let plugin = super::load_plugin(&PathBuf::from("test/init.lua"), &[], None);
        // load_plugin requires a real file, so test via a full plugin load instead
        let source = r#"
            return {
              id = "test",
              commands = {
                foo = "search_foo",
                bar = "search_bar",
              },
              search_foo = function() return {} end,
              search_bar = function() return {} end,
            }
        "#;
        let dir = tempfile::tempdir().expect("tempdir");
        let plugin_dir = dir.path().join("test");
        std::fs::create_dir(&plugin_dir).expect("mkdir");
        std::fs::write(plugin_dir.join("init.lua"), source).expect("write");

        let loaded = super::load_plugin(&plugin_dir.join("init.lua"), &[], None)
            .expect("plugin should load");
        drop(plugin);
        assert_eq!(loaded.default_commands.len(), 2);
        assert_eq!(loaded.default_commands["foo"], "search_foo");
        assert_eq!(loaded.default_commands["bar"], "search_bar");
    }

    #[test]
    fn extract_default_commands_returns_empty_when_no_commands_table() {
        let source = r#"
            return {
              id = "test",
              search = function() return {} end,
            }
        "#;
        let dir = tempfile::tempdir().expect("tempdir");
        let plugin_dir = dir.path().join("test");
        std::fs::create_dir(&plugin_dir).expect("mkdir");
        std::fs::write(plugin_dir.join("init.lua"), source).expect("write");

        let loaded = super::load_plugin(&plugin_dir.join("init.lua"), &[], None)
            .expect("plugin should load");
        assert!(loaded.default_commands.is_empty());
    }
}
