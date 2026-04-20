//! Provider for System Settings panes and extensions.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};
use plist::Value;

use crate::{
    icons::IconCache,
    scoring::fuzzy_score,
    types::{Action, SearchItem},
};

/// Searches a locally indexed list of System Settings items.
pub struct SettingsProvider {
    items: Vec<SettingRecord>,
    icons: Arc<IconCache>,
}

#[derive(Clone)]
struct SettingRecord {
    id: String,
    title: String,
    subtitle: String,
    bundle_path: Option<String>,
}

#[derive(Default)]
struct BundleMetadata {
    name: String,
    bundle_path: String,
}

impl SettingsProvider {
    /// Builds the local settings index from sidebar metadata and bundle scans.
    pub fn new(icons: Arc<IconCache>) -> Result<Self> {
        let bundle_map = scan_bundle_metadata()?;
        let ordered_ids = read_sidebar_ids()?;

        let mut seen = HashSet::new();
        let mut items = Vec::new();
        for raw_id in ordered_ids {
            if !seen.insert(raw_id.clone()) {
                continue;
            }
            items.push(make_record(&raw_id, &bundle_map));
        }

        for (id, metadata) in bundle_map {
            if seen.contains(&id) {
                continue;
            }
            items.push(SettingRecord {
                id,
                title: metadata.name,
                subtitle: "System Settings".to_owned(),
                bundle_path: Some(metadata.bundle_path),
            });
        }

        Ok(Self { items, icons })
    }

    /// Returns fuzzy matches for System Settings items.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchItem>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }

        let mut items = Vec::new();
        for setting in &self.items {
            let score = setting_score(setting, query);
            if score <= 0 {
                continue;
            }

            items.push((score, setting.clone()));
        }

        items.sort_by(|left, right| right.0.cmp(&left.0));
        items.truncate(limit);
        Ok(items
            .into_iter()
            .map(|(score, setting)| {
                let url = format!("x-apple.systempreferences:{}", setting.id);
                SearchItem {
                    id: format!("settings:{}", setting.id),
                    provider: "settings".to_owned(),
                    badge: "SET".to_owned(),
                    icon: setting
                        .bundle_path
                        .as_deref()
                        .and_then(|path| self.icons.icon_for_bundle(path))
                        .or_else(|| self.icons.system_settings_icon()),
                    title: setting.title.clone(),
                    subtitle: setting.subtitle.clone(),
                    raw_score: score,
                    action: Action::OpenSettings {
                        url,
                        title: setting.title,
                    },
                }
            })
            .collect())
    }
}

fn setting_score(setting: &SettingRecord, query: &str) -> i64 {
    let mut score = fuzzy_score(&setting.title, query) + (fuzzy_score(&setting.id, query) / 3);
    let title_lower = setting.title.to_ascii_lowercase();
    let query_lower = query.to_ascii_lowercase();

    if title_lower == query_lower {
        score += 260;
    } else if title_lower.starts_with(&query_lower) {
        score += 180;
    } else if title_lower.contains(&query_lower) {
        score += 90;
    }

    score -= setting.title.matches('›').count() as i64 * 40;
    score
}

fn scan_bundle_metadata() -> Result<HashMap<String, BundleMetadata>> {
    let mut bundles = HashMap::new();

    let roots = [
        PathBuf::from("/System/Library/PreferencePanes"),
        PathBuf::from("/System/Library/ExtensionKit/Extensions"),
    ];

    for root in roots {
        if !root.exists() {
            continue;
        }
        for entry in
            fs::read_dir(&root).with_context(|| format!("failed to read {}", root.display()))?
        {
            let path = entry?.path();
            let info_path = path.join("Contents/Info.plist");
            if !info_path.exists() {
                continue;
            }

            let plist = Value::from_file(&info_path)
                .with_context(|| format!("failed to parse {}", info_path.display()))?;
            let Some(dict) = plist.as_dictionary() else {
                continue;
            };

            if !is_settings_bundle(dict, &path) {
                continue;
            }

            let Some(identifier) = dict
                .get("CFBundleIdentifier")
                .and_then(Value::as_string)
                .map(str::to_owned)
            else {
                continue;
            };

            let name = dict
                .get("CFBundleDisplayName")
                .or_else(|| dict.get("CFBundleName"))
                .and_then(Value::as_string)
                .map(str::to_owned)
                .unwrap_or_else(|| prettify_identifier(&identifier));

            bundles.insert(
                identifier,
                BundleMetadata {
                    name,
                    bundle_path: path.to_string_lossy().to_string(),
                },
            );
        }
    }

    Ok(bundles)
}

fn is_settings_bundle(dict: &plist::Dictionary, path: &Path) -> bool {
    match path.extension().and_then(|value| value.to_str()) {
        Some("prefPane") => true,
        Some("appex") => {
            dict.get("EXAppExtensionAttributes")
                .and_then(Value::as_dictionary)
                .and_then(|attributes| {
                    attributes
                        .get("EXExtensionPointIdentifier")
                        .and_then(Value::as_string)
                })
                == Some("com.apple.Settings.extension.ui")
        }
        _ => false,
    }
}

fn read_sidebar_ids() -> Result<Vec<String>> {
    let candidates = [
        Path::new("/System/Applications/System Settings.app/Contents/Resources/Sidebar124.plist"),
        Path::new("/System/Applications/System Settings.app/Contents/Resources/Sidebar.plist"),
    ];

    for path in candidates {
        if !path.exists() {
            continue;
        }

        let value = Value::from_file(path)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        let mut ids = Vec::new();
        collect_ids(&value, &mut ids);
        if !ids.is_empty() {
            return Ok(ids);
        }
    }

    Ok(Vec::new())
}

fn collect_ids(value: &Value, ids: &mut Vec<String>) {
    match value {
        Value::String(text) => {
            if text.starts_with("com.apple.") {
                ids.push(text.to_owned());
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_ids(value, ids);
            }
        }
        Value::Dictionary(map) => {
            for key in ["ordered_content", "sortable_content", "content"] {
                if let Some(value) = map.get(key) {
                    collect_ids(value, ids);
                }
            }
        }
        _ => {}
    }
}

fn make_record(raw_id: &str, bundles: &HashMap<String, BundleMetadata>) -> SettingRecord {
    let mut parts = raw_id.splitn(2, ':');
    let base_id = parts.next().unwrap_or(raw_id);
    let suffix = parts.next();

    let base_name = bundles
        .get(base_id)
        .map(|bundle| bundle.name.clone())
        .unwrap_or_else(|| prettify_identifier(base_id));

    let title = match suffix {
        Some(fragment) => format!("{} › {}", base_name, prettify_identifier(fragment)),
        None => base_name.clone(),
    };

    SettingRecord {
        id: raw_id.to_owned(),
        title,
        subtitle: "System Settings".to_owned(),
        bundle_path: bundles
            .get(base_id)
            .map(|bundle| bundle.bundle_path.clone()),
    }
}

fn prettify_identifier(raw: &str) -> String {
    raw.split(['.', '-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let lower = part.to_ascii_lowercase();
            match lower.as_str() {
                "icloud" => "iCloud".to_owned(),
                "wifi" => "Wi-Fi".to_owned(),
                "id" => "ID".to_owned(),
                other => {
                    let mut chars = other.chars();
                    match chars.next() {
                        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                        None => String::new(),
                    }
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
