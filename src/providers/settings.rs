use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use plist::Value;

use crate::{
    scoring::fuzzy_score,
    types::{Action, SearchItem},
};

pub struct SettingsProvider {
    items: Vec<SettingRecord>,
}

#[derive(Clone)]
struct SettingRecord {
    id: String,
    title: String,
    subtitle: String,
}

#[derive(Default)]
struct BundleMetadata {
    name: String,
}

impl SettingsProvider {
    pub fn new() -> Result<Self> {
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
            });
        }

        Ok(Self { items })
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchItem>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }

        let mut items = Vec::new();
        for setting in &self.items {
            let score = fuzzy_score(&setting.title, query) + (fuzzy_score(&setting.id, query) / 3);
            if score <= 0 {
                continue;
            }

            let url = format!("x-apple.systempreferences:{}", setting.id);
            items.push(SearchItem {
                id: format!("settings:{}", setting.id),
                provider: "settings".to_owned(),
                badge: "SET".to_owned(),
                title: setting.title.clone(),
                subtitle: setting.subtitle.clone(),
                raw_score: score,
                action: Action::OpenSettings {
                    url,
                    title: setting.title.clone(),
                },
            });
        }

        items.sort_by(|left, right| right.raw_score.cmp(&left.raw_score));
        items.truncate(limit);
        Ok(items)
    }
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

            bundles.insert(identifier, BundleMetadata { name });
        }
    }

    Ok(bundles)
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
