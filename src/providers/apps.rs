//! Provider for installed application bundles.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
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
    roots: Vec<PathBuf>,
    index: Mutex<AppIndex>,
    icons: Arc<IconCache>,
}

#[derive(Clone)]
struct AppRecord {
    name: String,
    path: String,
    score_adjustment: i64,
}

struct AppIndex {
    apps: Vec<AppRecord>,
    scanned_at: Instant,
}

const APP_INDEX_REFRESH_COOLDOWN: Duration = Duration::from_secs(10);

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

        let apps = scan_apps(&roots);

        Ok(Self {
            roots,
            index: Mutex::new(AppIndex {
                apps,
                scanned_at: Instant::now(),
            }),
            icons,
        })
    }

    /// Returns fuzzy matches for installed apps.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchItem>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }

        let mut matches = {
            let index = self
                .index
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            score_apps(&index.apps, query, limit)
        };

        if !matches.is_empty() {
            return Ok(build_items(matches, &self.icons));
        }

        let rescanned = self.refresh_if_stale();
        if rescanned {
            let index = self
                .index
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            matches = score_apps(&index.apps, query, limit);
        }

        Ok(build_items(matches, &self.icons))
    }

    fn refresh_if_stale(&self) -> bool {
        let should_refresh = {
            let index = self
                .index
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            index.scanned_at.elapsed() >= APP_INDEX_REFRESH_COOLDOWN
        };

        if !should_refresh {
            return false;
        }

        let apps = scan_apps(&self.roots);
        let mut index = self
            .index
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if index.scanned_at.elapsed() < APP_INDEX_REFRESH_COOLDOWN {
            return false;
        }

        index.apps = apps;
        index.scanned_at = Instant::now();
        true
    }
}

fn scan_apps(roots: &[PathBuf]) -> Vec<AppRecord> {
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
    apps
}

fn score_apps(apps: &[AppRecord], query: &str, limit: usize) -> Vec<(i64, AppRecord)> {
    let mut matches = Vec::new();
    for app in apps {
        let score = fuzzy_score(&app.name, query) + app.score_adjustment;
        if score <= 0 {
            continue;
        }

        matches.push((score, app.clone()));
    }

    matches.sort_by(|left, right| right.0.cmp(&left.0));
    matches.truncate(limit);
    matches
}

fn build_items(matches: Vec<(i64, AppRecord)>, icons: &IconCache) -> Vec<SearchItem> {
    matches
        .into_iter()
        .map(|(score, app)| SearchItem {
            id: format!("app:{}", app.path),
            provider: "apps".to_owned(),
            badge: "APP".to_owned(),
            icon: icons.icon_for_bundle(&app.path),
            title: app.name,
            subtitle: app.path.clone(),
            raw_score: score,
            action: Action::OpenApplication { path: app.path },
        })
        .collect()
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
