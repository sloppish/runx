//! Provider for installed application bundles.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use directories::BaseDirs;
use plist::Value;
use walkdir::{DirEntry, WalkDir};

use crate::{
    config::AppsProviderConfig,
    icons::IconCache,
    scoring::fuzzy_score,
    types::{Action, SearchItem},
};

/// Searches the local app bundle index built from common application roots.
pub struct AppProvider {
    roots: Vec<AppScanRoot>,
    index: Arc<Mutex<AppIndex>>,
    icons: Arc<IconCache>,
    config: AppsProviderConfig,
}

#[derive(Clone)]
struct AppScanRoot {
    path: PathBuf,
    allow_nested_apps: bool,
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
    refreshing: bool,
}

const APP_INDEX_REFRESH_COOLDOWN: Duration = Duration::from_secs(10);

impl AppProvider {
    /// Builds the provider and kicks off an initial background scan.
    pub fn new(
        icons: Arc<IconCache>,
        config: AppsProviderConfig,
        additional_roots: Vec<PathBuf>,
    ) -> Result<Self> {
        let base_dirs = BaseDirs::new().context("could not determine the home directory")?;
        let mut roots = vec![
            AppScanRoot {
                path: PathBuf::from("/Applications"),
                allow_nested_apps: false,
            },
            AppScanRoot {
                path: PathBuf::from("/System/Applications"),
                allow_nested_apps: false,
            },
            AppScanRoot {
                path: PathBuf::from("/System/Applications/Utilities"),
                allow_nested_apps: false,
            },
            AppScanRoot {
                path: PathBuf::from("/System/Library/CoreServices"),
                allow_nested_apps: false,
            },
            AppScanRoot {
                path: base_dirs.home_dir().join("Applications"),
                allow_nested_apps: false,
            },
        ];
        roots.extend(additional_roots.into_iter().map(|path| AppScanRoot {
            path,
            allow_nested_apps: true,
        }));
        roots.retain(|root| root.path.exists());

        let index = Arc::new(Mutex::new(AppIndex {
            apps: Vec::new(),
            scanned_at: Instant::now(),
            refreshing: true,
        }));

        let shared_index = Arc::clone(&index);
        let scan_roots = roots.clone();
        let _ = thread::Builder::new()
            .name("runx-app-index-init".to_owned())
            .spawn(move || {
                let apps = scan_apps(&scan_roots);
                let mut guard = lock_or_recover(&shared_index);
                guard.apps = apps;
                guard.scanned_at = Instant::now();
                guard.refreshing = false;
            });

        Ok(Self {
            roots,
            index,
            icons,
            config,
        })
    }

    /// Returns fuzzy matches for installed apps.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchItem>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }

        let matches = {
            let index = self
                .index
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            score_apps(&index.apps, query, limit, &self.config)
        };

        if !matches.is_empty() {
            return Ok(build_items(matches, &self.icons));
        }

        self.refresh_if_stale();

        Ok(build_items(matches, &self.icons))
    }

    fn refresh_if_stale(&self) {
        let mut index = lock_or_recover(&self.index);
        if index.refreshing || index.scanned_at.elapsed() < APP_INDEX_REFRESH_COOLDOWN {
            return;
        }

        index.refreshing = true;
        let roots = self.roots.clone();
        let shared_index = Arc::clone(&self.index);
        let spawn_result = thread::Builder::new()
            .name("runx-app-index-refresh".to_owned())
            .spawn(move || {
                let apps = scan_apps(&roots);
                let mut index = lock_or_recover(&shared_index);
                index.apps = apps;
                index.scanned_at = Instant::now();
                index.refreshing = false;
            });

        if spawn_result.is_err() {
            index.refreshing = false;
        }
    }
}

fn lock_or_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

fn scan_apps(roots: &[AppScanRoot]) -> Vec<AppRecord> {
    let mut seen = HashSet::with_capacity(512);
    let mut apps = Vec::with_capacity(512);

    for root in roots {
        for entry in WalkDir::new(&root.path)
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

            if is_nested_app_bundle(path, root) {
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

fn is_nested_app_bundle(path: &Path, root: &AppScanRoot) -> bool {
    if root.allow_nested_apps && path.starts_with(&root.path) {
        return false;
    }

    path.parent().is_some_and(|parent| {
        parent
            .ancestors()
            .any(|ancestor| ancestor.extension().and_then(|value| value.to_str()) == Some("app"))
    })
}

fn score_apps(
    apps: &[AppRecord],
    query: &str,
    limit: usize,
    config: &AppsProviderConfig,
) -> Vec<(i64, AppRecord)> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let query_lower = query.to_ascii_lowercase();

    let mut matches = Vec::new();
    for app in apps {
        let score = app_match_score(app, &query_lower, config);
        if score <= 0 {
            continue;
        }

        matches.push((score, app.clone()));
    }

    matches.sort_by_key(|item| std::cmp::Reverse(item.0));
    matches.truncate(limit);
    matches
}

fn app_match_score(app: &AppRecord, query_lower: &str, config: &AppsProviderConfig) -> i64 {
    let mut score = fuzzy_score(&app.name, query_lower) + app.score_adjustment;
    let app_name = app.name.to_ascii_lowercase();

    if app_name == query_lower {
        score += config.exact_name_boost;
    } else if app_name.starts_with(query_lower) {
        score += config.prefix_name_boost;
    }

    score
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
            compact: false,
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
    let path_text = path.to_string_lossy();
    let info_path = path.join("Contents/Info.plist");

    let is_hidden = is_background_or_agent(&info_path).unwrap_or(false);
    let mut adjustment = if is_hidden { -180 } else { 0 };
    adjustment += path_penalty(&path_text);
    adjustment
}

/// Checks LSUIElement, NSUIElement, and LSBackgroundOnly in the plist.
/// Performs a cheap byte scan first — only does a full parse for the ~20%
/// of apps whose plist actually contains one of these keys.
fn is_background_or_agent(info_path: &Path) -> Option<bool> {
    let raw = std::fs::read(info_path).ok()?;

    let has_relevant_key = memchr::memmem::find(&raw, b"LSUIElement").is_some()
        || memchr::memmem::find(&raw, b"LSBackgroundOnly").is_some();

    if !has_relevant_key {
        return Some(false);
    }

    let value = Value::from_reader(std::io::Cursor::new(&raw)).ok()?;
    let dict = value.as_dictionary()?;

    let is_agent = dict
        .get("LSUIElement")
        .or_else(|| dict.get("NSUIElement"))
        .is_some_and(plist_truthy);
    if is_agent {
        return Some(true);
    }

    let is_background = dict.get("LSBackgroundOnly").is_some_and(plist_truthy);
    Some(is_background)
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

#[cfg(test)]
mod tests {
    use super::{AppRecord, AppScanRoot, app_match_score, is_nested_app_bundle, scan_apps};
    use crate::config::AppsProviderConfig;
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    fn standard_root(path: &str) -> AppScanRoot {
        AppScanRoot {
            path: PathBuf::from(path),
            allow_nested_apps: false,
        }
    }

    fn additional_root(path: &Path) -> AppScanRoot {
        AppScanRoot {
            path: path.to_path_buf(),
            allow_nested_apps: true,
        }
    }

    #[test]
    fn app_name_boosts_are_configurable() {
        let app = AppRecord {
            name: "Runx".to_owned(),
            path: "/Applications/Runx.app".to_owned(),
            score_adjustment: 0,
        };
        let config = AppsProviderConfig {
            exact_name_boost: 17,
            prefix_name_boost: 5,
            additional_directories: Vec::new(),
        };
        let without_boosts = AppsProviderConfig {
            exact_name_boost: 0,
            prefix_name_boost: 0,
            additional_directories: Vec::new(),
        };

        assert_eq!(
            app_match_score(&app, "runx", &config) - app_match_score(&app, "runx", &without_boosts),
            17
        );
        assert_eq!(
            app_match_score(&app, "ru", &config) - app_match_score(&app, "ru", &without_boosts),
            5
        );
    }

    #[test]
    fn nested_wrapped_app_bundles_are_not_indexed_as_apps() {
        assert!(!is_nested_app_bundle(
            Path::new("/Applications/Outer.app"),
            &standard_root("/Applications")
        ));
        assert!(is_nested_app_bundle(
            Path::new("/Applications/Outer.app/Wrapper/Inner.app"),
            &standard_root("/Applications")
        ));
    }

    #[test]
    fn additional_roots_can_index_nested_apps_under_that_root() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let explicit_root = tmp.path().join("Outer.app/Contents/Applications");
        let nested_app = explicit_root.join("Runx Settings.app");
        fs::create_dir_all(&nested_app).expect("create nested app");

        let apps = scan_apps(&[additional_root(&explicit_root)]);

        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].name, "Runx Settings");
        assert_eq!(apps[0].path, nested_app.to_string_lossy());
    }

    #[test]
    fn overlapping_roots_do_not_duplicate_apps() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let explicit_root = tmp.path().join("Applications");
        let app = explicit_root.join("Runx.app");
        fs::create_dir_all(&app).expect("create app");

        let apps = scan_apps(&[
            additional_root(&explicit_root),
            additional_root(&explicit_root),
        ]);

        assert_eq!(apps.len(), 1);
    }
}
