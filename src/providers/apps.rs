//! Provider for installed application bundles.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};
use directories::BaseDirs;
use plist::Value;
use walkdir::{DirEntry, WalkDir};

use crate::{
    icons::IconCache,
    scoring::fuzzy_score,
    types::{Action, SearchItem},
};

/// Searches the local app bundle index built from common application roots.
pub struct AppProvider {
    apps: Vec<AppRecord>,
    icons: Arc<IconCache>,
}

#[derive(Clone)]
struct AppRecord {
    name: String,
    path: String,
    score_adjustment: i64,
}

impl AppProvider {
    /// Scans well-known application directories and builds the in-memory index.
    pub fn new(icons: Arc<IconCache>) -> Result<Self> {
        let base_dirs = BaseDirs::new().context("could not determine the home directory")?;
        let mut roots = vec![
            PathBuf::from("/Applications"),
            PathBuf::from("/System/Applications"),
            PathBuf::from("/System/Applications/Utilities"),
            PathBuf::from("/System/Library/CoreServices"),
            base_dirs.home_dir().join("Applications"),
        ];
        roots.retain(|path| path.exists());

        let mut seen = HashSet::new();
        let mut apps = Vec::new();

        for root in roots {
            for entry in WalkDir::new(root)
                .max_depth(4)
                .follow_links(true)
                .into_iter()
                .filter_entry(filter_entry)
                .flatten()
            {
                if !entry.file_type().is_dir() {
                    continue;
                }

                let path = entry.path();
                if path.extension().and_then(|value| value.to_str()) != Some("app") {
                    continue;
                }

                let normalized = path.to_string_lossy().to_string();
                if !seen.insert(normalized.clone()) {
                    continue;
                }

                let name = path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or("Application")
                    .to_owned();
                let score_adjustment = app_score_adjustment(path);
                apps.push(AppRecord {
                    name,
                    path: normalized,
                    score_adjustment,
                });
            }
        }

        apps.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(Self { apps, icons })
    }

    /// Returns fuzzy matches for installed apps.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchItem>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }

        let mut matches = Vec::new();
        for app in &self.apps {
            let score = fuzzy_score(&app.name, query) + app.score_adjustment;
            if score <= 0 {
                continue;
            }

            matches.push((score, app.clone()));
        }

        matches.sort_by(|left, right| right.0.cmp(&left.0));
        matches.truncate(limit);
        Ok(matches
            .into_iter()
            .map(|(score, app)| SearchItem {
                id: format!("app:{}", app.path),
                provider: "apps".to_owned(),
                badge: "APP".to_owned(),
                icon: self.icons.icon_for_bundle(&app.path),
                title: app.name,
                subtitle: app.path.clone(),
                raw_score: score,
                action: Action::OpenApplication { path: app.path },
            })
            .collect())
    }
}

fn filter_entry(entry: &DirEntry) -> bool {
    if entry.file_type().is_dir() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) == Some("app") {
            return true;
        }
    }

    let name = entry.file_name().to_string_lossy();
    !name.starts_with('.')
}

fn app_score_adjustment(path: &Path) -> i64 {
    let mut adjustment = 0;
    let path_text = path.to_string_lossy();
    let info_path = path.join("Contents/Info.plist");
    let Ok(plist) = Value::from_file(&info_path) else {
        return path_penalty(&path_text);
    };
    let Some(dict) = plist.as_dictionary() else {
        return path_penalty(&path_text);
    };

    let is_agent = dict
        .get("LSUIElement")
        .or_else(|| dict.get("NSUIElement"))
        .is_some_and(plist_truthy);
    let is_background = dict.get("LSBackgroundOnly").is_some_and(plist_truthy);

    if is_agent || is_background {
        adjustment -= 180;
    }

    adjustment + path_penalty(&path_text)
}

fn path_penalty(path: &str) -> i64 {
    if path.contains("/System/Library/CoreServices/") {
        return -110;
    }

    if path.contains("/System/Applications/Utilities/") {
        return -70;
    }

    0
}

fn plist_truthy(value: &Value) -> bool {
    match value {
        Value::Boolean(value) => *value,
        Value::String(text) => matches!(text.to_ascii_lowercase().as_str(), "1" | "true" | "yes"),
        Value::Integer(number) => number.as_signed().is_some_and(|value| value != 0),
        _ => false,
    }
}
