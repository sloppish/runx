//! Provider backed by the `mdfind` Spotlight CLI.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use anyhow::{Context, Result, bail};

use crate::{
    icons::IconCache,
    scoring::fuzzy_score,
    types::{Action, SearchItem},
};

/// Searches Spotlight for app and preference-pane style matches.
pub struct SpotlightProvider {
    icons: Arc<IconCache>,
}

impl SpotlightProvider {
    /// Creates a Spotlight provider backed by the shared icon cache.
    pub fn new(icons: Arc<IconCache>) -> Self {
        Self { icons }
    }

    /// Returns Spotlight matches for the current query.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchItem>> {
        let trimmed = query.trim();
        if trimmed.len() < 2 {
            return Ok(Vec::new());
        }

        let expression = format!(
            "kMDItemDisplayName ==[cd] \"*{}*\" || kMDItemFSName ==[cd] \"*{}*\"",
            escape_query(trimmed),
            escape_query(trimmed)
        );

        let output = Command::new("mdfind")
            .arg(expression)
            .output()
            .context("failed to run `mdfind` for Spotlight results")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            if stderr.is_empty() {
                bail!("`mdfind` exited with status {}", output.status);
            }
            bail!("`mdfind` failed: {stderr}");
        }

        let stdout = String::from_utf8(output.stdout).context("Spotlight output was not UTF-8")?;
        let mut items = Vec::new();
        for path in stdout.lines() {
            if items.len() >= limit {
                break;
            }

            let item = build_item(path, trimmed, &self.icons);
            if let Some(item) = item {
                items.push(item);
            }
        }

        Ok(items)
    }
}

fn build_item(path: &str, query: &str, icons: &IconCache) -> Option<SearchItem> {
    let title = Path::new(path)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(path)
        .to_owned();
    let score = fuzzy_score(&title, query);
    if score <= 0 {
        return None;
    }

    if path.ends_with(".app") {
        return Some(SearchItem {
            id: format!("spotlight:{}", path),
            provider: "spotlight".to_owned(),
            badge: "SPT".to_owned(),
            icon: icons.icon_for_bundle(path),
            title,
            subtitle: "Spotlight application match".to_owned(),
            raw_score: score,
            action: Action::OpenApplication {
                path: path.to_owned(),
            },
        });
    }

    if path.ends_with(".prefPane") {
        return Some(SearchItem {
            id: format!("spotlight:{}", path),
            provider: "spotlight".to_owned(),
            badge: "SPT".to_owned(),
            icon: icons.system_settings_icon(),
            title,
            subtitle: "Spotlight preference pane".to_owned(),
            raw_score: score,
            action: Action::OpenPath {
                path: path.to_owned(),
            },
        });
    }

    None
}

fn escape_query(query: &str) -> String {
    query.replace('\\', "\\\\").replace('"', "\\\"")
}
