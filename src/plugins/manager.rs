//! Managed plugin installation and updates via Git.

use std::{collections::HashMap, fs, path::Path, process::Command, time::SystemTime};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::config::PluginInstallEntry;

/// State of a single managed plugin in the lockfile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockEntry {
    pub source: String,
    pub commit: String,
    #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub installed_at: String,
    pub updated_at: String,
}

/// Result of an update operation on a single plugin.
#[derive(Debug)]
pub enum UpdateResult {
    Updated,
    AlreadyUpToDate,
    Pinned,
    Failed(String),
}

/// Ensures all configured managed plugins are cloned locally.
/// Called during launcher bootstrap.
/// Result of ensure_installed: any install errors encountered.
pub struct InstallResult {
    pub errors: Vec<String>,
}

pub fn ensure_installed(plugins_dir: &Path, entries: &[PluginInstallEntry]) -> InstallResult {
    let lockfile_path = plugins_dir.parent().map(|p| p.join("plugins.lock.toml"));
    let mut lock = lockfile_path
        .as_deref()
        .map(read_lockfile)
        .unwrap_or_default();
    let mut errors = Vec::new();

    for entry in entries {
        match ensure_single_plugin(plugins_dir, entry, &mut lock) {
            Ok(_) => {}
            Err(error) => {
                let msg = format!("{}: {error:#}", entry.source);
                warn!(
                    source = %entry.source,
                    error = %format!("{error:#}"),
                    "failed to install managed plugin"
                );
                errors.push(msg);
            }
        }
    }

    prune_removed(plugins_dir, entries, &mut lock);

    if let Some(path) = lockfile_path.as_deref()
        && let Err(error) = write_lockfile(path, &lock)
    {
        warn!(error = %format!("{error:#}"), "failed to write plugins lockfile");
    }

    InstallResult { errors }
}

/// Updates all managed plugins that are not pinned to a tag.
/// Returns a map of plugin ID → update result.
pub fn update_all(
    plugins_dir: &Path,
    entries: &[PluginInstallEntry],
) -> HashMap<String, UpdateResult> {
    let lockfile_path = plugins_dir.parent().map(|p| p.join("plugins.lock.toml"));
    let mut lock = lockfile_path
        .as_deref()
        .map(read_lockfile)
        .unwrap_or_default();
    let mut results = HashMap::new();

    for entry in entries {
        let id = match plugin_id_for_entry(plugins_dir, entry) {
            Some(id) => id,
            None => {
                match ensure_single_plugin(plugins_dir, entry, &mut lock) {
                    Ok(id) => {
                        results.insert(id, UpdateResult::Updated);
                    }
                    Err(error) => {
                        let label = &entry.source;
                        results.insert(label.clone(), UpdateResult::Failed(format!("{error:#}")));
                    }
                }
                continue;
            }
        };

        let result = update_single_plugin(plugins_dir, &id, entry, &mut lock);
        results.insert(id, result);
    }

    if let Some(path) = lockfile_path.as_deref()
        && let Err(error) = write_lockfile(path, &lock)
    {
        warn!(error = %format!("{error:#}"), "failed to write plugins lockfile");
    }

    results
}

fn ensure_single_plugin(
    plugins_dir: &Path,
    entry: &PluginInstallEntry,
    lock: &mut HashMap<String, LockEntry>,
) -> Result<String> {
    let dir_name = entry
        .name
        .clone()
        .unwrap_or_else(|| dir_name_for_source(&entry.source));

    if let Some(id) = plugin_id_for_entry(plugins_dir, entry) {
        let plugin_path = plugins_dir.join(&id);
        if plugin_path.join("init.lua").exists() {
            return Ok(id);
        }
    }

    let target_dir = plugins_dir.join(&dir_name);
    if target_dir.join("init.lua").exists() {
        if !lock.contains_key(&dir_name) {
            let commit = read_head_commit(&target_dir).unwrap_or_default();
            let now = now_iso8601();
            lock.insert(
                dir_name.clone(),
                LockEntry {
                    source: entry.source.clone(),
                    commit,
                    git_ref: entry.git_ref.clone(),
                    branch: entry.branch.clone(),
                    installed_at: now.clone(),
                    updated_at: now,
                },
            );
        }
        return Ok(dir_name);
    }

    let temp_dir = tempfile::tempdir_in(plugins_dir)
        .context("failed to create temp directory for plugin clone")?;

    clone_repo(&entry.source, temp_dir.path(), entry)?;

    let init_lua = temp_dir.path().join("init.lua");
    if !init_lua.exists() {
        bail!("cloned repo does not contain init.lua at its root");
    }

    if target_dir.exists() {
        fs::remove_dir_all(&target_dir).with_context(|| {
            format!(
                "failed to remove existing directory {}",
                target_dir.display()
            )
        })?;
    }

    fs::rename(temp_dir.path(), &target_dir).with_context(|| {
        format!(
            "failed to move plugin from temp to {}",
            target_dir.display()
        )
    })?;

    let _ = temp_dir.keep();

    let commit = read_head_commit(&target_dir).unwrap_or_default();
    let now = now_iso8601();
    lock.insert(
        dir_name.clone(),
        LockEntry {
            source: entry.source.clone(),
            commit,
            git_ref: entry.git_ref.clone(),
            branch: entry.branch.clone(),
            installed_at: now.clone(),
            updated_at: now,
        },
    );

    info!(id = %dir_name, source = %entry.source, "installed managed plugin");
    Ok(dir_name)
}

fn update_single_plugin(
    plugins_dir: &Path,
    id: &str,
    entry: &PluginInstallEntry,
    lock: &mut HashMap<String, LockEntry>,
) -> UpdateResult {
    let plugin_path = plugins_dir.join(id);
    if !plugin_path.join(".git").exists() {
        return UpdateResult::Failed("not a git repository".to_owned());
    }

    if let Some(git_ref) = &entry.git_ref
        && !is_branch(&plugin_path, git_ref)
    {
        return UpdateResult::Pinned;
    }

    let before = read_head_commit(&plugin_path).unwrap_or_default();

    let fetch_output = match Command::new("git")
        .args(["fetch", "--prune"])
        .current_dir(&plugin_path)
        .output()
    {
        Ok(output) => output,
        Err(error) => return UpdateResult::Failed(format!("git fetch failed: {error}")),
    };
    if !fetch_output.status.success() {
        let stderr = String::from_utf8_lossy(&fetch_output.stderr);
        return UpdateResult::Failed(format!("git fetch failed: {stderr}"));
    }

    let tracking_ref = tracking_branch(&plugin_path);
    let target = match &tracking_ref {
        Some(r) => r.as_str(),
        None => return UpdateResult::Failed("no tracking branch found".to_owned()),
    };

    let reset_output = match Command::new("git")
        .args(["reset", "--hard", target])
        .current_dir(&plugin_path)
        .output()
    {
        Ok(output) => output,
        Err(error) => return UpdateResult::Failed(format!("git reset failed: {error}")),
    };
    if !reset_output.status.success() {
        let stderr = String::from_utf8_lossy(&reset_output.stderr);
        return UpdateResult::Failed(format!("git reset failed: {stderr}"));
    }

    let after = read_head_commit(&plugin_path).unwrap_or_default();

    if let Some(lock_entry) = lock.get_mut(id) {
        lock_entry.commit = after.clone();
        lock_entry.updated_at = now_iso8601();
    }

    if before == after {
        UpdateResult::AlreadyUpToDate
    } else {
        info!(id = %id, from = %&before[..7.min(before.len())], to = %&after[..7.min(after.len())], "updated managed plugin");
        UpdateResult::Updated
    }
}

fn is_branch(repo_path: &Path, name: &str) -> bool {
    Command::new("git")
        .args([
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{name}"),
        ])
        .current_dir(repo_path)
        .status()
        .is_ok_and(|s| s.success())
        || Command::new("git")
            .args([
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/remotes/origin/{name}"),
            ])
            .current_dir(repo_path)
            .status()
            .is_ok_and(|s| s.success())
}

fn tracking_branch(repo_path: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"])
        .current_dir(repo_path)
        .output()
        .ok()?;
    if output.status.success() {
        let name = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if !name.is_empty() {
            return Some(name);
        }
    }
    Some("origin/HEAD".to_owned())
}

fn clone_repo(source: &str, target: &Path, entry: &PluginInstallEntry) -> Result<()> {
    let mut cmd = Command::new("git");
    cmd.arg("clone");

    if entry.git_ref.is_none() {
        cmd.arg("--depth").arg("1");
    }

    if let Some(branch) = &entry.branch {
        cmd.arg("--branch").arg(branch);
    }

    cmd.arg(source).arg(target);

    let output = cmd.output().context("failed to run git clone")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git clone failed: {stderr}");
    }

    if let Some(git_ref) = &entry.git_ref {
        let checkout_output = Command::new("git")
            .args(["checkout", git_ref])
            .current_dir(target)
            .output()
            .context("failed to run git checkout")?;
        if !checkout_output.status.success() {
            let stderr = String::from_utf8_lossy(&checkout_output.stderr);
            bail!("git checkout `{git_ref}` failed: {stderr}");
        }
    }

    Ok(())
}

fn dir_name_for_source(source: &str) -> String {
    let stripped = source.trim_end_matches('/').trim_end_matches(".git");
    stripped
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("plugin")
        .to_owned()
}

fn prune_removed(
    plugins_dir: &Path,
    entries: &[PluginInstallEntry],
    lock: &mut HashMap<String, LockEntry>,
) {
    let active_dir_names: std::collections::HashSet<String> = entries
        .iter()
        .map(|e| {
            e.name
                .clone()
                .unwrap_or_else(|| dir_name_for_source(&e.source))
        })
        .collect();

    let stale: Vec<String> = lock
        .iter()
        .filter(|(k, _)| !active_dir_names.contains(k.as_str()))
        .map(|(k, _)| k.clone())
        .collect();

    for id in stale {
        lock.remove(&id);
        let plugin_path = plugins_dir.join(&id);
        if plugin_path.join(".git").exists() {
            if let Err(error) = fs::remove_dir_all(&plugin_path) {
                warn!(id = %id, error = %error, "failed to remove uninstalled managed plugin");
            } else {
                info!(id = %id, "removed managed plugin (no longer in config)");
            }
        }
    }
}

fn plugin_id_for_entry(plugins_dir: &Path, entry: &PluginInstallEntry) -> Option<String> {
    let lockfile_path = plugins_dir.parent()?.join("plugins.lock.toml");
    let lock = read_lockfile(&lockfile_path);
    let target_name = entry
        .name
        .clone()
        .unwrap_or_else(|| dir_name_for_source(&entry.source));
    lock.into_iter()
        .find(|(k, _)| *k == target_name)
        .map(|(k, _)| k)
}

fn read_head_commit(repo_path: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_path)
        .output()
        .context("failed to run git rev-parse HEAD")?;
    if !output.status.success() {
        bail!("git rev-parse HEAD failed");
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn read_lockfile(path: &Path) -> HashMap<String, LockEntry> {
    let Ok(content) = fs::read_to_string(path) else {
        return HashMap::new();
    };
    toml::from_str(&content).unwrap_or_default()
}

fn write_lockfile(path: &Path, lock: &HashMap<String, LockEntry>) -> Result<()> {
    let content = toml::to_string_pretty(lock).context("failed to serialize plugins lockfile")?;
    fs::write(path, content).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn now_iso8601() -> String {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Simple ISO 8601 without external crate
    let secs_per_day = 86400u64;
    let days = now / secs_per_day;
    let remaining = now % secs_per_day;
    let hours = remaining / 3600;
    let minutes = (remaining % 3600) / 60;
    let seconds = remaining % 60;

    // Days since 1970-01-01
    let mut y = 1970i64;
    let mut d = days as i64;
    loop {
        let year_days = if is_leap_year(y) { 366 } else { 365 };
        if d < year_days {
            break;
        }
        d -= year_days;
        y += 1;
    }
    let month_days: [i64; 12] = if is_leap_year(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut m = 0usize;
    for (i, &md) in month_days.iter().enumerate() {
        if d < md {
            m = i;
            break;
        }
        d -= md;
    }

    format!(
        "{y:04}-{:02}-{:02}T{hours:02}:{minutes:02}:{seconds:02}Z",
        m + 1,
        d + 1
    )
}

fn is_leap_year(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lockfile_roundtrip() {
        let mut lock = HashMap::new();
        lock.insert(
            "test-plugin".to_owned(),
            LockEntry {
                source: "https://github.com/user/test.git".to_owned(),
                commit: "abc123".to_owned(),
                git_ref: Some("v1.0.0".to_owned()),
                branch: None,
                installed_at: "2026-05-06T12:00:00Z".to_owned(),
                updated_at: "2026-05-06T12:00:00Z".to_owned(),
            },
        );

        let serialized = toml::to_string_pretty(&lock).unwrap();
        let deserialized: HashMap<String, LockEntry> = toml::from_str(&serialized).unwrap();

        assert_eq!(
            deserialized["test-plugin"].source,
            lock["test-plugin"].source
        );
        assert_eq!(deserialized["test-plugin"].commit, "abc123");
        assert_eq!(
            deserialized["test-plugin"].git_ref,
            Some("v1.0.0".to_owned())
        );
        assert!(deserialized["test-plugin"].branch.is_none());
    }

    #[test]
    fn now_iso8601_produces_valid_format() {
        let timestamp = now_iso8601();
        assert!(timestamp.ends_with('Z'));
        assert_eq!(timestamp.len(), 20);
        assert_eq!(&timestamp[4..5], "-");
        assert_eq!(&timestamp[7..8], "-");
        assert_eq!(&timestamp[10..11], "T");
        assert_eq!(&timestamp[13..14], ":");
        assert_eq!(&timestamp[16..17], ":");
    }

    #[test]
    fn dir_name_from_https_url() {
        assert_eq!(
            dir_name_for_source("https://github.com/user/my-plugin.git"),
            "my-plugin"
        );
    }

    #[test]
    fn dir_name_from_url_without_git_suffix() {
        assert_eq!(
            dir_name_for_source("https://github.com/user/cool-plugin"),
            "cool-plugin"
        );
    }

    #[test]
    fn dir_name_from_local_path() {
        assert_eq!(dir_name_for_source("/Users/dev/projects/emoji"), "emoji");
    }

    #[test]
    fn dir_name_strips_trailing_slash() {
        assert_eq!(dir_name_for_source("/some/path/to/plugin/"), "plugin");
    }
}
