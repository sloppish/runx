use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use anyhow::{Context, Result};
use directories::BaseDirs;

pub(super) fn runtime_paths() -> Result<(PathBuf, PathBuf, PathBuf)> {
    let base_dirs =
        BaseDirs::new().context("could not resolve the current user's home directory")?;
    let root_dir = base_dirs.config_dir().join("runx");
    let plugin_dir = root_dir.join("plugins");
    let config_path = root_dir.join("config.toml");
    Ok((root_dir, plugin_dir, config_path))
}

/// Returns the current modification time for the config file, if it exists.
pub fn config_modified_at(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok()?.modified().ok()
}

pub(super) fn resolve_path(root_dir: &Path, home_dir: &Path, raw: &str) -> PathBuf {
    if let Some(stripped) = raw.strip_prefix("~/") {
        return home_dir.join(stripped);
    }

    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        root_dir.join(path)
    }
}

pub(super) fn dedup_paths(paths: &mut Vec<PathBuf>) {
    let mut seen = HashSet::new();
    paths.retain(|path| seen.insert(path.clone()));
}
