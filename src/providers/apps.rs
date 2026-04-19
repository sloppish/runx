use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};
use directories::BaseDirs;
use walkdir::{DirEntry, WalkDir};

use crate::{
    icons::IconCache,
    scoring::fuzzy_score,
    types::{Action, SearchItem},
};

pub struct AppProvider {
    apps: Vec<AppRecord>,
    icons: Arc<IconCache>,
}

#[derive(Clone)]
struct AppRecord {
    name: String,
    path: String,
}

impl AppProvider {
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
                apps.push(AppRecord {
                    name,
                    path: normalized,
                });
            }
        }

        apps.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(Self { apps, icons })
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchItem>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }

        let mut matches = Vec::new();
        for app in &self.apps {
            let score = fuzzy_score(&app.name, query);
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

#[allow(dead_code)]
fn display_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Application")
        .to_owned()
}
