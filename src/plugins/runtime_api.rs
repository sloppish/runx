use std::{
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use mlua::{Lua, LuaSerdeExt, Table};
use serde_json::Value as JsonValue;

use crate::{
    macos::{copy_text_to_clipboard, read_clipboard_text, type_text_into_previous_app},
    scoring::fuzzy_score,
};

use super::{
    PluginExecutionContext,
    commands::{exec_capture, exec_status, parse_shell_args, walk_files},
};

const PLUGIN_API_VERSION: u32 = 1;

pub(super) fn load_table(
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

pub(super) fn load_table_with_context(
    path: &Path,
    source: &str,
    context: &PluginExecutionContext,
    plugin_config: &JsonValue,
    search_paths: &[PathBuf],
) -> Result<(Lua, Table)> {
    let lua = Lua::new();
    lua.gc_stop();
    install_runtime(&lua, path, context, plugin_config, search_paths)?;
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
            Ok(fuzzy_score(&candidate, &query))
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

    runtime.set("plugin_path", plugin_path.display().to_string())?;
    runtime.set("plugin_dir", plugin_dir.display().to_string())?;
    runtime.set("plugin_config", lua.to_value(plugin_config)?)?;

    lua.globals().set("runx", runtime)?;
    Ok(())
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
