use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use mlua::{Lua, LuaSerdeExt, Table};
use serde_json::Value as JsonValue;

use crate::{
    macos::{
        accessibility_windows_for_pid, copy_text_to_clipboard, focus_window,
        focus_window_and_activate_all_windows, read_clipboard_text, running_applications,
        type_text_into_previous_app,
    },
    scoring::fuzzy_score,
};

use super::{
    PluginExecutionContext, PluginSessionStore,
    commands::{exec_capture, exec_status, parse_shell_args, walk_files},
};

const PLUGIN_API_VERSION: u32 = 2;

pub(super) fn load_table(
    path: &Path,
    source: &str,
    plugin_config: &JsonValue,
    search_paths: &[PathBuf],
) -> Result<(Lua, Table)> {
    load_table_internal(
        path,
        source,
        &PluginExecutionContext::default(),
        plugin_config,
        search_paths,
        None,
    )
}

pub(super) fn load_table_with_session(
    path: &Path,
    source: &str,
    plugin_id: &str,
    plugin_config: &JsonValue,
    search_paths: &[PathBuf],
    session_store: Arc<Mutex<PluginSessionStore>>,
) -> Result<(Lua, Table)> {
    load_table_internal(
        path,
        source,
        &PluginExecutionContext::default(),
        plugin_config,
        search_paths,
        Some(PluginRuntimeSession {
            plugin_id: plugin_id.to_owned(),
            store: session_store,
        }),
    )
}

pub(super) fn load_table_with_context_and_session(
    path: &Path,
    source: &str,
    plugin_id: &str,
    context: &PluginExecutionContext,
    plugin_config: &JsonValue,
    search_paths: &[PathBuf],
    session_store: Arc<Mutex<PluginSessionStore>>,
) -> Result<(Lua, Table)> {
    load_table_internal(
        path,
        source,
        context,
        plugin_config,
        search_paths,
        Some(PluginRuntimeSession {
            plugin_id: plugin_id.to_owned(),
            store: session_store,
        }),
    )
}

fn load_table_internal(
    path: &Path,
    source: &str,
    context: &PluginExecutionContext,
    plugin_config: &JsonValue,
    search_paths: &[PathBuf],
    session: Option<PluginRuntimeSession>,
) -> Result<(Lua, Table)> {
    let lua = Lua::new();
    lua.gc_stop();
    install_runtime(&lua, path, context, plugin_config, search_paths, session)?;
    let table: Table = lua
        .load(source)
        .set_name(path.to_string_lossy().as_ref())
        .eval()
        .with_context(|| format!("failed to evaluate {}", path.display()))?;
    Ok((lua, table))
}

fn install_runtime(
    lua: &Lua,
    plugin_path: &Path,
    context: &PluginExecutionContext,
    plugin_config: &JsonValue,
    search_paths: &[PathBuf],
    session: Option<PluginRuntimeSession>,
) -> Result<()> {
    let runtime = lua.create_table()?;
    let search_paths = search_paths.to_vec();
    let plugin_path = plugin_path.to_path_buf();
    let plugin_dir = plugin_path
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);

    let dir = plugin_dir.display();
    lua.globals()
        .get::<Table>("package")?
        .set("path", format!("{dir}/?.lua;{dir}/?/init.lua"))?;

    runtime.set("api_version", PLUGIN_API_VERSION)?;

    runtime.set(
        "fuzzy_score",
        lua.create_function(|_, (candidate, query): (String, String)| {
            let query_lower = query.trim().to_ascii_lowercase();
            Ok(fuzzy_score(&candidate, &query_lower))
        })?,
    )?;

    runtime.set(
        "getenv",
        lua.create_function(|_, name: String| Ok(env::var(name).ok()))?,
    )?;

    runtime.set(
        "parse_args",
        lua.create_function(|lua, raw: String| {
            let args = parse_shell_args(&raw).map_err(mlua::Error::external)?;
            lua.create_sequence_from(args)
        })?,
    )?;

    runtime.set(
        "walk_files",
        lua.create_function(|_, root: String| {
            walk_files(Path::new(&root)).map_err(mlua::Error::external)
        })?,
    )?;

    runtime.set(
        "read_text",
        lua.create_function(|_, path: String| {
            fs::read_to_string(&path).map_err(mlua::Error::external)
        })?,
    )?;

    let exec_capture_paths = search_paths.clone();
    runtime.set(
        "exec_capture",
        lua.create_function(
            move |_,
                  (program, args, first_line_only, trim): (
                String,
                Vec<String>,
                Option<bool>,
                Option<bool>,
            )| {
                exec_capture(
                    &program,
                    &args,
                    first_line_only.unwrap_or(false),
                    trim.unwrap_or(true),
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

    let exec_json_paths = search_paths;
    runtime.set(
        "exec_json",
        lua.create_function(move |lua, (program, args): (String, Vec<String>)| {
            let output = exec_capture(&program, &args, false, true, &exec_json_paths)
                .map_err(mlua::Error::external)?;
            let json = serde_json::from_str::<JsonValue>(&output).map_err(mlua::Error::external)?;
            lua.to_value(&json)
        })?,
    )?;

    runtime.set(
        "json_decode",
        lua.create_function(move |lua, text: String| {
            let value = serde_json::from_str::<JsonValue>(&text).map_err(mlua::Error::external)?;
            lua.to_value(&value)
        })?,
    )?;

    if let Some(session) = session {
        let set_session = session.clone();
        runtime.set(
            "session_set",
            lua.create_function(move |lua, (key, value): (String, mlua::Value)| {
                let json: JsonValue = lua.from_value(value)?;
                with_session_store(&set_session.store, |store| {
                    store.set(&set_session.plugin_id, key, json);
                });
                Ok(true)
            })?,
        )?;

        let get_session = session;
        runtime.set(
            "session_get",
            lua.create_function(move |lua, key: String| {
                let value = with_session_store(&get_session.store, |store| {
                    store.get(&get_session.plugin_id, &key)
                });
                match value {
                    Some(value) => lua.to_value(&value),
                    None => Ok(mlua::Value::Nil),
                }
            })?,
        )?;
    }

    runtime.set(
        "copy_text",
        lua.create_function(move |_, text: String| {
            copy_text_to_clipboard(&text).map_err(mlua::Error::external)
        })?,
    )?;

    runtime.set(
        "clipboard_text",
        lua.create_function(move |_, ()| read_clipboard_text().map_err(mlua::Error::external))?,
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

    runtime.set(
        "running_apps",
        lua.create_function(|lua, ()| lua.to_value(&running_applications()))?,
    )?;

    runtime.set(
        "windows_for_pid",
        lua.create_function(|lua, pid: i64| lua.to_value(&accessibility_windows_for_pid(pid)))?,
    )?;

    runtime.set(
        "focus_window",
        lua.create_function(|_, options: Table| {
            let pid: i64 = options.get("pid")?;
            let window_id: u32 = options.get("window_id")?;
            let app_name = options
                .get::<Option<String>>("app_name")?
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| format!("pid {pid}"));
            let window_title = options
                .get::<Option<String>>("window_title")?
                .or(options.get::<Option<String>>("title")?)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| format!("window {window_id}"));
            let all_windows = options.get::<Option<bool>>("all_windows")?.unwrap_or(false);

            let message = if all_windows {
                focus_window_and_activate_all_windows(&app_name, &window_title, window_id, pid)
            } else {
                focus_window(&app_name, &window_title, window_id, pid)
            }
            .map_err(mlua::Error::external)?;

            Ok(message.unwrap_or_else(|| format!("Focused {window_title}")))
        })?,
    )?;

    runtime.set("plugin_path", plugin_path.display().to_string())?;
    runtime.set("plugin_dir", plugin_dir.display().to_string())?;
    runtime.set("plugin_config", lua.to_value(plugin_config)?)?;

    lua.globals().set("runx", runtime)?;
    Ok(())
}

#[derive(Clone)]
struct PluginRuntimeSession {
    plugin_id: String,
    store: Arc<Mutex<PluginSessionStore>>,
}

fn with_session_store<T>(
    store: &Arc<Mutex<PluginSessionStore>>,
    f: impl FnOnce(&mut PluginSessionStore) -> T,
) -> T {
    match store.lock() {
        Ok(mut store) => f(&mut store),
        Err(poisoned) => {
            let mut store = poisoned.into_inner();
            f(&mut store)
        }
    }
}

pub(super) fn empty_plugin_config() -> JsonValue {
    JsonValue::Object(Default::default())
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
